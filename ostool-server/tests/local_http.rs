//! Local edition retains anonymous HTTP without accounts, rentals or Redis.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{tftp::service::build_tftp_manager, *};
use serde_json::{Value, json};
use tower::ServiceExt;

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
    assert_eq!(call("POST","/api/v1/admin/boards",json!({"id":"local","board_type":"local-test","tags":[],"disabled":false,"boot":{"kind":"uboot","use_tftp":false},"power_management":{"kind":"custom","power_on_cmd":"true","power_off_cmd":"true"}})).await?.status(),StatusCode::CREATED);
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
