//! Baseline (0331dfa) gateway HTTP contract, independent of ostool-server handlers.
//! This fixture verifies request shapes and accepts responses without newer fields.
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
async fn baseline_gateway_wire_contract_and_no_automatic_replay() -> Result<()> {
    // Isolate process-only credentials from the test runner and user keyring.
    for prefix in ["", "/webapp/ostoolmanagephp"] {
        let output = tokio::process::Command::new(std::env::current_exe()?)
            .args([
                "--ignored",
                "--exact",
                "gateway_contract_probe",
                "--nocapture",
            ])
            .env("OSTOOL_GATEWAY_CONTRACT_PROBE", "true")
            .env("OSTOOL_GATEWAY_PREFIX", prefix)
            .env("OSTOOL_BOARD_ACCESS_TOKEN", "contract-only-token")
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "gateway contract probe failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "internal isolated subprocess for baseline_gateway_wire_contract_and_no_automatic_replay"]
async fn gateway_contract_probe() -> Result<()> {
    if std::env::var("OSTOOL_GATEWAY_CONTRACT_PROBE").as_deref() != Ok("true") {
        return Ok(());
    }
    let seen = Arc::new(AtomicUsize::new(0));
    let prefix = std::env::var("OSTOOL_GATEWAY_PREFIX")?;
    let app = Router::new()
        .fallback(gateway)
        .with_state((seen.clone(), prefix.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // HTTPS validation is separately covered in ostool. Only this loopback fixture
    // constructs the endpoint directly to exercise the original wire protocol.
    let client = BoardServerClient::new_with_endpoint(BoardEndpoint {
        base_url: format!("http://{address}{prefix}/").parse()?,
        auth_mode: AuthMode::Required,
    })?;
    let boards = client.list_board_types().await?;
    assert_eq!(boards[0].board_type, "baseline");
    assert!(boards[0].leases.is_none());
    let session = client.create_session("baseline").await?;
    assert_eq!(session.board_id, "baseline-board");
    assert_eq!(
        client
            .resolve_ws_url(session.ws_url.as_deref().unwrap())?
            .as_str(),
        format!("ws://{address}{prefix}/api/v1/sessions/baseline-session/serial/ws")
    );
    client.heartbeat(&session.session_id).await?;
    client.get_boot_profile(&session.session_id).await?;
    let serial = client.get_serial_status(&session.session_id).await?;
    assert!(serial.runtime.is_none());
    assert_eq!(serial.manager.pending, 0);
    assert_eq!(serial.manager.candidates, 0);
    assert_eq!(serial.manager.leased, 0);
    client.get_tftp_status(&session.session_id).await?;
    client.get_session_dtb(&session.session_id).await?;
    client
        .upload_session_file(
            &session.session_id,
            "tools/probe.sh",
            b"echo baseline".to_vec(),
        )
        .await?;
    client.delete_session(&session.session_id).await?;
    let error = client.create_session("baseline").await.unwrap_err();
    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    assert_eq!(error.code, None);
    assert_eq!(error.message, "baseline gateway rejected credentials");
    assert_eq!(
        seen.load(Ordering::SeqCst),
        10,
        "a rejected mutation must never be replayed"
    );
    server.abort();
    Ok(())
}

async fn gateway(
    State((seen, prefix)): State<(Arc<AtomicUsize>, String)>,
    request: Request<Body>,
) -> impl IntoResponse {
    let index = seen.fetch_add(1, Ordering::SeqCst);
    let expected = [
        ("GET", "/api/v1/board-types"),
        ("POST", "/api/v1/sessions"),
        ("POST", "/api/v1/sessions/baseline-session/heartbeat"),
        ("GET", "/api/v1/sessions/baseline-session/boot-profile"),
        ("GET", "/api/v1/sessions/baseline-session/serial"),
        ("GET", "/api/v1/sessions/baseline-session/tftp"),
        ("GET", "/api/v1/sessions/baseline-session/dtb"),
        ("PUT", "/api/v1/sessions/baseline-session/files"),
        ("DELETE", "/api/v1/sessions/baseline-session"),
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
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer contract-only-token"
    );
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
            json!({"board_type":"baseline","required_tags":[],"client_name":"ostool"})
        );
    } else if index == 7 {
        assert_eq!(body.as_ref(), b"echo baseline");
    } else {
        assert!(body.is_empty());
    }
    if index == 9 {
        return (
            StatusCode::UNAUTHORIZED,
            "baseline gateway rejected credentials".to_string(),
        );
    }
    let value = match index {
        0 => json!([{"board_type":"baseline","tags":[],"total":1,"available":1}]),
        1 => {
            json!({"session_id":"baseline-session","board_id":"baseline-board","lease_expires_at":"2026-10-21T00:00:00Z","serial_available":true,"boot_mode":"uboot","ws_url":"api/v1/sessions/baseline-session/serial/ws"})
        }
        2 => json!({"session_id":"baseline-session","lease_expires_at":"2026-10-21T00:00:10Z"}),
        3 => {
            json!({"boot":{"kind":"uboot","use_tftp":true},"server_ip":"192.168.20.1","netmask":"255.255.255.0","interface":"eth0"})
        }
        4 => {
            json!({"available":false,"connected":false,"port":null,"baud_rate":null,"ws_url":null})
        }
        5 => {
            json!({"available":true,"provider":"builtin","server_ip":"192.168.20.1","netmask":"255.255.255.0","writable":true,"files":[]})
        }
        6 => json!({"dtb_name":null,"relative_path":null,"session_file_path":null,"tftp_url":null}),
        7 => {
            json!({"filename":"probe.sh","relative_path":"ostool/sessions/baseline-session/tools/probe.sh","tftp_url":null,"size":13,"uploaded_at":"2026-10-21T00:00:00Z"})
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
