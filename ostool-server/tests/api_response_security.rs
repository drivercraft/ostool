//! Public security boundaries with real PostgreSQL; no production bypass switches.
#[path = "support/database.rs"]
mod test_database;
#[path = "support/security.rs"]
mod test_security;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use ostool_server::{
    AppState, AuthConfig, BuiltinTftpConfig, ServerConfig, TftpConfig, build_app_state,
    build_router, config::Edition, security::crypto, tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}
struct Fixture {
    state: AppState,
    app: Router,
    _root: tempfile::TempDir,
    origin: String,
}
impl Fixture {
    async fn new(public: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = ServerConfig {
            edition: if public {
                Edition::Public
            } else {
                Edition::Local
            },
            data_dir: root.path().join("data"),
            auth: AuthConfig {
                public_base_url: Some("http://127.0.0.1:4190".into()),
                bootstrap_admin_email: Some("admin@security.test".into()),
                bootstrap_admin_password: Some("security-admin-password-123".into()),
                ..Default::default()
            },
            tftp: TftpConfig::Builtin(BuiltinTftpConfig {
                enabled: false,
                ..BuiltinTftpConfig::default_with_root(root.path().join("tftp"))
            }),
            ..Default::default()
        };
        config.security.mfa_encryption_key = Some(STANDARD.encode([7u8; 32]));
        test_database::prepare(&root.path().join("config.toml"), &mut config)
            .await
            .unwrap();
        let state = build_app_state(
            root.path().join("config.toml"),
            config.clone(),
            build_tftp_manager(&config.tftp),
        )
        .await
        .unwrap();
        Self {
            app: build_router(state.clone()),
            state,
            _root: root,
            origin: "http://127.0.0.1:4190".into(),
        }
    }
    async fn request(&self, method: &str, path: &str, cookies: Option<&str>, body: Value) -> Reply {
        self.custom(method, path, cookies, body, Some(&self.origin), None)
            .await
    }
    async fn custom(
        &self,
        method: &str,
        path: &str,
        cookies: Option<&str>,
        mut body: Value,
        origin: Option<&str>,
        bearer: Option<&str>,
    ) -> Reply {
        if (path == "/api/v1/auth/register" || path.starts_with("/api/v1/admin/users"))
            && body.is_object()
            && body.get("username").is_none()
        {
            body["username"] = json!(
                body["email"]
                    .as_str()
                    .unwrap_or("fixture")
                    .split('@')
                    .next()
                    .unwrap()
                    .replace('.', "_")
            );
        }
        if path == "/api/v1/auth/register" && body.get("legal").is_none() {
            body["legal"] = json!({"accepted":true,"terms_version":1,"privacy_version":1});
        }
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("user-agent", "ostool-security-test")
            .header(
                "x-ostool-surface",
                if cookies.is_some_and(|c| c.contains("ostool-admin-")) {
                    "admin"
                } else {
                    "website"
                },
            )
            .header("x-forwarded-for", "127.0.0.1");
        if let Some(cookies) = cookies {
            request = request.header("cookie", cookies);
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if let Some(token) = bearer {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let mut request = request.body(Body::from(body.to_string())).unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "203.0.113.5:4190".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        Reply {
            status,
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }
    async fn proof(&self, action: &str) -> Value {
        let result = self
            .request(
                "GET",
                &format!("/api/v1/auth/captcha?action={action}"),
                None,
                Value::Null,
            )
            .await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.body);
        let id: Uuid = result.body["id"].as_str().unwrap().parse().unwrap();
        sqlx::query("UPDATE security_challenges SET secret_hash=$2 WHERE id=$1")
            .bind(id)
            .bind(crypto::proof_hash(
                &format!("captcha:{action}"),
                id,
                "AB29Q",
            ))
            .execute(self.state.database.pool())
            .await
            .unwrap();
        json!({"captcha_id":id,"captcha_code":"AB29Q"})
    }
    async fn login(&self, admin: bool) -> Reply {
        self.request("POST",if admin{"/api/v1/admin/auth/login"}else{"/api/v1/auth/login"},None,json!({"email":"admin@security.test","password":"security-admin-password-123","verification":self.proof(if admin{"admin_login"}else{"login"}).await})).await
    }
}

async fn add_boards(f: &Fixture) -> anyhow::Result<()> {
    use ostool_server::{BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig};
    for index in 1..=2 {
        let board = BoardConfig {
            allow_instant_use: false,
            reset_command: Some(format!("true # PRIVATE_RESET_{index}")),
            id: format!("private-board-{index}"),
            board_type: "privacy-type".into(),
            tags: vec!["研发".into()],
            serial: None,
            boot: BootConfig::Uboot(Default::default()),
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: format!("true # PRIVATE_POWER_{index}"),
                power_off_cmd: "true".into(),
            }),
            notes: Some(format!("PRIVATE_NOTES_{index}")),
            network_identity: None,
            disabled: false,
        };
        f.state.board_store.write_board(&board).await?;
        f.state.boards.write().await.insert(board.id.clone(), board);
    }
    f.state.sync_board_runtime_states().await;
    f.state.admin_events.invalidate(&["boards"]);
    Ok(())
}
async fn stream(
    f: &Fixture,
    path: &str,
    cookies: Option<&str>,
) -> anyhow::Result<axum::response::Response> {
    let mut request = Request::builder()
        .uri(path)
        .header("user-agent", "ostool-security-test");
    if let Some(cookie) = cookies {
        request = request.header("cookie", cookie);
    }
    let mut request = request.body(Body::empty())?;
    request.extensions_mut().insert(ConnectInfo(
        "203.0.113.5:4190".parse::<std::net::SocketAddr>()?,
    ));
    Ok(f.app.clone().oneshot(request).await?)
}
async fn event(response: axum::response::Response, topic: &str) -> anyhow::Result<Value> {
    use futures_util::StreamExt;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(bytes) = stream.next().await {
            for line in String::from_utf8(bytes?.to_vec())?
                .lines()
                .filter_map(|s| s.strip_prefix("data: "))
            {
                let data: Value = serde_json::from_str(line)?;
                if let Some(value) = data["data"].get(topic) {
                    return Ok::<_, anyhow::Error>(value.clone());
                }
            }
        }
        anyhow::bail!("stream closed without requested topic")
    })
    .await?
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn inventory_and_unscoped_events_never_return_commands_but_selected_editor_keeps_working()
-> anyhow::Result<()> {
    let f = Fixture::new(true).await;
    add_boards(&f).await?;
    let login = f.login(true).await;
    assert_eq!(login.status, StatusCode::OK);
    let cookie = test_security::cookie_header(&login.headers);
    let list = f
        .request("GET", "/api/v1/admin/boards", Some(&cookie), Value::Null)
        .await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body.as_array().unwrap().len(), 2);
    for board in list.body.as_array().unwrap() {
        assert_eq!(board.as_object().unwrap().len(), 9);
        for field in ["power_management", "reset_command", "notes"] {
            assert!(board.get(field).is_none());
        }
        assert_eq!(board["boot"].as_object().unwrap().len(), 1);
    }
    assert!(!list.body.to_string().contains("PRIVATE_"));
    let global = event(
        stream(&f, "/api/v1/admin/events?topics=boards", Some(&cookie)).await?,
        "boards",
    )
    .await?;
    assert!(!global.to_string().contains("PRIVATE_"));
    for index in 1..=2 {
        let path = format!(
            "/api/v1/admin/events?topics=board_editor&board_editor_id=private-board-{index}"
        );
        let selected = event(stream(&f, &path, Some(&cookie)).await?, "board_editor").await?;
        assert_eq!(selected["id"], format!("private-board-{index}"));
        assert!(
            selected
                .to_string()
                .contains(&format!("PRIVATE_POWER_{index}"))
        );
        assert!(
            !selected
                .to_string()
                .contains(&format!("PRIVATE_POWER_{}", 3 - index))
        );
        let detail = f
            .request(
                "GET",
                &format!("/api/v1/admin/boards/private-board-{index}"),
                Some(&cookie),
                Value::Null,
            )
            .await;
        assert_eq!(detail.status, StatusCode::OK);
        assert!(
            detail
                .body
                .to_string()
                .contains(&format!("PRIVATE_POWER_{index}"))
        );
    }
    let none = stream(
        &f,
        "/api/v1/admin/events?topics=board_editor",
        Some(&cookie),
    )
    .await?;
    use futures_util::StreamExt;
    let mut bytes = none.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(3), bytes.next())
        .await?
        .unwrap()?;
    assert!(!String::from_utf8_lossy(&first).contains("PRIVATE_"));
    // Editing a board keeps its existing metadata and command while sending only summary updates globally.
    let mut detail = f
        .request(
            "GET",
            "/api/v1/admin/boards/private-board-1",
            Some(&cookie),
            Value::Null,
        )
        .await
        .body;
    detail["notes"] = json!("changed editor notes");
    assert_eq!(
        f.request(
            "PUT",
            "/api/v1/admin/boards/private-board-1",
            Some(&cookie),
            detail
        )
        .await
        .status,
        StatusCode::OK
    );
    let selected = event(
        stream(
            &f,
            "/api/v1/admin/events?topics=board_editor&board_editor_id=private-board-1",
            Some(&cookie),
        )
        .await?,
        "board_editor",
    )
    .await?;
    assert_eq!(selected["board"]["notes"], "changed editor notes");
    assert!(selected.to_string().contains("PRIVATE_POWER_1"));
    let list = f
        .request("GET", "/api/v1/admin/boards", Some(&cookie), Value::Null)
        .await;
    assert!(!list.body.to_string().contains("PRIVATE_"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn editor_scope_never_bypasses_management_authentication_or_public_member_projection()
-> anyhow::Result<()> {
    let f = Fixture::new(true).await;
    add_boards(&f).await?;
    let member = f
        .state
        .database
        .create_user(
            "privacy_member",
            "privacy@test.local",
            "privacy-password-123",
            None,
            false,
        )
        .await?;
    let config = f.state.config.read().await.clone();
    let cli = f
        .state
        .database
        .issue_grant(
            member.clone(),
            "ostool-cli",
            "board:operate offline_access",
            &config.auth,
        )
        .await?;
    let web = f
        .state
        .database
        .issue_grant(member, "web", "board:operate", &config.auth)
        .await?;
    let path = "/api/v1/admin/events?topics=board_editor&board_editor_id=private-board-1";
    assert_eq!(
        stream(&f, path, None).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    for token in [&cli.access_token, &web.access_token] {
        let request = Request::builder()
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("x-ostool-surface", "admin")
            .body(Body::empty())?;
        assert_eq!(
            f.app.clone().oneshot(request).await?.status(),
            StatusCode::FORBIDDEN
        );
    }
    use futures_util::StreamExt;
    for (path, token) in [
        (
            "/api/v1/site/events?topics=board_editor&board_editor_id=private-board-1",
            None,
        ),
        (
            "/api/v1/account/events?topics=board_editor&board_editor_id=private-board-1",
            Some(&web.access_token),
        ),
    ] {
        let mut request = Request::builder().uri(path);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = f.app.clone().oneshot(request.body(Body::empty())?).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        let mut collected = String::new();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some(bytes) = stream.next().await {
                collected.push_str(&String::from_utf8_lossy(&bytes.unwrap()));
            }
        })
        .await;
        assert!(collected.contains("snapshot"));
        assert!(!collected.contains("PRIVATE_"));
        assert!(!collected.contains("board_editor"));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn errors_and_power_results_are_redacted_and_have_correlatable_request_ids()
-> anyhow::Result<()> {
    let f = Fixture::new(true).await;
    let login = f.login(true).await;
    let cookie = test_security::cookie_header(&login.headers);
    let action=f.request("POST","/api/v1/admin/power-actions",Some(&cookie),json!({"request_id":"private-power-job","action":"on","power_management":{"kind":"custom","power_on_cmd":"true # PRIVATE_CONTROL_TOKEN","power_off_cmd":"true"}})).await;
    assert_eq!(action.status, StatusCode::ACCEPTED);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let reply = f
                .request(
                    "GET",
                    "/api/v1/admin/power-actions/private-power-job",
                    Some(&cookie),
                    Value::Null,
                )
                .await;
            if reply.body["state"] == "succeeded" {
                assert!(!reply.body.to_string().contains("PRIVATE_CONTROL_TOKEN"));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    sqlx::query("ALTER TABLE ota_images RENAME COLUMN sha256 TO private_digest")
        .execute(f.state.database.pool())
        .await?;
    let error = f
        .request(
            "DELETE",
            &format!("/api/v1/admin/loader-images/{}", "a".repeat(64)),
            Some(&cookie),
            Value::Null,
        )
        .await;
    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    let text = error.body.to_string();
    for secret in ["Postgres", "SQL", "column", "sha256", "private_digest"] {
        assert!(!text.contains(secret), "{text}");
    }
    let id = error.headers.get("x-request-id").unwrap().to_str()?;
    let request_id = Uuid::parse_str(id)?;
    let logged: Uuid =
        sqlx::query_scalar("SELECT request_id FROM audit_events WHERE request_id=$1 LIMIT 1")
            .bind(request_id)
            .fetch_one(f.state.database.pool())
            .await?;
    assert_eq!(logged, request_id);
    assert_eq!(error.headers.get("cache-control").unwrap(), "no-store");
    Ok(())
}

#[test]
fn runtime_and_history_serialization_omit_binding_credentials_and_raw_diagnostics()
-> anyhow::Result<()> {
    use ostool_server::{api::models::LoaderDeviceSummary, serial::runtime::SerialRuntimeStatus};
    let runtime = SerialRuntimeStatus {
        port: Some("/dev/serial/by-id/uart".into()),
        binding_id: Some("PRIVATE_BINDING".into()),
        boot_epoch: Some("PRIVATE_EPOCH".into()),
        error: Some("PRIVATE_DEVICE_RESPONSE".into()),
        warning: Some("PRIVATE_WARNING".into()),
        ..Default::default()
    };
    let view = serde_json::to_value(runtime.management_view())?;
    assert!(view.get("binding_id").is_none());
    assert!(view.get("boot_epoch").is_none());
    assert!(!view.to_string().contains("PRIVATE_"));
    assert_eq!(view["port"], "/dev/serial/by-id/uart");
    let loader = LoaderDeviceSummary {
        mac_address: "02:00:00:00:00:01".parse()?,
        current_mac_address: "02:00:00:00:00:01".parse()?,
        ip_address: "192.168.1.2".into(),
        arch: httpboot_protocol::BootArch::X86_64,
        loader_version: "1".into(),
        hardware: Default::default(),
        last_seen_at: Utc::now(),
        online: true,
        conflict: false,
        bound_board_id: None,
    };
    assert!(
        serde_json::to_value(loader)?
            .get("current_registration_id")
            .is_none()
    );
    let history: ostool_server::persistence::leases::LeaseRecord = serde_json::from_value(json!({
        "session_id":"lease","board_id":"board","board_type":"privacy",
        "created_at":Utc::now(),"last_heartbeat_at":Utc::now(),"expires_at":Utc::now(),
        "release_failures":1,"last_release_error":"PRIVATE_RELEASE_ERROR"
    }))?;
    assert!(!serde_json::to_string(&history)?.contains("PRIVATE_RELEASE_ERROR"));
    let job = ostool_server::ota::Job {
        board_id: "board".into(),
        mac_address: "02:00:00:00:00:01".parse()?,
        update_id: "update".into(),
        image: ostool_server::ota::Image {
            sha256: "a".repeat(64),
            size: 1,
            version: None,
        },
        phase: ostool_server::ota::Phase::Failed,
        error: Some("PRIVATE_OTA_ERROR".into()),
        delivery_attempts: 1,
    };
    assert!(!serde_json::to_string(&job.management_view())?.contains("PRIVATE_OTA_ERROR"));
    for error in [
        ostool_server::api::error::ApiError::internal("PRIVATE_DB_ERROR"),
        ostool_server::api::error::ApiError::service_unavailable("PRIVATE_PROCESS_ERROR"),
    ] {
        assert!(!error.message.contains("PRIVATE_"));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn revoked_editor_stream_does_not_receive_subsequent_device_configuration()
-> anyhow::Result<()> {
    use futures_util::StreamExt;
    let f = Fixture::new(true).await;
    add_boards(&f).await?;
    let login = f.login(true).await;
    let cookie = test_security::cookie_header(&login.headers);
    let response = stream(
        &f,
        "/api/v1/admin/events?topics=board_editor&board_editor_id=private-board-1",
        Some(&cookie),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let mut events = response.into_body().into_data_stream();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(bytes) = events.next().await {
            if String::from_utf8_lossy(&bytes.unwrap()).contains("PRIVATE_POWER_1") {
                return;
            }
        }
        panic!("editor did not receive initial authorized configuration");
    })
    .await?;
    sqlx::query("UPDATE oauth_sessions SET revoked_at=now()")
        .execute(f.state.database.pool())
        .await?;
    f.state
        .boards
        .write()
        .await
        .get_mut("private-board-1")
        .unwrap()
        .reset_command = Some("true # AFTER_REVOKE_PRIVATE_COMMAND".into());
    f.state.admin_events.invalidate(&["boards"]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(bytes) = events.next().await {
            let text = String::from_utf8_lossy(&bytes.unwrap()).into_owned();
            assert!(!text.contains("AFTER_REVOKE_PRIVATE_COMMAND"));
            if text.contains("auth-expired") {
                return;
            }
        }
        panic!("revoked editor stream ended without expiry notification");
    })
    .await?;
    Ok(())
}
