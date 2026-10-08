//! v6 devices own the HTTP endpoint. The server only discovers and calls it.

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, ensure};
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION, DeviceBootImage, DeviceBootJob, LoaderAnnouncement,
    LoaderDeviceStatus, LoaderStatusPhase,
};
use reqwest::{Client, Response};
use sha2::{Digest, Sha256};

use crate::{
    api::router::board_id_for_mac,
    config::BootConfig,
    ota::{Decision, Job, Phase},
    session::SessionBootCommand,
    state::{AppState, BoardLeaseState},
};

const V5_EFI_ENTRY_SYMBOL: &str = "__x86_64_efi_pe_entry";

pub async fn reconcile(
    state: AppState,
    announcement: LoaderAnnouncement,
    peer: SocketAddr,
) -> anyhow::Result<()> {
    ensure!(
        matches!(
            announcement.protocol_version,
            DEVICE_PROTOCOL_VERSION | httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION
        ),
        "unsupported device protocol"
    );
    ensure!(announcement.http_port > 0, "missing device HTTP port");
    let device_addr = SocketAddr::new(peer.ip(), announcement.http_port);
    let endpoint = format!("http://{device_addr}");
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let observed: LoaderDeviceStatus = client
        .get(format!("{endpoint}/api/v1/status"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if state
        .loader_registry
        .accept_announcement(&announcement, &observed, peer.ip().to_string())
        .await
        .map_err(|error| anyhow::anyhow!("invalid device announcement: {error:?}"))?
    {
        return Ok(());
    }
    let board_id = {
        let boards = state.boards.read().await;
        let Some(board_id) = board_id_for_mac(&boards, announcement.mac_address) else {
            return Ok(());
        };
        let Some(board) = boards.get(&board_id) else {
            return Ok(());
        };
        if board.disabled || !matches!(board.boot, BootConfig::UefiHttp(_)) {
            return Ok(());
        }
        board_id
    };
    let Some(runtime) = state.board_runtime_status(&board_id).await else {
        return Ok(());
    };
    let idle = runtime.active_session_id.is_none()
        && runtime.lease_state == BoardLeaseState::Idle
        && observed.boot.is_none();
    if let Some(ota) = observed.ota.as_ref() {
        match state
            .ota
            .decide(&board_id, announcement.mac_address, ota, idle)
            .await?
        {
            Decision::Update(job) => {
                state.admin_events.invalidate(&["ota"]);
                if let Err(error) =
                    push_ota_image(&state, &client, &endpoint, &observed, &job).await
                {
                    state
                        .ota
                        .record_delivery_failure(
                            &board_id,
                            announcement.mac_address,
                            &job.update_id,
                            format!("{error:#}"),
                        )
                        .await?;
                    state.admin_events.invalidate(&["ota"]);
                    return Err(error);
                }
                state
                    .ota
                    .report(
                        &board_id,
                        announcement.mac_address,
                        &job.update_id,
                        Phase::Staged,
                        None,
                        None,
                    )
                    .await?;
                state.admin_events.invalidate(&["ota"]);
                return Ok(());
            }
            Decision::Confirm(update_id) => {
                state.admin_events.invalidate(&["ota"]);
                let response = client
                    .post(format!("{endpoint}/api/v1/ota/confirm"))
                    .header("X-Boot-Epoch", &observed.boot_epoch)
                    .header("X-Update-Source", "server")
                    .json(&serde_json::json!({"update_id": update_id}))
                    .send()
                    .await?;
                require_status(response, 200).await?;
                let confirmed: LoaderDeviceStatus = client
                    .get(format!("{endpoint}/api/v1/status"))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                let active = confirmed
                    .ota
                    .as_ref()
                    .context("missing confirmed OTA state")?;
                ensure!(
                    active.active_sha256 == job_digest(&state, &board_id).await?,
                    "confirmed digest mismatch"
                );
                state
                    .ota
                    .report(
                        &board_id,
                        announcement.mac_address,
                        &update_id,
                        Phase::Succeeded,
                        None,
                        Some(&active.active_sha256),
                    )
                    .await?;
                state.admin_events.invalidate(&["ota"]);
                return Ok(());
            }
            Decision::Wait => return Ok(()),
            Decision::Idle => {}
        }
        if ota.trial {
            return Ok(());
        }
    } else {
        log::warn!(
            "loader {} at {} did not report OTA state; skipping upgrades",
            announcement.mac_address,
            peer.ip()
        );
    }
    if runtime.lease_state != BoardLeaseState::Using {
        return Ok(());
    }
    let Some(session_id) = runtime.active_session_id else {
        return Ok(());
    };
    let Some(session) = state.session_state(&session_id).await else {
        return Ok(());
    };
    let Some(command) = session.boot_command().await else {
        return Ok(());
    };
    if command.arch != observed.arch {
        return Ok(());
    }
    if observed.protocol_version != DEVICE_PROTOCOL_VERSION {
        session.serial_runtime.fail(
            "automatic serial binding requires axloader protocol v6; upgrade the loader".into(),
        );
        return Ok(());
    }
    if !session.is_serial_connected() || session.is_stop_requested() || session.is_releasing() {
        return Ok(());
    }
    if !session.serial_runtime.accepts_epoch(&observed.boot_epoch) {
        return Ok(());
    }
    let session_id = session.snapshot().await.id;
    let mut generation = session.subscribe_boot_generation();
    let mut shutdown = session.subscribe_shutdown();
    let operation = async {
        let serial = observed
            .serial
            .as_ref()
            .context("missing v6 serial status")?;
        let runtime = session.serial_runtime.snapshot();
        if let Some(binding) = &serial.binding
            && (runtime.binding_id.as_deref() != Some(binding.binding_id.as_str())
                || runtime.phase == crate::serial::runtime::SerialRuntimePhase::Recovering)
        {
            let response = client
                .delete(format!(
                    "{endpoint}/api/v1/serial/bindings/{}",
                    binding.binding_id
                ))
                .header("X-Boot-Epoch", &observed.boot_epoch)
                .send()
                .await?;
            require_status(response, 200).await?;
        }
        let binding = session.serial_runtime.bind(observed.clone()).await?;
        ensure!(
            !session.is_stop_requested() && !session.is_releasing(),
            "session stopped before serial continue"
        );
        let mut grant = SerialGrant::new(
            &client,
            &endpoint,
            &observed.boot_epoch,
            &binding.binding_id,
        );
        let response = client
            .post(format!("{endpoint}/api/v1/serial/continue"))
            .header("X-Boot-Epoch", &observed.boot_epoch)
            .json(&binding)
            .send()
            .await?;
        require_status(response, 200).await?;
        session.serial_runtime.confirm(&binding.binding_id).await?;
        push_boot(
            &state,
            &client,
            &endpoint,
            &observed,
            &command,
            &session,
            &binding.binding_id,
        )
        .await?;
        grant.armed = false;
        Ok(())
    };
    tokio::select! {
        result = operation => {
            if let Err(error) = &result {
                log::warn!(
                    "axloader serial handoff for session `{}` did not complete: {error:#}; waiting for the next device broadcast",
                    session_id,
                );
            }
            result
        },
        _ = shutdown.wait_for(|s|*s) => Ok(()),
        _ = generation.changed() => Ok(()),
    }
}

// Cancellation can race the HTTP reply after the firmware has accepted continue.
// Revoke the exact epoch/token unless handoff was successfully requested.
struct SerialGrant {
    client: Client,
    endpoint: String,
    epoch: String,
    binding_id: String,
    armed: bool,
}
impl SerialGrant {
    fn new(client: &Client, endpoint: &str, epoch: &str, binding_id: &str) -> Self {
        Self {
            client: client.clone(),
            endpoint: endpoint.into(),
            epoch: epoch.into(),
            binding_id: binding_id.into(),
            armed: true,
        }
    }
}
impl Drop for SerialGrant {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let client = self.client.clone();
        let url = format!(
            "{}/api/v1/serial/bindings/{}",
            self.endpoint, self.binding_id
        );
        let epoch = self.epoch.clone();
        tokio::spawn(async move {
            let _ = client
                .delete(url)
                .header("X-Boot-Epoch", epoch)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await;
        });
    }
}

async fn push_ota_image(
    state: &AppState,
    client: &Client,
    endpoint: &str,
    observed: &LoaderDeviceStatus,
    job: &Job,
) -> anyhow::Result<()> {
    let bytes = tokio::fs::read(state.ota.image_path(&job.image.sha256)?).await?;
    ensure!(
        bytes.len() as u64 == job.image.size && hex_sha256(&bytes) == job.image.sha256,
        "OTA image changed in storage"
    );
    let response = client
        .put(format!("{endpoint}/api/v1/ota/image"))
        .header("X-Boot-Epoch", &observed.boot_epoch)
        .header("X-Image-Sha256", &job.image.sha256)
        .header("X-Update-Source", "server")
        .header("X-Update-Id", &job.update_id)
        .header("Content-Length", bytes.len())
        .body(bytes)
        .send()
        .await?;
    require_status(response, 202).await
}

async fn push_boot(
    state: &AppState,
    client: &Client,
    endpoint: &str,
    observed: &LoaderDeviceStatus,
    command: &SessionBootCommand,
    session: &crate::session::SessionState,
    binding_id: &str,
) -> anyhow::Result<()> {
    let epoch = observed.boot_epoch.as_str();
    let observed_boot_id = observed.boot.as_ref().map(|boot| boot.boot_id.as_str());
    let manifest = v5_boot_manifest(command);

    let base = format!("{endpoint}/api/v1/boot/jobs");
    let create = || {
        client
            .post(&base)
            .header("X-Boot-Epoch", epoch)
            .json(&manifest)
            .send()
    };
    let response = create().await?;
    let response = if response.status().as_u16() == 409
        && let Some(stale_boot_id) = observed_boot_id
        && stale_boot_id != command.boot_id
    {
        drop(response);
        let delete_response = client
            .delete(format!("{base}/{stale_boot_id}"))
            .header("X-Boot-Epoch", epoch)
            .send()
            .await?;
        require_status_in(delete_response, &[204, 404]).await?;
        create().await?
    } else {
        response
    };
    ensure!(
        response.status().as_u16() == 201 || response.status().as_u16() == 200,
        "device refused boot job: {}",
        response.text().await?
    );
    session
        .update_loader_status(epoch.into(), &command.boot_id, LoaderStatusPhase::Accepted)
        .await
        .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
    let session_id = session.snapshot().await.id;
    let prefix = format!("/boot/sessions/{session_id}/");
    let push = PushContext {
        state,
        client,
        base: &base,
        epoch,
        session_id: &session_id,
        boot_id: &command.boot_id,
    };
    push_file(
        &push,
        command
            .kernel_path
            .strip_prefix(&prefix)
            .context("kernel path does not belong to session")?,
        "kernel",
        &manifest.kernel,
    )
    .await?;
    if let (Some(file), Some(expected)) = (&command.initramfs, &manifest.initramfs) {
        push_file(
            &push,
            file.path
                .strip_prefix(&prefix)
                .context("initramfs path does not belong to session")?,
            "initramfs",
            expected,
        )
        .await?;
    }
    session
        .update_loader_status(epoch.into(), &command.boot_id, LoaderStatusPhase::Verified)
        .await
        .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
    let response = client
        .post(format!("{base}/{}/start", command.boot_id))
        .header("X-Boot-Epoch", epoch)
        .header("X-Serial-Binding", binding_id)
        .send()
        .await?;
    require_status(response, 202).await?;
    session
        .update_loader_status(
            epoch.into(),
            &command.boot_id,
            LoaderStatusPhase::ReadyToHandoff,
        )
        .await
        .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
    Ok(())
}

fn v5_boot_manifest(command: &SessionBootCommand) -> DeviceBootJob {
    DeviceBootJob {
        boot_id: command.boot_id.clone(),
        arch: command.arch,
        image_format: command.image_format,
        kernel: DeviceBootImage {
            size: command.kernel_size,
            sha256: command.kernel_sha256.clone(),
        },
        initramfs: command.initramfs.as_ref().map(|file| DeviceBootImage {
            size: file.size,
            sha256: file.sha256.clone(),
        }),
        cmdline: command.cmdline.clone(),
        entry_symbol: Some(V5_EFI_ENTRY_SYMBOL.into()),
    }
}

struct PushContext<'a> {
    state: &'a AppState,
    client: &'a Client,
    base: &'a str,
    epoch: &'a str,
    session_id: &'a str,
    boot_id: &'a str,
}

async fn push_file(
    push: &PushContext<'_>,
    path: &str,
    kind: &str,
    image: &DeviceBootImage,
) -> anyhow::Result<()> {
    let manager = push.state.tftp_manager.read().await.clone();
    let file = manager
        .get_session_file(push.session_id, path)
        .await?
        .context("session file missing")?;
    let bytes = tokio::fs::read(file.disk_path).await?;
    ensure!(
        bytes.len() as u64 == image.size && hex_sha256(&bytes) == image.sha256,
        "published boot file changed on disk"
    );
    let response = push
        .client
        .put(format!("{}/{}/{kind}", push.base, push.boot_id))
        .header("X-Boot-Epoch", push.epoch)
        .header("X-Image-Sha256", &image.sha256)
        .header("Content-Length", bytes.len())
        .body(bytes)
        .send()
        .await?;
    require_status(response, 200).await
}

async fn require_status(response: Response, expected: u16) -> anyhow::Result<()> {
    require_status_in(response, &[expected]).await
}

async fn require_status_in(response: Response, expected: &[u16]) -> anyhow::Result<()> {
    let status = response.status();
    ensure!(
        expected.contains(&status.as_u16()),
        "device HTTP {status}: {}",
        response.text().await?
    );
    Ok(())
}

async fn job_digest(state: &AppState, board_id: &str) -> anyhow::Result<String> {
    Ok(state
        .ota
        .job(board_id)
        .await
        .context("OTA task missing")?
        .image
        .sha256)
}

fn hex_sha256(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::new();
    for byte in Sha256::digest(bytes) {
        write!(result, "{byte:02x}").expect("write to String");
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Json, Router,
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{delete, get, post, put},
    };
    use httpboot_protocol::{
        BootArch, BootFile, DEVICE_PROTOCOL_VERSION, DeviceBootJob, DeviceBootStatus, ImageFormat,
        LoaderAnnouncement, LoaderDeviceStatus, LoaderHardwareInfo, LoaderOtaState, OtaOutcome,
        OtaSource,
    };
    use serde::Deserialize;
    use tokio::{net::TcpListener, sync::Mutex};

    use super::{V5_EFI_ENTRY_SYMBOL, hex_sha256, reconcile, v5_boot_manifest};
    use crate::{
        BoardConfig, BoardNetworkIdentity, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
        PowerManagementConfig, ServerConfig, TftpConfig, UefiBootArch, UefiHttpProfile,
        build_app_state, ota::Phase, session::SessionBootCommand, state::BoardRuntimeState,
        tftp::service::build_tftp_manager,
    };

    #[derive(Clone)]
    struct FakeDevice {
        status: LoaderDeviceStatus,
        accepted_epoch: String,
        expected_image: Vec<u8>,
        update_id: Option<String>,
        put_attempts: usize,
        confirm_attempts: usize,
        allow_confirm: bool,
    }

    type FakeState = Arc<Mutex<FakeDevice>>;

    #[derive(Clone)]
    struct FakeBootDevice {
        status: LoaderDeviceStatus,
        manifest: Option<DeviceBootJob>,
        create_attempts: usize,
        delete_attempts: usize,
        upload_attempts: usize,
        start_attempts: usize,
        delete_returns_not_found: bool,
        reject_create_without_manifest: bool,
        reject_upload: bool,
        kernel: Vec<u8>,
    }

    type FakeBootState = Arc<Mutex<FakeBootDevice>>;

    #[test]
    fn v5_manifest_preserves_optional_boot_payloads_and_uses_efi_entry() {
        for (cmdline, initramfs) in [
            (None, None),
            (Some("console=ttyS0".to_string()), None),
            (
                None,
                Some(BootFile {
                    path: "/boot/sessions/session/initramfs.cpio".into(),
                    size: 123,
                    sha256: "22".repeat(32),
                }),
            ),
            (
                Some("console=ttyS0".to_string()),
                Some(BootFile {
                    path: "/boot/sessions/session/initramfs.cpio".into(),
                    size: 123,
                    sha256: "22".repeat(32),
                }),
            ),
        ] {
            let command = SessionBootCommand {
                boot_id: "boot-1".into(),
                kernel_path: "/boot/sessions/session/kernel.elf".into(),
                kernel_size: 456,
                kernel_sha256: "11".repeat(32),
                arch: BootArch::X86_64,
                image_format: ImageFormat::Elf64,
                entry_symbol: Some("httpboot_entry".into()),
                initramfs: initramfs.clone(),
                cmdline: cmdline.clone(),
            };

            let manifest = v5_boot_manifest(&command);

            assert_eq!(manifest.cmdline, cmdline);
            assert_eq!(manifest.entry_symbol.as_deref(), Some(V5_EFI_ENTRY_SYMBOL));
            assert_eq!(
                manifest.initramfs,
                initramfs.map(|file| httpboot_protocol::DeviceBootImage {
                    size: file.size,
                    sha256: file.sha256,
                })
            );
        }
    }

    async fn get_status(State(device): State<FakeState>) -> Json<LoaderDeviceStatus> {
        Json(device.lock().await.status.clone())
    }

    async fn put_ota(State(device): State<FakeState>, headers: HeaderMap, body: Bytes) -> Response {
        let mut device = device.lock().await;
        device.put_attempts += 1;
        let header = |name: &'static str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        if header("X-Boot-Epoch").as_deref() != Some(device.accepted_epoch.as_str()) {
            return (StatusCode::CONFLICT, "stale boot epoch").into_response();
        }
        let digest = hex_sha256(&body);
        if body.as_ref() != device.expected_image
            || header("X-Image-Sha256").as_deref() != Some(digest.as_str())
            || header("X-Update-Source").as_deref() != Some("server")
        {
            return (StatusCode::BAD_REQUEST, "image mismatch").into_response();
        }
        let Some(update_id) = header("X-Update-Id") else {
            return (StatusCode::BAD_REQUEST, "missing update id").into_response();
        };
        device.update_id = Some(update_id.clone());
        let ota = device.status.ota.as_mut().unwrap();
        ota.pending_update_id = Some(update_id);
        ota.running_sha256 = digest;
        ota.trial = true;
        ota.source = Some(OtaSource::Server);
        StatusCode::ACCEPTED.into_response()
    }

    #[derive(Deserialize)]
    struct ConfirmRequest {
        update_id: String,
    }

    async fn confirm_ota(
        State(device): State<FakeState>,
        headers: HeaderMap,
        Json(request): Json<ConfirmRequest>,
    ) -> Response {
        let mut device = device.lock().await;
        device.confirm_attempts += 1;
        let epoch = headers
            .get("X-Boot-Epoch")
            .and_then(|value| value.to_str().ok());
        if epoch != Some(device.accepted_epoch.as_str())
            || headers
                .get("X-Update-Source")
                .and_then(|value| value.to_str().ok())
                != Some("server")
            || device.update_id.as_deref() != Some(request.update_id.as_str())
        {
            return (StatusCode::CONFLICT, "confirmation mismatch").into_response();
        }
        if device.allow_confirm {
            let ota = device.status.ota.as_mut().unwrap();
            ota.active_sha256.clone_from(&ota.running_sha256);
            ota.pending_update_id = None;
            ota.trial = false;
            ota.last_update_id = Some(request.update_id);
            ota.last_outcome = Some(OtaOutcome::Confirmed);
        }
        StatusCode::OK.into_response()
    }

    async fn get_boot_status(State(device): State<FakeBootState>) -> Json<LoaderDeviceStatus> {
        Json(device.lock().await.status.clone())
    }

    async fn create_boot_job(
        State(device): State<FakeBootState>,
        headers: HeaderMap,
        Json(manifest): Json<DeviceBootJob>,
    ) -> Response {
        let mut device = device.lock().await;
        device.create_attempts += 1;
        if headers
            .get("X-Boot-Epoch")
            .and_then(|value| value.to_str().ok())
            != Some(device.status.boot_epoch.as_str())
        {
            return (StatusCode::CONFLICT, "stale boot epoch").into_response();
        }
        match &device.manifest {
            None => {
                if device.reject_create_without_manifest {
                    return (StatusCode::CONFLICT, "create still conflicts").into_response();
                }
                device.status.boot = Some(DeviceBootStatus {
                    boot_id: manifest.boot_id.clone(),
                    phase: "accepted".into(),
                    kernel_received: false,
                    initramfs_received: manifest.initramfs.is_none(),
                    last_error: None,
                });
                device.manifest = Some(manifest);
                StatusCode::CREATED.into_response()
            }
            Some(current) if current == &manifest => StatusCode::OK.into_response(),
            Some(_) => (StatusCode::CONFLICT, "boot manifest changed").into_response(),
        }
    }

    async fn delete_boot_job(
        State(device): State<FakeBootState>,
        Path(id): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        let mut device = device.lock().await;
        device.delete_attempts += 1;
        if headers
            .get("X-Boot-Epoch")
            .and_then(|value| value.to_str().ok())
            != Some(device.status.boot_epoch.as_str())
        {
            return (StatusCode::CONFLICT, "stale boot epoch").into_response();
        }
        if device.delete_returns_not_found {
            device.manifest = None;
            device.status.boot = None;
            return StatusCode::NOT_FOUND.into_response();
        }
        if device
            .manifest
            .as_ref()
            .map(|manifest| manifest.boot_id.as_str())
            != Some(id.as_str())
        {
            return StatusCode::NOT_FOUND.into_response();
        }
        device.manifest = None;
        device.status.boot = None;
        StatusCode::NO_CONTENT.into_response()
    }

    async fn put_kernel(
        State(device): State<FakeBootState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let mut device = device.lock().await;
        device.upload_attempts += 1;
        if device.reject_upload {
            device.manifest = None;
            device.status.boot = None;
            return (StatusCode::BAD_REQUEST, "digest mismatch").into_response();
        }
        let digest = hex_sha256(&body);
        if body.as_ref() != device.kernel
            || headers
                .get("X-Image-Sha256")
                .and_then(|value| value.to_str().ok())
                != Some(digest.as_str())
            || headers
                .get("X-Boot-Epoch")
                .and_then(|value| value.to_str().ok())
                != Some(device.status.boot_epoch.as_str())
        {
            return (StatusCode::BAD_REQUEST, "kernel mismatch").into_response();
        }
        StatusCode::OK.into_response()
    }

    async fn start_boot(State(device): State<FakeBootState>, headers: HeaderMap) -> Response {
        let mut device = device.lock().await;
        device.start_attempts += 1;
        if headers
            .get("X-Boot-Epoch")
            .and_then(|value| value.to_str().ok())
            != Some(device.status.boot_epoch.as_str())
        {
            return (StatusCode::CONFLICT, "stale boot epoch").into_response();
        }
        StatusCode::ACCEPTED.into_response()
    }

    fn efi_image() -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(0x80_u32).to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&[0x64, 0x86]);
        bytes[0x98..0x9a].copy_from_slice(&[0x0b, 0x02]);
        bytes[0xdc..0xde].copy_from_slice(&[10, 0]);
        bytes
    }

    async fn test_state(
        root: &std::path::Path,
        mac: httpboot_protocol::MacAddress,
    ) -> crate::AppState {
        let config_path = root.join("ostool-server.toml");
        let mut config = ServerConfig::default_for_path(&config_path);
        config.data_dir = root.join("data");
        config.board_dir = root.join("boards");
        config.dtb_dir = root.join("dtbs");
        config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(root.join("tftp")));
        config.http_boot.root_dir = root.join("http-boot");
        config.virtual_qemu.runtime_dir = root.join("qemu");
        let manager = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();
        let board = BoardConfig {
            id: "board-1".into(),
            board_type: "x86_64-uefi-http".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            boot: BootConfig::UefiHttp(UefiHttpProfile {
                boot_arch: Some(UefiBootArch::X86_64),
            }),
            network_identity: Some(BoardNetworkIdentity { mac_address: mac }),
            notes: None,
            disabled: false,
        };
        state.boards.write().await.insert(board.id.clone(), board);
        state
            .board_runtimes
            .write()
            .await
            .insert("board-1".into(), BoardRuntimeState::default());
        state
    }

    fn announcement(
        mac: httpboot_protocol::MacAddress,
        boot_epoch: &str,
        http_port: u16,
    ) -> LoaderAnnouncement {
        LoaderAnnouncement {
            serial_id: None,
            serial_ready: false,
            protocol_version: DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: BootArch::X86_64,
            loader_version: "fake-v5".into(),
            boot_epoch: boot_epoch.into(),
            http_port,
        }
    }

    #[tokio::test]
    async fn reverse_ota_repushes_after_epoch_change_and_retries_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let state = test_state(dir.path(), mac).await;
        let image_bytes = efi_image();
        let image = state
            .ota
            .put_image(&image_bytes, Some("test".into()))
            .await
            .unwrap();
        let job = state
            .ota
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        let fake = Arc::new(Mutex::new(FakeDevice {
            status: LoaderDeviceStatus {
                serial: None,
                protocol_version: httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION,
                boot_epoch: "epoch-1".into(),
                mac_address: mac,
                current_mac_address: mac,
                arch: BootArch::X86_64,
                loader_version: "fake-v5".into(),
                hardware: LoaderHardwareInfo::default(),
                boot: None,
                ota: Some(LoaderOtaState {
                    active_sha256: "11".repeat(32),
                    running_sha256: "11".repeat(32),
                    pending_update_id: None,
                    trial: false,
                    source: None,
                    last_update_id: None,
                    last_outcome: None,
                }),
            },
            accepted_epoch: "epoch-2".into(),
            expected_image: image_bytes,
            update_id: None,
            put_attempts: 0,
            confirm_attempts: 0,
            allow_confirm: false,
        }));
        let app = Router::new()
            .route("/api/v1/status", get(get_status))
            .route("/api/v1/ota/image", put(put_ota))
            .route("/api/v1/ota/confirm", post(confirm_ota))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let peer = "127.0.0.1:40000".parse().unwrap();

        let old_announcement = |epoch| {
            let mut a = announcement(mac, epoch, port);
            a.protocol_version = httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION;
            a
        };
        let error = reconcile(state.clone(), old_announcement("epoch-1"), peer)
            .await
            .unwrap_err();
        let error = format!("{error:#}");
        assert!(
            error.contains("device HTTP 409 Conflict: stale boot epoch"),
            "{error}"
        );
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            Phase::Downloading
        );
        let failed = state.ota.job("board-1").await.unwrap();
        assert_eq!(failed.delivery_attempts, 1);
        assert_eq!(
            failed.error.as_deref(),
            Some("device HTTP 409 Conflict: stale boot epoch")
        );

        {
            let mut fake = fake.lock().await;
            fake.status.boot_epoch = "epoch-2".into();
        }
        reconcile(state.clone(), old_announcement("epoch-2"), peer)
            .await
            .unwrap();
        assert_eq!(state.ota.job("board-1").await.unwrap().phase, Phase::Staged);
        assert_eq!(
            fake.lock().await.update_id.as_deref(),
            Some(job.update_id.as_str())
        );

        fake.lock().await.status.protocol_version = DEVICE_PROTOCOL_VERSION;
        let error = reconcile(state.clone(), announcement(mac, "epoch-2", port), peer)
            .await
            .unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("confirmed digest mismatch"), "{error}");
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            Phase::Confirming
        );
        fake.lock().await.allow_confirm = true;

        reconcile(state.clone(), announcement(mac, "epoch-2", port), peer)
            .await
            .unwrap();
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            Phase::Succeeded
        );
        let fake = fake.lock().await;
        assert_eq!(fake.put_attempts, 2);
        assert_eq!(fake.confirm_attempts, 2);
        assert_eq!(
            fake.status.ota.as_ref().unwrap().active_sha256,
            image.sha256
        );
        server.abort();
    }

    #[tokio::test]
    async fn boot_push_replaces_stale_job_and_only_retries_create_once() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:02".parse().unwrap();
        let state = test_state(dir.path(), mac).await;
        let created = state
            .create_session_with_board_id(
                "x86_64-uefi-http",
                "board-1",
                &[],
                Some("device-test".into()),
            )
            .await
            .unwrap();
        let kernel = b"fake ELF64 kernel".to_vec();
        let manager = state.tftp_manager.read().await.clone();
        manager
            .put_session_file(&created.id, "kernel.elf", &kernel)
            .await
            .unwrap();
        let session = state.session_state(&created.id).await.unwrap();
        let boot_id = "boot-1";
        session
            .publish_boot_command(SessionBootCommand {
                boot_id: boot_id.into(),
                kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                kernel_size: kernel.len() as u64,
                kernel_sha256: hex_sha256(&kernel),
                arch: BootArch::X86_64,
                image_format: ImageFormat::Elf64,
                entry_symbol: None,
                initramfs: None,
                cmdline: Some("console=ttyS0".into()),
            })
            .await;
        let stale_manifest = DeviceBootJob {
            boot_id: "stale-boot".into(),
            arch: BootArch::X86_64,
            image_format: ImageFormat::Elf64,
            kernel: httpboot_protocol::DeviceBootImage {
                size: 1,
                sha256: "00".repeat(32),
            },
            initramfs: None,
            cmdline: None,
            entry_symbol: None,
        };
        let fake = Arc::new(Mutex::new(FakeBootDevice {
            status: LoaderDeviceStatus {
                serial: None,
                protocol_version: DEVICE_PROTOCOL_VERSION,
                boot_epoch: "boot-epoch".into(),
                mac_address: mac,
                current_mac_address: mac,
                arch: BootArch::X86_64,
                loader_version: "fake-v5".into(),
                hardware: LoaderHardwareInfo::default(),
                boot: Some(DeviceBootStatus {
                    boot_id: stale_manifest.boot_id.clone(),
                    phase: "accepted".into(),
                    kernel_received: false,
                    initramfs_received: true,
                    last_error: None,
                }),
                ota: None,
            },
            manifest: Some(stale_manifest),
            create_attempts: 0,
            delete_attempts: 0,
            upload_attempts: 0,
            start_attempts: 0,
            delete_returns_not_found: false,
            reject_create_without_manifest: false,
            reject_upload: true,
            kernel,
        }));
        let app = Router::new()
            .route("/api/v1/status", get(get_boot_status))
            .route("/api/v1/boot/jobs", post(create_boot_job))
            .route("/api/v1/boot/jobs/{id}", delete(delete_boot_job))
            .route("/api/v1/boot/jobs/{id}/kernel", put(put_kernel))
            .route("/api/v1/boot/jobs/{id}/start", post(start_boot))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let observed = fake.lock().await.status.clone();
        let command = session.boot_command().await.unwrap();
        let client = reqwest::Client::new();
        let endpoint = format!("http://127.0.0.1:{port}");

        let error = super::push_boot(
            &state,
            &client,
            &endpoint,
            &observed,
            &command,
            &session,
            "test-binding",
        )
        .await
        .unwrap_err();
        let error = format!("{error:#}");
        assert!(
            error.contains("device HTTP 400 Bad Request: digest mismatch"),
            "{error}"
        );
        fake.lock().await.reject_upload = false;

        super::push_boot(
            &state,
            &client,
            &endpoint,
            &observed,
            &command,
            &session,
            "test-binding",
        )
        .await
        .unwrap();
        {
            let fake = fake.lock().await;
            assert_eq!(fake.create_attempts, 3);
            assert_eq!(fake.delete_attempts, 1);
            assert_eq!(fake.upload_attempts, 2);
            assert_eq!(fake.start_attempts, 1);
            assert_eq!(fake.manifest.as_ref().unwrap().boot_id, boot_id);
        }

        session
            .publish_boot_command(SessionBootCommand {
                boot_id: "boot-2".into(),
                kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                kernel_size: b"fake ELF64 kernel".len() as u64,
                kernel_sha256: hex_sha256(b"fake ELF64 kernel"),
                arch: BootArch::X86_64,
                image_format: ImageFormat::Elf64,
                entry_symbol: None,
                initramfs: None,
                cmdline: Some("console=ttyS0".into()),
            })
            .await;
        {
            let mut fake = fake.lock().await;
            fake.delete_returns_not_found = true;
            fake.reject_create_without_manifest = true;
        }
        let error = super::push_boot(
            &state,
            &client,
            &endpoint,
            &observed,
            &session.boot_command().await.unwrap(),
            &session,
            "test-binding",
        )
        .await
        .unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("device refused boot job: create still conflicts"));
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 5);
        assert_eq!(fake.delete_attempts, 2);
        assert_eq!(fake.upload_attempts, 2);
        assert_eq!(fake.start_attempts, 1);
        assert!(fake.manifest.is_none());
        server.abort();
    }
}
