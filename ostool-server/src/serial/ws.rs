use std::time::Duration;

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::{Sink, SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tokio_serial::SerialPortBuilderExt;

use crate::{
    config::BoardConfig,
    power::{PowerAction, PowerActionError},
    session::SessionState,
    state::AppState,
};

#[derive(Debug, Deserialize)]
struct ClientControlMessage {
    #[serde(rename = "type")]
    kind: String,
    encoding: Option<String>,
    data: Option<String>,
}

pub async fn run_serial_ws(
    socket: WebSocket,
    state: AppState,
    session: std::sync::Arc<SessionState>,
) {
    let result = run_serial_ws_inner(socket, &state, session.clone()).await;
    session.clear_serial_connected();
    if let Err(err) = result {
        log::warn!("serial websocket ended with error: {err:#}");
    }
}

async fn run_serial_ws_inner(
    socket: WebSocket,
    state: &AppState,
    session: std::sync::Arc<SessionState>,
) -> anyhow::Result<()> {
    let session_id = session.snapshot().await.id;
    let board = session.board().clone();
    let serial = board
        .serial
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("board has no serial configuration"))?;
    let port = tokio_serial::new(&serial.port, serial.baud_rate)
        .timeout(Duration::from_millis(200))
        .open_native_async()
        .with_context(|| format!("failed to open serial port {}", serial.port))?;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (mut serial_rx, mut serial_tx) = tokio::io::split(port);
    let mut serial_buffer = [0u8; 1024];
    let mut power_on_task = Some(spawn_power_action_task(
        state.clone(),
        board.clone(),
        PowerAction::On,
    ));
    let power_linked = true;
    let mut shutdown_rx = session.subscribe_shutdown();

    ws_sender
        .send(Message::Text(r#"{"type":"opened"}"#.to_string().into()))
        .await
        .ok();
    let result = async {
        loop {
            if let Some(task) = power_on_task.as_mut() {
                tokio::select! {
                    power_result = task => {
                        power_on_task = None;
                        match power_result {
                            Ok(Ok(_)) => {}
                            Ok(Err(err)) => {
                                let message = format!("automatic power-on failed: {err}");
                                log::warn!("session `{session_id}` {message}");
                                send_power_on_failure_and_close(&mut ws_sender, &message).await;
                                break;
                            }
                            Err(err) => {
                                let message = format!("automatic power-on task join failed: {err}");
                                log::warn!("session `{session_id}` {message}");
                                send_power_on_failure_and_close(&mut ws_sender, &message).await;
                                break;
                            }
                        }
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_ok() && *shutdown_rx.borrow() {
                            let _ = ws_sender
                                .send(Message::Text(r#"{"type":"closed"}"#.to_string().into()))
                                .await;
                            break;
                        }
                    }
                    maybe_message = ws_receiver.next() => {
                        let Some(message) = maybe_message else {
                            break;
                        };
                        match message {
                            Ok(Message::Binary(bytes)) => {
                                write_serial_payload(&mut serial_tx, &bytes).await?;
                            }
                            Ok(Message::Text(text)) => {
                                let control: ClientControlMessage = serde_json::from_str(&text)?;
                                match control.kind.as_str() {
                                    "close" => {
                                        let _ = ws_sender
                                            .send(Message::Text(r#"{"type":"closed"}"#.to_string().into()))
                                            .await;
                                        break;
                                    }
                                    "tx" => {
                                        let Some(data) = control.data.as_deref() else {
                                            anyhow::bail!("missing tx data");
                                        };
                                        let payload = match control.encoding.as_deref() {
                                            Some("base64") => base64::engine::general_purpose::STANDARD
                                                .decode(data)
                                                .context("invalid base64 payload")?,
                                            Some("utf8") | None => data.as_bytes().to_vec(),
                                            Some(other) => anyhow::bail!("unsupported encoding `{other}`"),
                                        };
                                        write_serial_payload(&mut serial_tx, &payload).await?;
                                    }
                                    other => anyhow::bail!("unsupported websocket control type `{other}`"),
                                }
                            }
                            Ok(Message::Close(_)) => break,
                            Ok(Message::Ping(payload)) => {
                                ws_sender.send(Message::Pong(payload)).await.ok();
                            }
                            Ok(Message::Pong(_)) => {}
                            Err(err) => return Err(err.into()),
                        }
                        let _ = session.heartbeat().await;
                    }
                    read = serial_rx.read(&mut serial_buffer) => {
                        let read = read.context("serial read failed")?;
                        if read == 0 {
                            break;
                        }
                        ws_sender
                            .send(Message::Binary(serial_buffer[..read].to_vec().into()))
                            .await
                            .context("failed to send serial output over websocket")?;
                        let _ = session.heartbeat().await;
                    }
                }
            } else {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_ok() && *shutdown_rx.borrow() {
                            let _ = ws_sender
                                .send(Message::Text(r#"{"type":"closed"}"#.to_string().into()))
                                .await;
                            break;
                        }
                    }
                    maybe_message = ws_receiver.next() => {
                        let Some(message) = maybe_message else {
                            break;
                        };
                        match message {
                            Ok(Message::Binary(bytes)) => {
                                write_serial_payload(&mut serial_tx, &bytes).await?;
                            }
                            Ok(Message::Text(text)) => {
                                let control: ClientControlMessage = serde_json::from_str(&text)?;
                                match control.kind.as_str() {
                                    "close" => {
                                        let _ = ws_sender
                                            .send(Message::Text(r#"{"type":"closed"}"#.to_string().into()))
                                            .await;
                                        break;
                                    }
                                    "tx" => {
                                        let Some(data) = control.data.as_deref() else {
                                            anyhow::bail!("missing tx data");
                                        };
                                        let payload = match control.encoding.as_deref() {
                                            Some("base64") => base64::engine::general_purpose::STANDARD
                                                .decode(data)
                                                .context("invalid base64 payload")?,
                                            Some("utf8") | None => data.as_bytes().to_vec(),
                                            Some(other) => anyhow::bail!("unsupported encoding `{other}`"),
                                        };
                                        write_serial_payload(&mut serial_tx, &payload).await?;
                                    }
                                    other => anyhow::bail!("unsupported websocket control type `{other}`"),
                                }
                            }
                            Ok(Message::Close(_)) => break,
                            Ok(Message::Ping(payload)) => {
                                ws_sender.send(Message::Pong(payload)).await.ok();
                            }
                            Ok(Message::Pong(_)) => {}
                            Err(err) => return Err(err.into()),
                        }
                        let _ = session.heartbeat().await;
                    }
                    read = serial_rx.read(&mut serial_buffer) => {
                        let read = read.context("serial read failed")?;
                        if read == 0 {
                            break;
                        }
                        ws_sender
                            .send(Message::Binary(serial_buffer[..read].to_vec().into()))
                            .await
                            .context("failed to send serial output over websocket")?;
                        let _ = session.heartbeat().await;
                    }
                }
            }
        }

        Ok(())
    }
    .await;

    let result =
        finalize_power_linked_session(state, &board, power_linked, power_on_task, result).await;
    let _ = state
        .request_session_stop(&session_id, crate::session::SessionStopReason::SerialClosed)
        .await;
    let _ = ws_sender.send(Message::Close(None)).await;
    result
}

fn spawn_power_action_task(
    state: AppState,
    board: BoardConfig,
    action: PowerAction,
) -> JoinHandle<Result<String, PowerActionError>> {
    tokio::spawn(async move { state.execute_board_power_action(&board, action).await })
}

async fn cleanup_power_link(
    board: &BoardConfig,
    power_linked: bool,
    power_on_task: Option<JoinHandle<Result<String, PowerActionError>>>,
) {
    if !power_linked {
        return;
    }

    if let Some(task) = power_on_task {
        match task.await {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                log::warn!(
                    "session `{}` power-on task ended with error: {err}",
                    board.id
                )
            }
            Err(err) => log::warn!("session `{}` power-on task join failed: {err}", board.id),
        }
    }
}

async fn finalize_power_linked_session<T>(
    _state: &AppState,
    board: &BoardConfig,
    power_linked: bool,
    power_on_task: Option<JoinHandle<Result<String, PowerActionError>>>,
    result: anyhow::Result<T>,
) -> anyhow::Result<T> {
    cleanup_power_link(board, power_linked, power_on_task).await;
    result
}

async fn send_power_on_failure_and_close<S>(ws_sender: &mut S, message: &str)
where
    S: Sink<Message> + Unpin,
{
    let payload = serde_json::json!({
        "type": "error",
        "message": message,
    })
    .to_string();
    let _ = ws_sender.send(Message::Text(payload.into())).await;
    let _ = ws_sender
        .send(Message::Text(r#"{"type":"closed"}"#.to_string().into()))
        .await;
    let _ = ws_sender.send(Message::Close(None)).await;
}

async fn write_serial_payload(
    port: &mut tokio::io::WriteHalf<tokio_serial::SerialStream>,
    payload: &[u8],
) -> anyhow::Result<()> {
    port.write_all(payload).await?;
    port.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };

    use axum::extract::ws::Message;
    use futures_util::Sink;
    use tempfile::tempdir;

    use super::{
        ClientControlMessage, cleanup_power_link, finalize_power_linked_session,
        send_power_on_failure_and_close,
    };
    use crate::{
        build_app_state,
        config::{
            BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
            PowerManagementConfig, PxeProfile, ServerConfig, TftpConfig,
        },
        power::PowerActionError,
        tftp::service::{TftpManager, build_tftp_manager},
    };

    #[derive(Default)]
    struct VecSink {
        messages: Vec<Message>,
    }

    impl Sink<Message> for VecSink {
        type Error = ();

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.get_mut().messages.push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn control_message_parses_close_type() {
        let message: ClientControlMessage = serde_json::from_str(r#"{"type":"close"}"#).unwrap();
        assert_eq!(message.kind, "close");
    }

    async fn test_state(root: &std::path::Path) -> crate::AppState {
        let config_path = root.join(".ostool-server.toml");
        let config = ServerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: root.join("data"),
            board_dir: root.join("boards"),
            dtb_dir: root.join("dtbs"),
            tftp: TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(root.join("tftp"))),
            ..ServerConfig::default()
        };
        let manager: std::sync::Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        build_app_state(config_path, config, manager).await.unwrap()
    }

    #[tokio::test]
    async fn cleanup_waits_for_power_on_task_before_power_off() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("power.log");
        let board = BoardConfig {
            id: "demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: String::new(),
                power_off_cmd: format!("printf 'off\\n' >> {}", output_path.display()),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        };

        let power_on_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            fs::write(&output_path, "on\n").unwrap();
            Ok::<String, PowerActionError>("executed".into())
        });

        cleanup_power_link(&board, true, Some(power_on_task)).await;

        let content = fs::read_to_string(dir.path().join("power.log")).unwrap();
        assert_eq!(content, "on\n");
    }

    #[tokio::test]
    async fn finalize_runs_power_off_even_when_session_errors() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("power.log");
        let state = test_state(dir.path()).await;
        let board = BoardConfig {
            id: "demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: String::new(),
                power_off_cmd: format!("printf 'off\\n' >> {}", output_path.display()),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        };

        let power_on_task =
            tokio::spawn(async { Ok::<String, PowerActionError>("executed".into()) });
        let result = finalize_power_linked_session::<()>(
            &state,
            &board,
            true,
            Some(power_on_task),
            Err(anyhow::anyhow!("websocket send failed")),
        )
        .await;

        assert!(result.is_err());
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn power_on_failure_sends_error_then_close_messages() {
        let mut sender = VecSink::default();
        send_power_on_failure_and_close(&mut sender, "automatic power-on failed").await;
        let mut messages = sender.messages.into_iter();
        let first = messages.next().unwrap();
        let second = messages.next().unwrap();
        let third = messages.next().unwrap();

        match first {
            Message::Text(text) => assert!(text.contains(r#""type":"error""#)),
            other => panic!("unexpected first message: {other:?}"),
        }
        match second {
            Message::Text(text) => assert_eq!(text, r#"{"type":"closed"}"#),
            other => panic!("unexpected second message: {other:?}"),
        }
        assert!(matches!(third, Message::Close(_)));
    }
}
