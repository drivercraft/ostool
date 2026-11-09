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
    ota::{Decision, DeliveryFailure, Job, Phase},
    session::SessionBootCommand,
    state::{AppState, BoardLeaseState},
};

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
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .read_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(300))
        .build()?;
    let observed = read_status(
        client
            .get(format!("{endpoint}/api/v1/status"))
            .timeout(Duration::from_secs(5))
            .send()
            .await?,
    )
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
    // Observe power changes before OTA persistence can suspend reconciliation.
    // A handoff discovered in this generation must not use a later publication.
    let handoff = if let Some(session_id) = runtime.active_session_id.as_deref() {
        state.session_state(session_id).await.map(|session| {
            let generation = session.subscribe_boot_generation();
            let shutdown = session.subscribe_shutdown();
            (session, generation, shutdown)
        })
    } else {
        None
    };
    let idle = runtime.active_session_id.is_none()
        && runtime.lease_state == BoardLeaseState::Idle
        && observed.boot.is_none();
    let decision = state
        .ota
        .decide(
            &board_id,
            observed.mac_address,
            observed.ota.as_ref(),
            idle,
            &observed.boot_epoch,
        )
        .await?;
    state.admin_events.invalidate(&["ota"]);
    match decision {
        Decision::Update(job) => {
            if let Err(failure) = push_ota_image(&state, &client, &endpoint, &observed, &job).await
            {
                let message = failure.to_string();
                let failure = if matches!(
                    failure,
                    DeliveryFailure::Retryable(_) | DeliveryFailure::Conflict(_)
                ) {
                    match recover_ota_delivery(
                        &state,
                        &client,
                        &endpoint,
                        &announcement,
                        peer,
                        &job,
                    )
                    .await?
                    {
                        DeliveryRecovery::Observed => {
                            state.admin_events.invalidate(&["ota"]);
                            return Ok(());
                        }
                        DeliveryRecovery::AwaitingEpoch => DeliveryFailure::Conflict(format!(
                            "{message}; device rebooted after accepting OTA; waiting for its current announcement"
                        )),
                        DeliveryRecovery::Unknown => failure,
                    }
                } else {
                    failure
                };
                state
                    .ota
                    .record_delivery_failure(
                        &board_id,
                        observed.mac_address,
                        &job.update_id,
                        &observed.boot_epoch,
                        failure,
                    )
                    .await?;
                state.admin_events.invalidate(&["ota"]);
                return Err(anyhow::anyhow!(message));
            }
            state
                .ota
                .report(
                    &board_id,
                    &observed.boot_epoch,
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
                confirmed.protocol_version == observed.protocol_version
                    && confirmed.mac_address == observed.mac_address
                    && confirmed.boot_epoch == observed.boot_epoch,
                "confirmed OTA device identity changed"
            );
            ensure!(
                !active.trial
                    && active.pending_update_id.is_none()
                    && active.last_update_id.as_deref() == Some(update_id.as_str())
                    && active.last_outcome == Some(httpboot_protocol::OtaOutcome::Confirmed)
                    && active.active_sha256 == active.running_sha256
                    && active.active_sha256 == job_digest(&state, &board_id).await?,
                "OTA trial is not confirmed"
            );
            state
                .ota
                .report(
                    &board_id,
                    &observed.boot_epoch,
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
    if runtime.lease_state != BoardLeaseState::Using {
        return Ok(());
    }
    let Some((session, mut generation, mut shutdown)) = handoff else {
        return Ok(());
    };
    // Power changes and shutdown must also cancel a queued handoff.
    let _boot_operation = tokio::select! {
        biased;
        _ = shutdown.wait_for(|s| *s) => return Ok(()),
        _ = generation.changed() => return Ok(()),
        guard = session.boot_operation.lock() => guard,
    };
    let Some(command) = session.boot_command().await else {
        return Ok(());
    };
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
    let operation = async {
        let preflight = async {
            ensure!(
                command.arch == observed.arch,
                "kernel architecture {:?} does not match device {:?}",
                command.arch,
                observed.arch
            );
            let request = BootRequest::new(&command)?;
            request.validate_kernel(&state, &session).await?;
            Ok::<_, anyhow::Error>(request)
        }
        .await;
        let request = match preflight {
            Ok(request) => request,
            Err(error) => {
                let message = format!("{error:#}");
                session
                    .update_loader_status(
                        observed.boot_epoch.clone(),
                        &command.boot_id,
                        LoaderStatusPhase::Failed {
                            code: "boot_preflight_failed".into(),
                            message: message.clone(),
                        },
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("stale boot preflight: {error:?}"))?;
                session.serial_runtime.fail(message);
                return Err(error);
            }
        };
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
            &request,
            &session,
            &binding.binding_id,
        )
        .await?;
        grant.armed = false;
        Ok(())
    };
    tokio::select! {
        biased;
        _ = shutdown.wait_for(|s| *s) => Ok(()),
        _ = generation.changed() => Ok(()),
        result = operation => {
            if let Err(error) = &result
                && session.serial_runtime.snapshot().phase != crate::serial::runtime::SerialRuntimePhase::Failed {
                log::warn!(
                    "axloader serial handoff for session `{}` did not complete: {error:#}; waiting for the next device broadcast",
                    session_id,
                );
            }
            result
        },
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
            if let Ok(response) = client
                .delete(url)
                .header("X-Boot-Epoch", epoch)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
            {
                let _ = response.bytes().await;
            }
        });
    }
}

enum DeliveryRecovery {
    Observed,
    AwaitingEpoch,
    Unknown,
}

async fn recover_ota_delivery(
    state: &AppState,
    client: &Client,
    endpoint: &str,
    announcement: &LoaderAnnouncement,
    peer: SocketAddr,
    job: &Job,
) -> anyhow::Result<DeliveryRecovery> {
    // A reply can be lost after durable staging, including on the last attempt.
    let actual = async {
        client
            .get(format!("{endpoint}/api/v1/status"))
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json::<LoaderDeviceStatus>()
            .await
    }
    .await;
    let Ok(actual) = actual else {
        return Ok(DeliveryRecovery::Unknown);
    };
    if actual.mac_address != job.mac_address {
        return Ok(DeliveryRecovery::Unknown);
    }
    let accepted = actual.ota.as_ref().is_some_and(|ota| {
        // Firmware may omit source before the pending slot actually starts.
        ota.pending_update_id.as_deref() == Some(&job.update_id)
            && (!ota.trial
                || (ota.source == Some(httpboot_protocol::OtaSource::Server)
                    && ota.running_sha256 == job.image.sha256))
    });
    if actual.boot_epoch != announcement.boot_epoch {
        // Do not authenticate a new epoch using a stale announcement, or make
        // an already accepted trial terminal before its next broadcast arrives.
        return Ok(
            if accepted
                && matches!(
                    actual.protocol_version,
                    DEVICE_PROTOCOL_VERSION | httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION
                )
            {
                DeliveryRecovery::AwaitingEpoch
            } else {
                DeliveryRecovery::Unknown
            },
        );
    }
    if !matches!(
        state
            .loader_registry
            .accept_announcement(announcement, &actual, peer.ip().to_string(),)
            .await,
        Ok(false)
    ) {
        return Ok(DeliveryRecovery::Unknown);
    }
    let Some(current) = state.ota.job(&job.board_id).await else {
        return Ok(DeliveryRecovery::Observed);
    };
    if current.update_id != job.update_id || current.phase.is_terminal() {
        return Ok(DeliveryRecovery::Observed);
    }
    let idle = state
        .board_runtime_status(&job.board_id)
        .await
        .is_some_and(|runtime| {
            runtime.active_session_id.is_none()
                && runtime.lease_state == BoardLeaseState::Idle
                && actual.boot.is_none()
        });
    let _ = state
        .ota
        .decide(
            &job.board_id,
            actual.mac_address,
            actual.ota.as_ref(),
            idle,
            &actual.boot_epoch,
        )
        .await?;
    Ok(if accepted {
        DeliveryRecovery::Observed
    } else {
        DeliveryRecovery::Unknown
    })
}

async fn push_ota_image(
    state: &AppState,
    client: &Client,
    endpoint: &str,
    observed: &LoaderDeviceStatus,
    job: &Job,
) -> Result<(), DeliveryFailure> {
    let bytes = async {
        let bytes = state.ota.image_bytes(&job.image.sha256).await?;
        ensure!(
            bytes.len() as u64 == job.image.size && hex_sha256(&bytes) == job.image.sha256,
            "OTA image changed in storage"
        );
        Ok::<_, anyhow::Error>(bytes)
    }
    .await
    .map_err(|error| DeliveryFailure::Rejected(format!("{error:#}")))?;
    let response = client
        .put(format!("{endpoint}/api/v1/ota/image"))
        .header("X-Boot-Epoch", &observed.boot_epoch)
        .header("X-Image-Sha256", &job.image.sha256)
        .header("X-Update-Source", "server")
        .header("X-Update-Id", &job.update_id)
        .header("Content-Length", bytes.len())
        .body(bytes)
        .send()
        .await
        .map_err(|error| {
            DeliveryFailure::Retryable(format!("OTA image transport failed: {error:#}"))
        })?;
    let status = response.status();
    let body = response.text().await;
    if status.as_u16() == 202 {
        body.map_err(|error| {
            DeliveryFailure::Retryable(format!(
                "device HTTP {status}: response body failed: {error}"
            ))
        })?;
        return Ok(());
    }
    let body = body.unwrap_or_else(|error| format!("response body failed: {error}"));
    let message = format!("device HTTP {status}: {body}");
    Err(match status.as_u16() {
        409 => DeliveryFailure::Conflict(message),
        503 => DeliveryFailure::Unavailable(message),
        429 | 500..=599 => DeliveryFailure::Retryable(message),
        _ => DeliveryFailure::Rejected(message),
    })
}

// Keep the validated manifest paired with its session-owned payload paths.
struct BootRequest<'a> {
    command: &'a SessionBootCommand,
    manifest: DeviceBootJob,
}

impl<'a> BootRequest<'a> {
    async fn validate_kernel(
        &self,
        state: &AppState,
        session: &crate::session::SessionState,
    ) -> anyhow::Result<()> {
        let session_id = session.snapshot().await.id;
        let prefix = format!("/boot/sessions/{session_id}/");
        let path = self
            .command
            .kernel_path
            .strip_prefix(&prefix)
            .context("kernel path does not belong to session")?;
        let manager = state.tftp_manager.read().await.clone();
        let file = manager
            .get_session_file(&session_id, path)
            .await?
            .context("session kernel file missing")?;
        let bytes = tokio::fs::read(file.disk_path).await?;
        ensure!(
            bytes.len() as u64 == self.manifest.kernel.size
                && hex_sha256(&bytes) == self.manifest.kernel.sha256,
            "published boot file changed on disk"
        );
        let actual = crate::boot_image::elf_arch(&bytes)?;
        ensure!(
            actual == self.command.arch,
            "kernel ELF architecture {actual:?} does not match declared {:?}",
            self.command.arch
        );
        Ok(())
    }

    fn new(command: &'a SessionBootCommand) -> anyhow::Result<Self> {
        let entry_symbol = match command.arch {
            httpboot_protocol::BootArch::X86_64 => "__x86_64_efi_pe_entry",
            httpboot_protocol::BootArch::Loongarch64 | httpboot_protocol::BootArch::Riscv64 => {
                "efi_pe_entry"
            }
            arch => anyhow::bail!("axloader EFI boot is unsupported for architecture {arch:?}"),
        };
        Ok(Self {
            command,
            manifest: DeviceBootJob {
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
                entry_symbol: Some(entry_symbol.into()),
            },
        })
    }
}

async fn push_boot(
    state: &AppState,
    client: &Client,
    endpoint: &str,
    observed: &LoaderDeviceStatus,
    request: &BootRequest<'_>,
    session: &crate::session::SessionState,
    binding_id: &str,
) -> anyhow::Result<()> {
    let BootRequest { command, manifest } = request;
    let epoch = observed.boot_epoch.as_str();
    let observed_boot_id = observed.boot.as_ref().map(|boot| boot.boot_id.as_str());

    let base = format!("{endpoint}/api/v1/boot/jobs");
    let create = || {
        client
            .post(&base)
            .header("X-Boot-Epoch", epoch)
            .json(manifest)
            .send()
    };
    let response = create().await?;
    let response = if response.status().as_u16() == 409
        && let Some(stale_boot_id) = observed_boot_id
        && stale_boot_id != command.boot_id
    {
        require_status(response, 409).await?;
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
    let status = response.status();
    let bytes = read_body(response).await?;
    let body = String::from_utf8_lossy(&bytes);
    ensure!(
        status.as_u16() == 201 || status.as_u16() == 200,
        "device refused boot job: {body}"
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

async fn read_body(mut response: Response) -> anyhow::Result<Vec<u8>> {
    ensure!(
        response.content_length().is_none_or(|n| n <= 64 * 1024),
        "device response too large"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            body.len() + chunk.len() <= 64 * 1024,
            "device response too large"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
async fn read_status(response: Response) -> anyhow::Result<LoaderDeviceStatus> {
    Ok(serde_json::from_slice(
        &read_body(response.error_for_status()?).await?,
    )?)
}

async fn require_status_in(response: Response, expected: &[u16]) -> anyhow::Result<()> {
    let status = response.status();
    // The device may wait for the complete response to be acknowledged before
    // handoff. Dropping a body after its headers can instead reset that TCP flow.
    let bytes = read_body(response).await?;
    let body = String::from_utf8_lossy(&bytes);
    ensure!(
        expected.contains(&status.as_u16()),
        "device HTTP {status}: {body}"
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
    use tower::ServiceExt;

    use super::{BootRequest, hex_sha256, reconcile};
    use crate::{
        BoardConfig, BoardNetworkIdentity, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
        PowerManagementConfig, ServerConfig, TftpConfig, UefiHttpProfile, ota::Phase,
        session::SessionBootCommand, state::BoardRuntimeState,
        test_support::build_test_app_state as build_app_state, tftp::service::build_tftp_manager,
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
        upload_response: Option<(StatusCode, &'static str)>,
        stage_before_response: bool,
        pending_only: bool,
        reboot_on_upload: bool,
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
        initramfs: Vec<u8>,
    }

    type FakeBootState = Arc<Mutex<FakeBootDevice>>;

    #[test]
    fn boot_manifest_selects_efi_entry_for_supported_architectures() {
        for (arch, entry_symbol) in [
            (BootArch::X86_64, "__x86_64_efi_pe_entry"),
            (BootArch::Loongarch64, "efi_pe_entry"),
            (BootArch::Riscv64, "efi_pe_entry"),
        ] {
            let command = SessionBootCommand {
                boot_id: "boot-1".into(),
                kernel_path: "/boot/sessions/session/kernel.elf".into(),
                kernel_size: 456,
                kernel_sha256: "11".repeat(32),
                arch,
                image_format: ImageFormat::Elf64,
                entry_symbol: Some("httpboot_entry".into()),
                initramfs: None,
                cmdline: None,
            };
            let manifest = BootRequest::new(&command).unwrap().manifest;
            assert_eq!(manifest.arch, arch);
            assert_eq!(manifest.entry_symbol.as_deref(), Some(entry_symbol));
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
        if let Some((status, body)) = device.upload_response
            && !device.stage_before_response
        {
            return (status, body).into_response();
        }
        device.update_id = Some(update_id.clone());
        let pending_only = device.pending_only;
        let ota = device.status.ota.as_mut().unwrap();
        ota.pending_update_id = Some(update_id);
        if !pending_only {
            ota.running_sha256 = digest;
        }
        ota.trial = !pending_only;
        ota.source = (!pending_only).then_some(OtaSource::Server);
        if device.reboot_on_upload {
            device.status.boot_epoch = "epoch-2".into();
            device.accepted_epoch = "epoch-2".into();
        }
        device.upload_response.map_or_else(
            || StatusCode::ACCEPTED.into_response(),
            |(status, body)| (status, body).into_response(),
        )
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
        } else {
            let ota = device.status.ota.as_mut().unwrap();
            ota.active_sha256.clone_from(&ota.running_sha256);
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

    async fn put_initramfs(State(device): State<FakeBootState>, body: Bytes) -> StatusCode {
        device.lock().await.initramfs = body.to_vec();
        StatusCode::OK
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

        config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(root.join("tftp")));
        config.http_boot.root_dir = root.join("http-boot");
        config.virtual_qemu.runtime_dir = root.join("qemu");
        let manager = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();
        let board = BoardConfig {
            allow_instant_use: false,
            reset_command: None,
            id: "board-1".into(),
            board_type: "x86_64-uefi-http".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            boot: BootConfig::UefiHttp(UefiHttpProfile {
                serial_parameters: None,
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

    async fn spawn_ota_device(
        mac: httpboot_protocol::MacAddress,
        arch: BootArch,
        has_ota: bool,
        image_bytes: &[u8],
    ) -> (FakeState, u16, tokio::task::JoinHandle<()>) {
        let fake = Arc::new(Mutex::new(FakeDevice {
            status: LoaderDeviceStatus {
                serial: None,
                protocol_version: DEVICE_PROTOCOL_VERSION,
                boot_epoch: "epoch-1".into(),
                mac_address: mac,
                current_mac_address: mac,
                arch,
                loader_version: "fake-v5".into(),
                hardware: LoaderHardwareInfo::default(),
                boot: None,
                ota: has_ota.then(|| LoaderOtaState {
                    active_sha256: "11".repeat(32),
                    running_sha256: "11".repeat(32),
                    pending_update_id: None,
                    trial: false,
                    source: None,
                    last_update_id: None,
                    last_outcome: None,
                }),
            },
            accepted_epoch: "epoch-1".into(),
            expected_image: image_bytes.to_vec(),
            update_id: None,
            put_attempts: 0,
            confirm_attempts: 0,
            allow_confirm: false,
            upload_response: None,
            stage_before_response: false,
            pending_only: false,
            reboot_on_upload: false,
        }));
        let app = Router::new()
            .route("/api/v1/status", get(get_status))
            .route("/api/v1/ota/image", put(put_ota))
            .route("/api/v1/ota/confirm", post(confirm_ota))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (fake, port, server)
    }

    #[tokio::test]
    async fn reverse_ota_repushes_after_epoch_change_and_retries_confirmation() {
        for (arch, has_ota) in [
            (BootArch::X86_64, true),
            (BootArch::Loongarch64, true),
            (BootArch::Riscv64, false),
        ] {
            check_reverse_ota(arch, has_ota).await;
        }
    }

    async fn check_reverse_ota(arch: BootArch, has_ota: bool) {
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
        let (fake, port, server) = spawn_ota_device(mac, arch, has_ota, &image_bytes).await;
        {
            let mut device = fake.lock().await;
            device.accepted_epoch = "epoch-2".into();
            device.status.protocol_version = httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION;
        }
        let peer = "127.0.0.1:40000".parse().unwrap();

        let old_announcement = |epoch| {
            let mut a = announcement(mac, epoch, port);
            a.protocol_version = httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION;
            a.arch = arch;
            a
        };
        if !has_ota {
            reconcile(state.clone(), old_announcement("epoch-1"), peer)
                .await
                .unwrap();
            let waiting = state.ota.job("board-1").await.unwrap();
            assert_eq!(waiting.phase, Phase::Queued);
            assert!(waiting.error.unwrap().contains("OTA"));
            assert_eq!(fake.lock().await.put_attempts, 0);
            server.abort();
            return;
        }
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
        assert_eq!(failed.delivery_attempts, 0);
        assert_eq!(
            failed.error.as_deref(),
            Some("device HTTP 409 Conflict: stale boot epoch")
        );

        reconcile(state.clone(), old_announcement("epoch-1"), peer)
            .await
            .unwrap();
        assert_eq!(fake.lock().await.put_attempts, 1);
        let deadline = failed.next_delivery_at.unwrap();
        tokio::time::sleep((deadline - chrono::Utc::now()).to_std().unwrap_or_default()).await;
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
        let mut current_announcement = announcement(mac, "epoch-2", port);
        current_announcement.arch = arch;
        let error = reconcile(state.clone(), current_announcement.clone(), peer)
            .await
            .unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("OTA trial is not confirmed"), "{error}");
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            Phase::Confirming
        );
        fake.lock().await.allow_confirm = true;

        reconcile(state.clone(), current_announcement, peer)
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
    async fn ota_delivery_results_preserve_device_errors_and_control_retransmission() {
        for (status, body, terminal) in [
            (
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid_request"}"#,
                true,
            ),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                r#"{"error":"image_too_large"}"#,
                true,
            ),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                r#"{"error":"unsupported_image_arch","message":"device refused image"}"#,
                true,
            ),
            (StatusCode::UNPROCESSABLE_ENTITY, "{}", true),
            (
                StatusCode::CONFLICT,
                r#"{"error":"ota_state_conflict"}"#,
                false,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                r#"{"error":"ota_storage_unavailable"}"#,
                false,
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error":"ota_storage_failed"}"#,
                false,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mac = "02:00:00:00:00:01".parse().unwrap();
            let mut state = test_state(dir.path(), mac).await;
            let bytes = efi_image();
            let image = state.ota.put_image(&bytes, None).await.unwrap();
            let assignment = state
                .ota
                .queue("board-1".into(), mac, &image.sha256)
                .await
                .unwrap();
            let (fake, port, server) = spawn_ota_device(mac, BootArch::X86_64, true, &bytes).await;
            fake.lock().await.upload_response = Some((status, body));
            let event = announcement(mac, "epoch-1", port);
            let peer = "127.0.0.1:40000".parse().unwrap();
            let error = reconcile(state.clone(), event.clone(), peer)
                .await
                .unwrap_err();
            assert!(format!("{error:#}").contains(&format!("HTTP {}", status.as_u16())));
            let job = state.ota.job("board-1").await.unwrap();
            assert_eq!(
                job.phase,
                if terminal {
                    Phase::Failed
                } else {
                    Phase::Downloading
                },
                "{status}"
            );
            assert!(job.error.as_deref().unwrap().contains(body), "{status}");
            assert_eq!(
                job.delivery_attempts,
                u8::from(status == StatusCode::INTERNAL_SERVER_ERROR)
            );

            // Reconciliation must not send rejected, unavailable or rate-limited bytes again.
            reconcile(state.clone(), event.clone(), peer).await.unwrap();
            assert_eq!(fake.lock().await.put_attempts, 1, "{status}");
            let response = crate::api::router::build_router(state.clone())
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/api/v1/admin/boards/board-1/loader-updates")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let response = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let visible: crate::ota::Job = serde_json::from_slice(&response).unwrap();
            assert_eq!(visible.update_id, assignment.update_id);
            assert_eq!(
                visible.error.as_deref(),
                Some("设备升级失败，请检查服务器日志")
            );
            if status == StatusCode::SERVICE_UNAVAILABLE {
                state.ota = crate::ota::OtaStore::open(state.database.clone())
                    .await
                    .unwrap();
                reconcile(state.clone(), event.clone(), peer).await.unwrap();
                assert_eq!(fake.lock().await.put_attempts, 1);
                {
                    let mut device = fake.lock().await;
                    device.status.boot_epoch = "epoch-2".into();
                    device.accepted_epoch = "epoch-2".into();
                    device.upload_response = None;
                }
                let mut next = event;
                next.boot_epoch = "epoch-2".into();
                reconcile(state.clone(), next, peer).await.unwrap();
                assert_eq!(fake.lock().await.put_attempts, 2);
                assert_eq!(state.ota.job("board-1").await.unwrap().phase, Phase::Staged);
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn ota_reconciles_an_accepted_pending_image_before_retransmission() {
        check_uncertain_ota_delivery(false).await;
    }

    #[tokio::test]
    async fn ota_recovers_a_rebooted_trial_before_retry_exhaustion() {
        check_uncertain_ota_delivery(true).await;
    }

    async fn check_uncertain_ota_delivery(rebooted: bool) {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let mut state = test_state(dir.path(), mac).await;
        let bytes = efi_image();
        let image = state.ota.put_image(&bytes, None).await.unwrap();
        state
            .ota
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        let (fake, port, server) = spawn_ota_device(mac, BootArch::X86_64, true, &bytes).await;
        {
            let mut device = fake.lock().await;
            device.upload_response = Some((
                StatusCode::INTERNAL_SERVER_ERROR,
                "response uncertain after staging",
            ));
            device.stage_before_response = true;
            device.pending_only = !rebooted;
            device.reboot_on_upload = rebooted;
        }
        let status = fake.lock().await.status.clone();
        state
            .ota
            .decide("board-1", mac, status.ota.as_ref(), true, "epoch-1")
            .await
            .unwrap();
        let job = state.ota.job("board-1").await.unwrap();
        for _ in 0..crate::ota::MAX_DELIVERY_ATTEMPTS - 1 {
            state
                .ota
                .record_delivery_failure(
                    "board-1",
                    mac,
                    &job.update_id,
                    "epoch-1",
                    crate::ota::DeliveryFailure::Retryable("earlier uncertain transport".into()),
                )
                .await
                .unwrap();
        }
        let mut jobs = std::collections::BTreeMap::from([(
            "board-1",
            state.ota.job("board-1").await.unwrap(),
        )]);
        jobs.get_mut("board-1").unwrap().next_delivery_at =
            Some(chrono::Utc::now() - chrono::Duration::seconds(1));
        state.database.save_record("ota-jobs", &jobs).await.unwrap();
        state.ota = crate::ota::OtaStore::open(state.database.clone())
            .await
            .unwrap();
        let event = announcement(mac, "epoch-1", port);
        let peer = "127.0.0.1:40000".parse().unwrap();
        let _ = reconcile(state.clone(), event.clone(), peer).await;
        assert_ne!(state.ota.job("board-1").await.unwrap().phase, Phase::Failed);
        if !rebooted {
            reconcile(state.clone(), event.clone(), peer).await.unwrap();
            assert_eq!(state.ota.job("board-1").await.unwrap().phase, Phase::Staged);
        }
        assert_eq!(fake.lock().await.put_attempts, 1);
        assert_eq!(
            state.ota.job("board-1").await.unwrap().delivery_attempts,
            crate::ota::MAX_DELIVERY_ATTEMPTS - 1
        );
        {
            let mut device = fake.lock().await;
            device.status.ota.as_mut().unwrap().trial = true;
            device.status.ota.as_mut().unwrap().source = Some(OtaSource::Server);
            device.status.ota.as_mut().unwrap().running_sha256 = image.sha256;
            device.allow_confirm = true;
        }
        let mut event = event;
        if rebooted {
            event.boot_epoch = "epoch-2".into();
        }
        reconcile(state.clone(), event, peer).await.unwrap();
        assert_eq!(fake.lock().await.put_attempts, 1);
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            Phase::Succeeded
        );
        server.abort();
    }

    struct NoSerialPorts;

    impl ostool_serial::SerialBackend for NoSerialPorts {
        fn ports(&self) -> ostool_serial::BackendFuture<'_, Vec<ostool_serial::PortLocator>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn open(
            &self,
            _port: ostool_serial::PortLocator,
            _parameters: httpboot_protocol::SerialParameters,
        ) -> ostool_serial::BackendFuture<'_, Box<dyn ostool_serial::SerialIo>> {
            Box::pin(async { Err(std::io::Error::other("unexpected serial open")) })
        }
    }

    #[tokio::test]
    async fn boot_preflight_fails_before_serial_handoff() {
        use futures_util::StreamExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio_tungstenite::tungstenite::Message;

        for arch in [
            BootArch::Aarch64,
            BootArch::Other,
            BootArch::Riscv64,
            BootArch::X86_64,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mac = "02:00:00:00:00:02".parse().unwrap();
            let mut state = test_state(dir.path(), mac).await;
            state.serial_manager = ostool_serial::SerialManager::new(Arc::new(NoSerialPorts));
            let created = state
                .create_session_with_board_id("x86_64-uefi-http", "board-1", &[], None)
                .await
                .unwrap();
            let session = state.session_state(&created.id).await.unwrap();
            let bytes = crate::boot_image::test_elf(243);
            state
                .tftp_manager
                .read()
                .await
                .clone()
                .put_session_file(&created.id, "kernel.elf", &bytes)
                .await
                .unwrap();
            let expected = match arch {
                BootArch::Riscv64 => "does not match device",
                BootArch::X86_64 => "does not match declared",
                _ => "unsupported for architecture",
            };
            session
                .publish_boot_command(SessionBootCommand {
                    boot_id: "unsupported-boot".into(),
                    kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                    kernel_size: bytes.len() as u64,
                    kernel_sha256: hex_sha256(&bytes),
                    arch,
                    image_format: ImageFormat::Elf64,
                    entry_symbol: None,
                    initramfs: None,
                    cmdline: None,
                })
                .await;
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let server_address = listener.local_addr().unwrap();
            let app = crate::api::router::build_router(state.clone());
            let server_task =
                tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let (mut websocket, _) = tokio_tungstenite::connect_async(format!(
                "ws://{server_address}/api/v1/sessions/{}/serial/ws",
                created.id
            ))
            .await
            .unwrap();
            let opened = tokio::time::timeout(std::time::Duration::from_secs(3), websocket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(opened, Message::Text(r#"{"type":"opened"}"#.into()));
            let mut event = announcement(mac, "unsupported-epoch", 0);
            event.arch = if arch == BootArch::Riscv64 {
                BootArch::X86_64
            } else {
                arch
            };
            event.serial_id = Some(format!("{:032x}", 1));
            event.serial_ready = true;
            let serial = httpboot_protocol::LoaderSerialStatus {
                serial_id: event.serial_id.clone().unwrap(),
                ready: true,
                parameters: Some(Default::default()),
                binding: None,
                error: None,
            };
            let observed = LoaderDeviceStatus {
                protocol_version: DEVICE_PROTOCOL_VERSION,
                boot_epoch: "unsupported-epoch".into(),
                mac_address: mac,
                current_mac_address: mac,
                arch: event.arch,
                loader_version: event.loader_version.clone(),
                hardware: LoaderHardwareInfo::default(),
                boot: None,
                ota: None,
                serial: Some(serial),
            };
            let mutations = Arc::new(AtomicUsize::new(0));
            let writes = mutations.clone();
            let app = Router::new()
                .route(
                    "/api/v1/status",
                    get(move || {
                        let observed = observed.clone();
                        async move { Json(observed) }
                    }),
                )
                .fallback(move || {
                    writes.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::OK }
                });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            event.http_port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut discovery = state.serial_manager.subscribe();
            let operation = reconcile(state.clone(), event, "127.0.0.1:12345".parse().unwrap());
            let error = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                tokio::select! {
                    result = operation => result.unwrap_err(),
                    _ = discovery.wait_for(|snapshot| snapshot.pending > 0 || snapshot.candidates > 0 || snapshot.leased > 0) => {
                        panic!("unsupported architecture started serial discovery before rejection")
                    }
                }
            }).await.unwrap();
            assert!(error.to_string().contains(expected), "{error:#}");
            let control = tokio::time::timeout(std::time::Duration::from_secs(3), websocket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let Message::Text(control) = control else {
                panic!("expected error control message");
            };
            let control: serde_json::Value = serde_json::from_str(&control).unwrap();
            assert_eq!(control["type"], "error");
            assert!(
                control["message"].as_str().unwrap().contains(expected),
                "{control}"
            );
            let status = session.loader_status().await.unwrap().status.unwrap();
            assert!(
                matches!(status, httpboot_protocol::LoaderStatusPhase::Failed { ref code, ref message }
                if code == "boot_preflight_failed" && message.contains(expected))
            );
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while !session.is_stop_requested() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let runtime = session.serial_runtime.snapshot();
            assert_eq!(
                runtime.phase,
                crate::serial::runtime::SerialRuntimePhase::Failed
            );
            assert!(runtime.error.unwrap().contains(expected));
            assert!(runtime.port.is_none() && runtime.binding_id.is_none());
            assert_eq!(mutations.load(Ordering::SeqCst), 0);
            assert_eq!(
                *discovery.borrow(),
                ostool_serial::ManagerSnapshot::default()
            );
            server.abort();
            server_task.abort();
        }
    }

    #[tokio::test]
    async fn boot_handoff_cancels_lock_wait_on_power_change() {
        use crate::serial::runtime::SerialRuntimePhase;

        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:03".parse().unwrap();
        let mut state = test_state(dir.path(), mac).await;
        state.serial_manager = ostool_serial::SerialManager::new(Arc::new(NoSerialPorts));
        let created = state
            .create_session_with_board_id("x86_64-uefi-http", "board-1", &[], None)
            .await
            .unwrap();
        let session = state.session_state(&created.id).await.unwrap();
        session.set_serial_connected(true);
        let _serial = session
            .serial_runtime
            .attach(state.clone(), session.clone())
            .await
            .unwrap();
        let publishing = session.boot_operation.lock().await;
        let bytes = crate::boot_image::test_elf(62);
        state
            .tftp_manager
            .read()
            .await
            .clone()
            .put_session_file(&created.id, "kernel.elf", &bytes)
            .await
            .unwrap();
        let mut event = announcement(mac, "before-power-change", 0);
        event.serial_id = Some(format!("{:032x}", 2));
        event.serial_ready = true;
        let observed = LoaderDeviceStatus {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: event.boot_epoch.clone(),
            mac_address: mac,
            current_mac_address: mac,
            arch: event.arch,
            loader_version: event.loader_version.clone(),
            hardware: LoaderHardwareInfo::default(),
            boot: None,
            ota: None,
            serial: Some(httpboot_protocol::LoaderSerialStatus {
                serial_id: event.serial_id.clone().unwrap(),
                ready: true,
                parameters: Some(Default::default()),
                binding: None,
                error: None,
            }),
        };
        let app = Router::new().route(
            "/api/v1/status",
            get(move || {
                let observed = observed.clone();
                async move { Json(observed) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        event.http_port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut operation = Box::pin(reconcile(
            state.clone(),
            event,
            "127.0.0.1:12345".parse().unwrap(),
        ));
        // Drive the real HTTP response through discovery before testing the lock wait.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::select! {
                result = operation.as_mut() => panic!("handoff finished before publication: {result:?}"),
                _ = async {
                    while state.loader_registry.snapshots().await.is_empty() {
                        tokio::task::yield_now().await;
                    }
                } => {}
            }
        }).await.unwrap();
        assert!(futures_util::poll!(operation.as_mut()).is_pending());
        session.change_power(false).await;
        session
            .publish_boot_command(SessionBootCommand {
                boot_id: "after-power-change".into(),
                kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                kernel_size: bytes.len() as u64,
                kernel_sha256: hex_sha256(&bytes),
                arch: BootArch::X86_64,
                image_format: ImageFormat::Elf64,
                entry_symbol: None,
                initramfs: None,
                cmdline: None,
            })
            .await;
        tokio::time::timeout(std::time::Duration::from_secs(3), operation.as_mut())
            .await
            .expect(
                "power change must cancel the stale handoff while publication still holds the lock",
            )
            .unwrap();
        drop(publishing);
        let runtime = session.serial_runtime.snapshot();
        assert_eq!(runtime.phase, SerialRuntimePhase::WaitingDevice);
        assert!(runtime.boot_epoch.is_none() && runtime.error.is_none());
        assert!(!session.is_stop_requested());
        assert_eq!(
            *state.serial_manager.subscribe().borrow(),
            ostool_serial::ManagerSnapshot::default()
        );
        server.abort();
    }

    #[tokio::test]
    async fn successful_status_waits_for_complete_response_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // A device can send its headers before the rest of the response. The
        // start acknowledgement is complete only after every body byte arrives.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (finish_body, body_ready) = tokio::sync::oneshot::channel();
        let delayed_response = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() <= 4096, "unexpectedly large request head");
            }
            stream
                .write_all(
                    b"HTTP/1.1 202 Accepted\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{",
                )
                .await
                .unwrap();
            body_ready.await.unwrap();
            stream.write_all(b"\"ok\":true}").await.unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/start"))
            .send()
            .await
            .unwrap();
        let mut completed_response = Box::pin(super::require_status(response, 202));
        assert!(
            futures_util::poll!(completed_response.as_mut()).is_pending(),
            "a successful status must not return before the full response body"
        );
        finish_body.send(()).unwrap();
        completed_response.await.unwrap();
        delayed_response.await.unwrap();
    }

    #[tokio::test]
    async fn boot_push_transfers_payloads_and_retries_stale_job_once() {
        let initramfs_bytes = b"test initramfs payload";
        for (cmdline, has_initramfs) in [
            (None, false),
            (Some("console=ttyS0"), false),
            (None, true),
            (Some("console=ttyS0"), true),
        ] {
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
            let initramfs_file = has_initramfs.then(|| BootFile {
                path: format!("/boot/sessions/{}/initramfs.cpio", created.id),
                size: initramfs_bytes.len() as u64,
                sha256: hex_sha256(initramfs_bytes),
            });
            if has_initramfs {
                manager
                    .put_session_file(&created.id, "initramfs.cpio", initramfs_bytes)
                    .await
                    .unwrap();
            }
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
                    initramfs: initramfs_file.clone(),
                    cmdline: cmdline.map(str::to_owned),
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
                initramfs: Vec::new(),
            }));
            let app = Router::new()
                .route("/api/v1/status", get(get_boot_status))
                .route("/api/v1/boot/jobs", post(create_boot_job))
                .route("/api/v1/boot/jobs/{id}", delete(delete_boot_job))
                .route("/api/v1/boot/jobs/{id}/kernel", put(put_kernel))
                .route("/api/v1/boot/jobs/{id}/initramfs", put(put_initramfs))
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
                &BootRequest::new(&command).unwrap(),
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
                &BootRequest::new(&command).unwrap(),
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
                let manifest = fake.manifest.as_ref().unwrap();
                assert_eq!(manifest.boot_id, boot_id);
                assert_eq!(manifest.cmdline.as_deref(), cmdline);
                assert_eq!(
                    manifest.initramfs,
                    initramfs_file
                        .as_ref()
                        .map(|file| httpboot_protocol::DeviceBootImage {
                            size: file.size,
                            sha256: file.sha256.clone(),
                        })
                );
                assert_eq!(
                    fake.initramfs.as_slice(),
                    if has_initramfs {
                        initramfs_bytes.as_slice()
                    } else {
                        &[]
                    }
                );
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
                    initramfs: initramfs_file.clone(),
                    cmdline: cmdline.map(str::to_owned),
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
                &BootRequest::new(&session.boot_command().await.unwrap()).unwrap(),
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
}
