//! Integration tests for serial WebSocket session lifecycle.
#![cfg(unix)]

use std::{
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use ostool_server::{
    BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement, PowerManagementConfig,
    SerialConfig, SerialPortKey, SerialPortKeyKind, ServerConfig, TftpConfig, UbootProfile,
    UploadLimitsConfig, build_app_state, build_router,
    tftp::service::{TftpManager, build_tftp_manager},
};
use reqwest::StatusCode;
use serialport::{SerialPort, TTYPort};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

const TEST_BOARD_ID: &str = "custom-board-1";
const TEST_BOARD_TYPE: &str = "custom-demo";
const TEST_SERIAL_BAUD_RATE: u32 = 115_200;
const EXPECTED_SERIAL_PAYLOAD: &[u8] = b"hello from board\n";
const FAST_ASSERT_TIMEOUT: Duration = Duration::from_millis(800);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy)]
enum ClientShutdownMode {
    GracefulClose,
    AbruptDrop,
}

struct TestServerHandle {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: thread::JoinHandle<Result<()>>,
}

impl TestServerHandle {
    fn shutdown(mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.join
            .join()
            .map_err(|_| anyhow!("server thread panicked"))?
    }
}

#[derive(Debug, serde::Deserialize)]
struct SessionCreatedResponse {
    session_id: String,
    board_id: String,
    ws_url: Option<String>,
}

fn sample_board_with_power_on(serial_port: String, power_on_cmd: String) -> BoardConfig {
    BoardConfig {
        id: TEST_BOARD_ID.into(),
        board_type: TEST_BOARD_TYPE.into(),
        tags: vec![],
        serial: Some(SerialConfig {
            key: SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: serial_port,
            },
            baud_rate: TEST_SERIAL_BAUD_RATE,
            resolved_device_path: None,
            resolved_usb_path: None,
        }),
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd,
            power_off_cmd: "true".into(),
        }),
        boot: BootConfig::Uboot(UbootProfile {
            use_tftp: false,
            dtb_name: None,
            ..Default::default()
        }),
        notes: None,
        disabled: false,
    }
}

/// Starts an in-process ostool-server with one board and PTY serial port.
fn spawn_test_server(root: &Path, serial_port: String) -> Result<TestServerHandle> {
    spawn_test_server_with_power_on(root, serial_port, "true".into())
}

fn spawn_test_server_with_power_on(
    root: &Path,
    serial_port: String,
    power_on_cmd: String,
) -> Result<TestServerHandle> {
    let config_path = root.join("config.toml");
    let data_dir = root.join("data");
    let board_dir = root.join("boards");
    let dtb_dir = root.join("dtbs");
    let tftp_root = root.join("tftp-root");
    let http_boot_root = root.join("http-boot");

    std::fs::create_dir_all(&board_dir)
        .with_context(|| format!("failed to create {}", board_dir.display()))?;
    let mut tftp = BuiltinTftpConfig::default_with_root(tftp_root);
    tftp.enabled = false;

    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        data_dir,
        board_dir: board_dir.clone(),
        dtb_dir,
        tftp: TftpConfig::Builtin(tftp),
        http_boot: ostool_server::config::HttpBootConfig::default_with_root(http_boot_root),
        network: ostool_server::TftpNetworkConfig {
            interface: "lo".into(),
        },
        upload_limits: UploadLimitsConfig::default(),
    };
    std::fs::write(&config_path, toml::to_string_pretty(&config)?)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    let board = sample_board_with_power_on(serial_port, power_on_cmd);
    let board_path = board_dir.join(format!("{}.toml", board.id));
    std::fs::write(&board_path, toml::to_string_pretty(&board)?)
        .with_context(|| format!("failed to write {}", board_path.display()))?;

    let (addr_tx, addr_rx) = mpsc::channel::<std::result::Result<SocketAddr, String>>();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let config_path_for_thread = config_path.clone();
    let addr_tx_for_start = addr_tx.clone();

    let join = thread::spawn(move || -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build server runtime")?;

        let result: Result<()> = runtime.block_on(async move {
            let config = ServerConfig::load_or_create(&config_path_for_thread).await?;
            let tftp_manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
            let state =
                build_app_state(config_path_for_thread, config, tftp_manager.clone()).await?;
            state.ensure_data_dirs().await?;
            for (board_id, err) in state.power_off_all_boards_on_startup().await {
                log::warn!(
                    "failed to power off board `{board_id}` during test server startup: {err}"
                );
            }
            tftp_manager.start_if_needed().await?;

            let gc_state = state.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if let Err(err) = gc_state.cleanup_expired_sessions().await {
                        log::warn!(
                            "failed to cleanup expired sessions in integration test: {err:#}"
                        );
                    }
                }
            });

            let app = build_router(state.clone());
            let listen_addr = state.config.read().await.listen_addr;
            let listener = tokio::net::TcpListener::bind(listen_addr).await?;
            let local_addr = listener.local_addr()?;
            addr_tx_for_start
                .send(Ok(local_addr))
                .map_err(|_| anyhow!("failed to publish test server listen address"))?;

            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await?;
            Ok(())
        });

        if let Err(err) = &result {
            let _ = addr_tx.send(Err(err.to_string()));
        }
        result
    });

    let addr = match addr_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(addr)) => addr,
        Ok(Err(err)) => return Err(anyhow!("test server failed to start: {err}")),
        Err(_) => return Err(anyhow!("timed out waiting for test server listen address")),
    };

    Ok(TestServerHandle {
        base_url: format!("http://{addr}"),
        shutdown_tx: Some(shutdown_tx),
        join,
    })
}

fn run_delayed_client_write_case() -> Result<()> {
    let temp = tempfile::tempdir().context("failed to create tempdir")?;
    let gate_path = temp.path().join("power-on-ready");
    let power_on_cmd = format!(
        "while [ ! -f '{}' ]; do sleep 0.05; done",
        gate_path.display()
    );
    let (mut serial_master, mut serial_handle) =
        TTYPort::pair().context("failed to create PTY pair")?;
    serial_handle
        .set_exclusive(false)
        .context("failed to disable PTY exclusivity")?;
    serial_master
        .set_timeout(POLL_INTERVAL)
        .context("failed to configure PTY timeout")?;
    let serial_port = serial_handle.name().context("failed to get PTY path")?;
    drop(serial_handle);

    let server = spawn_test_server_with_power_on(temp.path(), serial_port, power_on_cmd)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build client runtime")?;

    let client = reqwest::Client::new();
    let (created, mut websocket) = runtime.block_on(async {
        wait_for_server_ready(&client, &server.base_url).await?;
        let created = create_session(&client, &server.base_url).await?;
        let ws_url = resolve_ws_url(
            &server.base_url,
            created.ws_url.as_deref().context("missing websocket URL")?,
        )?;
        let (mut websocket, _) = tokio_tungstenite::connect_async(ws_url.as_str())
            .await
            .with_context(|| format!("failed to connect websocket {ws_url}"))?;
        wait_for_opened(&mut websocket).await?;
        websocket
            .send(Message::Binary(b"early-input".to_vec().into()))
            .await
            .context("failed to send early websocket input")?;
        Ok::<_, anyhow::Error>((created, websocket))
    })?;

    assert_no_serial_payload(&mut serial_master, Duration::from_millis(300))?;

    std::fs::write(&gate_path, b"ready").context("failed to release power-on gate")?;
    let payload = read_serial_master_payload(&mut serial_master, b"early-input")?;
    assert_eq!(payload, b"early-input");

    runtime.block_on(async {
        websocket
            .send(Message::Text(r#"{"type":"close"}"#.to_string().into()))
            .await
            .context("failed to send websocket close control message")?;
        wait_for_closed(&mut websocket).await?;
        wait_for_session_release(&client, &server.base_url, &created.session_id).await
    })?;
    server.shutdown()
}

fn assert_no_serial_payload(port: &mut TTYPort, duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;
    let mut buffer = [0u8; 64];
    loop {
        match port.read(&mut buffer) {
            Ok(read) if read > 0 => bail!(
                "serial received payload before power-on completed: {:?}",
                &buffer[..read]
            ),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err).context("failed to read PTY while checking early input"),
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
    }
}

fn read_serial_master_payload(port: &mut TTYPort, expected: &[u8]) -> Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut payload = Vec::new();
    let mut buffer = [0u8; 64];
    while Instant::now() < deadline {
        match port.read(&mut buffer) {
            Ok(read) if read > 0 => {
                payload.extend_from_slice(&buffer[..read]);
                if payload.len() >= expected.len() {
                    return Ok(payload);
                }
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err).context("failed to read PTY serial payload"),
        }
    }
    bail!(
        "timed out waiting for PTY serial payload `{}`; got `{}`",
        String::from_utf8_lossy(expected),
        String::from_utf8_lossy(&payload)
    )
}

fn run_ws_lifecycle_case(mode: ClientShutdownMode) -> Result<()> {
    let temp = tempfile::tempdir().context("failed to create tempdir")?;
    let (mut serial_master, mut serial_handle) =
        TTYPort::pair().context("failed to create PTY pair")?;
    serial_handle
        .set_exclusive(false)
        .context("failed to disable PTY exclusivity")?;
    let serial_port = serial_handle.name().context("failed to get PTY path")?;
    drop(serial_handle);

    let server = spawn_test_server(temp.path(), serial_port)?;
    let (serial_ready_tx, serial_ready_rx) = mpsc::channel::<()>();
    let base_url = server.base_url.clone();
    let client_thread = thread::spawn(move || -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build client runtime")?;
        runtime.block_on(run_client_flow(&base_url, mode, serial_ready_tx))
    });

    if let Ok(()) = serial_ready_rx.recv_timeout(Duration::from_secs(3)) {
        serial_master
            .write_all(EXPECTED_SERIAL_PAYLOAD)
            .context("failed to write PTY payload")?;
        serial_master
            .flush()
            .context("failed to flush PTY payload")?;
    }

    let client_result = client_thread
        .join()
        .map_err(|_| anyhow!("client thread panicked"))?;
    let shutdown_result = server.shutdown();

    client_result?;
    shutdown_result
}

/// Drives one client session through power-on, serial I/O, and release assertions.
async fn run_client_flow(
    base_url: &str,
    mode: ClientShutdownMode,
    serial_ready_tx: mpsc::Sender<()>,
) -> Result<()> {
    let client = reqwest::Client::new();
    wait_for_server_ready(&client, base_url).await?;

    let created = create_session(&client, base_url).await?;
    assert_eq!(created.board_id, TEST_BOARD_ID);
    let ws_url = resolve_ws_url(
        base_url,
        created.ws_url.as_deref().context("missing websocket URL")?,
    )?;
    let (mut websocket, _) = tokio_tungstenite::connect_async(ws_url.as_str())
        .await
        .with_context(|| format!("failed to connect websocket {ws_url}"))?;

    wait_for_opened(&mut websocket).await?;

    serial_ready_tx
        .send(())
        .map_err(|_| anyhow!("failed to signal PTY writer"))?;
    let payload = read_binary_payload(&mut websocket).await?;
    assert_eq!(payload, EXPECTED_SERIAL_PAYLOAD);

    match mode {
        ClientShutdownMode::GracefulClose => {
            websocket
                .send(Message::Text(r#"{"type":"close"}"#.to_string().into()))
                .await
                .context("failed to send websocket close control message")?;
            wait_for_closed(&mut websocket).await?;
        }
        ClientShutdownMode::AbruptDrop => {
            drop(websocket);
        }
    }

    wait_for_session_release(&client, base_url, &created.session_id).await?;
    Ok(())
}

async fn wait_for_server_ready(client: &reqwest::Client, base_url: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let response = client
            .get(format!("{base_url}/api/v1/admin/overview"))
            .send()
            .await;
        if matches!(response, Ok(response) if response.status() == StatusCode::OK) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for test server readiness");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn create_session(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<SessionCreatedResponse> {
    let response = client
        .post(format!("{base_url}/api/v1/sessions"))
        .json(&serde_json::json!({
            "board_type": TEST_BOARD_TYPE,
            "required_tags": [],
            "client_name": "integration-test",
        }))
        .send()
        .await
        .context("failed to create session")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read session body")?;
    if status != StatusCode::CREATED {
        bail!("unexpected create session status {status}: {body}");
    }
    serde_json::from_str(&body).context("failed to parse session response")
}

async fn wait_for_opened<S>(websocket: &mut S) -> Result<()>
where
    S: futures_util::Stream<
            Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message =
            tokio::time::timeout(remaining.max(Duration::from_millis(10)), websocket.next())
                .await
                .context("timed out waiting for websocket opened event")?
                .ok_or_else(|| anyhow!("websocket closed before opened event"))?
                .context("failed to read websocket opened event")?;
        match message {
            Message::Text(text) if text.contains(r#""type":"opened""#) => return Ok(()),
            Message::Text(text) if text.contains(r#""type":"error""#) => {
                bail!("received websocket error before opened: {text}");
            }
            Message::Close(frame) => bail!("websocket closed before opened: {frame:?}"),
            _ => {}
        }
    }
}

/// Reads the first binary payload from the serial WebSocket.
async fn read_binary_payload<S>(websocket: &mut S) -> Result<Vec<u8>>
where
    S: futures_util::Stream<
            Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message =
            tokio::time::timeout(remaining.max(Duration::from_millis(10)), websocket.next())
                .await
                .context("timed out waiting for serial payload")?
                .ok_or_else(|| anyhow!("websocket closed before serial payload"))?
                .context("failed to read websocket payload")?;
        match message {
            Message::Binary(bytes) => return Ok(bytes.to_vec()),
            Message::Text(text) if text.contains(r#""type":"error""#) => {
                bail!("received websocket error while waiting for serial payload: {text}");
            }
            Message::Close(frame) => bail!("websocket closed before serial payload: {frame:?}"),
            _ => {}
        }
    }
}

/// Waits until the serial WebSocket reports closed or the connection closes.
async fn wait_for_closed<S>(websocket: &mut S) -> Result<()>
where
    S: futures_util::Stream<
            Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_closed_control = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message =
            tokio::time::timeout(remaining.max(Duration::from_millis(10)), websocket.next())
                .await
                .context("timed out waiting for websocket close")?;
        let Some(message) = message else {
            return if saw_closed_control {
                Ok(())
            } else {
                Err(anyhow!("websocket closed before closed control message"))
            };
        };
        match message.context("failed to read websocket close message")? {
            Message::Text(text) if text.contains(r#""type":"closed""#) => {
                saw_closed_control = true;
            }
            Message::Text(text) if text.contains(r#""type":"error""#) => {
                bail!("received websocket error while waiting for close: {text}");
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
}

async fn wait_for_session_release(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
) -> Result<()> {
    let deadline = Instant::now() + FAST_ASSERT_TIMEOUT;
    loop {
        let response = client
            .get(format!("{base_url}/api/v1/sessions/{session_id}"))
            .send()
            .await
            .with_context(|| format!("failed to query session {session_id}"))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for session `{session_id}` release, last status: {status}, body: {body}"
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn resolve_ws_url(base_url: &str, ws_path: &str) -> Result<reqwest::Url> {
    let base =
        reqwest::Url::parse(base_url).with_context(|| format!("invalid base URL `{base_url}`"))?;
    if ws_path.starts_with("ws://") || ws_path.starts_with("wss://") {
        return reqwest::Url::parse(ws_path)
            .with_context(|| format!("invalid websocket URL `{ws_path}`"));
    }

    let ws_scheme = if base.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    let mut ws_base = base;
    ws_base
        .set_scheme(ws_scheme)
        .map_err(|_| anyhow!("failed to set websocket scheme"))?;
    ws_base
        .join(ws_path)
        .with_context(|| format!("failed to resolve websocket path `{ws_path}`"))
}

#[test]
fn graceful_ws_close_powers_off_and_releases_session() -> Result<()> {
    run_ws_lifecycle_case(ClientShutdownMode::GracefulClose)
}

#[test]
fn abrupt_ws_drop_powers_off_and_releases_session() -> Result<()> {
    run_ws_lifecycle_case(ClientShutdownMode::AbruptDrop)
}

#[test]
fn websocket_buffers_client_serial_input_until_power_on_finishes() -> Result<()> {
    run_delayed_client_write_case()
}
