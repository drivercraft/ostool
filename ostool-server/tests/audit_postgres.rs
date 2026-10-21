//! Durable audit identity, secrets exclusion, pagination/export, retention and fail-closed writes.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use ostool_server::{
    AuthConfig, BuiltinTftpConfig, DatabaseConfig, ServerConfig, TftpConfig, build_app_state,
    build_router, config::Edition, tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("user-agent", "audit integration test")
        .header("x-forwarded-for", "198.51.100.7,10.0.0.1")
        .header("x-secret", "NEVER_COPY_HEADER");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let mut request = request.body(Body::from(body.to_string())).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:43210".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn board(id: &str) -> Value {
    json!({"id":id,"board_type":"fixture","serial":null,"network_identity":null,"notes":"NEVER_COPY_PAYLOAD","tags":[],"disabled":false,"boot":{"kind":"pxe","notes":null},"power_management":{"kind":"custom","power_on_cmd":"true # NEVER_COPY_COMMAND","power_off_cmd":"true # NEVER_COPY_COMMAND"}})
}
#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn complete_audit_contract_with_real_postgres() {
    let database_url = std::env::var("OSTOOL_TEST_DATABASE_URL").unwrap();
    let root = sqlx::PgPool::connect(&database_url).await.unwrap();
    let schema = format!("ostool_audit_{}", Uuid::new_v4().simple());
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
            bootstrap_admin_email: Some("admin@audit.test".into()),
            bootstrap_admin_password: Some("audit-admin-password-NEVER_COPY_PASSWORD".into()),
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
    config.audit.trusted_proxies = vec!["127.0.0.1".parse().unwrap(), "10.0.0.1".parse().unwrap()];
    let state = build_app_state(
        dir.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    let app = build_router(state.clone());
    let pool = state.database.pool();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"email":"admin@audit.test","password":"NEVER_COPY_WRONG_PASSWORD"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, grant) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"email":"admin@audit.test","password":"audit-admin-password-NEVER_COPY_PASSWORD"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin = grant["access_token"].as_str().unwrap();
    let (_, member) = call(
        &app,
        "POST",
        "/api/v1/auth/register",
        None,
        json!({"email":"member@audit.test","password":"member-password-NEVER_COPY_PASSWORD"}),
    )
    .await;
    let member = member["access_token"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/auth/login",
            None,
            json!({"email":"member@audit.test","password":"member-password-NEVER_COPY_PASSWORD"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, backend_grant) = call(
        &app,
        "POST",
        "/api/v1/admin/auth/login",
        None,
        json!({"email":"admin@audit.test","password":"audit-admin-password-NEVER_COPY_PASSWORD"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(backend_grant["access_token"], grant["access_token"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM oauth_sessions WHERE client_id='admin-web'"
        )
        .fetch_one(pool)
        .await
        .unwrap(),
        1
    );
    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/oauth/revoke")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("token", backend_grant["refresh_token"].as_str().unwrap())
                        .finish(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", "/api/v1/auth/me", Some(admin), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert!(
        state
            .database
            .authenticate_token(backend_grant["access_token"].as_str().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/admin/audits",
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
            "POST",
            "/api/v1/admin/boards",
            Some(member),
            board("forbidden-board")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(!state.boards.read().await.contains_key("forbidden-board"));
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/boards",
            Some(admin),
            board("audit-board")
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let (status, logs) = call(
        &app,
        "GET",
        "/api/v1/admin/audits?action=board.create&page_size=1",
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(logs["items"].as_array().unwrap().len(), 1);
    let item = &logs["items"][0];
    assert_eq!(item["target_id"], "audit-board");
    assert_eq!(item["ip"], "198.51.100.7");
    assert_eq!(item["actor_kind"], "admin");
    assert_eq!(item["result"], "success");
    let first_id = item["id"].as_str().unwrap();
    let (_, detail) = call(
        &app,
        "GET",
        &format!("/api/v1/admin/audits/{first_id}"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(detail["id"], first_id);
    assert!(
        detail["details"]["changed_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "power_management")
    );
    let (_, lease) = call(
        &app,
        "POST",
        "/api/v1/sessions",
        Some(admin),
        json!({"board_type":"fixture"}),
    )
    .await;
    let session_id = lease["session_id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/sessions/{session_id}"),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_events WHERE action='session.released' AND target_id=$1 AND result='success')").bind(session_id).fetch_one(pool).await.unwrap();if exists{break;}tokio::time::sleep(std::time::Duration::from_millis(20)).await;}}).await.unwrap();
    let created_target:String=sqlx::query_scalar("SELECT target_id FROM audit_events WHERE action='session.create' AND result='success' LIMIT 1").fetch_one(pool).await.unwrap();
    assert_eq!(created_target, session_id);
    // Generated identities are correlated without parsing token-bearing response bodies.
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/v1/account/profile",
            Some(admin),
            json!({"email":"admin@audit.test","display_name":"=SUM(1,2)"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/boards",
            Some(admin),
            board("audit-board-two")
        )
        .await
        .0,
        StatusCode::CREATED
    );
    let export = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/audits/export?action=board.create")
                .header("authorization", format!("Bearer {admin}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::OK);
    assert_eq!(export.headers()["content-type"], "text/csv; charset=utf-8");
    let csv = String::from_utf8(
        to_bytes(export.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(csv.starts_with('\u{feff}'));
    assert!(csv.contains("\"'=SUM(1,2)"));
    assert!(csv.contains("audit-board-two"));
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/admin/audits?from=2026-10-22T00:00:00Z&to=2026-10-21T00:00:00Z",
            Some(admin),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // Each subscribed view is independently paginated; no full-history browser download.
    let response=app.clone().oneshot(Request::builder().uri(format!("/api/v1/admin/events?access_token={admin}&audit_view=1&audit_action=board.create&audit_page_size=1")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame = String::from_utf8(frame.to_vec()).unwrap();
    let envelope: Value = serde_json::from_str(
        frame
            .lines()
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        envelope["data"]["audit"]["items"].as_array().unwrap().len(),
        1
    );
    assert_eq!(envelope["data"]["audit"]["query"]["action"], "board.create");
    drop(stream);
    // Durable background OTA transitions are committed with their state changes.
    let mut image = vec![0; 512];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    image[0x40..0x44].copy_from_slice(b"PE\0\0");
    image[0x44..0x46].copy_from_slice(&[0x64, 0x86]);
    image[0x58..0x5a].copy_from_slice(&[0x0b, 0x02]);
    image[0x9c..0x9e].copy_from_slice(&[10, 0]);
    let image = state.ota.put_image(&image, None).await.unwrap();
    let mac = "02:00:00:00:01:23".parse().unwrap();
    let job = state
        .ota
        .queue("audit-board".into(), mac, &image.sha256)
        .await
        .unwrap();
    state
        .ota
        .record_delivery_failure(
            "audit-board",
            mac,
            &job.update_id,
            "NEVER_COPY_OTA_ERROR".into(),
        )
        .await
        .unwrap();
    state
        .ota
        .cancel("audit-board", &job.update_id)
        .await
        .unwrap();
    let transitions: Vec<Value> = sqlx::query_scalar(
        "SELECT details FROM audit_events WHERE resource='ota' ORDER BY created_at,id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(transitions.len(), 3);
    assert_eq!(transitions.last().unwrap()["phase"], "cancelled");
    assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM audit_events WHERE action='ota.delivery_failed' AND result='failed')").fetch_one(pool).await.unwrap());
    let records: Value = serde_json::to_value(
        sqlx::query_as::<_, ostool_server::audit::Event>("SELECT * FROM audit_events")
            .fetch_all(pool)
            .await
            .unwrap(),
    )
    .unwrap();
    let serialized = records.to_string();
    for secret in [
        "NEVER_COPY_PASSWORD",
        "NEVER_COPY_WRONG_PASSWORD",
        "NEVER_COPY_PAYLOAD",
        "NEVER_COPY_COMMAND",
        "NEVER_COPY_HEADER",
        "NEVER_COPY_OTA_ERROR",
        admin,
        grant["refresh_token"].as_str().unwrap(),
        backend_grant["access_token"].as_str().unwrap(),
        backend_grant["refresh_token"].as_str().unwrap(),
    ] {
        assert!(
            !serialized.contains(secret),
            "secret copied into audit storage"
        );
    }
    assert!(
        records
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["action"] == "auth.login" && item["result"] == "denied")
    );
    // Audit writes must be available before business mutations are executed.
    sqlx::query("ALTER TABLE audit_events RENAME TO audit_events_unavailable")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/admin/boards",
            Some(admin),
            board("must-not-exist")
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!state.boards.read().await.contains_key("must-not-exist"));
    sqlx::query("ALTER TABLE audit_events_unavailable RENAME TO audit_events")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_events SET created_at=now()-interval '500 days' WHERE id=$1")
        .bind(first_id.parse::<Uuid>().unwrap())
        .execute(pool)
        .await
        .unwrap();
    let pending = Uuid::new_v4();
    sqlx::query("INSERT INTO audit_events(id,request_id,created_at,actor_name,actor_kind,source,action,summary,resource,result) VALUES ($1,$1,now()-interval '500 days','system','system','system','pending.fixture','Pending fixture','system','pending')").bind(pending).execute(pool).await.unwrap();
    let (status, cleaned) = call(
        &app,
        "POST",
        "/api/v1/admin/audits/cleanup",
        Some(admin),
        json!({"keep_days":90}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleaned["deleted"], 1);
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM audit_events WHERE id=$1)")
            .bind(pending)
            .fetch_one(pool)
            .await
            .unwrap()
    );
    let cleanup_detail:Value=sqlx::query_scalar("SELECT details FROM audit_events WHERE action='audit.cleanup' AND result='success' ORDER BY created_at DESC LIMIT 1").fetch_one(pool).await.unwrap();
    assert_eq!(cleanup_detail["deleted"], 1);
    assert_eq!(
        ostool_server::audit::run_retention(&state, 90)
            .await
            .unwrap(),
        0
    );
    assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM audit_events WHERE action='audit.retention' AND result='success' AND details->>'deleted'='0')").fetch_one(pool).await.unwrap());
    config.edition = Edition::Local;
    let local = build_app_state(
        dir.path().join("local.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    let local_app = build_router(local.clone());
    assert_eq!(call(&local_app,"POST","/api/v1/admin/auth/login",None,json!({"email":"admin@audit.test","password":"audit-admin-password-NEVER_COPY_PASSWORD"})).await.0,StatusCode::NOT_FOUND);
    assert_eq!(
        call(&local_app, "GET", "/api/v1/admin/audits", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM audit_events WHERE actor_kind='local' AND action='audit.view')").fetch_one(pool).await.unwrap());
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
