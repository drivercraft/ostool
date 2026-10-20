//! Run with OSTOOL_TEST_DATABASE_URL pointing to a disposable PostgreSQL DB.
//! Each run creates an isolated schema; no existing tables are modified.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{
    AuthConfig, DatabaseConfig, ServerConfig,
    board_store::fs::FileBoardStore,
    build_app_state, build_router,
    config::{BuiltinTftpConfig, TftpConfig},
    tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
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
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
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
        data_dir: dir.path().join("data"),
        board_dir: dir.path().join("boards"),
        dtb_dir: dir.path().join("dtbs"),
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
    let legacy = FileBoardStore::new(config.board_dir.clone());
    legacy.ensure_dir().await.unwrap();
    let board = ostool_server::BoardConfig {
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
    legacy.write_board(&board).await.unwrap();
    let state = build_app_state(
        dir.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    let app = build_router(state.clone());
    let (status, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"email":"admin@test.local","password":"test-admin-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin = login["access_token"].as_str().unwrap();
    let payload = json!({"email":"member@test.local","password":"test-member-password","display_name":"Member"});
    let (status, registered) =
        call(&app, "POST", "/api/v1/auth/register", None, payload.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    let member = registered["access_token"].as_str().unwrap();
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
    let (_, me) = call(&app, "GET", "/api/v1/auth/me", Some(member), Value::Null).await;
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
    // Member leases are isolated from other users, including serial/file paths.
    let (status, lease) = call(
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
    let (_, other) = call(
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
            other["access_token"].as_str(),
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
    let roles_path = "/api/v1/admin/roles";
    assert_eq!(
        call(
            &app,
            "POST",
            roles_path,
            Some(admin),
            json!({"name":"Test role","description":"test","administrator":false})
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (_, roles) = call(&app, "GET", roles_path, Some(admin), Value::Null).await;
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
    let (_, admin_me) = call(&app, "GET", "/api/v1/auth/me", Some(admin), Value::Null).await;
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
    // Retained TOML is a backup, never the inventory authority after import.
    state.board_store.delete_board("test-board").await.unwrap();
    let restarted = FileBoardStore::new(config.board_dir.clone())
        .with_database(state.database.clone().unwrap())
        .await
        .unwrap();
    assert!(restarted.load_all().await.unwrap().is_empty());
    assert!(legacy.path_for_id("test-board").exists());
    let mut edited = board.clone();
    edited.id = "db-only".into();
    restarted.write_board(&edited).await.unwrap();
    assert!(!legacy.path_for_id("db-only").exists());
    assert_eq!(restarted.load_all().await.unwrap()["db-only"].id, "db-only");
    // A settings save must not copy runtime credentials into the TOML file.
    state.save_config().await.unwrap();
    let stored = tokio::fs::read_to_string(&*state.config_path)
        .await
        .unwrap();
    assert!(!stored.contains("test-admin-password"));
    assert!(!stored.contains("postgres://"));
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
