use std::{
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::{Sink, SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::task::JoinHandle;
#[cfg(not(unix))]
use tokio_serial::SerialPortBuilderExt;
use tokio_serial::{ClearBuffer, SerialPort};

use crate::{
    config::{BoardConfig, SerialConfig, SerialPortKeyKind},
    power::{PowerAction, PowerActionError},
    serial::discovery::resolve_serial_config,
    session::SessionState,
    state::AppState,
};

const SERIAL_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const SERIAL_READ_TIMEOUT: Duration = Duration::from_millis(20);

#[cfg(unix)]
use super::physical::PhysicalSerial;
#[cfg(not(unix))]
type PhysicalSerial = tokio_serial::SerialStream;

enum BoardSerialStream {
    Physical(PhysicalSerial),
    Qemu(tokio::io::DuplexStream),
}

impl AsyncRead for BoardSerialStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Physical(stream) => Pin::new(stream).poll_read(cx, buffer),
            Self::Qemu(stream) => Pin::new(stream).poll_read(cx, buffer),
        }
    }
}

impl AsyncWrite for BoardSerialStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            Self::Physical(stream) => Pin::new(stream).poll_write(cx, bytes),
            Self::Qemu(stream) => Pin::new(stream).poll_write(cx, bytes),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Physical(stream) => Pin::new(stream).poll_flush(cx),
            Self::Qemu(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Physical(stream) => Pin::new(stream).poll_shutdown(cx),
            Self::Qemu(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ClientControlMessage {
    #[serde(rename = "type")]
    pub(super) kind: String,
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
    let mut port = open_board_serial(state, serial).await?;
    clear_serial_input_after_open(&session_id, &mut port);

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (mut serial_rx, mut serial_tx) = tokio::io::split(port);
    let mut power_on_task = Some(spawn_power_action_task(
        state.clone(),
        board.clone(),
        PowerAction::On,
    ));
    let power_linked = true;
    let mut shutdown_rx = session.subscribe_shutdown();

    let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);
    let result = {
        let transport = super::transport::run(
            &mut serial_rx,
            &mut serial_tx,
            &mut ws_sender,
            &mut ws_receiver,
            ready_rx,
            || async {
                let _ = session.heartbeat().await;
            },
        );
        let power_on = async {
            let result = power_on_task.as_mut().expect("power-on task exists").await;
            power_on_task = None;
            result
                .context("automatic power-on task join failed")?
                .map_err(|err| anyhow::anyhow!("automatic power-on failed: {err}"))?;
            ready_tx.send_replace(true);
            std::future::pending::<anyhow::Result<()>>().await
        };
        tokio::select! {
            result = transport => result,
            result = power_on => result,
            _ = shutdown_rx.wait_for(|shutdown| *shutdown) => Ok(()),
        }
    };

    // A stalled WebSocket must not prevent session/serial cleanup. The peer
    // sees an explicit error when writable, otherwise it sees the connection close.
    let _ = tokio::time::timeout(SERIAL_CLOSE_TIMEOUT, async {
        if let Err(err) = &result {
            send_power_on_failure_and_close(&mut ws_sender, &format!("{err:#}")).await;
        } else {
            let _ = ws_sender
                .send(Message::Text(r#"{"type":"closed"}"#.into()))
                .await;
            let _ = ws_sender.send(Message::Close(None)).await;
        }
    })
    .await;

    let result =
        finalize_power_linked_session(state, &board, power_linked, power_on_task, result).await;
    let mut port = serial_rx.unsplit(serial_tx);
    let result = preserve_result_after_serial_cleanup(&session_id, result, &mut port).await;
    let _ = state
        .request_session_stop(&session_id, crate::session::SessionStopReason::SerialClosed)
        .await;
    result
}

async fn open_board_serial(
    state: &AppState,
    serial: &SerialConfig,
) -> anyhow::Result<BoardSerialStream> {
    if serial.key.kind == SerialPortKeyKind::Qemu {
        return Ok(BoardSerialStream::Qemu(
            state
                .virtual_boards
                .attach_serial(&serial.key.value)
                .await?,
        ));
    }

    let resolved_serial = resolve_serial_config(serial)?;
    let builder = tokio_serial::new(&resolved_serial.current_device_path, serial.baud_rate)
        .timeout(SERIAL_READ_TIMEOUT);
    #[cfg(unix)]
    return physical_from_port(builder.open_native()?);
    #[cfg(not(unix))]
    let port = builder.open_native_async().with_context(|| {
        format!(
            "failed to open serial port {}",
            resolved_serial.current_device_path
        )
    })?;
    #[cfg(not(unix))]
    Ok(BoardSerialStream::Physical(port))
}

#[cfg(unix)]
fn physical_from_port(port: serialport::TTYPort) -> anyhow::Result<BoardSerialStream> {
    Ok(BoardSerialStream::Physical(PhysicalSerial::new(port)?))
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

#[cfg(unix)]
impl SerialOpenCleanup for PhysicalSerial {
    fn clear_input_buffer(&mut self) -> std::io::Result<()> {
        // Cleared before the receive worker starts, while the board is off.
        Ok(())
    }
}

impl SerialOpenCleanup for BoardSerialStream {
    fn clear_input_buffer(&mut self) -> std::io::Result<()> {
        match self {
            Self::Physical(stream) => stream.clear_input_buffer(),
            Self::Qemu(_) => Ok(()),
        }
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

pub(super) fn decode_serial_payload(control: ClientControlMessage) -> anyhow::Result<Vec<u8>> {
    let data = control.data.context("missing tx data")?;
    match control.encoding.as_deref() {
        Some("base64") => base64::engine::general_purpose::STANDARD
            .decode(data)
            .context("invalid base64 payload"),
        Some("utf8") | None => Ok(data.into_bytes()),
        Some(other) => anyhow::bail!("unsupported encoding `{other}`"),
    }
}

pub(super) async fn write_serial_payload<T>(port: &mut T, payload: &[u8]) -> anyhow::Result<()>
where
    T: AsyncWrite + Unpin,
{
    // Writes are unbuffered. Do not call tcdrain while sharing the receive lock.
    port.write_all(payload).await?;
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
        // tcdrain is a blocking syscall even through tokio-serial. Observe the
        // driver's output queue without parking a runtime worker indefinitely.
        wait_output_empty(
            || self.bytes_to_write().map_err(std::io::Error::from),
            SERIAL_CLOSE_TIMEOUT,
        )
        .await
    }

    fn clear_all_buffers(&mut self) -> std::io::Result<()> {
        self.clear(ClearBuffer::All).map_err(std::io::Error::from)
    }
}

#[cfg(unix)]
#[async_trait::async_trait]
impl SerialQueueCleanup for PhysicalSerial {
    async fn flush_output(&mut self) -> std::io::Result<()> {
        self.stop_reader();
        self.writer.flush_output().await
    }
    fn clear_all_buffers(&mut self) -> std::io::Result<()> {
        self.writer.clear_all_buffers()
    }
}

#[async_trait::async_trait]
impl SerialQueueCleanup for BoardSerialStream {
    async fn flush_output(&mut self) -> std::io::Result<()> {
        match self {
            Self::Physical(stream) => stream.flush_output().await,
            Self::Qemu(stream) => stream.flush().await,
        }
    }

    fn clear_all_buffers(&mut self) -> std::io::Result<()> {
        match self {
            Self::Physical(stream) => stream.clear_all_buffers(),
            Self::Qemu(_) => Ok(()),
        }
    }
}

async fn wait_output_empty(
    mut bytes_to_write: impl FnMut() -> std::io::Result<u32>,
    timeout: Duration,
) -> std::io::Result<()> {
    tokio::time::timeout(timeout, async {
        while bytes_to_write()? != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        Ok(())
    })
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "serial output queue did not drain",
        )
    })?
}

async fn cleanup_serial_queue_before_close<T>(port: &mut T) -> anyhow::Result<()>
where
    T: SerialQueueCleanup + ?Sized,
{
    let drain = port
        .flush_output()
        .await
        .context("failed to drain serial output before close");
    let clear = port
        .clear_all_buffers()
        .context("failed to clear serial buffers before close");
    // Even a stuck transmitter must be purged before its lease is released.
    drain.and(clear)
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

    struct DrainMustNotBeCalled(Vec<u8>);

    impl tokio::io::AsyncRead for DrainMustNotBeCalled {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl tokio::io::AsyncWrite for DrainMustNotBeCalled {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.0.extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            panic!(
                "serial payload must not synchronously drain the UART while holding the shared read/write lock"
            );
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn serial_payload_does_not_block_receive_on_uart_drain() {
        let (rx, mut tx) = tokio::io::split(DrainMustNotBeCalled(Vec::new()));
        super::write_serial_payload(&mut tx, b"command\n")
            .await
            .unwrap();
        assert_eq!(rx.unsplit(tx).0, b"command\n");
    }

    #[tokio::test]
    async fn serial_close_drain_is_bounded() {
        let err = super::wait_output_empty(|| Ok(1), Duration::ZERO)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        super::wait_output_empty(|| Ok(0), Duration::from_secs(1))
            .await
            .unwrap();
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
    async fn serial_cleanup_clears_buffers_even_when_drain_fails() {
        struct StuckTransmitter(bool);
        #[async_trait::async_trait]
        impl SerialQueueCleanup for StuckTransmitter {
            async fn flush_output(&mut self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::TimedOut, "stuck transmitter"))
            }
            fn clear_all_buffers(&mut self) -> io::Result<()> {
                self.0 = true;
                Ok(())
            }
        }
        let mut port = StuckTransmitter(false);
        let err = cleanup_serial_queue_before_close(&mut port)
            .await
            .unwrap_err();
        assert!(port.0, "failed drain must not skip purging queued commands");
        assert!(err.to_string().contains("failed to drain serial output"));
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
            network_identity: None,
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
            network_identity: None,
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

#[cfg(all(test, unix))]
mod reader_progress_tests {
    use serialport::SerialPort;
    use std::{io::Write, time::Duration};
    use tokio::io::AsyncReadExt;

    #[tokio::test(flavor = "current_thread")]
    async fn physical_receive_survives_a_blocked_websocket_executor() {
        let (mut board, mut port) = serialport::TTYPort::pair().unwrap();
        board.set_timeout(Duration::from_millis(200)).unwrap();
        port.set_timeout(super::SERIAL_READ_TIMEOUT).unwrap();
        let mut serial = super::physical_from_port(port).unwrap();
        let payload: Vec<u8> = (0..65536).map(|i| (i % 251) as u8).collect();
        let expected = payload.clone();
        let (sent, finished) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            let result = payload
                .chunks(128)
                .try_for_each(|chunk| board.write_all(chunk));
            sent.send(result).unwrap();
            // Keep the PTY open until the receiver has consumed buffered bytes.
            board
        });
        // Deliberately block the only Tokio worker. More bytes than the PTY
        // queue can hold must be drained before this executor runs again.
        let result = finished.recv_timeout(Duration::from_secs(2)).unwrap();
        let _board = writer.join().unwrap();
        assert!(
            result.is_ok(),
            "physical RX depends on the blocked executor: {result:?}"
        );
        let mut received = vec![0; expected.len()];
        tokio::time::timeout(Duration::from_secs(2), serial.read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, expected);
    }
    #[tokio::test(flavor = "current_thread")]
    async fn physical_receive_overflow_is_reported_after_buffered_bytes() {
        let (mut board, mut port) = serialport::TTYPort::pair().unwrap();
        board.set_timeout(Duration::from_millis(200)).unwrap();
        port.set_timeout(super::SERIAL_READ_TIMEOUT).unwrap();
        let mut serial = super::physical_from_port(port).unwrap();
        let writer = std::thread::spawn(move || {
            // Deliberately exceed the receive bound without polling AsyncRead.
            let _ = (0..4096).try_for_each(|_| board.write_all(&[0x5a; 128]));
            board
        });
        let _board = writer.join().unwrap();
        let mut bytes = Vec::new();
        let error = tokio::time::timeout(Duration::from_secs(2), serial.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap_err();
        assert!(
            error.to_string().contains("physical serial RX buffer full"),
            "{error}"
        );
        assert!(!bytes.is_empty());
        assert!(bytes.len() <= 256 * 1024);
        assert!(bytes.iter().all(|byte| *byte == 0x5a));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_physical_receive_joins_reader_before_reusing_tty() {
        use std::io::Read;
        let (mut board, mut port) = serialport::TTYPort::pair().unwrap();
        board.set_timeout(Duration::from_millis(200)).unwrap();
        port.set_timeout(super::SERIAL_READ_TIMEOUT).unwrap();
        let mut observer = port.try_clone_native().unwrap();
        let mut serial = super::physical_from_port(port).unwrap();
        board.write_all(b"first").unwrap();
        let mut first = [0; 5];
        serial.read_exact(&mut first).await.unwrap();
        assert_eq!(&first, b"first");
        drop(serial);
        board.write_all(b"next").unwrap();
        let mut next = [0; 4];
        observer.read_exact(&mut next).unwrap();
        assert_eq!(&next, b"next");
    }
}
