#![cfg(unix)]

use std::{
    io::Write,
    net::SocketAddr,
    path::Path,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use ostool_server::{
    BoardConfig, BootConfig, BuiltinTftpConfig, PowerManagementConfig, ServerConfig, TftpConfig,
    UbootProfile, VirtualPowerManagement, build_app_state, build_router,
    tftp::service::{TftpManager, build_tftp_manager},
};
use reqwest::StatusCode;
use serialport::{SerialPort, TTYPort};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

const TEST_BOARD_ID: &str = "virtual-board-1";
const TEST_BOARD_TYPE: &str = "virtual-demo";
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

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BoardPowerAction {
    PowerOn,
    PowerOff,
}

#[derive(Debug, serde::Deserialize)]
struct BoardPowerStatusResponse {
    available: bool,
    powered: Option<bool>,
    last_action: Option<BoardPowerAction>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn sample_virtual_board(serial_port: String) -> BoardConfig {
    BoardConfig {
        id: TEST_BOARD_ID.into(),
        board_type: TEST_BOARD_TYPE.into(),
        tags: vec![],
        serial: Some(ostool_server::SerialConfig {
            port: serial_port,
            baud_rate: TEST_SERIAL_BAUD_RATE,
        }),
        power_management: PowerManagementConfig::Virtual(VirtualPowerManagement::default()),
        boot: BootConfig::Uboot(UbootProfile {
            use_tftp: false,
            dtb_name: None,
        }),
        notes: None,
        disabled: false,
    }
}

fn spawn_test_server(root: &Path, serial_port: String) -> Result<TestServerHandle> {
    let config_path = root.join("config.toml");
    let data_dir = root.join("data");
    let board_dir = root.join("boards");
    let dtb_dir = root.join("dtbs");
    let tftp_root = root.join("tftp-root");

    std::fs::create_dir_all(&board_dir)
        .with_context(|| format!("failed to create {}", board_dir.display()))?;
    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        data_dir,
        board_dir: board_dir.clone(),
        dtb_dir,
        tftp: TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(tftp_root)),
        network: ostool_server::TftpNetworkConfig {
            interface: "lo".into(),
        },
    };
    std::fs::write(&config_path, toml::to_string_pretty(&config)?)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    let board = sample_virtual_board(serial_port);
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
                    "failed to power off board `{}` during test server startup: {}",
                    board_id,
                    err
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

async fn run_client_flow(
    base_url: &str,
    mode: ClientShutdownMode,
    serial_ready_tx: mpsc::Sender<()>,
) -> Result<()> {
    let client = reqwest::Client::new();
    wait_for_server_ready(&client, base_url).await?;

    let initial_status = fetch_power_status(&client, base_url, TEST_BOARD_ID).await?;
    assert!(initial_status.available);
    assert_eq!(initial_status.powered, Some(false));
    assert_eq!(initial_status.last_action, Some(BoardPowerAction::PowerOff));
    assert!(initial_status.updated_at.is_some());

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

    let powered_on = poll_power_status(
        &client,
        base_url,
        TEST_BOARD_ID,
        FAST_ASSERT_TIMEOUT,
        |status| {
            status.available
                && status.powered == Some(true)
                && status.last_action == Some(BoardPowerAction::PowerOn)
        },
    )
    .await?;
    let power_on_time = powered_on.updated_at;

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
            websocket
                .send(Message::Close(None))
                .await
                .context("failed to send websocket close frame")?;
        }
        ClientShutdownMode::AbruptDrop => {
            drop(websocket);
        }
    }

    wait_for_session_release(&client, base_url, &created.session_id).await?;
    let powered_off = poll_power_status(
        &client,
        base_url,
        TEST_BOARD_ID,
        FAST_ASSERT_TIMEOUT,
        |status| {
            status.available
                && status.powered == Some(false)
                && status.last_action == Some(BoardPowerAction::PowerOff)
                && status.updated_at != power_on_time
        },
    )
    .await?;
    assert!(powered_off.updated_at.is_some());

    Ok(())
}

async fn wait_for_server_ready(client: &reqwest::Client, base_url: &str) -> Result<()> {
    poll_power_status(
        client,
        base_url,
        TEST_BOARD_ID,
        Duration::from_secs(5),
        |status| status.available && status.powered == Some(false),
    )
    .await
    .map(|_| ())
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

async fn fetch_power_status(
    client: &reqwest::Client,
    base_url: &str,
    board_id: &str,
) -> Result<BoardPowerStatusResponse> {
    let response = client
        .get(format!(
            "{base_url}/api/v1/admin/boards/{board_id}/power-status"
        ))
        .send()
        .await
        .with_context(|| format!("failed to query power status for {board_id}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read power status body")?;
    if status != StatusCode::OK {
        bail!("unexpected power status {status}: {body}");
    }
    serde_json::from_str(&body).context("failed to parse power status")
}

async fn poll_power_status<F>(
    client: &reqwest::Client,
    base_url: &str,
    board_id: &str,
    timeout: Duration,
    mut predicate: F,
) -> Result<BoardPowerStatusResponse>
where
    F: FnMut(&BoardPowerStatusResponse) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let status = fetch_power_status(client, base_url, board_id).await?;
        if predicate(&status) {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for power status predicate, last status: {:?}",
                status
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
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
                "timed out waiting for session `{session_id}` release, last status: {}, body: {}",
                status,
                body
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
