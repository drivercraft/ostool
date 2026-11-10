//! Real PTY, HTTP and WebSocket coverage of automatic serial ownership.
#![cfg(target_os = "linux")]
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use futures_util::StreamExt;
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
    start_attempts: usize,
    create_attempts: usize,
    upload_attempts: usize,
    delete_attempts: usize,
    reject_start: bool,
    upload_rejection: Option<DeviceOperationError>,
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
    d.start_attempts += 1;
    let expected_version = DEVICE_PROTOCOL_VERSION.to_string();
    assert_eq!(
        header(&h, DEVICE_PROTOCOL_VERSION_HEADER),
        Some(expected_version.as_str())
    );
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
    if d.reject_start {
        d.status.boot.as_mut().unwrap().phase = "ready".into();
        return StatusCode::CONFLICT;
    }
    d.starts += 1;
    if let Some(boot) = d.status.boot.as_mut() {
        boot.phase = "handoff".into();
    }
    d.output
        .write_all(b"KERNEL_OUTPUT_AFTER_CONTINUE\n")
        .await
        .unwrap();
    StatusCode::ACCEPTED
}
async fn create_boot(
    State(d): State<DeviceState>,
    h: HeaderMap,
    Json(manifest): Json<DeviceBootJob>,
) -> (StatusCode, Json<DeviceBootStatus>) {
    let expected_version = DEVICE_PROTOCOL_VERSION.to_string();
    assert_eq!(
        header(&h, DEVICE_PROTOCOL_VERSION_HEADER),
        Some(expected_version.as_str())
    );
    let mut d = d.lock().await;
    d.create_attempts += 1;
    let status = DeviceBootStatus {
        boot_id: manifest.boot_id,
        phase: "receiving".into(),
        kernel_received: false,
        initramfs_received: true,
        last_error: None,
    };
    d.status.boot = Some(status.clone());
    (StatusCode::CREATED, Json(status))
}
async fn upload_kernel(State(d): State<DeviceState>, h: HeaderMap) -> axum::response::Response {
    let mut d = d.lock().await;
    d.upload_attempts += 1;
    let expected_version = DEVICE_PROTOCOL_VERSION.to_string();
    assert_eq!(
        header(&h, DEVICE_PROTOCOL_VERSION_HEADER),
        Some(expected_version.as_str())
    );
    if let Some(error) = d.upload_rejection.clone() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
    }
    if let Some(boot) = d.status.boot.as_mut() {
        boot.kernel_received = true;
        boot.phase = "ready".into();
    }
    StatusCode::OK.into_response()
}
async fn delete_boot(State(d): State<DeviceState>, h: HeaderMap) -> StatusCode {
    let mut d = d.lock().await;
    d.delete_attempts += 1;
    let expected_version = DEVICE_PROTOCOL_VERSION.to_string();
    assert_eq!(
        header(&h, DEVICE_PROTOCOL_VERSION_HEADER),
        Some(expected_version.as_str())
    );
    d.status.boot = None;
    StatusCode::NO_CONTENT
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
    scenario: &str,
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
    .unwrap_or_else(|error| {
        panic!("wait_config timed out for `{scenario}` (path={path}, baud={baud}): {error:?}")
    });
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
            serial_parameters: Some(AxloaderSerialParameters {
                baud_rate: 38_400,
                data_bits: 8,
                parity: AxloaderSerialParity::None,
                stop_bits: AxloaderSerialStopBits::One,
                flow_control: AxloaderSerialFlowControl::None,
            }),
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
            arch: "x86_64".into(),
            loader_version: "test-v7".into(),
            hardware: LoaderHardwareInfo::default(),
            boot: None,
            ota: None,
            serial: Some(serial_status(1, 57600)),
        },
        output: tokio_serial::SerialStream::try_from(master).unwrap(),
        starts: 0,
        start_attempts: 0,
        create_attempts: 0,
        upload_attempts: 0,
        delete_attempts: 0,
        reject_start: false,
        upload_rejection: None,
    }));
    let fake = Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/serial/continue", post(grant))
        .route("/api/v1/boot/jobs", post(create_boot))
        .route("/api/v1/boot/jobs/{id}", delete(delete_boot))
        .route("/api/v1/boot/jobs/{id}/kernel", put(upload_kernel))
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
    let mut kernel = vec![0; 64];
    kernel[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    kernel[16..18].copy_from_slice(&2u16.to_le_bytes());
    kernel[18..20].copy_from_slice(&62_u16.to_le_bytes());
    kernel[20..24].copy_from_slice(&1u32.to_le_bytes());
    kernel[52..54].copy_from_slice(&64u16.to_le_bytes());
    kernel[54..56].copy_from_slice(&56u16.to_le_bytes());
    kernel[58..60].copy_from_slice(&64u16.to_le_bytes());
    state
        .tftp_manager
        .read()
        .await
        .put_session_file(&created.id, "kernel.elf", &kernel)
        .await
        .unwrap();
    session
        .publish_boot_command(SessionBootCommand {
            boot_id: "kernel".into(),
            kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
            kernel_size: kernel.len() as u64,
            kernel_sha256: format!("{:x}", Sha256::digest(&kernel)),
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
    {
        let mut d = device.lock().await;
        d.status.boot_epoch = format!("{:032x}", 0);
        let mut serial = serial_status(0, 57600);
        serial.ready = false;
        serial.error = Some("UART is not ready".into());
        d.status.serial = Some(serial);
    }
    let not_ready = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "x86_64".into(),
        loader_version: "test-v7".into(),
        boot_epoch: format!("{:032x}", 0),
        http_port: device_port,
        serial_id: Some(format!("{:032x}", 0)),
        serial_ready: false,
    };
    let error = ostool_server::device::reconcile(
        state.clone(),
        not_ready,
        "127.0.0.1:12345".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("automatic serial unavailable"));
    {
        let mut d = device.lock().await;
        let mut serial = serial_status(0, 57600);
        serial.serial_id = "not-a-serial-id".into();
        d.status.serial = Some(serial);
    }
    let malformed_identity = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "x86_64".into(),
        loader_version: "test-v7".into(),
        boot_epoch: format!("{:032x}", 0),
        http_port: device_port,
        serial_id: Some("not-a-serial-id".into()),
        serial_ready: true,
    };
    let malformed_error = ostool_server::device::reconcile(
        state.clone(),
        malformed_identity,
        "127.0.0.1:12345".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(format!("{malformed_error:#}").contains("invalid device serial identity"));
    assert_eq!(
        session.serial_runtime.snapshot().phase,
        ostool_server::serial::runtime::SerialRuntimePhase::Recovering
    );
    assert!(!session.is_stop_requested());
    {
        let mut d = device.lock().await;
        d.status.boot_epoch.clear();
        d.status.serial = Some(serial_status(0, 57600));
    }
    let missing_epoch = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "x86_64".into(),
        loader_version: "test-v7".into(),
        boot_epoch: String::new(),
        http_port: device_port,
        serial_id: Some(format!("{:032x}", 0)),
        serial_ready: true,
    };
    let missing_epoch_error = ostool_server::device::reconcile(
        state.clone(),
        missing_epoch,
        "127.0.0.1:12345".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(format!("{missing_epoch_error:#}").contains("invalid device boot epoch"));
    assert!(!session.is_stop_requested());
    session.serial_runtime.restart(true).await;
    let repeated_missing_epoch = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "x86_64".into(),
        loader_version: "test-v7".into(),
        boot_epoch: String::new(),
        http_port: device_port,
        serial_id: Some(format!("{:032x}", 0)),
        serial_ready: true,
    };
    let repeated_error = ostool_server::device::reconcile(
        state.clone(),
        repeated_missing_epoch,
        "127.0.0.1:12345".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(format!("{repeated_error:#}").contains("invalid device boot epoch"));
    assert!(!session.is_stop_requested());
    {
        let mut d = device.lock().await;
        d.status.boot_epoch = format!("{:032x}", 0);
        d.status.serial = Some(serial_status(0, 57600));
        d.status.boot = None;
    }
    for (n, baud) in [(1, 57600), (2, 115200)] {
        if n == 2 {
            state
                .execute_board_power_action(session.board(), ostool_server::power::PowerAction::On)
                .await
                .unwrap();
        }
        session
            .publish_boot_command(SessionBootCommand {
                boot_id: format!("handoff-{n}"),
                kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                kernel_size: kernel.len() as u64,
                kernel_sha256: format!("{:x}", Sha256::digest(&kernel)),
                initramfs: None,
                cmdline: None,
            })
            .await;
        {
            let mut d = device.lock().await;
            d.status.boot_epoch = format!("{n:032x}");
            d.status.serial = Some(serial_status(n, baud));
            if n == 2 {
                d.status.boot = None;
            }
        }
        let announcement = LoaderAnnouncement {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: "x86_64".into(),
            loader_version: "test-v7".into(),
            boot_epoch: format!("{n:032x}"),
            http_port: device_port,
            serial_id: Some(format!("{n:032x}")),
            serial_ready: true,
        };
        let s = state.clone();
        let mut operation = tokio::spawn(async move {
            ostool_server::device::reconcile(s, announcement, "127.0.0.1:12345".parse().unwrap())
                .await
        });
        wait_config(
            &mut configs,
            &path,
            38_400,
            &format!("epoch {n} initial serial configuration"),
        )
        .await;
        if n == 1 {
            // The manager's discovery window is deliberately shorter than the
            // session wait deadline. A missing identity frame must recover the
            // runtime, rather than request session shutdown, so a later
            // broadcast can retry the same boot epoch.
            let error = tokio::time::timeout(Duration::from_secs(10), &mut operation)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(format!("{error:#}").contains("serial binding"));
            assert!(!session.is_stop_requested());
            assert!(!session.is_releasing());
            assert_eq!(
                session.serial_runtime.snapshot().phase,
                ostool_server::serial::runtime::SerialRuntimePhase::Recovering
            );
            assert!(
                state
                    .tftp_manager
                    .read()
                    .await
                    .get_session_file(&created.id, "kernel.elf")
                    .await
                    .unwrap()
                    .is_some()
            );
            wait_snapshot(&state.serial_manager, |s| {
                s.pending == 0 && s.candidates == 0 && s.leased == 0
            })
            .await;

            let retry_state = state.clone();
            let retry_announcement = LoaderAnnouncement {
                protocol_version: DEVICE_PROTOCOL_VERSION,
                mac_address: mac,
                current_mac_address: mac,
                arch: "x86_64".into(),
                loader_version: "test-v7".into(),
                boot_epoch: format!("{n:032x}"),
                http_port: device_port,
                serial_id: Some(format!("{n:032x}")),
                serial_ready: true,
            };
            operation = tokio::spawn(async move {
                ostool_server::device::reconcile(
                    retry_state,
                    retry_announcement,
                    "127.0.0.1:12345".parse().unwrap(),
                )
                .await
            });
            wait_config(
                &mut configs,
                &path,
                38_400,
                &format!("epoch {n} retry after missing serial frame"),
            )
            .await;
        }
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
        assert_eq!(u64::from(actual_baud(&inspection)), 38_400);
        if n == 1 {
            let bound = session.serial_runtime.snapshot();
            let bound_binding_id = bound.binding_id.clone().unwrap();
            assert_eq!(bound.port.as_deref(), Some(path.as_str()));

            // The previous handoff is terminal for its boot ID. Start a new
            // command before probing transient serial unavailability.
            session
                .publish_boot_command(SessionBootCommand {
                    boot_id: "serial-unavailable-probe".into(),
                    kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
                    kernel_size: kernel.len() as u64,
                    kernel_sha256: format!("{:x}", Sha256::digest(&kernel)),
                    initramfs: None,
                    cmdline: None,
                })
                .await;

            // A transient device-side not-ready report must preserve the
            // still-live lease identity in the public runtime status.
            {
                let mut d = device.lock().await;
                let mut serial = serial_status(n, 230400);
                serial.ready = false;
                serial.error = Some("UART is temporarily unavailable".into());
                d.status.serial = Some(serial);
            }
            let unavailable = LoaderAnnouncement {
                protocol_version: DEVICE_PROTOCOL_VERSION,
                mac_address: mac,
                current_mac_address: mac,
                arch: "x86_64".into(),
                loader_version: "test-v7".into(),
                boot_epoch: format!("{n:032x}"),
                http_port: device_port,
                serial_id: Some(format!("{n:032x}")),
                serial_ready: false,
            };
            let unavailable_error = ostool_server::device::reconcile(
                state.clone(),
                unavailable,
                "127.0.0.1:12345".parse().unwrap(),
            )
            .await
            .unwrap_err();
            assert!(format!("{unavailable_error:#}").contains("automatic serial unavailable"));
            let recovering = session.serial_runtime.snapshot();
            assert_eq!(
                recovering.phase,
                ostool_server::serial::runtime::SerialRuntimePhase::Recovering
            );
            assert_eq!(recovering.port.as_deref(), Some(path.as_str()));
            assert_eq!(
                recovering.binding_id.as_deref(),
                Some(bound_binding_id.as_str())
            );
            assert!(!session.is_stop_requested());
            assert!(
                state
                    .tftp_manager
                    .read()
                    .await
                    .get_session_file(&created.id, "kernel.elf")
                    .await
                    .unwrap()
                    .is_some()
            );
            {
                let mut d = device.lock().await;
                d.status.serial = Some(serial_status(n, baud));
            }
        }
    }
    state
        .execute_board_power_action(session.board(), ostool_server::power::PowerAction::On)
        .await
        .unwrap();
    session
        .publish_boot_command(SessionBootCommand {
            boot_id: "uncertain-boot".into(),
            kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
            kernel_size: kernel.len() as u64,
            kernel_sha256: format!("{:x}", Sha256::digest(&kernel)),
            initramfs: None,
            cmdline: None,
        })
        .await;
    {
        let mut d = device.lock().await;
        d.status.boot_epoch = format!("{:032x}", 3);
        d.status.serial = Some(serial_status(3, 230400));
        d.status.boot = None;
        d.reject_start = true;
    }
    let failed_announcement = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "x86_64".into(),
        loader_version: "test-v7".into(),
        boot_epoch: format!("{:032x}", 3),
        http_port: device_port,
        serial_id: Some(format!("{:032x}", 3)),
        serial_ready: true,
    };
    let attempts_before_failure = device.lock().await.start_attempts;
    let failed_reconcile = tokio::spawn(ostool_server::device::reconcile(
        state.clone(),
        failed_announcement,
        "127.0.0.1:12345".parse().unwrap(),
    ));
    wait_config(&mut configs, &path, 38_400, "uncertain handoff epoch 3").await;
    device
        .lock()
        .await
        .output
        .write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 3).as_bytes())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), failed_reconcile)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        device.lock().await.start_attempts,
        attempts_before_failure + 2,
        "one uncertain start plus one same-ID ready retry"
    );
    assert!(session.serial_runtime.snapshot().warning.is_some());
    assert!(!session.is_stop_requested());

    // A later epoch must not cross the unconfirmed handoff generation.
    {
        let mut d = device.lock().await;
        d.status.boot_epoch = format!("{:032x}", 4);
        d.status.serial = Some(serial_status(4, 230400));
        d.status.boot = None;
        d.reject_start = false;
    }
    let retry_announcement = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "x86_64".into(),
        loader_version: "test-v7".into(),
        boot_epoch: format!("{:032x}", 4),
        http_port: device_port,
        serial_id: Some(format!("{:032x}", 4)),
        serial_ready: true,
    };
    tokio::time::timeout(
        Duration::from_secs(2),
        ostool_server::device::reconcile(
            state.clone(),
            retry_announcement,
            "127.0.0.1:12345".parse().unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let device_state = device.lock().await;
    assert_eq!(device_state.create_attempts, 3);
    assert_eq!(device_state.upload_attempts, 3);
    assert_eq!(device_state.delete_attempts, 0);
    assert_eq!(device_state.start_attempts, attempts_before_failure + 2);
    assert_eq!(device_state.starts, 2);
    drop(device_state);
    while let Ok((port, _)) = configs.try_recv() {
        assert_ne!(port, relay_path);
        assert_ne!(port, path, "bound UART must not reopen on second boot");
    }

    // Explicit power/new-test generation may clear the observation and start a new handoff.
    session.change_power(true).await;
    session
        .publish_boot_command(SessionBootCommand {
            boot_id: "terminal-boot".into(),
            kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
            kernel_size: kernel.len() as u64,
            kernel_sha256: format!("{:x}", Sha256::digest(&kernel)),
            initramfs: None,
            cmdline: None,
        })
        .await;
    {
        let mut d = device.lock().await;
        d.status.boot_epoch = format!("{:032x}", 5);
        d.status.serial = Some(serial_status(5, 230400));
        d.status.boot = None;
        d.upload_rejection = Some(DeviceOperationError {
            code: "future_upload_rejection".into(),
            message: "device-specific handoff refusal".into(),
        });
    }
    let terminal_announcement = LoaderAnnouncement {
        protocol_version: DEVICE_PROTOCOL_VERSION,
        mac_address: mac,
        current_mac_address: mac,
        arch: "future-isa".into(),
        loader_version: "test-v7".into(),
        boot_epoch: format!("{:032x}", 5),
        http_port: device_port,
        serial_id: Some(format!("{:032x}", 5)),
        serial_ready: true,
    };
    let terminal = tokio::spawn(ostool_server::device::reconcile(
        state.clone(),
        terminal_announcement,
        "127.0.0.1:12345".parse().unwrap(),
    ));
    wait_config(&mut configs, &path, 38_400, "terminal 422 epoch 5").await;
    device
        .lock()
        .await
        .output
        .write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 5).as_bytes())
        .await
        .unwrap();
    let terminal_error = tokio::time::timeout(Duration::from_secs(4), terminal)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(format!("{terminal_error:#}").contains("future_upload_rejection"));
    let error_message = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(text) if text.contains("device-specific handoff refusal") => {
                    break text;
                }
                Message::Text(_) | Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(frame) => panic!("WebSocket closed without reason: {frame:?}"),
                Message::Frame(_) => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(error_message.contains("future_upload_rejection"));
    assert!(matches!(
        session.loader_status().await.unwrap().status,
        Some(LoaderStatusPhase::Failed { code, message })
            if code == "future_upload_rejection"
                && message == "device-specific handoff refusal"
    ));
    assert_eq!(
        session.serial_runtime.snapshot().phase,
        ostool_server::serial::runtime::SerialRuntimePhase::Failed
    );
    assert!(session.is_stop_requested());

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
