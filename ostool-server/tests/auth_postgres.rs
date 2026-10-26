//! Run with OSTOOL_TEST_DATABASE_URL pointing to a disposable PostgreSQL DB.
//! Each run creates an isolated schema; no existing tables are modified.
#[path = "support/security.rs"]
mod test_security;
use test_security::access_cookie;
static TEST_DATABASE: std::sync::OnceLock<ostool_server::persistence::Database> =
    std::sync::OnceLock::new();
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{
    AuthConfig, DatabaseConfig, ServerConfig, build_app_state, build_router,
    config::{BuiltinTftpConfig, TftpConfig},
    persistence::{BoardStore, Database},
    tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    mut body: Value,
) -> (StatusCode, Value, axum::http::HeaderMap) {
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
    if method == "POST"
        && matches!(
            path,
            "/api/v1/auth/login" | "/api/v1/admin/auth/login" | "/api/v1/auth/register"
        )
    {
        let headers = axum::http::HeaderMap::new();
        let client = ostool_server::security::store::ClientContext::new("unknown".into(), &headers);
        let action = if path.ends_with("register") {
            ostool_server::security::captcha::HumanAction::Register
        } else if path.starts_with("/api/v1/admin/") {
            ostool_server::security::captcha::HumanAction::AdminLogin
        } else {
            ostool_server::security::captcha::HumanAction::Login
        };
        if path.ends_with("/register") {
            body["legal"] = json!({"accepted":true,"terms_version":1,"privacy_version":1});
        }
        body["verification"] =
            test_security::proof(TEST_DATABASE.get().unwrap(), action, &client).await;
    }
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    )
}

async fn form(app: &Router, path: &str, fields: &[(&str, &str)]) -> (StatusCode, Value) {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and PostgreSQL"]
async fn registration_roles_revocation_and_inventory_restart() {
    let database_url =
        std::env::var("OSTOOL_TEST_DATABASE_URL").expect("OSTOOL_TEST_DATABASE_URL required");
    let root = sqlx::PgPool::connect(&database_url).await.unwrap();
    let schema = format!("ostool_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await
        .unwrap();
    let mut url = url::Url::parse(&database_url).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema},public"));
    let dir = tempfile::tempdir().unwrap();
    let mut config = ServerConfig {
        edition: ostool_server::config::Edition::Public,
        data_dir: dir.path().join("data"),

        ..Default::default()
    };
    config.network.interface = "lo".into();
    config.database = DatabaseConfig {
        url: Some(url.to_string()),
        ..Default::default()
    };
    config.auth = AuthConfig {
        enabled: true,
        bootstrap_admin_email: Some("admin@test.local".into()),
        bootstrap_admin_password: Some("test-admin-password".into()),
        public_base_url: Some("https://boards.example.com".into()),
        ..Default::default()
    };
    let mut tftp = BuiltinTftpConfig::default_with_root(dir.path().join("tftp"));
    tftp.enabled = false;
    config.tftp = TftpConfig::Builtin(tftp);
    let repository = BoardStore::new(Database::connect(&config.database).await.unwrap());
    let board = ostool_server::BoardConfig {
        allow_instant_use: false,
        reset_command: None,
        id: "test-board".into(),
        board_type: "test".into(),
        tags: vec![],
        serial: None,
        power_management: ostool_server::PowerManagementConfig::Custom(
            ostool_server::CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            },
        ),
        boot: ostool_server::BootConfig::Pxe(ostool_server::PxeProfile::default()),
        network_identity: None,
        notes: None,
        disabled: false,
    };
    repository.write_board(&board).await.unwrap();
    config.security.redis_url =
        Some(std::env::var("OSTOOL_TEST_REDIS_URL").expect("disposable Redis required"));
    let state = build_app_state(
        dir.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    TEST_DATABASE.set(state.database.clone()).unwrap();
    let app = build_router(state.clone());
    let (status, _login, _login_headers) = call(
        &app,
        "POST",
        "/api/v1/admin/auth/login",
        None,
        json!({"email":"admin@test.local","password":"test-admin-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin = access_cookie(&_login_headers);
    let payload = json!({"email":"member@test.local","password":"test-member-password","display_name":"Member"});
    let (status, _registered, _registered_headers) =
        call(&app, "POST", "/api/v1/auth/register", None, payload.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    let member = access_cookie(&_registered_headers);
    assert_eq!(
        call(&app, "POST", "/api/v1/auth/register", None, payload)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/register",
            None,
            json!({"email":"bad","password":"short"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/admin/users", None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/admin/users",
            Some(member),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/admin/events",
            Some(member),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, me, _me_headers) =
        call(&app, "GET", "/api/v1/auth/me", Some(member), Value::Null).await;
    assert_eq!(me["is_admin"], false);
    let id = me["id"].as_str().unwrap();
    let (_, device) = form(
        &app,
        "/oauth/device/code",
        &[
            ("client_id", "ostool-cli"),
            ("scope", "board:operate offline_access"),
        ],
    )
    .await;
    assert!(
        device["verification_uri_complete"]
            .as_str()
            .unwrap()
            .starts_with("https://boards.example.com/login?")
    );
    let code = device["device_code"].as_str().unwrap();
    let exchange = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", code),
        ("client_id", "ostool-cli"),
    ];
    assert_eq!(
        form(&app, "/oauth/token", &exchange).await.1["error"],
        "authorization_pending"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/device/authorize",
            Some(member),
            json!({"user_code":device["user_code"]})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, grant) = form(&app, "/oauth/token", &exchange).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        form(&app, "/oauth/token", &exchange).await.1["error"],
        "expired_token"
    );
    let refresh = grant["refresh_token"].as_str().unwrap();
    let (_, rotated) = form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", "ostool-cli"),
        ],
    )
    .await;
    assert!(rotated["refresh_token"].is_string());
    assert_eq!(
        form(
            &app,
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", refresh)]
        )
        .await
        .1["error"],
        "invalid_grant"
    );
    assert_eq!(
        form(
            &app,
            "/oauth/revoke",
            &[("token", rotated["refresh_token"].as_str().unwrap())]
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // Public members need an active allocation, including board-type discovery.
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/sessions",
            Some(member),
            json!({"board_type":"test"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/board-types",
            Some(member),
            Value::Null
        )
        .await
        .1,
        json!([])
    );
    let rental_start = chrono::Utc::now() - chrono::Duration::minutes(1);
    let rental_end = chrono::Utc::now() + chrono::Duration::hours(1);
    let assignment = json!({"board_id":"test-board","user_id":id,"starts_at":rental_start,"ends_at":rental_end,"notes":"integration rental"});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/rentals",
            Some(member),
            assignment.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/rentals",
            Some(admin),
            assignment.clone()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/rentals",
            Some(admin),
            assignment.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let adjacent = json!({"board_id":"test-board","user_id":id,"starts_at":rental_end,"ends_at":rental_end+chrono::Duration::hours(1)});
    assert_eq!(
        call(&app, "POST", "/api/v1/admin/rentals", Some(admin), adjacent)
            .await
            .0,
        StatusCode::CREATED
    );
    let concurrent = json!({"board_id":"test-board","user_id":id,"starts_at":rental_end+chrono::Duration::hours(2),"ends_at":rental_end+chrono::Duration::hours(3)});
    let (first, second) = tokio::join!(
        call(
            &app,
            "POST",
            "/api/v1/admin/rentals",
            Some(admin),
            concurrent.clone()
        ),
        call(
            &app,
            "POST",
            "/api/v1/admin/rentals",
            Some(admin),
            concurrent
        )
    );
    assert!([first.0, second.0].contains(&StatusCode::CREATED));
    assert!([first.0, second.0].contains(&StatusCode::CONFLICT));
    let (_, rentals, _rentals_headers) = call(
        &app,
        "GET",
        "/api/v1/admin/rentals",
        Some(admin),
        Value::Null,
    )
    .await;
    let active_id = rentals["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] == "active")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        call(
            &app,
            "DELETE",
            "/api/v1/admin/boards/test-board",
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // Member leases are isolated from other users, including serial/file paths.
    let (status, lease, _lease_headers) = call(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(member),
        json!({"board_type":"test"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let sid = lease["session_id"].as_str().unwrap();
    let path = format!("/api/v1/sessions/{sid}");
    assert_eq!(
        call(&app, "GET", &path, Some(member), Value::Null).await.0,
        StatusCode::OK
    );
    let (_, _other, _other_headers) = call(
        &app,
        "POST",
        "/api/v1/auth/register",
        None,
        json!({"email":"other@test.local","password":"other-password-123"}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "GET",
            &path,
            Some(access_cookie(&_other_headers)),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", &path, Some(admin), Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "DELETE", &path, Some(member), Value::Null)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.get_session(sid).await.is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // A serial-style heartbeat cannot extend beyond the allocation deadline.
    let (_, leased, _leased_headers) = call(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(member),
        json!({"board_type":"test"}),
    )
    .await;
    let leased_id = leased["session_id"].as_str().unwrap();
    let cutoff = chrono::Utc::now() + chrono::Duration::seconds(2);
    let shortened = json!({"board_id":"test-board","user_id":id,"starts_at":rental_start,"ends_at":cutoff,"notes":"shortened"});
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/api/v1/admin/rentals/{active_id}"),
            Some(admin),
            shortened
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let runtime = state.session_state(leased_id).await.unwrap();
    assert!(runtime.heartbeat().await.expires_at <= cutoff);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.get_session(leased_id).await.is_some() {
            runtime.heartbeat().await;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/sessions",
            Some(member),
            json!({"board_type":"test"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let replacement = json!({"board_id":"test-board","user_id":id,"starts_at":chrono::Utc::now(),"ends_at":chrono::Utc::now()+chrono::Duration::minutes(5)});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/rentals",
            Some(admin),
            replacement
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (_, list, _list_headers) = call(
        &app,
        "GET",
        "/api/v1/admin/rentals",
        Some(admin),
        Value::Null,
    )
    .await;
    let cancel_id = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] == "active")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (_, leased, _leased_headers) = call(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(member),
        json!({"board_type":"test"}),
    )
    .await;
    let cancelled_session = leased["session_id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/admin/rentals/{cancel_id}"),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.get_session(cancelled_session).await.is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/sessions",
            Some(member),
            json!({"board_type":"test"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let roles_path = "/api/v1/admin/roles";
    assert_eq!(
        call(
            &app,
            "POST",
            roles_path,
            Some(admin),
            json!({"name":"Test role","description":"test","permissions":["board.use","rental.request"]})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (_, roles, _roles_headers) = call(&app, "GET", roles_path, Some(admin), Value::Null).await;
    let role = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "Test role")
        .unwrap();
    let role_id = role["id"].as_str().unwrap();
    assert_eq!(call(&app,"PUT",&format!("/api/v1/admin/users/{id}"),Some(admin),json!({"email":"member@test.local","display_name":"Member","role_id":role_id,"disabled":true})).await.0,StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", "/api/v1/auth/me", Some(member), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("{roles_path}/{role_id}"),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, admin_me, _admin_me_headers) =
        call(&app, "GET", "/api/v1/auth/me", Some(admin), Value::Null).await;
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/admin/users/{}", admin_me["id"].as_str().unwrap()),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/admin/users/{id}"),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("{roles_path}/{role_id}"),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // Deletion remains authoritative after reopening the PostgreSQL repository.
    state.board_store.delete_board("test-board").await.unwrap();
    let restarted = BoardStore::new(state.database.clone());
    assert!(restarted.load_all().await.unwrap().is_empty());

    let mut edited = board.clone();
    edited.id = "db-only".into();
    restarted.write_board(&edited).await.unwrap();

    assert_eq!(restarted.load_all().await.unwrap()["db-only"].id, "db-only");
    // Editable settings are authoritative in PostgreSQL and exclude deployment credentials.
    state.save_config().await.unwrap();
    let stored = state
        .database
        .record::<Value>("settings")
        .await
        .unwrap()
        .unwrap()
        .to_string();
    assert!(!stored.contains("test-admin-password"));
    assert!(!stored.contains("postgres://"));
    let mut local = config.clone();
    local.edition = ostool_server::config::Edition::Local;
    let local_state = build_app_state(
        dir.path().join("local.toml"),
        local.clone(),
        build_tftp_manager(&local.tftp),
    )
    .await
    .unwrap();
    let local_app = build_router(local_state);
    assert_eq!(
        call(&local_app, "GET", "/api/v1/auth/config", None, Value::Null)
            .await
            .1["edition"],
        "local"
    );
    for path in [
        "/api/v1/admin/users",
        "/api/v1/admin/roles",
        "/api/v1/admin/rentals",
        "/api/v1/auth/me",
    ] {
        assert_eq!(
            call(&local_app, "GET", path, None, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        call(
            &local_app,
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"email":"admin@test.local","password":"test-admin-password"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        form(
            &local_app,
            "/oauth/device/code",
            &[("client_id", "ostool-cli")]
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&local_app, "GET", "/api/v1/admin/boards", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let mut no_database = config.clone();
    no_database.database.url = None;
    assert!(
        build_app_state(
            dir.path().join("invalid.toml"),
            no_database.clone(),
            build_tftp_manager(&no_database.tftp)
        )
        .await
        .is_err()
    );
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
