//! Local edition retains anonymous HTTP without accounts, rentals or Redis.
#[path = "support/database.rs"]
mod test_database;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{tftp::service::build_tftp_manager, *};
use serde_json::{Value, json};
use tower::ServiceExt;

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn editions_share_board_rest_and_serial_contracts() -> anyhow::Result<()> {
    use std::{
        io::{Read, Write},
        time::Duration,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};

    let mut local_contract = Vec::new();
    for edition in [config::Edition::Local, config::Edition::Public] {
        let root = tempfile::tempdir()?;
        let (mut serial_master, mut serial_slave) = serialport::TTYPort::pair()?;
        serial_slave.set_exclusive(false)?;
        let serial_path = serialport::SerialPort::name(&serial_slave).unwrap();
        drop(serial_slave);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/", listener.local_addr()?);
        let mut config = ServerConfig {
            edition,
            data_dir: root.path().join("data"),
            listen_addr: listener.local_addr()?,
            tftp: TftpConfig::Builtin(BuiltinTftpConfig {
                enabled: false,
                ..BuiltinTftpConfig::default_with_root(root.path().join("tftp"))
            }),
            ..Default::default()
        };
        config.auth.public_base_url = Some(base.clone());
        let database = test_database::prepare(&root.path().join("test.env"), &mut config).await?;
        let token = if edition.is_public() {
            let admin = database
                .create_user(
                    "contract_admin",
                    "admin@contract.test",
                    "contract-test-password",
                    None,
                    true,
                )
                .await?;
            Some(
                database
                    .issue_grant(admin, "ostool-cli", "board:operate", &config.auth)
                    .await?
                    .access_token,
            )
        } else {
            config.security.redis_url = None;
            None
        };
        let board = BoardConfig {
            id: "contract-board".into(),
            board_type: "contract-test".into(),
            tags: vec!["fixture".into()],
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
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            notes: None,
            disabled: false,
            network_identity: None,
            allow_instant_use: false,
            reset_command: None,
        };
        BoardStore::new(database.clone())
            .write_board(&board)
            .await?;
        let state = build_app_state(
            root.path().join("test.env"),
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
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let call = |method: &str, path: &str, body: &str, authenticated: bool| {
            let mut request = client
                .request(
                    method.parse().unwrap(),
                    format!("{base}{}", path.trim_start_matches('/')),
                )
                .header("content-type", "application/json")
                .header("x-file-path", "probe.txt")
                .body(body.to_string());
            if authenticated && let Some(token) = &token {
                request = request.bearer_auth(token);
            }
            request.send()
        };
        let mut contract = Vec::new();
        assert_eq!(
            call("GET", "/api/v1/admin/boards", "", false)
                .await?
                .status(),
            if edition.is_public() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::OK
            }
        );
        if edition.is_public() {
            assert_eq!(
                call("GET", "/api/v1/admin/boards", "", true)
                    .await?
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let response = call("GET", "/api/v1/auth/config", "", false)
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        assert_eq!(response["enabled"], edition.is_public());
        assert_eq!(
            response["edition"],
            if edition.is_public() {
                "public"
            } else {
                "local"
            }
        );

        for path in [
            "/api/v1/board-types",
            "/api/v1/sessions/missing",
            "/api/v1/sessions/missing/serial/ws",
        ] {
            if edition.is_public() {
                assert_eq!(
                    call("GET", path, "", false).await?.status(),
                    StatusCode::UNAUTHORIZED,
                    "{path}"
                );
            }
        }
        let response = call("GET", "/api/v1/board-types", "", true)
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        assert_eq!(
            response,
            json!([{"board_type":"contract-test","tags":["fixture"],"total":1,"available":1}])
        );
        if edition.is_public() {
            assert_eq!(
                call(
                    "POST",
                    "/api/v1/sessions",
                    r#"{"board_type":"contract-test"}"#,
                    false
                )
                .await?
                .status(),
                StatusCode::UNAUTHORIZED
            );
            assert!(state.sessions.read().await.is_empty());
        }
        let response = call("POST", "/api/v1/sessions", r#"{"board_type":"contract-test","board_id":"contract-board","required_tags":["fixture"],"client_name":"ostool"}"#, true).await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let session = response.json::<Value>().await?;
        contract.push(json_shape(&session));
        let sid = session["session_id"].as_str().unwrap();
        assert_eq!(session["boot_mode"], "uboot");
        assert_eq!(session["serial_available"], true);
        for (method, suffix, body, expected) in [
            ("GET", "", "", StatusCode::OK),
            ("POST", "/heartbeat", "", StatusCode::OK),
            ("GET", "/boot-profile", "", StatusCode::OK),
            ("GET", "/serial", "", StatusCode::OK),
            ("GET", "/tftp", "", StatusCode::OK),
            ("GET", "/dtb", "", StatusCode::OK),
            ("PUT", "/files", "same file bytes", StatusCode::CREATED),
            ("GET", "/files", "", StatusCode::OK),
            ("GET", "/files/probe.txt", "", StatusCode::OK),
            ("POST", "/commands", r#"["echo contract"]"#, StatusCode::OK),
        ] {
            let path = format!("/api/v1/sessions/{sid}{suffix}");
            if edition.is_public() {
                assert_eq!(
                    call(method, &path, body, false).await?.status(),
                    StatusCode::UNAUTHORIZED,
                    "{method} {suffix}"
                );
            }
            let response = call(method, &path, body, true).await?;
            assert_eq!(
                response.status(),
                expected,
                "{edition:?}: {method} {suffix}"
            );
            if expected != StatusCode::NO_CONTENT {
                contract.push(json_shape(&response.json::<Value>().await?));
            }
        }
        let shared_path = format!("/share/sessions/{sid}/probe.txt");
        if edition.is_public() {
            assert_eq!(
                call("GET", &shared_path, "", false).await?.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let download = call("GET", &shared_path, "", true).await?;
        assert_eq!(download.status(), StatusCode::OK);
        assert_eq!(download.bytes().await?.as_ref(), b"same file bytes");
        assert_eq!(
            call(
                "DELETE",
                &format!("/api/v1/sessions/{sid}/files/probe.txt"),
                "",
                true
            )
            .await?
            .status(),
            StatusCode::NO_CONTENT
        );
        // Both deployments accept exactly the same WebSocket handshake and serial bytes.
        let ws: url::Url = format!(
            "ws://{}/api/v1/sessions/{sid}/serial/ws",
            config.listen_addr
        )
        .parse()?;
        if edition.is_public() {
            let error = ostool::board::serial_stream::connect_serial_stream(ws.clone(), None)
                .await
                .err()
                .expect("public serial requires authentication");
            assert!(format!("{error:#}").contains("401"), "{error:#}");
        }
        let (tx, rx, tasks) =
            ostool::board::serial_stream::connect_serial_stream(ws, token.clone()).await?;
        let mut rx = rx.compat();
        let mut tx = tx.compat_write();
        let device = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            serialport::SerialPort::set_timeout(&mut serial_master, Duration::from_secs(5))?;
            let mut ping = [0; 4];
            serial_master.read_exact(&mut ping)?;
            assert_eq!(&ping, b"ping");
            serial_master.write_all(b"contract serial\r\n")?;
            Ok(serial_master)
        });
        tx.write_all(b"ping").await?;
        let _serial_master = device.await??;
        let mut bytes = vec![0; b"contract serial\r\n".len()];
        tokio::time::timeout(Duration::from_secs(5), rx.read_exact(&mut bytes)).await??;
        assert_eq!(bytes, b"contract serial\r\n");
        drop(tx);
        drop(rx);
        tasks.shutdown_with_timeout(Duration::from_secs(2)).await?;
        state.shutdown_sessions().await?;
        server.abort();
        if edition.is_public() {
            assert_eq!(contract, local_contract);
        } else {
            local_contract = contract;
        }
    }
    Ok(())
}

fn json_shape(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), json_shape(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(json_shape).collect()),
        Value::String(_) => json!("string"),
        Value::Number(_) => json!("number"),
        Value::Bool(_) => json!("boolean"),
        Value::Null => Value::Null,
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn local_anonymous_http_allocates_exclusively_and_persists_files_without_accounts()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let mut config = ServerConfig {
        data_dir: root.path().join("data"),
        listen_addr: "127.0.0.1:18444".parse()?,
        ..Default::default()
    };
    config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
        root.path().join("cache"),
    ));
    config.network.interface = "lo".into();
    config.database.url = Some(std::env::var("OSTOOL_TEST_DATABASE_URL")?);
    config.database.schema = Some(format!(
        "ostool_test_local_{}",
        uuid::Uuid::new_v4().simple()
    ));
    config.database.max_connections = 4;
    config.security.redis_url = None;
    config.auth.public_base_url = None;
    let state = build_app_state(
        root.path().join("server.env"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await?;
    assert!(!state.config.read().await.auth.enabled);
    assert!(state.security_cache.is_none());
    let app = build_router(state.clone());
    let call = |method: &str, path: &str, body: Value| {
        app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };
    let response = call("GET", "/api/v1/auth/config", Value::Null).await?;
    let config: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(config["enabled"], false);
    assert_eq!(config["allow_registration"], false);
    for path in [
        "/api/v1/account",
        "/api/v1/admin/users",
        "/api/v1/admin/rentals",
    ] {
        assert_eq!(
            call("GET", path, Value::Null).await?.status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(call("POST","/api/v1/admin/boards",json!({"id":"local","board_type":"local-test","tags":[],"disabled":false,"serial":{"key":{"kind":"serial_number","value":"local-console"},"baud_rate":115200},"boot":{"kind":"uboot","use_tftp":false},"power_management":{"kind":"custom","power_on_cmd":"true","power_off_cmd":"true"}})).await?.status(),StatusCode::CREATED);
    let response = call(
        "POST",
        "/api/v1/sessions",
        json!({"board_type":"local-test"}),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let session: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    let sid = session["session_id"].as_str().unwrap();
    assert_eq!(
        call(
            "POST",
            "/api/v1/sessions",
            json!({"board_type":"local-test"})
        )
        .await?
        .status(),
        StatusCode::CONFLICT
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/sessions/{sid}/files"))
                .header("x-file-path", "shared.bin")
                .body(Body::from("local file"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let url = device_security::file_url(&state, sid, "shared.bin").await?;
    assert_eq!(url.scheme(), "http");
    assert!(url.query().is_none());
    let response = call("GET", url.path(), Value::Null).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await?[..],
        b"local file"
    );
    for table in ["users", "oauth_sessions", "session_owners", "board_rentals"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(state.database.pool())
            .await?;
        assert_eq!(count, 0, "local must not create {table}");
    }
    assert_eq!(
        call("DELETE", &format!("/api/v1/sessions/{sid}"), Value::Null)
            .await?
            .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.get_session(sid).await.is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM session_files")
        .fetch_one(state.database.pool())
        .await?;
    assert_eq!(count, 0);
    assert_eq!(
        call(
            "POST",
            "/api/v1/sessions",
            json!({"board_type":"local-test"})
        )
        .await?
        .status(),
        StatusCode::CREATED
    );
    state.shutdown_sessions().await?;
    Ok(())
}
