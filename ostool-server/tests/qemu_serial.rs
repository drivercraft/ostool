//! Actual OVMF UART identity -> managed PTY -> WebSocket -> uploaded kernel.
#![cfg(target_os = "linux")]
#[path = "common/serial.rs"]
mod serial_fixture;
use futures_util::{SinkExt, StreamExt};
use httpboot_protocol::{BootArch, BootFile, ImageFormat, LoaderAnnouncement, LoaderDeviceStatus};
use ostool_serial::{PortLocator, SerialManager};
use ostool_server::{session::SessionBootCommand, tftp::service::build_tftp_manager, *};
use serial_fixture::PtyBackend;
use serialport::{SerialPort, TTYPort};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, UnixStream},
    process::Command,
    sync::mpsc,
    time::Instant,
};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires OSTOOL_QEMU_TGOS and built axloader/ArceOS artifacts; run test-axloader-local.py"]
async fn ovmf_identity_binds_serial_null_session_and_boots_kernel() -> anyhow::Result<()> {
    let tgos = PathBuf::from(std::env::var("OSTOOL_QEMU_TGOS")?);
    let root = tempfile::tempdir()?;
    let esp = root.path().join("esp/EFI/BOOT");
    tokio::fs::create_dir_all(&esp).await?;
    tokio::fs::copy(
        tgos.join("target/x86_64-unknown-uefi/release/axloader.efi"),
        esp.join("BOOTX64.EFI"),
    )
    .await?;
    let vars = root.path().join("vars.fd");
    tokio::fs::copy("/usr/share/OVMF/OVMF_VARS_4M.fd", &vars).await?;
    let kernel_path = std::env::var_os("OSTOOL_QEMU_KERNEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| tgos.join("target/x86_64-unknown-linux-musl/release/arceos-helloworld"));
    let kernel = tokio::fs::read(kernel_path).await?;
    let forward = TcpListener::bind("127.0.0.1:0").await?;
    let port = forward.local_addr()?.port();
    drop(forward);
    let serial_socket = root.path().join("serial.sock");
    let mut qemu = Command::new("qemu-system-x86_64")
        .args([
            "-m", "512M", "-smp", "1", "-machine", "q35", "-accel", "kvm", "-cpu", "host",
            "-display", "none", "-monitor", "none",
        ])
        .args([
            "-serial",
            &format!("unix:{},server=on,wait=off", serial_socket.display()),
        ])
        .args([
            "-netdev",
            &format!("user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:2999"),
        ])
        .args([
            "-device",
            "virtio-net-pci,netdev=net0,mac=02:00:00:00:00:42",
        ])
        .args([
            "-drive",
            "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd",
        ])
        .args([
            "-drive",
            &format!("if=pflash,format=raw,file={}", vars.display()),
        ])
        .args([
            "-drive",
            &format!(
                "format=raw,file=fat:rw:{}",
                root.path().join("esp").display()
            ),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let limit = Instant::now() + Duration::from_secs(10);
    let mut wire = loop {
        match UnixStream::connect(&serial_socket).await {
            Ok(wire) => break wire,
            Err(e) => {
                anyhow::ensure!(Instant::now() < limit, "QEMU serial socket: {e}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    let (master, mut slave) = TTYPort::pair()?;
    slave.set_exclusive(false)?;
    let path = slave.name().unwrap();
    // Keep the PTY alive while discovery has not opened it yet. This descriptor
    // never reads bytes or configures the UART.
    let _keeper = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?;
    drop(slave);
    let mut master = tokio_serial::SerialStream::try_from(master)?;
    let bridge =
        tokio::spawn(async move { tokio::io::copy_bidirectional(&mut wire, &mut master).await });
    let (configured, mut settings) = mpsc::unbounded_channel();
    let config_path = root.path().join("server.toml");
    let mut config = ServerConfig::default_for_path(&config_path);
    config.data_dir = root.path().join("data");
    config.board_dir = root.path().join("boards");
    config.dtb_dir = root.path().join("dtbs");
    config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
        root.path().join("files"),
    ));
    let mut state = build_app_state(
        config_path,
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await?;
    state.serial_manager = SerialManager::new(Arc::new(PtyBackend {
        ports: vec![PortLocator {
            name: path.clone(),
            aliases: vec![],
            serial_number: None,
        }],
        configured,
    }));
    let mac = "02:00:00:00:00:42".parse()?;
    let board = BoardConfig {
        id: "qemu-auto".into(),
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
    state.boards.write().await.insert(board.id.clone(), board);
    state.sync_board_runtime_states().await;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = build_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let created = state
        .create_session_with_board_id("uefi", "qemu-auto", &[], None)
        .await
        .map_err(|error| anyhow::anyhow!("session allocation: {error:?}"))?;
    let session = state.session_state(&created.id).await.unwrap();
    state
        .tftp_manager
        .read()
        .await
        .put_session_file(&created.id, "kernel.elf", &kernel)
        .await?;
    let initramfs = if let Some(file) = std::env::var_os("OSTOOL_QEMU_INITRAMFS") {
        let bytes = tokio::fs::read(file).await?;
        state
            .tftp_manager
            .read()
            .await
            .put_session_file(&created.id, "initramfs.cpio", &bytes)
            .await?;
        Some(BootFile {
            path: format!("/boot/sessions/{}/initramfs.cpio", created.id),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        })
    } else {
        None
    };
    let expect_initramfs = initramfs.is_some();
    session
        .publish_boot_command(SessionBootCommand {
            boot_id: "qemu-auto".into(),
            kernel_path: format!("/boot/sessions/{}/kernel.elf", created.id),
            kernel_size: kernel.len() as u64,
            kernel_sha256: format!("{:x}", Sha256::digest(&kernel)),
            arch: BootArch::X86_64,
            image_format: ImageFormat::Elf64,
            entry_symbol: None,
            initramfs,
            cmdline: Some("axloader.cmdline=ostool-local".into()),
        })
        .await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/api/v1/sessions/{}/serial/ws",
        created.id
    ))
    .await?;
    assert!(matches!(ws.next().await.unwrap()?,Message::Text(t) if t.contains("opened")));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let observed: LoaderDeviceStatus = loop {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/api/v1/status"))
            .send()
            .await
            && let Ok(status) = response.json::<LoaderDeviceStatus>().await
        {
            break status;
        }
        anyhow::ensure!(Instant::now() < deadline, "OVMF device HTTP not ready");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let uart = observed.serial.as_ref().unwrap();
    anyhow::ensure!(uart.ready, "OVMF UART unavailable: {:?}", uart.error);
    let announcement = LoaderAnnouncement {
        protocol_version: observed.protocol_version,
        boot_epoch: observed.boot_epoch.clone(),
        mac_address: observed.mac_address,
        current_mac_address: observed.current_mac_address,
        arch: observed.arch,
        loader_version: observed.loader_version.clone(),
        http_port: port,
        serial_id: Some(uart.serial_id.clone()),
        serial_ready: uart.ready,
    };
    // The test maps guest TCP4 to hostfwd; UART bytes are forwarded unchanged.
    let reconcile =
        ostool_server::device::reconcile(state.clone(), announcement, "127.0.0.1:12345".parse()?);
    let receive = async {
        let mut output = Vec::new();
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    output.extend(bytes);
                    if String::from_utf8_lossy(&output).contains("Hello, world!") {
                        return Ok::<_, anyhow::Error>(output);
                    }
                }
                Some(Err(error)) => return Err(error.into()),
                Some(Ok(_)) => {}
                None => anyhow::bail!(
                    "WebSocket closed before kernel output: {}",
                    String::from_utf8_lossy(&output)
                ),
            }
        }
    };
    let (_, output) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::try_join!(reconcile, receive)
    })
    .await?
    .map_err(|e| {
        anyhow::anyhow!(
            "{e:#}; runtime={:?}; manager={:?}; device={:?}",
            session.serial_runtime.snapshot(),
            *state.serial_manager.subscribe().borrow(),
            observed.serial
        )
    })?;
    assert!(
        String::from_utf8_lossy(&output).contains(&format!("AXLOADER-SERIAL/1 {}", uart.serial_id))
    );
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("HOST_CMDLINE: axloader.cmdline=ostool-local"));
    if expect_initramfs {
        assert!(text.contains("HOST_INITRAMFS_PASSED"));
    }
    assert_eq!(
        settings.recv().await.unwrap(),
        (path, uart.parameters.unwrap())
    );
    assert_eq!(
        session.serial_runtime.snapshot().phase,
        ostool_server::serial::runtime::SerialRuntimePhase::Bound
    );
    if let Some(artifacts) = std::env::var_os("OSTOOL_QEMU_ARTIFACTS") {
        tokio::fs::write(PathBuf::from(artifacts).join("kernel-serial.log"), &output).await?;
    }
    ws.send(Message::Close(None)).await?;
    drop(ws);
    tokio::time::timeout(
        Duration::from_secs(3),
        state.serial_manager.wait_owner_released(&created.id),
    )
    .await??;
    qemu.kill().await?;
    qemu.wait().await?;
    bridge.abort();
    server.abort();
    Ok(())
}
