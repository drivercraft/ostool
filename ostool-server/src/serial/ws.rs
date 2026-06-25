use std::time::Duration;

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::{Sink, SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tokio_serial::{ClearBuffer, SerialPort, SerialPortBuilderExt};

use crate::{
    config::BoardConfig,
    power::{PowerAction, PowerActionError},
    serial::discovery::resolve_serial_config,
    session::SessionState,
    state::AppState,
};

const SERIAL_READ_BUFFER_SIZE: usize = 64;
const SERIAL_READ_TIMEOUT: Duration = Duration::from_millis(20);

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
    let resolved_serial = resolve_serial_config(serial)?;
    let mut port = tokio_serial::new(&resolved_serial.current_device_path, serial.baud_rate)
        .timeout(SERIAL_READ_TIMEOUT)
        .open_native_async()
        .with_context(|| {
            format!(
                "failed to open serial port {}",
                resolved_serial.current_device_path
            )
        })?;
    clear_serial_input_after_open(&session_id, &mut port);

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (mut serial_rx, mut serial_tx) = tokio::io::split(port);
    let mut serial_buffer = [0u8; SERIAL_READ_BUFFER_SIZE];
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
    let mut port = serial_rx.unsplit(serial_tx);
    let result = preserve_result_after_serial_cleanup(&session_id, result, &mut port).await;
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

trait SerialOpenCleanup {
    fn clear_input_buffer(&mut self) -> std::io::Result<()>;
}

impl SerialOpenCleanup for tokio_serial::SerialStream {
    fn clear_input_buffer(&mut self) -> std::io::Result<()> {
        self.clear(ClearBuffer::Input).map_err(std::io::Error::from)
    }
}

fn clear_serial_input_after_open<T>(session_id: &str, port: &mut T)
where
    T: SerialOpenCleanup + ?Sized,
{
    if let Err(err) = port.clear_input_buffer() {
        log::warn!("session `{session_id}` failed to clear serial input after open: {err}");
    }
}

async fn write_serial_payload(
    port: &mut tokio::io::WriteHalf<tokio_serial::SerialStream>,
    payload: &[u8],
) -> anyhow::Result<()> {
    port.write_all(payload).await?;
    port.flush().await?;
    Ok(())
}

#[async_trait::async_trait]
trait SerialQueueCleanup {
    async fn flush_output(&mut self) -> std::io::Result<()>;
    fn clear_all_buffers(&mut self) -> std::io::Result<()>;
}

#[async_trait::async_trait]
impl SerialQueueCleanup for tokio_serial::SerialStream {
    async fn flush_output(&mut self) -> std::io::Result<()> {
        AsyncWriteExt::flush(self).await
    }

    fn clear_all_buffers(&mut self) -> std::io::Result<()> {
        self.clear(ClearBuffer::All).map_err(std::io::Error::from)
    }
}

async fn cleanup_serial_queue_before_close<T>(port: &mut T) -> anyhow::Result<()>
where
    T: SerialQueueCleanup + ?Sized,
{
    port.flush_output()
        .await
        .context("failed to flush serial output before close")?;
    port.clear_all_buffers()
        .context("failed to clear serial buffers before close")?;
    Ok(())
}

async fn preserve_result_after_serial_cleanup<T, P>(
    session_id: &str,
    result: anyhow::Result<T>,
    port: &mut P,
) -> anyhow::Result<T>
where
    P: SerialQueueCleanup + ?Sized,
{
    if let Err(err) = cleanup_serial_queue_before_close(port).await {
        log::warn!("session `{session_id}` failed to clean serial queue before close: {err:#}");
    }
    result
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
        time::Duration,
    };

    use axum::extract::ws::Message;
    use futures_util::Sink;
    use tempfile::tempdir;

    use super::{
        ClientControlMessage, SerialOpenCleanup, SerialQueueCleanup, cleanup_power_link,
        cleanup_serial_queue_before_close, clear_serial_input_after_open,
        finalize_power_linked_session, preserve_result_after_serial_cleanup,
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum CleanupEvent {
        ClearInput,
        Flush,
        ClearAll,
    }

    struct RecordingSerialOpenCleanup {
        events: Arc<Mutex<Vec<CleanupEvent>>>,
        clear_result: io::Result<()>,
    }

    struct RecordingSerialCleanup {
        events: Arc<Mutex<Vec<CleanupEvent>>>,
        clear_result: io::Result<()>,
    }

    impl SerialOpenCleanup for RecordingSerialOpenCleanup {
        fn clear_input_buffer(&mut self) -> io::Result<()> {
            self.events.lock().unwrap().push(CleanupEvent::ClearInput);
            self.clear_result
                .as_ref()
                .map(|_| ())
                .map_err(|err| io::Error::new(err.kind(), err.to_string()))
        }
    }

    #[async_trait::async_trait]
    impl SerialQueueCleanup for RecordingSerialCleanup {
        async fn flush_output(&mut self) -> io::Result<()> {
            self.events.lock().unwrap().push(CleanupEvent::Flush);
            Ok(())
        }

        fn clear_all_buffers(&mut self) -> io::Result<()> {
            self.events.lock().unwrap().push(CleanupEvent::ClearAll);
            self.clear_result
                .as_ref()
                .map(|_| ())
                .map_err(|err| io::Error::new(err.kind(), err.to_string()))
        }
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

    #[test]
    fn serial_open_cleanup_clears_only_input_buffer() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut cleanup = RecordingSerialOpenCleanup {
            events: events.clone(),
            clear_result: Ok(()),
        };

        clear_serial_input_after_open("session-1", &mut cleanup);

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[CleanupEvent::ClearInput]
        );
    }

    #[test]
    fn serial_open_cleanup_does_not_fail_session_on_clear_error() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut cleanup = RecordingSerialOpenCleanup {
            events: events.clone(),
            clear_result: Err(io::Error::other("clear failed")),
        };

        clear_serial_input_after_open("session-1", &mut cleanup);

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[CleanupEvent::ClearInput]
        );
    }

    #[tokio::test]
    async fn serial_cleanup_flushes_before_clearing_all_buffers() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut cleanup = RecordingSerialCleanup {
            events: events.clone(),
            clear_result: Ok(()),
        };

        cleanup_serial_queue_before_close(&mut cleanup)
            .await
            .unwrap();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[CleanupEvent::Flush, CleanupEvent::ClearAll]
        );
    }

    #[tokio::test]
    async fn serial_cleanup_reports_clear_failures() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut cleanup = RecordingSerialCleanup {
            events: events.clone(),
            clear_result: Err(io::Error::other("clear failed")),
        };

        let err = cleanup_serial_queue_before_close(&mut cleanup)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("failed to clear serial buffers"));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[CleanupEvent::Flush, CleanupEvent::ClearAll]
        );
    }

    #[tokio::test]
    async fn serial_cleanup_failure_preserves_original_session_error() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut cleanup = RecordingSerialCleanup {
            events,
            clear_result: Err(io::Error::other("clear failed")),
        };

        let err = preserve_result_after_serial_cleanup::<(), _>(
            "session-1",
            Err(anyhow::anyhow!("websocket failed")),
            &mut cleanup,
        )
        .await
        .unwrap_err();

        assert_eq!(err.to_string(), "websocket failed");
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
