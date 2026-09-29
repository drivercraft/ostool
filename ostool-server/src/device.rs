//! v5 devices own the HTTP endpoint. The server only discovers and calls it.

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
    ota::{Decision, Phase},
    session::SessionBootCommand,
    state::{AppState, BoardLeaseState},
};

pub async fn reconcile(
    state: AppState,
    announcement: LoaderAnnouncement,
    peer: SocketAddr,
) -> anyhow::Result<()> {
    ensure!(
        announcement.protocol_version == DEVICE_PROTOCOL_VERSION,
        "unsupported device protocol"
    );
    ensure!(announcement.http_port > 0, "missing device HTTP port");
    let endpoint = format!("http://{}:{}", peer.ip(), announcement.http_port);
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
                require_status(response, 202).await?;
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
        return Ok(());
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
    push_boot(
        &state,
        &client,
        &endpoint,
        &observed.boot_epoch,
        &session_id,
        &command,
        &session,
    )
    .await
}

async fn push_boot(
    state: &AppState,
    client: &Client,
    endpoint: &str,
    epoch: &str,
    session_id: &str,
    command: &SessionBootCommand,
    session: &crate::session::SessionState,
) -> anyhow::Result<()> {
    let manifest = DeviceBootJob {
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
        entry_symbol: command.entry_symbol.clone(),
    };
    let base = format!("{endpoint}/api/v1/boot/jobs");
    let response = client
        .post(&base)
        .header("X-Boot-Epoch", epoch)
        .json(&manifest)
        .send()
        .await?;
    ensure!(
        response.status().as_u16() == 201 || response.status().as_u16() == 200,
        "device refused boot job: {}",
        response.text().await?
    );
    session
        .update_loader_status(epoch.into(), &command.boot_id, LoaderStatusPhase::Accepted)
        .await
        .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
    let prefix = format!("/boot/sessions/{session_id}/");
    let push = PushContext {
        state,
        client,
        base: &base,
        epoch,
        session_id,
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
    let status = response.status();
    ensure!(
        status.as_u16() == expected,
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
