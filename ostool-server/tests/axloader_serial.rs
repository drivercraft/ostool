//! Real PTY, HTTP and WebSocket coverage of automatic serial ownership.
#![cfg(target_os = "linux")]
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use futures_util::{SinkExt, StreamExt};
use httpboot_protocol::*;
use ostool_serial::{PortLocator, SerialManager};
#[path = "common/serial.rs"]
mod serial_fixture;
use ostool_server::{session::SessionBootCommand, tftp::service::build_tftp_manager, *};
use serial_fixture::PtyBackend;
use serialport::{SerialPort, TTYPort};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, mpsc},
    time::Instant,
};
use tokio_tungstenite::tungstenite::Message;

fn actual_baud(file: &std::fs::File) -> u32 {
    use std::os::fd::AsRawFd;
    let mut settings = std::mem::MaybeUninit::<nix::libc::termios2>::uninit();
    // SAFETY: a live PTY descriptor and aligned output; TCGETS2 initializes the
    // complete object on success and retains no pointer.
    assert_eq!(
        unsafe { nix::libc::ioctl(file.as_raw_fd(), nix::libc::TCGETS2, settings.as_mut_ptr()) },
        0
    );
    // SAFETY: the successful ioctl initialized all fields.
    unsafe { settings.assume_init() }.c_ospeed
}

struct Device {
    status: LoaderDeviceStatus,
    output: tokio_serial::SerialStream,
    starts: usize,
}
type DeviceState = Arc<Mutex<Device>>;
async fn status(State(d): State<DeviceState>) -> Json<LoaderDeviceStatus> {
    Json(d.lock().await.status.clone())
}
async fn grant(
    State(d): State<DeviceState>,
    h: HeaderMap,
    Json(b): Json<SerialBinding>,
) -> StatusCode {
    let mut d = d.lock().await;
    if header(&h, "X-Boot-Epoch") != Some(d.status.boot_epoch.as_str()) {
        return StatusCode::CONFLICT;
    }
    if d.status.serial.as_mut().unwrap().grant(b).is_err() {
        return StatusCode::CONFLICT;
    }
    StatusCode::OK
}
async fn start(State(d): State<DeviceState>, h: HeaderMap) -> StatusCode {
    let mut d = d.lock().await;
    if header(&h, "X-Boot-Epoch") != Some(d.status.boot_epoch.as_str())
        || !d
            .status
            .serial
            .as_ref()
            .unwrap()
            .permits_start(header(&h, "X-Serial-Binding").unwrap_or(""))
    {
        return StatusCode::CONFLICT;
    }
    d.starts += 1;
    d.output
        .write_all(b"KERNEL_OUTPUT_AFTER_CONTINUE\n")
        .await
        .unwrap();
    StatusCode::ACCEPTED
}
fn header<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}
fn parameters(baud: u64) -> SerialParameters {
    SerialParameters {
        baud_rate: baud,
        data_bits: 8,
        parity: SerialParity::None,
        stop_bits: SerialStopBits::One,
        flow_control: SerialFlowControl::None,
    }
}
fn serial_status(n: u8, baud: u64) -> LoaderSerialStatus {
    LoaderSerialStatus {
        serial_id: format!("{n:032x}"),
        ready: true,
        parameters: Some(parameters(baud)),
        binding: None,
        error: None,
    }
}
fn locator(path: &str) -> PortLocator {
    PortLocator {
        name: path.into(),
        aliases: vec![format!("alias:{path}")],
        serial_number: None,
    }
}
async fn wait_config(
    rx: &mut mpsc::UnboundedReceiver<(String, SerialParameters)>,
    path: &str,
    baud: u64,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let (port, p) = rx.recv().await.unwrap();
            if port == path && p.baud_rate == baud {
                break;
            }
        }
    })
    .await
    .unwrap();
}
async fn wait_snapshot(
    manager: &SerialManager,
    check: impl Fn(&ostool_serial::ManagerSnapshot) -> bool,
) {
    let mut snapshot = manager.subscribe();
    tokio::time::timeout(Duration::from_secs(3), snapshot.wait_for(check))
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serial_null_session_binds_reuses_uart_and_releases_real_reader() {
    let dir = tempfile::tempdir().unwrap();
    let mac = "02:00:00:00:00:42".parse().unwrap();
    let (master, mut slave) = TTYPort::pair().unwrap();
    slave.set_exclusive(false).unwrap();
    slave.set_baud_rate(38400).unwrap();
    let path = slave.name().unwrap();
    drop(slave);
    let inspection = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let original = nix::sys::termios::tcgetattr(&inspection).unwrap();
    let original_baud = actual_baud(&inspection);
    let (unused_master, mut unused_slave) = TTYPort::pair().unwrap();
    unused_slave.set_exclusive(false).unwrap();
    let unused_path = unused_slave.name().unwrap();
    drop(unused_slave);
    let (relay_master, mut relay_slave) = TTYPort::pair().unwrap();
    relay_slave.set_exclusive(false).unwrap();
    let relay_path = relay_slave.name().unwrap();
    drop(relay_slave);
    let (configured, mut configs) = mpsc::unbounded_channel();
    let backend = Arc::new(PtyBackend {
        ports: vec![locator(&path), locator(&unused_path), locator(&relay_path)],
        configured,
    });
    let config_path = dir.path().join("server.toml");
    let mut config = ServerConfig::default_for_path(&config_path);
    config.data_dir = dir.path().join("data");
    config.board_dir = dir.path().join("boards");
    config.dtb_dir = dir.path().join("dtbs");
    config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
        dir.path().join("files"),
    ));
    let mut state = build_app_state(
        config_path,
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    state.serial_manager = SerialManager::new(backend);
    let board = BoardConfig {
        id: "auto".into(),
        board_type: "uefi".into(),
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
    board.validate().unwrap();
    state.boards.write().await.insert(board.id.clone(), board);
    let relay = BoardConfig {
        id: "relay".into(),
        board_type: "relay".into(),
        tags: vec![],
        serial: None,
        power_management: PowerManagementConfig::ZhongshengRelay(ZhongshengRelayPowerManagement {
            key: SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: relay_path.clone(),
            },
        }),
        boot: BootConfig::Pxe(PxeProfile::default()),
        network_identity: None,
        notes: None,
        disabled: false,
    };
    state.boards.write().await.insert(relay.id.clone(), relay);
    state.sync_board_runtime_states().await;
    state.refresh_serial_exclusions().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();
    let app = build_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let device = Arc::new(Mutex::new(Device {
        status: LoaderDeviceStatus {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: format!("{:032x}", 1),
            mac_address: mac,
            current_mac_address: mac,
            arch: BootArch::X86_64,
            loader_version: "test-v6".into(),
            hardware: LoaderHardwareInfo::default(),
            boot: None,
            ota: None,
            serial: Some(serial_status(1, 57600)),
        },
        output: tokio_serial::SerialStream::try_from(master).unwrap(),
        starts: 0,
    }));
    let fake = Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/serial/continue", post(grant))
        .route("/api/v1/boot/jobs", post(|| async { StatusCode::CREATED }))
        .route(
            "/api/v1/boot/jobs/{id}/kernel",
            put(|| async { StatusCode::OK }),
        )
        .route("/api/v1/boot/jobs/{id}/start", post(start))
        .with_state(device.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let device_port = listener.local_addr().unwrap().port();
    let http = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    let created = state
        .create_session_with_board_id("uefi", "auto", &[], None)
        .await
        .unwrap();
    let session = state.session_state(&created.id).await.unwrap();
    state
        .tftp_manager
        .read()
        .await
        .put_session_file(&created.id, "kernel.elf", b"kernel")
        .await
        .unwrap();
    session
        .publish_boot_command(SessionBootCommand {
            boot_id: "kernel".into(),
            kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
            kernel_size: 6,
            kernel_sha256: format!("{:x}", Sha256::digest(b"kernel")),
            arch: BootArch::X86_64,
            image_format: ImageFormat::Elf64,
            entry_symbol: None,
            initramfs: None,
            cmdline: None,
        })
        .await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{server_addr}/api/v1/sessions/{}/serial/ws",
        created.id
    ))
    .await
    .unwrap();
    assert!(matches!(ws.next().await.unwrap().unwrap(),Message::Text(t) if t.contains("opened")));
    // Wait for the linked power action before advertising the first epoch.
    let limit = Instant::now() + Duration::from_secs(3);
    while *session.subscribe_boot_generation().borrow() == 0 {
        assert!(Instant::now() < limit);
        tokio::task::yield_now().await;
    }
    for (n, baud) in [(1, 57600), (2, 115200)] {
        if n == 2 {
            state
                .execute_board_power_action(session.board(), ostool_server::power::PowerAction::On)
                .await
                .unwrap();
        }
        {
            let mut d = device.lock().await;
            d.status.boot_epoch = format!("{n:032x}");
            d.status.serial = Some(serial_status(n, baud));
        }
        let announcement = LoaderAnnouncement {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: BootArch::X86_64,
            loader_version: "test-v6".into(),
            boot_epoch: format!("{n:032x}"),
            http_port: device_port,
            serial_id: Some(format!("{n:032x}")),
            serial_ready: true,
        };
        let s = state.clone();
        let operation = tokio::spawn(async move {
            ostool_server::device::reconcile(s, announcement, "127.0.0.1:12345".parse().unwrap())
                .await
        });
        wait_config(&mut configs, &path, baud).await;
        device
            .lock()
            .await
            .output
            .write_all(format!("\r\nAXLOADER-SERIAL/1 {n:032x}\r\n").as_bytes())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(4), operation)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Message::Binary(bytes) = ws.next().await.unwrap().unwrap() {
                    output.extend(bytes);
                    if output.ends_with(b"KERNEL_OUTPUT_AFTER_CONTINUE\n") {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(String::from_utf8_lossy(&output).contains(&format!("AXLOADER-SERIAL/1 {n:032x}")));
        assert_eq!(
            session.serial_runtime.snapshot().phase,
            ostool_server::serial::runtime::SerialRuntimePhase::Bound
        );
        wait_snapshot(&state.serial_manager, |s| {
            s.pending == 0 && s.candidates == 0 && s.leased == 1
        })
        .await;
        assert_eq!(u64::from(actual_baud(&inspection)), baud);
    }
    assert_eq!(device.lock().await.starts, 2);
    while let Ok((port, _)) = configs.try_recv() {
        assert_ne!(port, relay_path);
        assert_ne!(port, path, "bound UART must not reopen on second boot");
    }
    ws.send(Message::Close(None)).await.unwrap();
    drop(ws);
    tokio::time::timeout(
        Duration::from_secs(3),
        state.serial_manager.wait_owner_released(&created.id),
    )
    .await
    .unwrap()
    .unwrap();
    wait_snapshot(&state.serial_manager, |s| {
        s.leased == 0 && s.candidates == 0
    })
    .await;
    let restored = nix::sys::termios::tcgetattr(&inspection).unwrap();
    assert_eq!(restored.control_flags, original.control_flags);
    assert_eq!(restored.local_flags, original.local_flags);
    assert_eq!(actual_baud(&inspection), original_baud);
    server.abort();
    http.abort();
    drop(unused_master);
    drop(relay_master);
}
