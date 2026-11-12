//! v7 devices own the HTTP endpoint. The server only discovers and calls it.

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, ensure};
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION, DeviceBootImage, DeviceBootJob, DeviceBootStatus,
    DeviceOperationError, LoaderAnnouncement, LoaderDeviceStatus, LoaderStatusPhase,
    decode_device_operation_error, device_protocol_version_header_value,
    validate_device_protocol_version,
};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
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
    let public = state.config.read().await.edition.is_public();
    let connection = if public {
        let board_id = {
            let boards = state.boards.read().await;
            board_id_for_mac(&boards, announcement.mac_address)
                .context("device is not bound to a board")?
        };
        crate::device_security::connection(&state, &board_id).await?
    } else {
        crate::device_security::DeviceConnection {
            endpoint: format!(
                "http://{}",
                SocketAddr::new(peer.ip(), announcement.http_port)
            ),
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(300))
                .build()?,
        }
    };
    reconcile_connected(state, announcement, peer, connection).await
}
/// Poll a provisioned TLS endpoint; unsigned UDP announcements are not needed.
pub async fn reconcile_provisioned(state: AppState, board_id: &str) -> anyhow::Result<()> {
    let connection = crate::device_security::connection(&state, board_id).await?;
    let observed = get_device_status(
        &connection.client,
        &connection.endpoint,
        Duration::from_secs(5),
    )
    .await?;
    let board = state
        .boards
        .read()
        .await
        .get(board_id)
        .cloned()
        .context("board not found")?;
    let mac = board
        .network_identity
        .context("device has no configured identity")?
        .mac_address;
    ensure!(
        observed.mac_address == mac,
        "authenticated device reported another board MAC"
    );
    let announcement = LoaderAnnouncement {
        protocol_version: observed.protocol_version,
        mac_address: mac,
        current_mac_address: observed.current_mac_address,
        arch: observed.arch.clone(),
        loader_version: observed.loader_version.clone(),
        boot_epoch: observed.boot_epoch.clone(),
        http_port: 443,
        serial_id: observed.serial.as_ref().map(|s| s.serial_id.clone()),
        serial_ready: observed.serial.as_ref().is_some_and(|s| s.ready),
    };
    reconcile_connected(state, announcement, "127.0.0.1:0".parse()?, connection).await
}

async fn reconcile_connected(
    state: AppState,
    announcement: LoaderAnnouncement,
    peer: SocketAddr,
    connection: crate::device_security::DeviceConnection,
) -> anyhow::Result<()> {
    require_device_protocol(announcement.protocol_version)?;
    ensure!(announcement.http_port > 0, "missing device HTTP port");
    let endpoint = connection.endpoint;
    let client = connection.client;
    let response = client
        .get(format!("{endpoint}/api/v1/status"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    let peer = response.remote_addr().unwrap_or(peer);
    let observed = parse_device_status(response).await?;
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
            let response = device_request(client.post(format!("{endpoint}/api/v1/ota/confirm")))
                .header("X-Boot-Epoch", &observed.boot_epoch)
                .header("X-Update-Source", "server")
                .json(&serde_json::json!({"update_id": update_id}))
                .send()
                .await?;
            require_status(response, 200).await?;
            let confirmed = get_device_status(&client, &endpoint, Duration::from_secs(5)).await?;
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
    if !session.is_serial_connected() || session.is_stop_requested() || session.is_releasing() {
        return Ok(());
    }
    if !session.serial_runtime.accepts_epoch(&observed.boot_epoch) {
        return Ok(());
    }
    if session.handoff_was_submitted(&command.boot_id).await {
        return Ok(());
    }
    let boot_generation = *generation.borrow();
    if let Some((boot_epoch, boot_id)) = session
        .observed_handoff_for_generation(boot_generation)
        .await
    {
        if command.boot_id == boot_id
            && observed.boot_epoch == boot_epoch
            && let Some(boot) = observed
                .boot
                .as_ref()
                .filter(|boot| boot.boot_id == boot_id && boot.phase == "failed")
        {
            let failure = boot
                .last_error
                .clone()
                .unwrap_or_else(|| DeviceOperationError {
                    code: "boot_failed".into(),
                    message: "device reported a failed boot job without last_error".into(),
                });
            let diagnostic = format!("{}: {}", failure.code, failure.message);
            terminate_boot_failure(
                &state,
                &session,
                &observed.boot_epoch,
                &command.boot_id,
                failure,
            )
            .await?;
            return Err(anyhow::anyhow!("device boot job failed: {diagnostic}"));
        }
        session.serial_runtime.set_warning(Some(format!(
            "boot handoff result unconfirmed for boot_epoch={boot_epoch} boot_id={boot_id}"
        )));
        return Ok(());
    }
    let session_id = session.snapshot().await.id;
    let operation = async {
        let preflight = async {
            let request = BootRequest::new(&command);
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
                session
                    .serial_runtime
                    .fail(format!("boot_preflight_failed: {message}"));
                session.request_stop(crate::session::SessionStopReason::BootFailed);
                state.admin_events.invalidate(&["sessions"]);
                return Err(error);
            }
        };
        let serial = observed
            .serial
            .as_ref()
            .context("missing v7 serial status")?;
        let runtime = session.serial_runtime.snapshot();
        if let Some(binding) = &serial.binding
            && (runtime.binding_id.as_deref() != Some(binding.binding_id.as_str())
                || runtime.phase == crate::serial::runtime::SerialRuntimePhase::Recovering)
        {
            let response = device_request(client.delete(format!(
                "{endpoint}/api/v1/serial/bindings/{}",
                binding.binding_id
            )))
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
        let response = device_request(client.post(format!("{endpoint}/api/v1/serial/continue")))
            .header("X-Boot-Epoch", &observed.boot_epoch)
            .json(&binding)
            .send()
            .await?;
        require_status(response, 200).await?;
        session.serial_runtime.confirm(&binding.binding_id).await?;
        let push = BootPushContext {
            state: &state,
            client: &client,
            endpoint: &endpoint,
            observed: &observed,
            request: &request,
            session: &session,
            binding_id: &binding.binding_id,
        };
        let outcome = push_boot(&push).await?;
        grant.armed = false;
        if outcome == BootPushOutcome::Unconfirmed {
            session
                .mark_handoff_unconfirmed(
                    boot_generation,
                    observed.boot_epoch.clone(),
                    command.boot_id.clone(),
                )
                .await;
        }
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
// Revoke the exact boot epoch/token unless handoff was successfully requested.
struct SerialGrant {
    client: Client,
    endpoint: String,
    boot_epoch: String,
    binding_id: String,
    armed: bool,
}
impl SerialGrant {
    fn new(client: &Client, endpoint: &str, boot_epoch: &str, binding_id: &str) -> Self {
        Self {
            client: client.clone(),
            endpoint: endpoint.into(),
            boot_epoch: boot_epoch.into(),
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
        let boot_epoch = self.boot_epoch.clone();
        tokio::spawn(async move {
            if let Ok(response) = client
                .delete(url)
                .header(
                    httpboot_protocol::DEVICE_PROTOCOL_VERSION_HEADER,
                    device_protocol_version_header_value(),
                )
                .header("X-Boot-Epoch", boot_epoch)
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
    let actual = get_device_status(client, endpoint, Duration::from_secs(5)).await;
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
            if accepted && actual.protocol_version == DEVICE_PROTOCOL_VERSION {
                DeliveryRecovery::AwaitingEpoch
            } else {
                DeliveryRecovery::Unknown
            },
        );
    }
    if !matches!(
        state
            .loader_registry
            .accept_announcement(announcement, &actual, peer.ip().to_string())
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
    let response = device_request(client.put(format!("{endpoint}/api/v1/ota/image")))
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
    if status.as_u16() == 202 {
        let _ = read_bounded_body(response).await;
        return Ok(());
    }
    let failure = decode_device_failure(response).await;
    let message = failure.to_string();
    Err(match status.as_u16() {
        409 => DeliveryFailure::Conflict(message),
        503 => DeliveryFailure::Unavailable(message),
        429 | 500..=599 => DeliveryFailure::Retryable(message),
        _ => DeliveryFailure::Rejected(message),
    })
}

// Keep the manifest paired with its session-owned payload paths.
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
        Ok(())
    }

    fn new(command: &'a SessionBootCommand) -> Self {
        Self {
            command,
            manifest: DeviceBootJob {
                boot_id: command.boot_id.clone(),
                kernel: DeviceBootImage {
                    size: command.kernel_size,
                    sha256: command.kernel_sha256.clone(),
                },
                initramfs: command.initramfs.as_ref().map(|file| DeviceBootImage {
                    size: file.size,
                    sha256: file.sha256.clone(),
                }),
                cmdline: command.cmdline.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootPushOutcome {
    HandedOff,
    Unconfirmed,
}

struct BootPushContext<'a, 'b> {
    state: &'a AppState,
    client: &'a Client,
    endpoint: &'a str,
    observed: &'a LoaderDeviceStatus,
    request: &'a BootRequest<'b>,
    session: &'a crate::session::SessionState,
    binding_id: &'a str,
}

struct StartContext<'a> {
    state: &'a AppState,
    client: &'a Client,
    endpoint: &'a str,
    observed: &'a LoaderDeviceStatus,
    session: &'a crate::session::SessionState,
    binding_id: &'a str,
    boot_id: &'a str,
}

async fn push_boot(push: &BootPushContext<'_, '_>) -> anyhow::Result<BootPushOutcome> {
    let state = push.state;
    let client = push.client;
    let endpoint = push.endpoint;
    let observed = push.observed;
    let session = push.session;
    let binding_id = push.binding_id;
    let BootRequest { command, manifest } = push.request;
    let boot_epoch = observed.boot_epoch.as_str();
    let observed_boot_id = observed.boot.as_ref().map(|boot| boot.boot_id.as_str());

    let base = format!("{endpoint}/api/v1/boot/jobs");
    let create = || {
        device_request(client.post(&base))
            .header("X-Boot-Epoch", boot_epoch)
            .json(manifest)
            .send()
    };
    let response = create().await?;
    let response = if response.status() == StatusCode::CONFLICT {
        let failure = decode_device_failure(response).await;
        let Some(stale_boot_id) = observed_boot_id.filter(|id| *id != command.boot_id) else {
            return Err(anyhow::anyhow!(failure.to_string()));
        };
        let delete_response = device_request(client.delete(format!("{base}/{stale_boot_id}")))
            .header("X-Boot-Epoch", boot_epoch)
            .send()
            .await?;
        require_status_in(delete_response, &[204, 404]).await?;
        create().await?
    } else {
        response
    };
    let status = response.status();
    if !matches!(status.as_u16(), 200 | 201) {
        return Err(
            boot_response_error(state, session, boot_epoch, &command.boot_id, response).await,
        );
    }
    let body = match read_bounded_body(response).await {
        Ok(body) => body,
        Err(error) => {
            let failure = DeviceOperationError {
                code: "invalid_device_response".into(),
                message: format!("device HTTP {status} boot status body read failed: {error:#}"),
            };
            let diagnostic = format!("{}: {}", failure.code, failure.message);
            terminate_boot_failure(state, session, boot_epoch, &command.boot_id, failure).await?;
            return Err(anyhow::anyhow!(diagnostic));
        }
    };
    if body.truncated {
        let failure = DeviceOperationError {
            code: "invalid_device_response".into(),
            message: format!(
                "device HTTP {status} boot status truncated after {MAX_DEVICE_ERROR_BODY_BYTES} bytes; raw: {}",
                String::from_utf8_lossy(&body.bytes)
            ),
        };
        let diagnostic = format!("{}: {}", failure.code, failure.message);
        terminate_boot_failure(state, session, boot_epoch, &command.boot_id, failure).await?;
        return Err(anyhow::anyhow!(diagnostic));
    }
    let boot_status: DeviceBootStatus = match serde_json::from_slice(&body.bytes) {
        Ok(status) => status,
        Err(error) => {
            let failure = DeviceOperationError {
                code: "invalid_device_response".into(),
                message: format!(
                    "device HTTP {status} returned invalid boot status body: {error}; body: {}",
                    String::from_utf8_lossy(&body.bytes)
                ),
            };
            terminate_boot_failure(
                state,
                session,
                boot_epoch,
                &command.boot_id,
                failure.clone(),
            )
            .await?;
            return Err(anyhow::anyhow!("{}: {}", failure.code, failure.message));
        }
    };
    if boot_status.boot_id != command.boot_id {
        let failure = DeviceOperationError {
            code: "invalid_device_response".into(),
            message: format!(
                "device HTTP {status} boot status belongs to {}, expected {}",
                boot_status.boot_id, command.boot_id
            ),
        };
        let diagnostic = format!("{}: {}", failure.code, failure.message);
        terminate_boot_failure(state, session, boot_epoch, &command.boot_id, failure).await?;
        return Err(anyhow::anyhow!(diagnostic));
    }
    if boot_status.phase == "failed" {
        let failure = boot_status
            .last_error
            .clone()
            .unwrap_or(DeviceOperationError {
                code: "boot_failed".into(),
                message: "device reported a failed boot job without last_error".into(),
            });
        let diagnostic = format!("{}: {}", failure.code, failure.message);
        terminate_boot_failure(state, session, boot_epoch, &command.boot_id, failure).await?;
        return Err(anyhow::anyhow!("device boot job failed: {diagnostic}"));
    }
    session
        .update_loader_status(
            boot_epoch.into(),
            &command.boot_id,
            LoaderStatusPhase::Accepted,
        )
        .await
        .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
    let session_id = session.snapshot().await.id;
    let prefix = format!("/boot/sessions/{session_id}/");
    let push = PushContext {
        state,
        client,
        session,
        base: &base,
        boot_epoch,
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
        .update_loader_status(
            boot_epoch.into(),
            &command.boot_id,
            LoaderStatusPhase::Verified,
        )
        .await
        .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
    let start = StartContext {
        state,
        client,
        endpoint,
        observed,
        session,
        binding_id,
        boot_id: &command.boot_id,
    };
    let response = start.send().await;
    finish_start(start, response).await
}

struct PushContext<'a> {
    state: &'a AppState,
    client: &'a Client,
    session: &'a crate::session::SessionState,
    base: &'a str,
    boot_epoch: &'a str,
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
    let response = device_request(
        push.client
            .put(format!("{}/{}/{kind}", push.base, push.boot_id)),
    )
    .header("X-Boot-Epoch", push.boot_epoch)
    .header("X-Image-Sha256", &image.sha256)
    .header("Content-Length", bytes.len())
    .body(bytes)
    .send()
    .await?;
    if response.status() != StatusCode::OK {
        return Err(boot_response_error(
            push.state,
            push.session,
            push.boot_epoch,
            push.boot_id,
            response,
        )
        .await);
    }
    let _ = read_bounded_body(response).await;
    Ok(())
}

async fn require_status(response: Response, expected: u16) -> anyhow::Result<()> {
    require_status_in(response, &[expected]).await
}

async fn get_device_status(
    client: &Client,
    endpoint: &str,
    timeout: Duration,
) -> anyhow::Result<LoaderDeviceStatus> {
    let response = client
        .get(format!("{endpoint}/api/v1/status"))
        .timeout(timeout)
        .send()
        .await?;
    parse_device_status(response).await
}

async fn parse_device_status(response: Response) -> anyhow::Result<LoaderDeviceStatus> {
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_DEVICE_ERROR_BODY_BYTES as u64),
        "invalid_device_response: device response too large"
    );
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            decode_device_failure(response).await.to_string()
        ));
    }
    let body = read_bounded_body(response).await.map_err(|error| {
        anyhow::anyhow!(
            "invalid_device_response: device HTTP {status} status body read failed: {error:#}"
        )
    })?;
    if body.truncated {
        return Err(anyhow::anyhow!(
            "invalid_device_response: device HTTP {status} status body truncated after {MAX_DEVICE_ERROR_BODY_BYTES} bytes; raw: {}",
            String::from_utf8_lossy(&body.bytes)
        ));
    }
    let status: LoaderDeviceStatus = serde_json::from_slice(&body.bytes).map_err(|error| {
        anyhow::anyhow!(
            "invalid_device_response: device HTTP {status} returned invalid status: {error}; body: {}",
            String::from_utf8_lossy(&body.bytes)
        )
    })?;
    validate_device_protocol_version(status.protocol_version)
        .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
    Ok(status)
}

async fn require_status_in(response: Response, expected: &[u16]) -> anyhow::Result<()> {
    let status = response.status();
    if expected.contains(&status.as_u16()) {
        // The device may wait for the complete response to be acknowledged before
        // handoff. Dropping a body after its headers can instead reset that TCP flow.
        let _ = read_bounded_body(response).await;
        return Ok(());
    }
    Err(anyhow::anyhow!(
        decode_device_failure(response).await.to_string()
    ))
}

fn device_request(request: RequestBuilder) -> RequestBuilder {
    request.header(
        httpboot_protocol::DEVICE_PROTOCOL_VERSION_HEADER,
        device_protocol_version_header_value(),
    )
}

fn require_device_protocol(version: u16) -> anyhow::Result<()> {
    validate_device_protocol_version(version)
        .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))
}

#[derive(Debug)]
struct DeviceHttpFailure {
    status: StatusCode,
    error: DeviceOperationError,
}

impl std::fmt::Display for DeviceHttpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "device HTTP {}: {}: {}",
            self.status, self.error.code, self.error.message
        )
    }
}

const MAX_DEVICE_ERROR_BODY_BYTES: usize = 8 * 1024;

struct BoundedBody {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_bounded_body(mut response: Response) -> anyhow::Result<BoundedBody> {
    let mut body = Vec::new();
    let mut truncated = false;
    loop {
        let chunk = match response.chunk().await {
            Ok(chunk) => chunk,
            Err(error) => {
                anyhow::bail!(
                    "body read failed: {error:#}; raw: {}",
                    String::from_utf8_lossy(&body)
                );
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        let remaining = MAX_DEVICE_ERROR_BODY_BYTES.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(BoundedBody {
        bytes: body,
        truncated,
    })
}

async fn decode_device_failure(response: Response) -> DeviceHttpFailure {
    let status = response.status();
    match read_bounded_body(response).await {
        Ok(body) => decode_device_error_body(status, &body.bytes, body.truncated),
        Err(error) => {
            invalid_device_failure(status, format!("response body read failed: {error:#}"))
        }
    }
}

fn decode_device_error_body(status: StatusCode, body: &[u8], truncated: bool) -> DeviceHttpFailure {
    if truncated {
        return invalid_device_failure(
            status,
            format!(
                "body truncated after {MAX_DEVICE_ERROR_BODY_BYTES} bytes; raw: {}",
                String::from_utf8_lossy(body)
            ),
        );
    }
    match decode_device_operation_error(body) {
        Ok(error) if !error.code.is_empty() && !error.message.is_empty() => {
            DeviceHttpFailure { status, error }
        }
        _ => invalid_device_failure(
            status,
            format!("raw body: {}", String::from_utf8_lossy(body)),
        ),
    }
}

fn invalid_device_failure(status: StatusCode, detail: String) -> DeviceHttpFailure {
    DeviceHttpFailure {
        status,
        error: DeviceOperationError {
            code: "invalid_device_response".into(),
            message: format!("device HTTP {status}: {detail}"),
        },
    }
}

async fn boot_response_error(
    state: &AppState,
    session: &crate::session::SessionState,
    boot_epoch: &str,
    boot_id: &str,
    response: Response,
) -> anyhow::Error {
    let status = response.status();
    let failure = decode_device_failure(response).await;
    if matches!(status.as_u16(), 400 | 413 | 422) {
        let _ = terminate_boot_failure(state, session, boot_epoch, boot_id, failure.error.clone())
            .await;
    }
    anyhow::anyhow!(failure.to_string())
}

async fn terminate_boot_failure(
    state: &AppState,
    session: &crate::session::SessionState,
    boot_epoch: &str,
    boot_id: &str,
    failure: DeviceOperationError,
) -> anyhow::Result<()> {
    session
        .update_loader_status(
            boot_epoch.into(),
            boot_id,
            LoaderStatusPhase::Failed {
                code: failure.code.clone(),
                message: failure.message.clone(),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("stale boot failure: {error:?}"))?;
    session
        .serial_runtime
        .fail(format!("{}: {}", failure.code, failure.message));
    session.request_stop(crate::session::SessionStopReason::BootFailed);
    state.admin_events.invalidate(&["sessions"]);
    Ok(())
}

impl StartContext<'_> {
    async fn send(&self) -> reqwest::Result<Response> {
        device_request(self.client.post(format!(
            "{}/api/v1/boot/jobs/{}/start",
            self.endpoint, self.boot_id
        )))
        .header("X-Boot-Epoch", &self.observed.boot_epoch)
        .header("X-Serial-Binding", self.binding_id)
        .send()
        .await
    }

    async fn handed_off(&self) -> anyhow::Result<BootPushOutcome> {
        self.session.serial_runtime.set_warning(None);
        self.session
            .update_loader_status(
                self.observed.boot_epoch.clone(),
                self.boot_id,
                LoaderStatusPhase::ReadyToHandoff,
            )
            .await
            .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
        Ok(BootPushOutcome::HandedOff)
    }

    async fn unconfirmed(&self, diagnostic: String) -> anyhow::Result<BootPushOutcome> {
        let message = format!("boot handoff result unconfirmed: {diagnostic}");
        self.session.serial_runtime.set_warning(Some(message));
        self.session
            .update_loader_status(
                self.observed.boot_epoch.clone(),
                self.boot_id,
                LoaderStatusPhase::Verified,
            )
            .await
            .map_err(|error| anyhow::anyhow!("stale boot session: {error:?}"))?;
        Ok(BootPushOutcome::Unconfirmed)
    }

    async fn terminal(&self, failure: DeviceOperationError) -> anyhow::Error {
        let diagnostic = format!(
            "device boot job failed: {}: {}",
            failure.code, failure.message
        );
        let _ = terminate_boot_failure(
            self.state,
            self.session,
            &self.observed.boot_epoch,
            self.boot_id,
            failure,
        )
        .await;
        anyhow::anyhow!(diagnostic)
    }
}

async fn finish_start(
    start: StartContext<'_>,
    sent: reqwest::Result<Response>,
) -> anyhow::Result<BootPushOutcome> {
    let response = match sent {
        Ok(response) => response,
        Err(error) => {
            return start
                .query_uncertain(format!("start transport failed: {error}"))
                .await;
        }
    };
    match response.status() {
        StatusCode::ACCEPTED => {
            match read_bounded_body(response).await {
                Ok(body) if !body.truncated => start.handed_off().await,
                Ok(body) => {
                    start
                        .query_uncertain(format!(
                            "start response body truncated after {MAX_DEVICE_ERROR_BODY_BYTES} bytes; raw: {}",
                            String::from_utf8_lossy(&body.bytes)
                        ))
                        .await
                }
                Err(error) => {
                    start
                        .query_uncertain(format!("start response body read failed: {error:#}"))
                        .await
                }
            }
        }
        StatusCode::BAD_REQUEST
        | StatusCode::PAYLOAD_TOO_LARGE
        | StatusCode::UNPROCESSABLE_ENTITY => {
            let failure = decode_device_failure(response).await.error;
            Err(start.terminal(failure).await)
        }
        status => {
            let failure = decode_device_failure(response).await;
            start
                .query_uncertain(format!("start returned {status}: {failure}"))
                .await
        }
    }
}

impl StartContext<'_> {
    async fn query_uncertain(&self, diagnostic: String) -> anyhow::Result<BootPushOutcome> {
        let actual =
            match get_device_status(self.client, self.endpoint, Duration::from_secs(5)).await {
                Ok(actual)
                    if actual.protocol_version == self.observed.protocol_version
                        && actual.mac_address == self.observed.mac_address
                        && actual.current_mac_address == self.observed.current_mac_address
                        && actual.boot_epoch == self.observed.boot_epoch =>
                {
                    actual
                }
                Ok(_) => {
                    return self
                        .unconfirmed(format!(
                            "{diagnostic}; device identity or boot_epoch changed"
                        ))
                        .await;
                }
                Err(error) => {
                    return self
                        .unconfirmed(format!("{diagnostic}; status query failed: {error:#}"))
                        .await;
                }
            };
        let Some(boot) = actual
            .boot
            .as_ref()
            .filter(|boot| boot.boot_id == self.boot_id)
        else {
            return self
                .unconfirmed(format!(
                    "{diagnostic}; boot job is not visible in the same boot_epoch"
                ))
                .await;
        };
        if boot.phase == "failed" {
            let failure = boot
                .last_error
                .clone()
                .unwrap_or_else(|| DeviceOperationError {
                    code: "boot_failed".into(),
                    message: "device reported a failed boot job without last_error".into(),
                });
            return Err(self.terminal(failure).await);
        }
        if boot.phase != "ready" {
            return self
                .unconfirmed(format!("{diagnostic}; device status is {}", boot.phase))
                .await;
        }
        let retry = match self.send().await {
            Ok(response) => response,
            Err(error) => {
                return self
                    .unconfirmed(format!("{diagnostic}; same-ID start retry failed: {error}"))
                    .await;
            }
        };
        match retry.status() {
            StatusCode::ACCEPTED => {
                match read_bounded_body(retry).await {
                    Ok(body) if !body.truncated => self.handed_off().await,
                    Ok(body) => {
                        self.unconfirmed(format!(
                            "{diagnostic}; same-ID start retry body truncated after {MAX_DEVICE_ERROR_BODY_BYTES} bytes; raw: {}",
                            String::from_utf8_lossy(&body.bytes)
                        ))
                        .await
                    }
                    Err(error) => {
                        self.unconfirmed(format!(
                            "{diagnostic}; same-ID start retry body read failed: {error:#}"
                        ))
                        .await
                    }
                }
            }
            StatusCode::BAD_REQUEST
            | StatusCode::PAYLOAD_TOO_LARGE
            | StatusCode::UNPROCESSABLE_ENTITY => {
                let failure = decode_device_failure(retry).await.error;
                Err(self.terminal(failure).await)
            }
            status => {
                let failure = decode_device_failure(retry).await;
                self.unconfirmed(format!(
                    "{diagnostic}; same-ID start retry returned {status}: {failure}"
                ))
                .await
            }
        }
    }
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

/// A single bounded owner reconciles each provisioned device. No unauthenticated
/// discovery listener or caller-supplied callback URL is involved.
pub fn start_authenticated_devices(state: AppState) -> tokio::task::JoinHandle<()> {
    use sqlx::Row;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        let mut tasks = tokio::task::JoinSet::new();
        let mut active = std::collections::BTreeMap::<
            String,
            (chrono::DateTime<chrono::Utc>, tokio::task::AbortHandle),
        >::new();
        let mut owners = std::collections::HashMap::new();
        let mut cursor = 0usize;
        let mut retry = std::collections::BTreeMap::<
            String,
            (
                chrono::DateTime<chrono::Utc>,
                tokio::time::Instant,
                Duration,
            ),
        >::new();
        loop {
            tokio::select! {
                _=tick.tick()=>{
                    let rows=sqlx::query("SELECT d.board_id,d.updated_at FROM device_connections d JOIN board_records b ON b.id=d.board_id WHERE COALESCE((b.payload->>'disabled')::boolean,false)=false ORDER BY d.board_id LIMIT 4096").fetch_all(state.database.pool()).await;
                    let rows=match rows {Ok(rows)=>rows,Err(error)=>{
                        log::warn!("device identity lookup failed: {error:#}");
                        for (_,(_,handle)) in std::mem::take(&mut active){handle.abort();}
                        continue;
                    }};
                    let provisioned=rows.into_iter().filter_map(|row| match (row.try_get::<String,_>("board_id"),row.try_get::<chrono::DateTime<chrono::Utc>,_>("updated_at")) {
                        (Ok(id),Ok(revision))=>Some((id,revision)),
                        _=>{log::warn!("invalid stored device authentication metadata");None}
                    }).collect::<std::collections::BTreeMap<_,_>>();
                    let revoked=active.iter().filter(|(id,(revision,_))|provisioned.get(*id)!=Some(revision)).map(|(id,_)|id.clone()).collect::<Vec<_>>();
                    for id in revoked {if let Some((_,handle))=active.remove(&id){handle.abort();}}
                    retry.retain(|id,_|provisioned.contains_key(id));
                    let using=state.board_runtimes.read().await.iter().filter(|(_,runtime)|runtime.active_session_id.is_some()).map(|(id,_)|id.clone()).collect::<std::collections::BTreeSet<_>>();
                    let entries=provisioned.into_iter().collect::<Vec<_>>();
                    let start=cursor;
                    for offset in 0..entries.len() {
                        if active.len()>=16 {break;}
                        let index=(start+offset)%entries.len();
                        let (id,revision)=&entries[index];
                        if active.contains_key(id){continue;}
                        if !using.contains(id) && retry.get(id).is_some_and(|(version,deadline,_)|version==revision && tokio::time::Instant::now()<*deadline){continue;}
                        let board=id.clone();let state=state.clone();
                        let handle=tasks.spawn(async move {match reconcile_provisioned(state,&board).await {Ok(())=>true,Err(error)=>{log::warn!("authenticated device {board}: {error:#}");false}}});
                        owners.insert(handle.id(),id.clone());active.insert(id.clone(),(*revision,handle));
                        cursor=(index+1)%entries.len();
                    }
                },
                result=tasks.join_next_with_id(),if !tasks.is_empty()=>{
                    let (id,success)=match result{Some(Ok((id,success)))=>(Some(id),success),Some(Err(error))=>(Some(error.id()),false),None=>(None,false)};
                    if let Some(id)=id && let Some(board)=owners.remove(&id) && active.get(&board).is_some_and(|(_,handle)|handle.id()==id) && let Some((revision,_))=active.remove(&board){
                        if success {retry.remove(&board);}else{
                            let delay=retry.get(&board).filter(|(version,_,_)|*version==revision).map(|(_,_,delay)|(*delay*2).min(Duration::from_secs(60))).unwrap_or(Duration::from_secs(4));
                            retry.insert(board,(revision,tokio::time::Instant::now()+delay,delay));
                        }
                    }
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{delete, get, post, put},
    };
    use futures_util::StreamExt;
    use httpboot_protocol::{
        BootFile, DEVICE_PROTOCOL_VERSION, DeviceBootJob, DeviceBootStatus, DeviceOperationError,
        LoaderAnnouncement, LoaderDeviceStatus, LoaderHardwareInfo, LoaderStatusPhase,
    };
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, oneshot};

    use super::{
        BootPushContext, BootPushOutcome, BootRequest, StartContext, decode_device_error_body,
        finish_start, hex_sha256, push_boot, require_device_protocol,
    };
    async fn reconcile(
        state: crate::AppState,
        announcement: httpboot_protocol::LoaderAnnouncement,
        peer: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let connection = crate::device_security::DeviceConnection {
            endpoint: format!(
                "http://{}",
                std::net::SocketAddr::new(peer.ip(), announcement.http_port)
            ),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(3))
                .read_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(300))
                .build()?,
        };
        super::reconcile_connected(state, announcement, peer, connection).await
    }
    use crate::{
        BoardConfig, BoardNetworkIdentity, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
        PowerManagementConfig, ServerConfig, TftpConfig, UefiHttpProfile,
        session::SessionBootCommand, state::BoardRuntimeState,
        test_support::build_test_app_state as build_app_state, tftp::service::build_tftp_manager,
    };

    fn command(initramfs: bool, cmdline: bool) -> SessionBootCommand {
        SessionBootCommand {
            boot_id: "boot-1".into(),
            kernel_path: "/boot/sessions/session-1/kernel.elf".into(),
            kernel_size: 12,
            kernel_sha256: "11".repeat(32),
            initramfs: initramfs.then(|| BootFile {
                path: "/boot/sessions/session-1/initramfs.cpio".into(),
                size: 9,
                sha256: "22".repeat(32),
            }),
            cmdline: cmdline.then(|| "console=ttyS0".into()),
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum StartMode {
        Accepted,
        FailOnceThenReady,
        FailOnceReceiving,
        RetryBodyError,
        RetryBodyTruncated,
    }

    struct StartBodyErrorChannels {
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    }

    struct FakeBootDevice {
        status: LoaderDeviceStatus,
        manifest: Option<DeviceBootJob>,
        kernel: Vec<u8>,
        initramfs: Option<Vec<u8>>,
        create_attempts: usize,
        delete_attempts: usize,
        upload_attempts: usize,
        start_attempts: usize,
        create_failed: Option<DeviceOperationError>,
        create_boot_id_override: Option<String>,
        upload_rejection: Option<(StatusCode, DeviceOperationError)>,
        start_mode: StartMode,
        start_body_errors: VecDeque<StartBodyErrorChannels>,
    }

    type FakeBootState = Arc<Mutex<FakeBootDevice>>;

    fn device_error(status: StatusCode, code: &str, message: &str) -> Response {
        (
            status,
            Json(DeviceOperationError {
                code: code.into(),
                message: message.into(),
            }),
        )
            .into_response()
    }

    fn assert_v7(headers: &HeaderMap) {
        let expected = httpboot_protocol::device_protocol_version_header_value();
        assert_eq!(
            headers
                .get(httpboot_protocol::DEVICE_PROTOCOL_VERSION_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some(expected.as_str())
        );
    }

    async fn fake_status(State(device): State<FakeBootState>) -> Json<LoaderDeviceStatus> {
        Json(device.lock().await.status.clone())
    }

    async fn fake_create(
        State(device): State<FakeBootState>,
        headers: HeaderMap,
        Json(manifest): Json<DeviceBootJob>,
    ) -> Response {
        assert_v7(&headers);
        let mut device = device.lock().await;
        device.create_attempts += 1;
        if let Some(failure) = device.create_failed.clone() {
            let status = DeviceBootStatus {
                boot_id: manifest.boot_id,
                phase: "failed".into(),
                kernel_received: false,
                initramfs_received: false,
                last_error: Some(failure),
            };
            device.status.boot = Some(status.clone());
            return (StatusCode::OK, Json(status)).into_response();
        }
        match &device.manifest {
            Some(current) if current == &manifest => {
                let status = device
                    .status
                    .boot
                    .clone()
                    .expect("idempotent create has a boot status");
                (StatusCode::OK, Json(status)).into_response()
            }
            Some(_) => device_error(StatusCode::CONFLICT, "boot_job_busy", "busy"),
            None => {
                assert_eq!(manifest.kernel.sha256, hex_sha256(&device.kernel));
                let status = DeviceBootStatus {
                    boot_id: device
                        .create_boot_id_override
                        .clone()
                        .unwrap_or_else(|| manifest.boot_id.clone()),
                    phase: "receiving".into(),
                    kernel_received: false,
                    initramfs_received: manifest.initramfs.is_none(),
                    last_error: None,
                };
                device.status.boot = Some(status.clone());
                device.manifest = Some(manifest);
                (StatusCode::CREATED, Json(status)).into_response()
            }
        }
    }

    async fn fake_put_payload(
        State(device): State<FakeBootState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        assert_v7(&headers);
        let mut device = device.lock().await;
        device.upload_attempts += 1;
        if let Some((status, failure)) = device.upload_rejection.clone() {
            return device_error(status, &failure.code, &failure.message);
        }
        let kernel = body.len() == device.kernel.len() && body.as_ref() == device.kernel.as_slice();
        let initramfs = device
            .initramfs
            .as_ref()
            .is_some_and(|bytes| bytes.as_slice() == body.as_ref());
        let Some(boot) = device.status.boot.as_mut() else {
            return device_error(StatusCode::NOT_FOUND, "unknown_boot_job", "missing");
        };
        if kernel {
            boot.kernel_received = true;
        } else if initramfs {
            boot.initramfs_received = true;
        } else {
            return device_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "image_digest_mismatch",
                "payload mismatch",
            );
        }
        if boot.kernel_received && boot.initramfs_received {
            boot.phase = "ready".into();
        }
        let status = boot.clone();
        (StatusCode::OK, Json(status)).into_response()
    }

    async fn fake_delete(
        State(device): State<FakeBootState>,
        Path(_id): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        assert_v7(&headers);
        let mut device = device.lock().await;
        device.delete_attempts += 1;
        device.manifest = None;
        device.status.boot = None;
        StatusCode::NO_CONTENT.into_response()
    }

    fn interrupted_start_response(channels: StartBodyErrorChannels) -> Response {
        let body = futures_util::stream::once(async {
            Ok::<_, std::io::Error>(Bytes::from_static(b"partial"))
        })
        .chain(futures_util::stream::once(async move {
            let _ = channels.entered.send(());
            let _ = channels.release.await;
            Err(std::io::Error::other("start body interrupted"))
        }));
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(Body::from_stream(body))
            .unwrap()
    }

    async fn fake_start(State(device): State<FakeBootState>, headers: HeaderMap) -> Response {
        assert_v7(&headers);
        let mut device = device.lock().await;
        device.start_attempts += 1;
        match device.start_mode {
            StartMode::FailOnceThenReady | StartMode::FailOnceReceiving
                if device.start_attempts == 1 =>
            {
                if device.start_mode == StartMode::FailOnceReceiving {
                    device.status.boot.as_mut().unwrap().phase = "receiving".into();
                }
                return device_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "handoff_busy",
                    "start response uncertain",
                );
            }
            StartMode::RetryBodyError => {
                if let Some(channels) = device.start_body_errors.pop_front() {
                    return interrupted_start_response(channels);
                }
            }
            StartMode::RetryBodyTruncated => {
                return Response::builder()
                    .status(StatusCode::ACCEPTED)
                    .body(Body::from(vec![
                        b'x';
                        super::MAX_DEVICE_ERROR_BODY_BYTES + 1
                    ]))
                    .unwrap();
            }
            StartMode::Accepted | StartMode::FailOnceThenReady | StartMode::FailOnceReceiving => {}
        }
        device.status.boot.as_mut().unwrap().phase = "handoff".into();
        StatusCode::ACCEPTED.into_response()
    }

    async fn boot_test_context(
        root: &std::path::Path,
        mac: httpboot_protocol::MacAddress,
        start_mode: StartMode,
    ) -> (
        crate::AppState,
        Arc<crate::session::SessionState>,
        FakeBootState,
        u16,
    ) {
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
        let created = state
            .create_session_with_board_id(
                "x86_64-uefi-http",
                "board-1",
                &[],
                Some("device-test".into()),
            )
            .await
            .unwrap();
        let kernel = b"opaque future image".to_vec();
        state
            .tftp_manager
            .read()
            .await
            .put_session_file(&created.id, "kernel.elf", &kernel)
            .await
            .unwrap();
        let session = state.session_state(&created.id).await.unwrap();
        session
            .publish_boot_command(SessionBootCommand {
                boot_id: "boot-1".into(),
                kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                kernel_size: kernel.len() as u64,
                kernel_sha256: hex_sha256(&kernel),
                initramfs: None,
                cmdline: None,
            })
            .await;
        let fake = Arc::new(Mutex::new(FakeBootDevice {
            status: LoaderDeviceStatus {
                protocol_version: DEVICE_PROTOCOL_VERSION,
                boot_epoch: "epoch-1".into(),
                mac_address: mac,
                current_mac_address: mac,
                arch: "future-isa".into(),
                loader_version: "fake-v7".into(),
                hardware: LoaderHardwareInfo::default(),
                boot: None,
                ota: None,
                serial: None,
            },
            manifest: None,
            kernel,
            initramfs: None,
            create_attempts: 0,
            delete_attempts: 0,
            upload_attempts: 0,
            start_attempts: 0,
            create_failed: None,
            create_boot_id_override: None,
            upload_rejection: None,
            start_mode,
            start_body_errors: VecDeque::new(),
        }));
        let app = Router::new()
            .route("/api/v1/status", get(fake_status))
            .route("/api/v1/boot/jobs", post(fake_create))
            .route("/api/v1/boot/jobs/{id}", delete(fake_delete))
            .route("/api/v1/boot/jobs/{id}/kernel", put(fake_put_payload))
            .route("/api/v1/boot/jobs/{id}/initramfs", put(fake_put_payload))
            .route("/api/v1/boot/jobs/{id}/start", post(fake_start))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (state, session, fake, port)
    }

    async fn run_fake_push(
        state: &crate::AppState,
        session: &Arc<crate::session::SessionState>,
        port: u16,
    ) -> anyhow::Result<BootPushOutcome> {
        let command = session.boot_command().await.unwrap();
        if session.handoff_was_submitted(&command.boot_id).await {
            return Ok(BootPushOutcome::HandedOff);
        }
        let generation = *session.subscribe_boot_generation().borrow();
        if session
            .observed_handoff_for_generation(generation)
            .await
            .is_some()
        {
            return Ok(BootPushOutcome::Unconfirmed);
        }
        let request = BootRequest::new(&command);
        let mac = session
            .board()
            .network_identity
            .as_ref()
            .unwrap()
            .mac_address;
        let observed = LoaderDeviceStatus {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: "epoch-1".into(),
            mac_address: mac,
            current_mac_address: mac,
            arch: "future-isa".into(),
            loader_version: "fake-v7".into(),
            hardware: LoaderHardwareInfo::default(),
            boot: None,
            ota: None,
            serial: None,
        };
        let client = reqwest::Client::new();
        let endpoint = format!("http://127.0.0.1:{port}");
        let push = BootPushContext {
            state,
            client: &client,
            endpoint: &endpoint,
            observed: &observed,
            request: &request,
            session,
            binding_id: "binding-1",
        };
        let outcome = push_boot(&push).await?;
        if outcome == BootPushOutcome::Unconfirmed {
            session
                .mark_handoff_unconfirmed(generation, observed.boot_epoch, command.boot_id)
                .await;
        }
        Ok(outcome)
    }

    struct FakeOtaDevice {
        status: LoaderDeviceStatus,
        image: Vec<u8>,
        update_id: Option<String>,
        put_attempts: usize,
        confirm_attempts: usize,
        fail_after_stage_once: bool,
        rollback_on_confirm: bool,
    }

    type FakeOtaState = Arc<Mutex<FakeOtaDevice>>;

    async fn fake_ota_status(State(device): State<FakeOtaState>) -> Json<LoaderDeviceStatus> {
        Json(device.lock().await.status.clone())
    }

    async fn fake_ota_put(
        State(device): State<FakeOtaState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        assert_v7(&headers);
        let mut device = device.lock().await;
        device.put_attempts += 1;
        if body.as_ref() != device.image.as_slice() {
            return device_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "image_digest_mismatch",
                "OTA image mismatch",
            );
        }
        let update_id = headers
            .get("X-Update-Id")
            .and_then(|value| value.to_str().ok())
            .unwrap()
            .to_string();
        device.update_id = Some(update_id.clone());
        let ota = device.status.ota.as_mut().unwrap();
        ota.pending_update_id = Some(update_id);
        ota.source = None;
        ota.trial = false;
        if device.fail_after_stage_once {
            device.fail_after_stage_once = false;
            return device_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "staging_response_lost",
                "image staged before the response was lost",
            );
        }
        StatusCode::ACCEPTED.into_response()
    }

    #[derive(serde::Deserialize)]
    struct OtaConfirmRequest {
        update_id: String,
    }

    async fn fake_ota_confirm(
        State(device): State<FakeOtaState>,
        headers: HeaderMap,
        Json(request): Json<OtaConfirmRequest>,
    ) -> Response {
        assert_v7(&headers);
        let mut device = device.lock().await;
        device.confirm_attempts += 1;
        let rollback = device.rollback_on_confirm;
        let ota = device.status.ota.as_mut().unwrap();
        if rollback {
            ota.pending_update_id = None;
            ota.trial = false;
            ota.last_update_id = Some(request.update_id);
            ota.last_outcome = Some(httpboot_protocol::OtaOutcome::RolledBack);
            ota.active_sha256 = "00".repeat(32);
        } else {
            ota.active_sha256.clone_from(&ota.running_sha256);
            ota.pending_update_id = None;
            ota.trial = false;
            ota.last_update_id = Some(request.update_id);
            ota.last_outcome = Some(httpboot_protocol::OtaOutcome::Confirmed);
        }
        StatusCode::OK.into_response()
    }

    async fn ota_test_state(
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
        state.boards.write().await.insert(
            "board-1".into(),
            BoardConfig {
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
            },
        );
        state
            .board_runtimes
            .write()
            .await
            .insert("board-1".into(), BoardRuntimeState::default());
        state
    }

    fn fake_ota_device(mac: httpboot_protocol::MacAddress, image: Vec<u8>) -> FakeOtaState {
        Arc::new(Mutex::new(FakeOtaDevice {
            status: LoaderDeviceStatus {
                protocol_version: DEVICE_PROTOCOL_VERSION,
                boot_epoch: "epoch-1".into(),
                mac_address: mac,
                current_mac_address: mac,
                arch: "future-isa".into(),
                loader_version: "fake-v7".into(),
                hardware: LoaderHardwareInfo::default(),
                boot: None,
                ota: Some(httpboot_protocol::LoaderOtaState {
                    active_sha256: "11".repeat(32),
                    running_sha256: "11".repeat(32),
                    pending_update_id: None,
                    trial: false,
                    source: None,
                    last_update_id: None,
                    last_outcome: None,
                }),
                serial: None,
            },
            image,
            update_id: None,
            put_attempts: 0,
            confirm_attempts: 0,
            fail_after_stage_once: false,
            rollback_on_confirm: false,
        }))
    }

    fn fake_announcement(
        mac: httpboot_protocol::MacAddress,
        boot_epoch: &str,
        http_port: u16,
    ) -> LoaderAnnouncement {
        LoaderAnnouncement {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: "future-isa".into(),
            loader_version: "fake-v7".into(),
            boot_epoch: boot_epoch.into(),
            http_port,
            serial_id: None,
            serial_ready: false,
        }
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
    async fn boot_handoff_cancels_lock_wait_on_power_change() {
        use crate::serial::runtime::SerialRuntimePhase;

        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:03".parse().unwrap();
        let mut state = ota_test_state(dir.path(), mac).await;
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
        let bytes = b"opaque boot image".to_vec();
        state
            .tftp_manager
            .read()
            .await
            .clone()
            .put_session_file(&created.id, "kernel.elf", &bytes)
            .await
            .unwrap();
        let mut event = fake_announcement(mac, "before-power-change", 0);
        event.serial_id = Some(format!("{:032x}", 2));
        event.serial_ready = true;
        let observed = LoaderDeviceStatus {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: event.boot_epoch.clone(),
            mac_address: mac,
            current_mac_address: mac,
            arch: event.arch.clone(),
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
    async fn failed_create_status_preserves_first_error_and_never_uploads() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:41".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::Accepted).await;
        fake.lock().await.create_failed = Some(DeviceOperationError {
            code: "efi_entry_missing".into(),
            message: "missing local entry symbol".into(),
        });

        let error = run_fake_push(&state, &session, port).await.unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("efi_entry_missing"));
        assert!(error.contains("missing local entry symbol"));
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 1);
        assert_eq!(fake.upload_attempts, 0);
        assert_eq!(fake.start_attempts, 0);
        drop(fake);
        assert!(matches!(
            session.loader_status().await.unwrap().status,
            Some(LoaderStatusPhase::Failed { code, message })
                if code == "efi_entry_missing" && message == "missing local entry symbol"
        ));
        assert_eq!(
            session.serial_runtime.snapshot().phase,
            crate::serial::runtime::SerialRuntimePhase::Failed
        );
        assert!(session.is_stop_requested());
    }

    #[tokio::test]
    async fn explicit_upload_rejection_publishes_open_device_error() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:42".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::Accepted).await;
        fake.lock().await.upload_rejection = Some((
            StatusCode::UNPROCESSABLE_ENTITY,
            DeviceOperationError {
                code: "future_payload_rejected".into(),
                message: "device-specific reason".into(),
            },
        ));

        let error = run_fake_push(&state, &session, port).await.unwrap_err();
        assert!(format!("{error:#}").contains("future_payload_rejected"));
        let fake = fake.lock().await;
        assert_eq!(fake.upload_attempts, 1);
        assert_eq!(fake.start_attempts, 0);
        drop(fake);
        assert!(matches!(
            session.loader_status().await.unwrap().status,
            Some(LoaderStatusPhase::Failed { code, message })
                if code == "future_payload_rejected" && message == "device-specific reason"
        ));
        assert_eq!(
            session.serial_runtime.snapshot().phase,
            crate::serial::runtime::SerialRuntimePhase::Failed
        );
        assert!(session.is_stop_requested());
    }

    #[tokio::test]
    async fn create_response_must_match_the_current_boot_id() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:46".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::Accepted).await;
        fake.lock().await.create_boot_id_override = Some("another-boot".into());

        let error = run_fake_push(&state, &session, port).await.unwrap_err();
        assert!(format!("{error:#}").contains("another-boot"));
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 1);
        assert_eq!(fake.upload_attempts, 0);
        assert_eq!(fake.start_attempts, 0);
        drop(fake);
        assert!(matches!(
            session.loader_status().await.unwrap().status,
            Some(LoaderStatusPhase::Failed { code, .. })
                if code == "invalid_device_response"
        ));
        assert!(session.is_stop_requested());
    }

    #[tokio::test]
    async fn uncertain_start_observes_without_delete_or_repeating_uploads() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:43".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::FailOnceReceiving).await;

        assert_eq!(
            run_fake_push(&state, &session, port).await.unwrap(),
            BootPushOutcome::Unconfirmed
        );
        assert_eq!(
            run_fake_push(&state, &session, port).await.unwrap(),
            BootPushOutcome::Unconfirmed
        );
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 1);
        assert_eq!(fake.upload_attempts, 1);
        assert_eq!(fake.start_attempts, 1);
        assert_eq!(fake.delete_attempts, 0);
        drop(fake);
        assert!(
            session
                .observed_handoff_for_generation(*session.subscribe_boot_generation().borrow())
                .await
                .is_some()
        );
        assert!(session.serial_runtime.snapshot().warning.is_some());
        assert!(!session.is_stop_requested());
    }

    #[tokio::test]
    async fn uncertain_ready_retries_the_same_boot_id_once() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:44".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::FailOnceThenReady).await;

        assert_eq!(
            run_fake_push(&state, &session, port).await.unwrap(),
            BootPushOutcome::HandedOff
        );
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 1);
        assert_eq!(fake.delete_attempts, 0);
        assert_eq!(fake.upload_attempts, 1);
        assert_eq!(fake.start_attempts, 2);
        assert_eq!(fake.manifest.as_ref().unwrap().boot_id, "boot-1");
    }

    #[tokio::test]
    async fn first_start_body_read_error_retries_the_same_boot_id_once() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:48".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::RetryBodyError).await;
        {
            let mut fake = fake.lock().await;
            fake.status.boot = Some(DeviceBootStatus {
                boot_id: "boot-1".into(),
                phase: "ready".into(),
                kernel_received: true,
                initramfs_received: true,
                last_error: None,
            });
        }
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        fake.lock()
            .await
            .start_body_errors
            .push_back(StartBodyErrorChannels {
                entered: entered_tx,
                release: release_rx,
            });
        let client = reqwest::Client::new();
        let endpoint = format!("http://127.0.0.1:{port}");
        let response = client
            .post(format!("{endpoint}/api/v1/boot/jobs/boot-1/start"))
            .header(
                httpboot_protocol::DEVICE_PROTOCOL_VERSION_HEADER,
                httpboot_protocol::device_protocol_version_header_value(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        tokio::time::timeout(std::time::Duration::from_secs(3), entered_rx)
            .await
            .expect("202 response body was not polled")
            .unwrap();
        release_tx.send(()).unwrap();
        let observed = fake.lock().await.status.clone();
        let start = StartContext {
            state: &state,
            client: &client,
            endpoint: &endpoint,
            observed: &observed,
            session: &session,
            binding_id: "binding-1",
            boot_id: "boot-1",
        };
        assert_eq!(
            finish_start(start, Ok(response)).await.unwrap(),
            BootPushOutcome::HandedOff
        );
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 0);
        assert_eq!(fake.upload_attempts, 0);
        assert_eq!(fake.start_attempts, 2);
        assert_eq!(fake.delete_attempts, 0);
    }

    #[tokio::test]
    async fn retry_start_body_read_error_stays_unconfirmed() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:49".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::RetryBodyError).await;
        {
            let mut fake = fake.lock().await;
            fake.status.boot = Some(DeviceBootStatus {
                boot_id: "boot-1".into(),
                phase: "ready".into(),
                kernel_received: true,
                initramfs_received: true,
                last_error: None,
            });
        }
        let (first_entered_tx, first_entered_rx) = oneshot::channel();
        let (first_release_tx, first_release_rx) = oneshot::channel();
        let (retry_entered_tx, retry_entered_rx) = oneshot::channel();
        let (retry_release_tx, retry_release_rx) = oneshot::channel();
        fake.lock().await.start_body_errors = VecDeque::from([
            StartBodyErrorChannels {
                entered: first_entered_tx,
                release: first_release_rx,
            },
            StartBodyErrorChannels {
                entered: retry_entered_tx,
                release: retry_release_rx,
            },
        ]);
        let client = reqwest::Client::new();
        let endpoint = format!("http://127.0.0.1:{port}");
        let response = client
            .post(format!("{endpoint}/api/v1/boot/jobs/boot-1/start"))
            .header(
                httpboot_protocol::DEVICE_PROTOCOL_VERSION_HEADER,
                httpboot_protocol::device_protocol_version_header_value(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        tokio::time::timeout(std::time::Duration::from_secs(3), first_entered_rx)
            .await
            .expect("first 202 response body was not polled")
            .unwrap();
        first_release_tx.send(()).unwrap();
        let observed = fake.lock().await.status.clone();
        let start = StartContext {
            state: &state,
            client: &client,
            endpoint: &endpoint,
            observed: &observed,
            session: &session,
            binding_id: "binding-1",
            boot_id: "boot-1",
        };
        let mut workflow = Box::pin(finish_start(start, Ok(response)));
        tokio::select! {
            result = &mut workflow => panic!("retry body error was not gated: {result:?}"),
            entered = retry_entered_rx => entered.unwrap(),
        }
        retry_release_tx.send(()).unwrap();
        assert_eq!(workflow.await.unwrap(), BootPushOutcome::Unconfirmed);
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 0);
        assert_eq!(fake.upload_attempts, 0);
        assert_eq!(fake.start_attempts, 2);
        assert_eq!(fake.delete_attempts, 0);
        drop(fake);
        assert!(session.serial_runtime.snapshot().warning.is_some());
        assert!(!session.handoff_was_submitted("boot-1").await);
        assert!(!session.is_stop_requested());
    }

    #[tokio::test]
    async fn retry_start_body_truncation_stays_unconfirmed() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:4a".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::RetryBodyTruncated).await;

        assert_eq!(
            run_fake_push(&state, &session, port).await.unwrap(),
            BootPushOutcome::Unconfirmed
        );
        let fake = fake.lock().await;
        assert_eq!(fake.start_attempts, 2);
        assert_eq!(fake.delete_attempts, 0);
        drop(fake);
        assert!(!session.handoff_was_submitted("boot-1").await);
    }

    #[tokio::test]
    async fn successful_handoff_status_prevents_a_second_push() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:47".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::Accepted).await;

        assert_eq!(
            run_fake_push(&state, &session, port).await.unwrap(),
            BootPushOutcome::HandedOff
        );
        assert_eq!(
            run_fake_push(&state, &session, port).await.unwrap(),
            BootPushOutcome::HandedOff
        );
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 1);
        assert_eq!(fake.upload_attempts, 1);
        assert_eq!(fake.start_attempts, 1);
        assert_eq!(fake.delete_attempts, 0);
        assert!(session.handoff_was_submitted("boot-1").await);
    }

    #[tokio::test]
    async fn unconfirmed_observation_converges_to_a_later_device_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:48".parse().unwrap();
        let (state, session, fake, port) =
            boot_test_context(dir.path(), mac, StartMode::FailOnceReceiving).await;
        session.set_serial_connected(true);
        session
            .mark_handoff_unconfirmed(
                *session.subscribe_boot_generation().borrow(),
                "epoch-1".into(),
                "boot-1".into(),
            )
            .await;
        fake.lock().await.status.boot = Some(DeviceBootStatus {
            boot_id: "boot-1".into(),
            phase: "failed".into(),
            kernel_received: false,
            initramfs_received: false,
            last_error: Some(DeviceOperationError {
                code: "efi_entry_missing".into(),
                message: "device could not find local entry".into(),
            }),
        });

        let error = reconcile(
            state,
            fake_announcement(mac, "epoch-1", port),
            "127.0.0.1:40102".parse().unwrap(),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("efi_entry_missing"));
        let fake = fake.lock().await;
        assert_eq!(fake.create_attempts, 0);
        assert_eq!(fake.upload_attempts, 0);
        assert_eq!(fake.start_attempts, 0);
        assert_eq!(fake.delete_attempts, 0);
        drop(fake);
        assert!(matches!(
            session.loader_status().await.unwrap().status,
            Some(LoaderStatusPhase::Failed { code, message })
                if code == "efi_entry_missing"
                    && message == "device could not find local entry"
        ));
        assert_eq!(
            session.serial_runtime.snapshot().phase,
            crate::serial::runtime::SerialRuntimePhase::Failed
        );
        assert!(session.is_stop_requested());
    }

    #[tokio::test]
    async fn power_generation_change_clears_unconfirmed_observation() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:45".parse().unwrap();
        let (_, session, _, _) =
            boot_test_context(dir.path(), mac, StartMode::FailOnceReceiving).await;
        session
            .mark_handoff_unconfirmed(
                *session.subscribe_boot_generation().borrow(),
                "epoch-1".into(),
                "boot-1".into(),
            )
            .await;
        session.change_power(false).await;
        assert!(
            session
                .observed_handoff_for_generation(*session.subscribe_boot_generation().borrow())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn reverse_ota_recovers_an_uncertain_staged_response_and_confirms_trial() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:51".parse().unwrap();
        let state = ota_test_state(dir.path(), mac).await;
        let image_bytes = b"opaque loader image".to_vec();
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
        let fake = fake_ota_device(mac, image_bytes);
        fake.lock().await.fail_after_stage_once = true;
        let app = Router::new()
            .route("/api/v1/status", get(fake_ota_status))
            .route("/api/v1/ota/image", put(fake_ota_put))
            .route("/api/v1/ota/confirm", post(fake_ota_confirm))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let announcement = fake_announcement(mac, "epoch-1", port);
        let peer = "127.0.0.1:40100".parse().unwrap();

        reconcile(state.clone(), announcement.clone(), peer)
            .await
            .unwrap();
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            crate::ota::Phase::Staged
        );
        assert_eq!(fake.lock().await.put_attempts, 1);

        {
            let mut fake = fake.lock().await;
            let ota = fake.status.ota.as_mut().unwrap();
            ota.pending_update_id = Some(job.update_id.clone());
            ota.running_sha256 = image.sha256.clone();
            ota.trial = true;
            ota.source = Some(httpboot_protocol::OtaSource::Server);
        }
        reconcile(state.clone(), announcement, peer).await.unwrap();
        let fake = fake.lock().await;
        assert_eq!(fake.put_attempts, 1);
        assert_eq!(fake.confirm_attempts, 1);
        drop(fake);
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            crate::ota::Phase::Succeeded
        );
    }

    #[tokio::test]
    async fn reverse_ota_rollback_remains_a_terminal_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let mac = "02:00:00:00:00:52".parse().unwrap();
        let state = ota_test_state(dir.path(), mac).await;
        let image_bytes = b"rollback loader image".to_vec();
        let image = state.ota.put_image(&image_bytes, None).await.unwrap();
        let job = state
            .ota
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        let fake = fake_ota_device(mac, image_bytes);
        {
            let mut fake = fake.lock().await;
            let ota = fake.status.ota.as_mut().unwrap();
            ota.pending_update_id = Some(job.update_id.clone());
            ota.running_sha256 = image.sha256.clone();
            ota.trial = true;
            ota.source = Some(httpboot_protocol::OtaSource::Server);
            fake.rollback_on_confirm = true;
        }
        let app = Router::new()
            .route("/api/v1/status", get(fake_ota_status))
            .route("/api/v1/ota/image", put(fake_ota_put))
            .route("/api/v1/ota/confirm", post(fake_ota_confirm))
            .with_state(fake.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let announcement = fake_announcement(mac, "epoch-1", port);
        let peer = "127.0.0.1:40101".parse().unwrap();

        let ota = fake.lock().await.status.ota.clone();
        state
            .ota
            .decide("board-1", mac, ota.as_ref(), true, "epoch-1")
            .await
            .unwrap();
        let _ = reconcile(state.clone(), announcement.clone(), peer).await;
        reconcile(state.clone(), announcement, peer).await.unwrap();
        let fake = fake.lock().await;
        assert_eq!(fake.put_attempts, 0);
        assert_eq!(fake.confirm_attempts, 1);
        assert_eq!(
            state.ota.job("board-1").await.unwrap().phase,
            crate::ota::Phase::RolledBack
        );
    }

    #[test]
    fn boot_manifest_independently_preserves_all_payload_combinations() {
        for (has_initramfs, has_cmdline) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let command = command(has_initramfs, has_cmdline);
            let request = BootRequest::new(&command);
            assert_eq!(request.manifest.initramfs.is_some(), has_initramfs);
            assert_eq!(request.manifest.cmdline.is_some(), has_cmdline);
            let value = serde_json::to_value(&request.manifest).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 4);
            assert!(value.get("arch").is_none());
            assert!(value.get("image_format").is_none());
            assert!(value.get("entry_symbol").is_none());
        }
    }

    #[test]
    fn only_exact_v7_numeric_protocol_is_accepted() {
        require_device_protocol(DEVICE_PROTOCOL_VERSION).unwrap();
        assert!(require_device_protocol(DEVICE_PROTOCOL_VERSION - 1).is_err());
    }

    #[test]
    fn device_error_code_and_message_are_passed_through() {
        let failure = decode_device_error_body(
            StatusCode::UNPROCESSABLE_ENTITY,
            br#"{"code":"invalid_elf","message":"bad machine"}"#,
            false,
        );
        assert_eq!(failure.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(failure.error.code, "invalid_elf");
        assert_eq!(failure.error.message, "bad machine");
    }

    #[test]
    fn malformed_device_error_is_reported_with_http_status_and_raw_body() {
        let failure =
            decode_device_error_body(StatusCode::BAD_GATEWAY, b"upstream returned garbage", false);
        assert_eq!(failure.error.code, "invalid_device_response");
        assert!(failure.error.message.contains("502 Bad Gateway"));
        assert!(failure.error.message.contains("upstream returned garbage"));
    }

    #[test]
    fn truncated_device_error_is_never_accepted_as_a_complete_error() {
        let failure = decode_device_error_body(
            StatusCode::UNPROCESSABLE_ENTITY,
            br#"{"code":"invalid_elf","message":"bad"#,
            true,
        );
        assert_eq!(failure.error.code, "invalid_device_response");
        assert!(failure.error.message.contains("truncated"));
        assert!(failure.error.message.contains("invalid_elf"));
    }
}
