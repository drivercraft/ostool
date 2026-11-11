//! The same Board API requests serve anonymous local and authenticated public clients.
use anyhow::Result;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
};
use ostool::board::{
    client::BoardServerClient,
    global_config::{AuthMode, BoardEndpoint},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn editions_share_wire_contract_and_never_replay_rejected_requests() -> Result<()> {
    // Isolate process-only credentials from the test runner and user keyring.
    for prefix in ["", "/boards"] {
        for mode in ["disabled", "required", "logged-out"] {
            let root = tempfile::tempdir()?;
            let mut command = tokio::process::Command::new(std::env::current_exe()?);
            command
                .args([
                    "--ignored",
                    "--exact",
                    "gateway_contract_probe",
                    "--nocapture",
                ])
                .env("OSTOOL_GATEWAY_CONTRACT_PROBE", "true")
                .env("OSTOOL_GATEWAY_PREFIX", prefix)
                .env("OSTOOL_GATEWAY_AUTH_MODE", mode)
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .env("XDG_CACHE_HOME", root.path().join("cache"))
                .env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    "unix:path=/nonexistent/ostool-contract-bus",
                );
            if mode == "logged-out" {
                command.env_remove("OSTOOL_BOARD_ACCESS_TOKEN");
            } else {
                // Anonymous mode must ignore credentials even when present in the environment.
                command.env("OSTOOL_BOARD_ACCESS_TOKEN", "contract-only-token");
            }
            let output = command.output().await?;
            anyhow::ensure!(
                output.status.success(),
                "gateway contract probe failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "internal isolated subprocess for editions_share_wire_contract_and_never_replay_rejected_requests"]
async fn gateway_contract_probe() -> Result<()> {
    if std::env::var("OSTOOL_GATEWAY_CONTRACT_PROBE").as_deref() != Ok("true") {
        return Ok(());
    }
    let seen = Arc::new(AtomicUsize::new(0));
    let prefix = std::env::var("OSTOOL_GATEWAY_PREFIX")?;
    let mode = std::env::var("OSTOOL_GATEWAY_AUTH_MODE")?;
    let auth_mode = if mode == "disabled" {
        AuthMode::Disabled
    } else {
        AuthMode::Required
    };
    let app = Router::new()
        .fallback(gateway)
        .with_state((seen.clone(), prefix.clone(), auth_mode));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // HTTPS validation is separately covered in ostool. Only this loopback fixture
    // constructs the endpoint directly to exercise both authentication modes.
    let client = BoardServerClient::new_with_endpoint(BoardEndpoint {
        base_url: format!("http://{address}{prefix}/").parse()?,
        auth_mode,
    })?;
    if mode == "logged-out" {
        let error = client.list_board_types().await.unwrap_err();
        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
        assert_eq!(error.code.as_deref(), Some("authentication_required"));
        assert!(error.message.contains("ostool login"));
        assert!(client.websocket_authorization().await.is_err());
        assert_eq!(seen.load(Ordering::SeqCst), 0);
        server.abort();
        return Ok(());
    }
    let boards = client.list_board_types().await?;
    assert_eq!(boards[0].board_type, "contract");
    assert_eq!(
        client.websocket_authorization().await?.as_deref(),
        (auth_mode == AuthMode::Required).then_some("contract-only-token")
    );
    let session = client.create_session("contract").await?;
    assert_eq!(session.board_id, "contract-board");
    assert_eq!(
        client
            .resolve_ws_url(session.ws_url.as_deref().unwrap())?
            .as_str(),
        format!("ws://{address}{prefix}/api/v1/sessions/contract-session/serial/ws")
    );
    client.heartbeat(&session.session_id).await?;
    client.get_boot_profile(&session.session_id).await?;
    let serial = client.get_serial_status(&session.session_id).await?;
    assert_eq!(serial.runtime.as_ref().unwrap().phase, "waiting_device");
    assert_eq!(serial.manager.pending, 0);
    assert_eq!(serial.manager.candidates, 0);
    assert_eq!(serial.manager.leased, 0);
    client.get_tftp_status(&session.session_id).await?;
    client.get_session_dtb(&session.session_id).await?;
    client
        .upload_session_file(
            &session.session_id,
            "tools/probe.sh",
            b"echo contract".to_vec(),
        )
        .await?;
    client.delete_session(&session.session_id).await?;
    let error = client.create_session("contract").await.unwrap_err();
    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    assert_eq!(error.code.as_deref(), Some("invalid_device_response"));
    assert!(
        error
            .message
            .contains("contract gateway rejected credentials")
    );
    assert!(error.message.contains("HTTP 401"));
    assert_eq!(
        seen.load(Ordering::SeqCst),
        10,
        "a rejected mutation must never be replayed"
    );
    server.abort();
    Ok(())
}

async fn gateway(
    State((seen, prefix, auth_mode)): State<(Arc<AtomicUsize>, String, AuthMode)>,
    request: Request<Body>,
) -> impl IntoResponse {
    let index = seen.fetch_add(1, Ordering::SeqCst);
    let expected = [
        ("GET", "/api/v1/board-types"),
        ("POST", "/api/v1/sessions"),
        ("POST", "/api/v1/sessions/contract-session/heartbeat"),
        ("GET", "/api/v1/sessions/contract-session/boot-profile"),
        ("GET", "/api/v1/sessions/contract-session/serial"),
        ("GET", "/api/v1/sessions/contract-session/tftp"),
        ("GET", "/api/v1/sessions/contract-session/dtb"),
        ("PUT", "/api/v1/sessions/contract-session/files"),
        ("DELETE", "/api/v1/sessions/contract-session"),
        ("POST", "/api/v1/sessions"),
    ];
    assert!(
        index < expected.len(),
        "unexpected replay of a gateway request"
    );
    assert_eq!(request.method().as_str(), expected[index].0);
    assert_eq!(
        request.uri().path(),
        format!("{prefix}{}", expected[index].1)
    );
    match auth_mode {
        AuthMode::Required => assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer contract-only-token"
        ),
        AuthMode::Disabled => assert!(!request.headers().contains_key("authorization")),
    }
    if index == 7 {
        assert_eq!(
            request.headers().get("x-file-path").unwrap(),
            "tools/probe.sh"
        );
    }
    let body = to_bytes(request.into_body(), 4096).await.unwrap();
    if index == 1 || index == 9 {
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"board_type":"contract","required_tags":[],"client_name":"ostool"})
        );
    } else if index == 7 {
        assert_eq!(body.as_ref(), b"echo contract");
    } else {
        assert!(body.is_empty());
    }
    if index == 9 {
        return (
            StatusCode::UNAUTHORIZED,
            "contract gateway rejected credentials".to_string(),
        );
    }
    let value = match index {
        0 => json!([{"board_type":"contract","tags":[],"total":1,"available":1}]),
        1 => {
            json!({"session_id":"contract-session","board_id":"contract-board","lease_expires_at":"2026-10-21T00:00:00Z","serial_available":true,"boot_mode":"uboot","ws_url":"api/v1/sessions/contract-session/serial/ws"})
        }
        2 => json!({"session_id":"contract-session","lease_expires_at":"2026-10-21T00:00:10Z"}),
        3 => {
            json!({"server_side_operations":true,"boot":{"kind":"uboot","use_tftp":false},"server_ip":null,"netmask":null,"interface":null,"http_base_url":"http://127.0.0.1/"})
        }
        4 => {
            json!({"available":false,"connected":false,"port":null,"baud_rate":null,"ws_url":null,"runtime":{"phase":"waiting_device","port":null,"parameters":null,"error":null,"warning":null},"manager":{"pending":0,"candidates":0,"leased":0}})
        }
        5 => {
            json!({"available":false,"provider":"builtin","server_ip":null,"netmask":null,"writable":true,"files":[]})
        }
        6 => json!({"dtb_name":null,"relative_path":null,"session_file_path":null,"tftp_url":null}),
        7 => {
            json!({"filename":"probe.sh","relative_path":"ostool/sessions/contract-session/tools/probe.sh","tftp_url":null,"http_url":"http://127.0.0.1/share/sessions/contract-session/tools/probe.sh","size":13,"uploaded_at":"2026-10-21T00:00:00Z"})
        }
        8 => return (StatusCode::ACCEPTED, String::new()),
        _ => unreachable!(),
    };
    (
        if index == 1 || index == 7 {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        value.to_string(),
    )
}
