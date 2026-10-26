//! Member isolation, atomic application approval and account security against real PostgreSQL.
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
use futures_util::StreamExt;
use ostool_server::{
    AuthConfig, BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement, DatabaseConfig,
    PowerManagementConfig, PxeProfile, ServerConfig, TftpConfig, build_app_state, build_router,
    config::Edition,
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
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
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
async fn account(app: &Router, token: &str) -> Value {
    let (status, mut value, _value_headers) =
        call(app, "GET", "/api/v1/account", Some(token), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    for (field, path) in [
        ("my_rentals", "/api/v1/account/rentals"),
        ("my_requests", "/api/v1/account/rental-requests"),
    ] {
        let (status, page, _) = call(app, "GET", path, Some(token), Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        value[field] = page["items"].clone();
    }
    value
}

#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn member_workspace_isolation_approvals_and_security() {
    let database_url = std::env::var("OSTOOL_TEST_DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&database_url).await.unwrap();
    let schema = format!("ostool_account_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await
        .unwrap();
    let mut url = url::Url::parse(&database_url).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema},public"));
    let dir = tempfile::tempdir().unwrap();
    let mut config = ServerConfig {
        edition: Edition::Public,
        data_dir: dir.path().join("data"),

        database: DatabaseConfig {
            url: Some(url.to_string()),
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: true,
            public_base_url: Some("http://127.0.0.1:2999".into()),
            bootstrap_admin_email: Some("admin@test.local".into()),
            bootstrap_admin_password: Some("test-admin-password".into()),
            ..Default::default()
        },
        tftp: TftpConfig::Builtin(BuiltinTftpConfig {
            enabled: false,
            ..BuiltinTftpConfig::default_with_root(dir.path().join("tftp"))
        }),
        network: ostool_server::TftpNetworkConfig {
            interface: "lo".into(),
        },
        ..Default::default()
    };
    let store = BoardStore::new(Database::connect(&config.database).await.unwrap());
    store
        .write_board(&BoardConfig {
            allow_instant_use: false,
            reset_command: None,
            id: "portal-board".into(),
            board_type: "test".into(),
            tags: vec!["test".into()],
            serial: None,
            boot: BootConfig::Pxe(PxeProfile::default()),
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            network_identity: None,
            notes: Some("private inventory note".into()),
            disabled: false,
        })
        .await
        .unwrap();
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
    let (_, _grant, _grant_headers) = call(
        &app,
        "POST",
        "/api/v1/admin/auth/login",
        None,
        json!({"email":"admin@test.local","password":"test-admin-password"}),
    )
    .await;
    let admin = access_cookie(&_grant_headers);
    let (_, _first, _first_headers) = call(
        &app,
        "POST",
        "/api/v1/auth/register",
        None,
        json!({"email":"first@test.local","password":"first-password-123"}),
    )
    .await;
    let first = access_cookie(&_first_headers);
    let (_, _second, _second_headers) = call(
        &app,
        "POST",
        "/api/v1/auth/register",
        None,
        json!({"email":"second@test.local","password":"second-password-123"}),
    )
    .await;
    let second = access_cookie(&_second_headers);
    assert_eq!(
        call(&app, "GET", "/api/v1/account", None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/admin/rental-requests",
            Some(first),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let request = json!({"board_id":"portal-board","starts_at":start,"ends_at":start+chrono::Duration::hours(2),"notes":"first application"});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/account/rental-requests",
            Some(first),
            request.clone()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/account/rental-requests",
            Some(first),
            request.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/account/rental-requests",
            Some(second),
            request.clone()
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let first_snapshot = account(&app, first).await;
    assert_eq!(first_snapshot["my_requests"].as_array().unwrap().len(), 1);
    assert!(
        first_snapshot["catalog"][0]
            .get("power_management")
            .is_none()
    );
    assert!(first_snapshot["catalog"][0].get("notes").is_none());
    assert_eq!(first_snapshot["profile"]["email"], "first@test.local");
    let first_id = first_snapshot["my_requests"][0]["id"].as_str().unwrap();
    let second_snapshot = account(&app, second).await;
    let second_id = second_snapshot["my_requests"][0]["id"].as_str().unwrap();
    assert_ne!(first_id, second_id);
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/account/rental-requests/{first_id}"),
            Some(second),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/admin/rental-requests/{first_id}/review"),
            Some(first),
            json!({"decision":"approve"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let path = format!("/api/v1/admin/rental-requests/{first_id}/review");
    let (one, two) = tokio::join!(
        call(
            &app,
            "POST",
            &path,
            Some(admin),
            json!({"decision":"approve","note":"approved by test"})
        ),
        call(
            &app,
            "POST",
            &path,
            Some(admin),
            json!({"decision":"approve"})
        )
    );
    assert!([one.0, two.0].contains(&StatusCode::NO_CONTENT));
    assert!([one.0, two.0].contains(&StatusCode::CONFLICT));
    let first_snapshot = account(&app, first).await;
    assert_eq!(first_snapshot["my_rentals"].as_array().unwrap().len(), 1);
    assert_eq!(first_snapshot["my_requests"][0]["status"], "approved");
    assert_eq!(
        first_snapshot["my_requests"][0]["rental_id"],
        first_snapshot["my_rentals"][0]["id"]
    );
    assert!(
        account(&app, second).await["my_rentals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/admin/rental-requests/{second_id}/review"),
            Some(admin),
            json!({"decision":"approve"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        account(&app, second).await["my_requests"][0]["status"],
        "pending"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/admin/rental-requests/{second_id}/review"),
            Some(admin),
            json!({"decision":"reject","note":"time conflict"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        account(&app, second).await["my_requests"][0]["decision_note"],
        "time conflict"
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/account/rental-requests/{first_id}"),
            Some(first),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let later = json!({"board_id":"portal-board","starts_at":start+chrono::Duration::days(1),"ends_at":start+chrono::Duration::days(2)});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/account/rental-requests",
            Some(second),
            later
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let second_snapshot = account(&app, second).await;
    let withdrawn = second_snapshot["my_requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] == "pending")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/account/rental-requests/{withdrawn}"),
            Some(second),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/admin/rental-requests/{withdrawn}/review"),
            Some(admin),
            json!({"decision":"approve"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // Personal SSE has the same cursor semantics, with no management/private cross-user data.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/account/events?list_view=rentals&list_page=1&list_page_size=20&editor_view=1&editor_board=portal-board&editor_from={}&editor_to={}",(start-chrono::Duration::days(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs,true),(start+chrono::Duration::days(3)).to_rfc3339_opts(chrono::SecondsFormat::Secs,true)))
                .header("authorization", format!("Bearer {second}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let chunk = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(chunk.to_vec()).unwrap();
    let envelope: Value = serde_json::from_str(
        text.lines()
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
            })
            .unwrap(),
    )
    .unwrap();
    let data = &envelope["data"];
    assert_eq!(data["profile"]["email"], "second@test.local");
    assert!(data.get("users").is_none());
    assert!(data.get("boards").is_none());
    assert!(data.get("rentals").is_none());
    assert!(data["listing"]["items"].as_array().unwrap().is_empty());
    assert_eq!(data["availability"].as_array().unwrap().len(), 1);
    for window in data["availability"].as_array().unwrap() {
        assert!(window.get("user_id").is_none());
        assert!(window.get("notes").is_none());
        assert!(window.get("user_email").is_none());
    }
    assert!(!text.contains("first@test.local"));
    assert!(!text.contains("power_on_cmd"));
    drop(stream);
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/v1/account/profile",
            Some(first),
            json!({"email":"first@test.local","display_name":"Updated member","is_admin":true})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/v1/account/profile",
            Some(first),
            json!({"email":"first@test.local","display_name":"Updated member"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        account(&app, first).await["profile"]["display_name"],
        "Updated member"
    );
    assert_eq!(call(&app,"PUT","/api/v1/account/profile",Some(first),json!({"email":"changed@test.local","current_password":"wrong","display_name":"Updated member"})).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/v1/account/profile",
            Some(first),
            json!({"email":"second@test.local","current_password":"first-password-123"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(call(&app,"PUT","/api/v1/account/profile",Some(first),json!({"email":"changed@test.local","current_password":"first-password-123","display_name":"Updated member"})).await.0,StatusCode::NO_CONTENT);
    // Changing the login email revokes the previous credentials.
    assert_eq!(
        call(&app, "GET", "/api/v1/account", Some(first), Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _, login_headers) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"email":"changed@test.local","password":"first-password-123"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first = access_cookie(&login_headers);
    let (pat_status, pat, _pat_headers) = call(
        &app,
        "POST",
        "/api/v1/auth/personal-access-tokens",
        Some(first),
        json!({"name":"CLI test","expires_in_days":1}),
    )
    .await;
    assert_eq!(pat_status, StatusCode::CREATED, "{pat}");
    let pat = pat["token"].as_str().unwrap();
    assert_eq!(
        account(&app, first).await["personal_tokens"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        account(&app, first).await["personal_tokens"][0]
            .get("token")
            .is_none()
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/account/password",
            Some(first),
            json!({"current_password":"wrong","new_password":"updated-password-123"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/account/password",
            Some(first),
            json!({"current_password":"first-password-123","new_password":"updated-password-123"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for token in [first, pat] {
        assert_eq!(
            call(&app, "GET", "/api/v1/auth/me", Some(token), Value::Null)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"email":"changed@test.local","password":"first-password-123"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"email":"changed@test.local","password":"updated-password-123"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut local = config.clone();
    local.edition = Edition::Local;
    let local = build_router(
        build_app_state(
            dir.path().join("local.toml"),
            local.clone(),
            build_tftp_manager(&local.tftp),
        )
        .await
        .unwrap(),
    );
    for path in [
        "/api/v1/account",
        "/api/v1/account/events",
        "/api/v1/admin/rental-requests",
    ] {
        assert_eq!(
            call(&local, "GET", path, None, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
