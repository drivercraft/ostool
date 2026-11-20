//! Session-owned U-Boot orchestration. Device network parameters never leave the server.
use crate::{AppState, BootConfig, api::error::ApiError, session::SessionState};
use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Mutex, Notify, watch},
};
use tokio_util::compat::{
    FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt, TokioAsyncReadCompatExt,
    TokioAsyncWriteCompatExt,
};
use uboot_shell::UbootShell;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    #[serde(default)]
    pub commands: Vec<String>,
    pub kernel_load_addr: Option<u64>,
    pub fit_load_addr: Option<u64>,
    pub bootm_addr: Option<u64>,
    pub timeout_secs: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Prepared {
    pub kernel_load_addr: u64,
    pub fit_load_addr: u64,
    pub fdt_load_addr: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootRequest {
    pub path: String,
    pub cmdline: Option<String>,
}

#[derive(Debug)]
pub struct UbootSession {
    pub request: Mutex<Option<PrepareRequest>>,
    prepared: watch::Sender<Option<Result<Prepared, String>>>,
    boot: Mutex<Option<BootRequest>>,
    changed: Notify,
}
impl Default for UbootSession {
    fn default() -> Self {
        Self {
            request: Mutex::new(None),
            prepared: watch::channel(None).0,
            boot: Mutex::new(None),
            changed: Notify::new(),
        }
    }
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/sessions/{session_id}/uboot/prepare",
            post(prepare).get(prepared),
        )
        .route("/api/v1/sessions/{session_id}/uboot/boot", post(boot))
}
async fn active(state: &AppState, id: &str) -> Result<std::sync::Arc<SessionState>, ApiError> {
    let s = state
        .session_state(id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    if s.is_stop_requested()
        || s.is_releasing()
        || s.snapshot().await.expires_at <= chrono::Utc::now()
    {
        return Err(ApiError::conflict("会话已结束"));
    }
    if !matches!(s.board().boot, BootConfig::Uboot(_)) {
        return Err(ApiError::bad_request("设备不使用 U-Boot"));
    }
    Ok(s)
}
async fn prepare(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<PrepareRequest>,
) -> Result<StatusCode, ApiError> {
    if request.commands.len() > 64
        || request
            .commands
            .iter()
            .any(|c| c.len() > 8192 || c.chars().any(char::is_control))
        || request.commands.iter().map(String::len).sum::<usize>() > 32768
        || request
            .timeout_secs
            .is_some_and(|t| !(1..=600).contains(&t))
    {
        return Err(ApiError::bad_request("启动命令或超时参数无效"));
    }
    let s = active(&state, &id).await?;
    if !s.board().serial_available() {
        return Err(ApiError::conflict("设备未配置串口"));
    }
    let mut slot = s.uboot.request.lock().await;
    if slot.is_some() || s.uboot.prepared.borrow().is_some() || s.snapshot().await.serial_connected
    {
        return Err(ApiError::conflict("启动准备已提交或串口已连接"));
    }
    *slot = Some(request);
    Ok(StatusCode::ACCEPTED)
}
async fn prepared(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Prepared>, ApiError> {
    let s = active(&state, &id).await?;
    let mut ready = s.uboot.prepared.subscribe();
    let mut shutdown = s.subscribe_shutdown();
    let result = tokio::select! {
        r=tokio::time::timeout(Duration::from_secs(610),ready.wait_for(|r|r.is_some())) =>
            r.map_err(|_|ApiError::conflict("等待设备启动准备超时"))?
                .map_err(|_|ApiError::conflict("会话已结束"))?.as_ref().unwrap().clone(),
        _=shutdown.wait_for(|v|*v)=>return Err(ApiError::conflict("会话已结束")),
    };
    result
        .map(Json)
        .map_err(|_| ApiError::service_unavailable("设备启动准备失败，请联系管理员"))
}
async fn boot(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(mut request): Json<BootRequest>,
) -> Result<StatusCode, ApiError> {
    request.path = crate::tftp::files::normalize_relative_path(&request.path)
        .map_err(|_| ApiError::bad_request("启动文件路径无效"))?;
    if !request
        .path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b))
        || request
            .cmdline
            .as_ref()
            .is_some_and(|c| !httpboot_protocol::valid_host_cmdline(c) || c.contains('\''))
    {
        return Err(ApiError::bad_request("启动参数无效"));
    }
    let s = active(&state, &id).await?;
    if !s
        .uboot
        .prepared
        .borrow()
        .as_ref()
        .is_some_and(Result::is_ok)
    {
        return Err(ApiError::conflict("设备尚未准备好"));
    }
    let _files = s.file_gate.lock().await;
    if state
        .tftp_manager
        .read()
        .await
        .get_session_file(&id, &request.path)
        .await?
        .is_none()
    {
        return Err(ApiError::not_found("启动文件不存在"));
    }
    let mut slot = s.uboot.boot.lock().await;
    if slot.is_some() {
        return Err(ApiError::conflict("启动请求已提交"));
    }
    *slot = Some(request);
    s.uboot.changed.notify_one();
    Ok(StatusCode::ACCEPTED)
}

fn parse_address(value: &str) -> Option<u64> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

async fn read_address(shell: &mut UbootShell, name: &str) -> anyhow::Result<u64> {
    let value = shell.env(name).await?;
    // U-Boot address variables are hexadecimal, commonly without a 0x prefix.
    Ok(u64::from_str_radix(
        value
            .trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X"),
        16,
    )?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TransferProgress {
    sent: usize,
    // TFTP reports hash marks, not acknowledged byte counts.
    total: Option<usize>,
    done: bool,
}

#[derive(Default)]
struct ProgressOutput {
    previous: Option<TransferProgress>,
}

impl ProgressOutput {
    // Only synthesized progress enters the user console. In particular, U-Boot
    // command output can contain private device-LAN addresses and file paths.
    fn render(&mut self, progress: Option<TransferProgress>) -> Option<axum::extract::ws::Message> {
        let progress = progress?;
        if self.previous.as_ref() == Some(&progress) {
            return None;
        }
        let mut output = if let Some(total) = progress.total {
            let filled = if total == 0 {
                usize::from(progress.done) * 40
            } else {
                ((progress.sent.min(total) as u128 * 40) / total as u128) as usize
            };
            format!(
                "\rUploading image (YMODEM) [{}{}] {:>width$}/{} bytes",
                "#".repeat(filled),
                "-".repeat(40 - filled),
                progress.sent,
                total,
                width = total.to_string().len(),
            )
        } else {
            let mut output = if self.previous.is_none() {
                "Uploading image (TFTP) [".to_owned()
            } else {
                String::new()
            };
            let previous = self.previous.as_ref().map_or(0, |p| p.sent);
            output.push_str(&"#".repeat(progress.sent.saturating_sub(previous)));
            if progress.done {
                output.push(']');
            }
            output
        };
        if progress.done {
            output.push_str(" done\r\n");
        }
        self.previous = Some(progress);
        Some(axum::extract::ws::Message::Binary(
            output.into_bytes().into(),
        ))
    }
}

/// The bridge and orchestration are scoped to the websocket, so cancellation owns all UART I/O.
pub(super) async fn run<R, W, S, I, E, F, H>(
    rx: &mut R,
    tx: &mut W,
    sender: &mut S,
    receiver: &mut I,
    link: super::transport::Link<'_, F>,
    request: PrepareRequest,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: futures_util::Sink<axum::extract::ws::Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
    I: futures_util::Stream<Item = Result<axum::extract::ws::Message, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn() -> H,
    H: Future<Output = ()>,
{
    let (state, session) = link.commands.context("managed boot requires a session")?;
    let (console, relay) = tokio::io::duplex(256 * 1024);
    let (mut relay_rx, mut relay_tx) = tokio::io::split(relay);
    // Callbacks and serial reads only update the latest progress; they never
    // wait for the WebSocket or queue one message per packet.
    let (progress_tx, mut progress_rx) = watch::channel(None::<TransferProgress>);
    let bridge = async {
        let read = async {
            let mut buffer = [0; 4096];
            loop {
                let size = rx.read(&mut buffer).await?;
                if size == 0 {
                    return Ok::<_, std::io::Error>(());
                }
                // Observe only hash marks during TFTP. Forward every original
                // byte unchanged to UbootShell, which still owns the protocol.
                progress_tx.send_if_modified(|progress| {
                    if let Some(progress) = progress
                        && progress.total.is_none()
                        && !progress.done
                    {
                        let hashes = buffer[..size].iter().filter(|&&b| b == b'#').count();
                        progress.sent += hashes;
                        return hashes != 0;
                    }
                    false
                });
                relay_tx.write_all(&buffer[..size]).await?;
            }
        };
        tokio::select! {
            r=read=>r,
            r=tokio::io::copy(&mut relay_rx,tx)=>r.map(|_|()),
        }
    };
    let mut ready = link.ready.clone();
    let work = async {
        let startup = async {
            ready.wait_for(|v| *v).await.context("power-on cancelled")?;
            let (r, w) = tokio::io::split(console);
            let prepare = async {
                let mut shell = UbootShell::new(w.compat_write(), r.compat()).await?;
                shell.set_env("autoload", "yes").await?;
                for command in &request.commands {
                    let command = super::commands::expand(state, session, command).await?;
                    shell.cmd(&command).await?;
                }
                let BootConfig::Uboot(profile) = &session.board().boot else {
                    unreachable!()
                };
                let parse = |value: &Option<String>| value.as_ref().and_then(|v| parse_address(v));
                let kernel = if let Some(a) = request
                    .kernel_load_addr
                    .or(parse(&profile.kernel_load_addr))
                {
                    a
                } else if let Ok(a) = read_address(&mut shell, "kernel_addr_r").await {
                    a
                } else {
                    read_address(&mut shell, "loadaddr").await?
                };
                let fit = if let Some(a) = request.fit_load_addr.or(parse(&profile.fit_load_addr)) {
                    a
                } else if let Ok(a) = read_address(&mut shell, "kernel_comp_addr_r").await {
                    a
                } else if let Ok(a) = read_address(&mut shell, "kernel_addr_c").await {
                    a
                } else {
                    kernel
                        .checked_add(0x02000000)
                        .context("load address overflow")?
                        & 0xffff_ffff_ff00_0000
                };
                let fdt = read_address(&mut shell, "fdt_addr_r").await.ok();
                Ok::<_, anyhow::Error>((
                    shell,
                    Prepared {
                        kernel_load_addr: kernel,
                        fit_load_addr: fit,
                        fdt_load_addr: fdt,
                    },
                ))
            };
            let prepared = tokio::time::timeout(
                Duration::from_secs(request.timeout_secs.unwrap_or(300)),
                prepare,
            )
            .await
            .context("U-Boot prepare timed out")
            .and_then(|r| r);
            let (mut shell, addresses) = match prepared {
                Ok(value) => value,
                Err(error) => {
                    session
                        .uboot
                        .prepared
                        .send_replace(Some(Err("prepare failed".into())));
                    return Err(error);
                }
            };
            session
                .uboot
                .prepared
                .send_replace(Some(Ok(addresses.clone())));
            let boot = tokio::time::timeout(Duration::from_secs(600), async {
                loop {
                    let notified = session.uboot.changed.notified();
                    if let Some(boot) = session.uboot.boot.lock().await.clone() {
                        break boot;
                    }
                    notified.await;
                }
            })
            .await
            .context("waiting for boot image timed out")?;
            let load = async {
                let _files = session.file_gate.lock().await;
                let id = session.snapshot().await.id;
                let manager = state.tftp_manager.read().await.clone();
                let file = manager
                    .get_session_file(&id, &boot.path)
                    .await?
                    .context("boot file missing")?;
                let BootConfig::Uboot(profile) = &session.board().boot else {
                    unreachable!()
                };
                let network = crate::api::router::resolved_board_network(state, session.board())
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e.message))?;
                let status = manager.status().await?;
                if profile.use_tftp {
                    anyhow::ensure!(
                        status.enabled && status.healthy,
                        "TFTP was selected but the device LAN TFTP service is unavailable"
                    );
                    anyhow::ensure!(
                        network.as_ref().is_some_and(|n| n.server_ip.is_some()),
                        "TFTP was selected but the device LAN server address is unavailable"
                    );
                }
                shell
                    .set_env("loadaddr", format!("{:#x}", addresses.fit_load_addr))
                    .await?;
                if profile.use_tftp {
                    let n = network.as_ref().unwrap();
                    for (key, value) in [
                        ("serverip", n.server_ip.as_ref()),
                        ("netmask", n.netmask.as_ref()),
                        ("gatewayip", profile.gatewayip.as_ref()),
                        ("ipaddr", profile.board_ip.as_ref()),
                    ] {
                        if let Some(value) = value {
                            shell.set_env(key, value).await?;
                        }
                    }
                    if shell
                        .cmd("net list")
                        .await
                        .is_ok_and(|s| s.trim().is_empty())
                    {
                        let _ = shell.cmd("bootdev hunt ethernet").await;
                    }
                    shell.set_env("bootfile", &file.relative_path).await?;
                    let command = if profile.network_mode == crate::UbootNetworkMode::StaticIp {
                        "tftp"
                    } else {
                        "dhcp"
                    };
                    progress_tx.send_replace(Some(TransferProgress {
                        sent: 0,
                        total: None,
                        done: false,
                    }));
                    shell
                        .cmd(&format!("{command} {}", file.relative_path))
                        .await?;
                } else {
                    shell
                        .loady(
                            usize::try_from(addresses.fit_load_addr)?,
                            &file.disk_path,
                            |sent, total| {
                                progress_tx.send_replace(Some(TransferProgress {
                                    sent,
                                    total: Some(total),
                                    done: false,
                                }));
                            },
                        )
                        .await?;
                }
                progress_tx.send_modify(|progress| {
                    if let Some(progress) = progress {
                        progress.done = true;
                    }
                });
                if let Some(cmdline) = &boot.cmdline {
                    shell.set_env("bootargs", format!("'{cmdline}'")).await?;
                }
                let address = request
                    .bootm_addr
                    .or_else(|| profile.bootm_addr.as_ref().and_then(|v| parse_address(v)))
                    .unwrap_or(addresses.fit_load_addr);
                shell
                    .cmd_without_reply(&format!("bootm {address:#x}"))
                    .await?;
                Ok::<_, anyhow::Error>(())
            };
            tokio::time::timeout(
                Duration::from_secs(request.timeout_secs.unwrap_or(300)),
                load,
            )
            .await
            .context("U-Boot transfer timed out")??;
            Ok::<_, anyhow::Error>(shell)
        };
        tokio::pin!(startup);
        let mut progress_output = ProgressOutput::default();
        let mut shell = loop {
            use axum::extract::ws::Message;
            use futures_util::{SinkExt, StreamExt};
            tokio::select! {
                result=&mut startup=>{
                    // Completion can win the select before the last update.
                    let progress = progress_rx.borrow_and_update().clone();
                    if let Some(message) = progress_output.render(progress) {
                        sender.send(message).await?;
                    }
                    if result.is_err() && progress_output.previous.as_ref().is_some_and(|p| !p.done) {
                        sender.send(Message::Binary(b" failed\r\n".to_vec().into())).await?;
                    }
                    break result?;
                },
                changed=progress_rx.changed()=>{
                    changed?;
                    let progress = progress_rx.borrow_and_update().clone();
                    if let Some(message) = progress_output.render(progress) {
                        sender.send(message).await?;
                    }
                },
                message=receiver.next()=>match message {
                    Some(Ok(Message::Ping(bytes)))=>sender.send(Message::Pong(bytes)).await?,
                    Some(Ok(Message::Pong(_)))=>{},
                    Some(Ok(Message::Close(_)))|None=>return Ok(()),
                    Some(Ok(Message::Text(text))) if serde_json::from_str::<serde_json::Value>(&text).ok().is_some_and(|v|v["type"]=="close")=>return Ok(()),
                    Some(Err(error))=>return Err(error.into()),
                    _=>anyhow::bail!("serial input is unavailable during managed boot"),
                }
            }
        };
        let mut r = shell.rx.take().unwrap().compat();
        let mut w = shell.tx.take().unwrap().compat_write();
        super::transport::run(&mut r, &mut w, sender, receiver, link).await
    };
    tokio::select! {result=work=>result,result=bridge=>{result?;anyhow::bail!("device serial closed")}}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(message: Option<axum::extract::ws::Message>) -> String {
        let Some(axum::extract::ws::Message::Binary(bytes)) = message else {
            panic!("expected binary terminal progress");
        };
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn serial_progress_handles_retry_and_waits_for_confirmed_completion() {
        let mut output = ProgressOutput::default();
        let mut progress = TransferProgress {
            sent: 1024,
            total: Some(2048),
            done: false,
        };
        let halfway = text(output.render(Some(progress.clone())));
        assert!(halfway.contains(&format!("[{}{}]", "#".repeat(20), "-".repeat(20))));
        assert!(halfway.contains("1024/2048 bytes"));
        assert!(output.render(Some(progress.clone())).is_none());
        progress.sent = 0;
        assert!(text(output.render(Some(progress.clone()))).contains("0/2048 bytes"));
        progress.sent = 2048;
        assert!(!text(output.render(Some(progress.clone()))).contains("done"));
        progress.done = true;
        assert!(text(output.render(Some(progress))).ends_with(" done\r\n"));
    }

    #[test]
    fn coalesced_tftp_progress_preserves_all_hashes_and_completion() {
        let (tx, mut rx) = watch::channel(None);
        let mut output = ProgressOutput::default();
        assert!(output.render(rx.borrow_and_update().clone()).is_none());
        let mut progress = TransferProgress {
            sent: 0,
            total: None,
            done: false,
        };
        tx.send_replace(Some(progress.clone()));
        let mut rendered = text(output.render(rx.borrow_and_update().clone()));
        // A slow consumer skips intermediate updates, including startup.
        for hashes in 1..=3 {
            progress.sent = hashes;
            tx.send_replace(Some(progress.clone()));
        }
        rendered.push_str(&text(output.render(rx.borrow_and_update().clone())));
        progress.sent = 5;
        progress.done = true;
        tx.send_replace(Some(progress.clone()));
        rendered.push_str(&text(output.render(rx.borrow_and_update().clone())));
        assert_eq!(rendered, "Uploading image (TFTP) [#####] done\r\n");
        assert!(output.render(Some(progress.clone())).is_none());
        // Completion may also be the first state observed by the consumer.
        assert_eq!(
            text(ProgressOutput::default().render(Some(progress))),
            rendered
        );
    }
}
