use std::{path::Path, time::Duration};

use ostool::{
    board::{self, config::BoardRunConfig, global_config::AuthMode},
    build::{self, RuntimeArtifactInput},
    invocation::{Invocation, InvocationOptions},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

fn minimal_future_elf() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x104];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = object::elf::ELFCLASS64;
    bytes[5] = object::elf::ELFDATA2LSB;
    bytes[6] = object::elf::EV_CURRENT;
    bytes[16..18].copy_from_slice(&object::elf::ET_EXEC.to_le_bytes());
    bytes[18..20].copy_from_slice(&0xcafe_u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&u32::from(object::elf::EV_CURRENT).to_le_bytes());
    bytes[24..32].copy_from_slice(&0x8000_0000_u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&object::elf::PT_LOAD.to_le_bytes());
    bytes[68..72].copy_from_slice(&(object::elf::PF_R | object::elf::PF_X).to_le_bytes());
    bytes[72..80].copy_from_slice(&0x100_u64.to_le_bytes());
    bytes[80..88].copy_from_slice(&0x8000_0000_u64.to_le_bytes());
    bytes[88..96].copy_from_slice(&0x8000_0000_u64.to_le_bytes());
    bytes[96..104].copy_from_slice(&4_u64.to_le_bytes());
    bytes[104..112].copy_from_slice(&4_u64.to_le_bytes());
    bytes[112..120].copy_from_slice(&4_u64.to_le_bytes());
    bytes
}

fn write_test_package(root: &Path) {
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"future-kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = socket.read(&mut buffer).await.unwrap();
        assert_ne!(read, 0, "client closed before the request completed");
        bytes.extend_from_slice(&buffer[..read]);
        let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
            })
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= header_end + 4 + content_length {
            return (headers, bytes[header_end + 4..].to_vec());
        }
    }
}

async fn write_response(socket: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}

fn contains_bin(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.is_dir() && contains_bin(&path)
            || path.extension().and_then(|extension| extension.to_str()) == Some("bin")
    })
}

#[tokio::test]
async fn public_artifact_chain_uploads_future_machine_elf_without_bin() {
    let temp = tempfile::tempdir().unwrap();
    write_test_package(temp.path());

    let elf_path = temp.path().join("future-kernel.elf");
    let elf_bytes = minimal_future_elf();
    std::fs::write(&elf_path, &elf_bytes).unwrap();
    let mut invocation = Invocation::new(InvocationOptions::new(
        Some(temp.path().to_path_buf()),
        None,
        None,
        false,
    ))
    .unwrap();
    build::prepare_runtime_artifact(&mut invocation, RuntimeArtifactInput::new(&elf_path, false))
        .unwrap();
    assert!(!contains_bin(temp.path()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (upload_tx, upload_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut upload_tx = Some(upload_tx);
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = read_request(&mut socket).await;
            let request_line = headers.lines().next().unwrap_or_default();
            if request_line.starts_with("POST /api/v1/sessions ") {
                write_response(
                    &mut socket,
                    "201 Created",
                    r#"{"session_id":"session-1","board_id":"future-board","lease_expires_at":"2030-01-01T00:00:00Z","serial_available":false,"boot_mode":"httpboot","ws_url":null}"#,
                )
                .await;
            } else if request_line.starts_with("PUT /api/v1/sessions/session-1/http-boot/kernel ") {
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("x-axloader-protocol-version: 7\r\n")
                );
                upload_tx.take().unwrap().send(body).unwrap();
                write_response(
                    &mut socket,
                    "201 Created",
                    r#"{"boot_id":"boot-1","kernel_url":"http://127.0.0.1/boot/kernel.elf","kernel_size":260,"kernel_sha256":null}"#,
                )
                .await;
            } else if request_line.starts_with("GET /api/v1/sessions/session-1/serial ") {
                write_response(
                    &mut socket,
                    "404 Not Found",
                    r#"{"code":"not_found","message":"serial not configured"}"#,
                )
                .await;
            } else if request_line.starts_with("POST /api/v1/sessions/session-1/heartbeat ") {
                write_response(
                    &mut socket,
                    "200 OK",
                    r#"{"session_id":"session-1","lease_expires_at":"2030-01-01T00:00:00Z"}"#,
                )
                .await;
            } else if request_line.starts_with("DELETE /api/v1/sessions/session-1 ") {
                write_response(&mut socket, "202 Accepted", "").await;
                break;
            } else {
                panic!("unexpected request: {request_line}");
            }
        }
    });

    let board_config = BoardRunConfig {
        board_type: "future-board".into(),
        server: Some(format!("http://{address}")),
        auth_mode: Some(AuthMode::Disabled),
        timeout: Some(5),
        ..BoardRunConfig::default()
    };
    let error = board::run_prepared_board(
        &mut invocation,
        &board_config,
        board::RunBoardOptions::default(),
    )
    .await
    .unwrap_err();
    let error = format!("{error:#}");
    assert!(
        error.contains("failed to get HTTP Boot serial status"),
        "{error}"
    );

    let uploaded = tokio::time::timeout(Duration::from_secs(2), upload_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(uploaded, elf_bytes);
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert!(!contains_bin(temp.path()));
}

#[tokio::test]
async fn public_artifact_chain_generates_bin_only_for_uboot_session() {
    let temp = tempfile::tempdir().unwrap();
    write_test_package(temp.path());
    let elf_path = temp.path().join("uboot-kernel");
    std::fs::copy(std::env::current_exe().unwrap(), &elf_path).unwrap();

    let mut invocation = Invocation::new(InvocationOptions::new(
        Some(temp.path().to_path_buf()),
        None,
        None,
        false,
    ))
    .unwrap();
    build::prepare_runtime_artifact(&mut invocation, RuntimeArtifactInput::new(&elf_path, false))
        .unwrap();
    assert!(!contains_bin(temp.path()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, _) = read_request(&mut socket).await;
            let request_line = headers.lines().next().unwrap_or_default();
            if request_line.starts_with("POST /api/v1/sessions ") {
                write_response(
                    &mut socket,
                    "201 Created",
                    r#"{"session_id":"session-1","board_id":"uboot-board","lease_expires_at":"2030-01-01T00:00:00Z","serial_available":false,"boot_mode":"uboot","ws_url":null}"#,
                )
                .await;
            } else if request_line.starts_with("GET /api/v1/sessions/session-1/boot-profile ") {
                write_response(
                    &mut socket,
                    "404 Not Found",
                    r#"{"code":"not_found","message":"boot profile not configured"}"#,
                )
                .await;
            } else if request_line.starts_with("POST /api/v1/sessions/session-1/heartbeat ") {
                write_response(
                    &mut socket,
                    "200 OK",
                    r#"{"session_id":"session-1","lease_expires_at":"2030-01-01T00:00:00Z"}"#,
                )
                .await;
            } else if request_line.starts_with("DELETE /api/v1/sessions/session-1 ") {
                write_response(&mut socket, "202 Accepted", "").await;
                break;
            } else {
                panic!("unexpected request: {request_line}");
            }
        }
    });

    let board_config = BoardRunConfig {
        board_type: "uboot-board".into(),
        server: Some(format!("http://{address}")),
        auth_mode: Some(AuthMode::Disabled),
        timeout: Some(5),
        ..BoardRunConfig::default()
    };
    let error = board::run_prepared_board(
        &mut invocation,
        &board_config,
        board::RunBoardOptions::default(),
    )
    .await
    .unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("failed to get boot profile"), "{error}");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert!(contains_bin(temp.path()));
}
