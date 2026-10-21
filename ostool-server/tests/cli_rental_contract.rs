//! Website approval -> actual ostool HTTP/WS clients -> cancellation/expiry/revocation.
//! The subprocess isolates CLI credentials from the test runner and user's credential store.
#![cfg(unix)]
#[path = "support/database.rs"]
mod test_database;
use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use ostool::board::{
    client::{BoardServerClient, HttpBootKernelUpload},
    global_config::{AuthMode, BoardEndpoint},
    serial_stream::connect_serial_stream,
};
use ostool_server::{
    BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement, PowerManagementConfig,
    SerialConfig, SerialPortKey, SerialPortKeyKind, ServerConfig, TftpConfig, UbootProfile,
    build_app_state, build_router, config::Edition, persistence::BoardStore,
    tftp::service::build_tftp_manager,
};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use serialport::SerialPort;
use std::{io::Write, path::Path, time::Duration};
use tokio::process::Command;

// Production BoardEndpoint::new requires HTTPS. Only this isolated loopback fixture
// directly constructs an endpoint, to test the actual client against the in-process server.
fn endpoint(base: &str) -> BoardEndpoint {
    let url = url::Url::parse(base).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    BoardEndpoint {
        base_url: url,
        auth_mode: AuthMode::Required,
    }
}
async fn request(
    client: &Client,
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
    expected: StatusCode,
) -> Value {
    let mut builder = client
        .request(
            method.parse().unwrap(),
            format!("{}{path}", base.trim_end_matches('/')),
        )
        .json(&body);
    if let Some(token) = token {
        builder = builder.bearer_auth(token);
    }
    let response = builder.send().await.unwrap();
    assert_eq!(response.status(), expected, "{method} {path}");
    response.json().await.unwrap_or(Value::Null)
}
async fn wait_file(path: &Path) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(value) = tokio::fs::read_to_string(path).await {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("CLI probe did not reach its checkpoint")
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn website_rentals_authorize_real_ostool_clients() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (mut serial_master, mut serial_slave) = serialport::TTYPort::pair()?;
    serial_slave.set_exclusive(false)?;
    let serial_path = serial_slave.name().context("PTY path")?;
    drop(serial_slave);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/", listener.local_addr()?);
    let mut config = ServerConfig {
        edition: Edition::Public,
        data_dir: root.path().join("data"),
        tftp: TftpConfig::Builtin(BuiltinTftpConfig {
            enabled: false,
            ..BuiltinTftpConfig::default_with_root(root.path().join("tftp"))
        }),
        ..Default::default()
    };
    config.listen_addr = listener.local_addr()?;
    config.auth.public_base_url = Some(base.clone());
    config.auth.access_token_ttl_secs = 60;
    // The loopback client represents an Internet user in this fixture. Hardware-only
    // anonymous traffic is tested separately; it must not mask missing CLI credentials.
    config.security.hardware_allowed_networks = vec!["192.0.2.0/24".into()];
    config.http_boot.root_dir = root.path().join("http-boot");
    let database = test_database::prepare(&root.path().join("config.toml"), &mut config).await?;
    let board = BoardConfig {
        id: "contract-board".into(),
        board_type: "contract-test".into(),
        tags: vec!["arm64".into()],
        serial: Some(SerialConfig {
            key: SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: serial_path,
            },
            baud_rate: 115200,
            resolved_device_path: None,
            resolved_usb_path: None,
        }),
        boot: BootConfig::Uboot(UbootProfile {
            use_tftp: false,
            ..Default::default()
        }),
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "true # private-power-credential".into(),
            power_off_cmd: "true".into(),
        }),
        notes: Some("private-operator-notes".into()),
        network_identity: None,
        disabled: false,
    };
    BoardStore::new(database.clone())
        .write_board(&board)
        .await?;
    let state = build_app_state(
        root.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await?;
    state.ensure_data_dirs().await?;
    let app = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let member = database
        .create_user(
            "contract@test.local",
            "contract-member-password",
            None,
            false,
        )
        .await?;
    let admin = database
        .create_user("admin@contract.test", "contract-admin-password", None, true)
        .await?;
    let other = database
        .create_user(
            "other@contract.test",
            "contract-other-password",
            None,
            false,
        )
        .await?;
    let web = database
        .issue_grant(member.clone(), "web", "board:operate", &config.auth)
        .await?;
    let admin = database
        .issue_grant(admin, "admin-web", "board:operate", &config.auth)
        .await?;
    let other = database
        .issue_grant(other, "ostool-cli", "board:operate", &config.auth)
        .await?;
    // Equivalent to a completed browser login; proof/step-up flows have separate tests.
    sqlx::query(
        "UPDATE oauth_sessions SET step_up_until=now()+interval '10 minutes' WHERE id IN ($1,$2)",
    )
    .bind(web.session_id)
    .bind(admin.session_id)
    .execute(database.pool())
    .await?;
    let code: Value = client
        .post(format!("{base}oauth/device/code"))
        .form(&[
            ("client_id", "ostool-cli"),
            ("scope", "board:operate offline_access"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    request(
        &client,
        &base,
        "POST",
        "/api/v1/auth/device/authorize",
        Some(&web.access_token),
        json!({"user_code":code["user_code"]}),
        StatusCode::NO_CONTENT,
    )
    .await;
    let grant: Value = client
        .post(format!("{base}oauth/token"))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", "ostool-cli"),
            ("device_code", code["device_code"].as_str().unwrap()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let original = grant["access_token"].as_str().unwrap();
    let rotated: Value = client
        .post(format!("{base}oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", "ostool-cli"),
            ("refresh_token", grant["refresh_token"].as_str().unwrap()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let oauth = rotated["access_token"].as_str().unwrap();
    let refresh_token = rotated["refresh_token"].as_str().unwrap();
    request(
        &client,
        &base,
        "GET",
        "/api/v1/board-types",
        Some(original),
        Value::Null,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_eq!(
        request(
            &client,
            &base,
            "GET",
            "/api/v1/board-types",
            Some(oauth),
            Value::Null,
            StatusCode::OK
        )
        .await,
        json!([])
    );
    request(
        &client,
        &base,
        "POST",
        "/api/v1/sessions",
        Some(oauth),
        json!({"board_type":"contract-test"}),
        StatusCode::FORBIDDEN,
    )
    .await;

    // board:operate does not grant profile/settings access; offline_access alone
    // must not grant board operations, even if the user owns an active rental.
    request(
        &client,
        &base,
        "GET",
        "/api/v1/account",
        Some(oauth),
        Value::Null,
        StatusCode::FORBIDDEN,
    )
    .await;
    let narrow = database
        .issue_grant(member.clone(), "ostool-cli", "offline_access", &config.auth)
        .await?;
    request(
        &client,
        &base,
        "GET",
        "/api/v1/board-types",
        Some(&narrow.access_token),
        Value::Null,
        StatusCode::FORBIDDEN,
    )
    .await;

    for scenario in ["cancel", "expire", "revoke"] {
        let pat = if scenario != "cancel" {
            Some(
                request(
                    &client,
                    &base,
                    "POST",
                    "/api/v1/auth/personal-access-tokens",
                    Some(&web.access_token),
                    json!({"name":scenario}),
                    StatusCode::CREATED,
                )
                .await,
            )
        } else {
            None
        };
        let token = pat
            .as_ref()
            .map(|p| p["token"].as_str().unwrap())
            .unwrap_or(oauth);
        let start = Utc::now();
        let end = Utc::now() + ChronoDuration::hours(1);
        request(
            &client,
            &base,
            "POST",
            "/api/v1/account/rental-requests",
            Some(&web.access_token),
            json!({"board_id":board.id,"starts_at":start,"ends_at":end,"notes":scenario}),
            StatusCode::CREATED,
        )
        .await;
        request(
            &client,
            &base,
            "POST",
            "/api/v1/sessions",
            Some(oauth),
            json!({"board_type":"contract-test"}),
            StatusCode::FORBIDDEN,
        )
        .await;
        let account = request(
            &client,
            &base,
            "GET",
            "/api/v1/account",
            Some(&web.access_token),
            Value::Null,
            StatusCode::OK,
        )
        .await;
        let application = account["my_requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["status"] == "pending")
            .unwrap()["id"]
            .as_str()
            .unwrap();
        request(
            &client,
            &base,
            "POST",
            &format!("/api/v1/admin/rental-requests/{application}/review"),
            Some(&admin.access_token),
            json!({"decision":"approve"}),
            StatusCode::NO_CONTENT,
        )
        .await;
        let account = request(
            &client,
            &base,
            "GET",
            "/api/v1/account",
            Some(&web.access_token),
            Value::Null,
            StatusCode::OK,
        )
        .await;
        let rental = account["my_requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == application)
            .unwrap()["rental_id"]
            .as_str()
            .unwrap();
        let checkpoint = root.path().join(scenario);
        let mut probe = Command::new(std::env::current_exe()?)
            .args(["--ignored", "--exact", "cli_transport_probe", "--nocapture"])
            .env("OSTOOL_CONTRACT_BASE", &base)
            .env(
                "OSTOOL_CONTRACT_DEVICE_LOGIN",
                if scenario == "cancel" {
                    "true"
                } else {
                    "false"
                },
            )
            .env(
                "OSTOOL_BOARD_ACCESS_TOKEN",
                if scenario != "revoke" { "" } else { token },
            )
            .env(
                "OSTOOL_CONTRACT_STORED_TOKEN",
                if scenario == "expire" {
                    token
                } else {
                    "stored-test-token"
                },
            )
            .env("XDG_CONFIG_HOME", root.path().join("cli-config"))
            .env("XDG_CACHE_HOME", root.path().join("cli-cache"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", root.path().join("no-session-bus").display()),
            )
            .env("OSTOOL_CONTRACT_CHECKPOINT", &checkpoint)
            .env(
                "OSTOOL_CONTRACT_REVOKED",
                if scenario == "revoke" {
                    "true"
                } else {
                    "false"
                },
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        if scenario == "cancel" {
            // Approve the code produced by TokenManager::login_device in the child.
            // Browser CAPTCHA/MFA have their own real browser tests; no CLI bypass.
            let code: String = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let code: Option<String> = sqlx::query_scalar("SELECT user_code FROM oauth_device_codes WHERE user_id IS NULL AND expires_at>now() ORDER BY expires_at DESC LIMIT 1").fetch_optional(database.pool()).await.unwrap();
                    if let Some(code) = code { break code; }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }).await?;
            request(
                &client,
                &base,
                "POST",
                "/api/v1/auth/device/authorize",
                Some(&web.access_token),
                json!({"user_code":code}),
                StatusCode::NO_CONTENT,
            )
            .await;
        }
        if let Err(error) = wait_file(&checkpoint.with_extension("connected")).await {
            let _ = probe.kill().await;
            let output = probe.wait_with_output().await?;
            anyhow::bail!(
                "{error}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        serial_master.write_all(b"contract serial payload\n")?;
        let sid = wait_file(&checkpoint).await?;
        let owner: (uuid::Uuid, uuid::Uuid) =
            sqlx::query_as("SELECT user_id,rental_id FROM session_owners WHERE session_id=$1")
                .bind(&sid)
                .fetch_one(database.pool())
                .await?;
        assert_eq!(owner, (member.id, rental.parse()?));
        if scenario == "cancel" {
            let detail = request(
                &client,
                &base,
                "GET",
                &format!("/api/v1/sessions/{sid}"),
                Some(oauth),
                Value::Null,
                StatusCode::OK,
            )
            .await;
            assert_eq!(detail["board"].as_object().unwrap().len(), 4);
            assert!(detail["board"].get("power_management").is_none());
            assert!(detail["board"].get("serial").is_none());
            assert!(!detail.to_string().contains("private-power-credential"));
            assert!(!detail.to_string().contains("private-operator-notes"));
            let management = request(
                &client,
                &base,
                "GET",
                "/api/v1/admin/boards/contract-board",
                Some(&admin.access_token),
                Value::Null,
                StatusCode::OK,
            )
            .await;
            assert!(
                management["power_management"]["power_on_cmd"]
                    .as_str()
                    .unwrap()
                    .contains("private-power-credential")
            );
            // Exact paths used by ostool, plus downloads and serial upgrade: neither
            // a missing credential nor another user may access this leased session.
            for (method, suffix) in [
                ("GET", ""),
                ("DELETE", ""),
                ("POST", "/heartbeat"),
                ("GET", "/boot-profile"),
                ("GET", "/dtb"),
                ("GET", "/dtb/download"),
                ("GET", "/serial"),
                ("GET", "/serial/ws"),
                ("GET", "/loader-status"),
                ("GET", "/tftp"),
                ("GET", "/files"),
                ("PUT", "/files"),
                ("GET", "/files/probe.txt"),
                ("DELETE", "/files/probe.txt"),
                ("PUT", "/http-boot/files"),
                ("PUT", "/http-boot/kernel"),
                ("POST", "/board/power-on"),
                ("POST", "/board/power-off"),
            ] {
                let path = format!("/api/v1/sessions/{sid}{suffix}");
                request(
                    &client,
                    &base,
                    method,
                    &path,
                    None,
                    Value::Null,
                    StatusCode::UNAUTHORIZED,
                )
                .await;
                request(
                    &client,
                    &base,
                    method,
                    &path,
                    Some(&other.access_token),
                    Value::Null,
                    StatusCode::NOT_FOUND,
                )
                .await;
            }
            for prefix in ["/share/sessions/", "/boot/sessions/"] {
                let path = format!("{prefix}{sid}/probe.txt");
                request(
                    &client,
                    &base,
                    "GET",
                    &path,
                    None,
                    Value::Null,
                    StatusCode::UNAUTHORIZED,
                )
                .await;
                request(
                    &client,
                    &base,
                    "GET",
                    &path,
                    Some(&other.access_token),
                    Value::Null,
                    StatusCode::NOT_FOUND,
                )
                .await;
            }
            request(
                &client,
                &base,
                "DELETE",
                &format!("/api/v1/admin/rentals/{rental}"),
                Some(&admin.access_token),
                Value::Null,
                StatusCode::NO_CONTENT,
            )
            .await;
        } else if scenario == "expire" {
            request(&client,&base,"PUT",&format!("/api/v1/admin/rentals/{rental}"),Some(&admin.access_token),json!({"board_id":board.id,"user_id":member.id,"starts_at":start,"ends_at":Utc::now()+ChronoDuration::seconds(2),"notes":"expires during serial"}),StatusCode::NO_CONTENT).await;
        } else {
            request(
                &client,
                &base,
                "DELETE",
                &format!(
                    "/api/v1/auth/personal-access-tokens/{}",
                    pat.as_ref().unwrap()["id"].as_str().unwrap()
                ),
                Some(&web.access_token),
                Value::Null,
                StatusCode::NO_CONTENT,
            )
            .await;
        }
        let output =
            tokio::time::timeout(Duration::from_secs(15), probe.wait_with_output()).await??;
        anyhow::ensure!(
            output.status.success(),
            "CLI probe failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.get_session(&sid).await.is_some() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?;
        if scenario == "revoke" {
            request(
                &client,
                &base,
                "DELETE",
                &format!("/api/v1/admin/rentals/{rental}"),
                Some(&admin.access_token),
                Value::Null,
                StatusCode::NO_CONTENT,
            )
            .await;
        }
    }
    assert_eq!(
        client
            .post(format!("{base}oauth/revoke"))
            .form(&[
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
                ("client_id", "ostool-cli")
            ])
            .send()
            .await?
            .status(),
        StatusCode::NO_CONTENT
    );
    request(
        &client,
        &base,
        "GET",
        "/api/v1/board-types",
        Some(oauth),
        Value::Null,
        StatusCode::UNAUTHORIZED,
    )
    .await;
    server.abort();
    Ok(())
}

/// Invoked only by the parent with an isolated token in its subprocess environment.
#[tokio::test]
#[ignore = "internal subprocess probe for website_rentals_authorize_real_ostool_clients"]
async fn cli_transport_probe() -> Result<()> {
    let Ok(base) = std::env::var("OSTOOL_CONTRACT_BASE") else {
        return Ok(());
    };
    let checkpoint = std::path::PathBuf::from(std::env::var("OSTOOL_CONTRACT_CHECKPOINT")?);
    let revoked = std::env::var("OSTOOL_CONTRACT_REVOKED")? == "true";
    let manager = ostool::auth::token_manager::TokenManager::new(endpoint(&base))?;
    let stored_token = std::env::var("OSTOOL_CONTRACT_STORED_TOKEN")?;
    let stored =
        std::path::PathBuf::from(std::env::var("XDG_CONFIG_HOME")?).join("ostool/hosts.json");
    let device_login = std::env::var("OSTOOL_CONTRACT_DEVICE_LOGIN")? == "true";
    if device_login {
        manager.login_device().await?;
        // Keep the first client on a locally optimistic deadline. The second client
        // will force a real refresh; the first must then recover its rejected token.
        let mut records: Value = serde_json::from_slice(&tokio::fs::read(&stored).await?)?;
        let record = records
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        record["access_expires_at"] = json!(Utc::now() + ChronoDuration::minutes(10));
        tokio::fs::write(&stored, serde_json::to_vec(&records)?).await?;
    } else {
        manager
            .import_personal_access_token(stored_token.clone())
            .await?;
    }
    let client = BoardServerClient::new_with_endpoint(endpoint(&base))?;
    assert_eq!(
        client.list_board_types().await?[0].board_type,
        "contract-test"
    );
    if device_login {
        // Access TTL=60 seconds exercises two independent caches: the second client
        // rotates storage, then the first must reload that refresh token under lock.
        let mut records: Value = serde_json::from_slice(&tokio::fs::read(&stored).await?)?;
        records
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["access_expires_at"] = json!(Utc::now() - ChronoDuration::seconds(1));
        tokio::fs::write(&stored, serde_json::to_vec(&records)?).await?;
        let second = BoardServerClient::new_with_endpoint(endpoint(&base))?;
        assert_eq!(
            second.list_board_types().await?[0].board_type,
            "contract-test"
        );
        assert_eq!(
            client.list_board_types().await?[0].board_type,
            "contract-test"
        );
        // The first client's 401 must recover from the newer saved grant without
        // deleting it. Next, exercise short-TTL refresh with two stale caches.
        second.list_board_types().await?;
        client.list_board_types().await?;
    }
    let lease = client
        .create_session_with_board_id("contract-test", "contract-board")
        .await?;
    let sid = &lease.session_id;
    client.heartbeat(sid).await?;
    client.get_boot_profile(sid).await?;
    client.get_serial_status(sid).await?;
    assert_eq!(
        client.get_loader_status(sid).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    client.get_tftp_status(sid).await?;
    assert!(client.get_session_dtb(sid).await?.dtb_name.is_none());
    assert_eq!(
        client.download_session_dtb(sid).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    client.power_on_board(sid).await?;
    client.power_off_board(sid).await?;
    let file = client
        .upload_session_file(sid, "probe.txt", b"authenticated upload".to_vec())
        .await?;
    let http = Client::builder().no_proxy().build()?;
    let token = client
        .websocket_authorization()
        .await?
        .context("CLI bearer credential")?;
    let download = http
        .get(file.http_url.context("shared HTTP URL")?)
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.bytes().await?.as_ref(), b"authenticated upload");
    assert_eq!(
        client
            .upload_http_boot_file(sid, "probe.txt", vec![1])
            .await
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .upload_http_boot_kernel(
                sid,
                HttpBootKernelUpload {
                    remote_name: "kernel.bin".into(),
                    arch: "aarch64".into(),
                    image_format: "raw".into(),
                    entry_symbol: None,
                    bytes: vec![1],
                    initramfs: None,
                    cmdline: None
                }
            )
            .await
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    let url = client.resolve_ws_url(lease.ws_url.as_deref().context("serial websocket URL")?)?;
    let (writer, mut reader, tasks) =
        connect_serial_stream(url, client.websocket_authorization().await?).await?;
    tokio::fs::write(checkpoint.with_extension("connected"), sid).await?;
    let mut payload = vec![0u8; b"contract serial payload\n".len()];
    tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::io::AsyncReadExt::read_exact(&mut reader, &mut payload),
    )
    .await??;
    assert_eq!(payload, b"contract serial payload\n");
    tokio::fs::write(&checkpoint, sid).await?;
    let mut byte = [0u8; 1];
    let closed = tokio::time::timeout(
        Duration::from_secs(12),
        futures_util::io::AsyncReadExt::read(&mut reader, &mut byte),
    )
    .await?;
    assert!(
        closed.is_err() || closed.unwrap() == 0,
        "serial must stop when access ends"
    );
    drop(writer);
    let _ = tasks.shutdown_with_timeout(Duration::from_secs(2)).await;
    let expected = if revoked {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::NOT_FOUND
    };
    assert_eq!(client.heartbeat(sid).await.unwrap_err().status, expected);
    assert_eq!(
        client
            .create_session("contract-test")
            .await
            .unwrap_err()
            .status,
        if revoked {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::FORBIDDEN
        }
    );
    // Rejecting a process token preserves a separate saved login; explicit logout
    // removes saved PAT credentials and revokes a saved OAuth refresh family.
    let text = tokio::fs::read_to_string(&stored).await?;
    if !device_login {
        assert!(text.contains(&stored_token));
    } else {
        assert!(
            serde_json::from_str::<Value>(&text)?
                .as_object()
                .unwrap()
                .values()
                .any(|record| record["refresh_token"].is_string())
        );
    }
    manager.logout().await?;
    assert!(
        serde_json::from_slice::<Value>(&tokio::fs::read(&stored).await?)?
            .as_object()
            .unwrap()
            .is_empty()
    );

    Ok(())
}
