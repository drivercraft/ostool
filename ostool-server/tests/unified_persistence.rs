//! Both editions share PostgreSQL authority; legacy files are imported exactly once.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{
    BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement, PowerManagementConfig,
    PxeProfile, ServerConfig, TftpConfig, build_app_state, build_router,
    config::{DatabaseConfig, Edition},
    ota::OtaStore,
    persistence::legacy::boards::BoardFiles,
    persistence::legacy::dtbs::DtbFiles,
    tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;
async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
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
fn image(marker: u8) -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&[0x64, 0x86]);
    bytes[0x58..0x5a].copy_from_slice(&[0x0b, 0x02]);
    bytes[0x9c..0x9e].copy_from_slice(&[10, 0]);
    bytes[511] = marker;
    bytes
}

#[tokio::test]
async fn production_builders_require_postgres_in_both_editions() {
    for edition in [Edition::Local, Edition::Public] {
        let config = ServerConfig {
            edition,
            tftp: TftpConfig::Builtin(BuiltinTftpConfig {
                enabled: false,
                ..BuiltinTftpConfig::default_with_root("unused-test-path".into())
            }),
            ..Default::default()
        };
        let error = build_app_state(
            "unused-config.toml".into(),
            config.clone(),
            build_tftp_manager(&config.tftp),
        )
        .await
        .err()
        .expect("database must be required");
        assert!(
            error
                .to_string()
                .contains("both local and public editions require PostgreSQL")
        );
    }
}

#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn durable_data_survives_file_loss_and_edition_switch_without_local_login() {
    let root = sqlx::PgPool::connect(&std::env::var("OSTOOL_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let schema = format!("ostool_storage_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&root)
        .await
        .unwrap();
    let mut url = url::Url::parse(&std::env::var("OSTOOL_TEST_DATABASE_URL").unwrap()).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema},public"));
    let dir = tempfile::tempdir().unwrap();
    let mut config = ServerConfig {
        edition: Edition::Local,
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        database: DatabaseConfig {
            url: Some(url.to_string()),
            ..Default::default()
        },
        data_dir: dir.path().join("data"),
        board_dir: dir.path().join("boards"),
        dtb_dir: dir.path().join("dtbs"),
        tftp: TftpConfig::Builtin(BuiltinTftpConfig {
            enabled: false,
            ..BuiltinTftpConfig::default_with_root(dir.path().join("tftp"))
        }),
        network: ostool_server::TftpNetworkConfig {
            interface: "lo".into(),
        },
        ..Default::default()
    };
    let legacy = BoardFiles::new(config.board_dir.clone());
    let board = BoardConfig {
        id: "board-one".into(),
        board_type: "fixture".into(),
        tags: vec![],
        serial: None,
        notes: None,
        disabled: false,
        network_identity: None,
        boot: BootConfig::Pxe(PxeProfile::default()),
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "true".into(),
            power_off_cmd: "true".into(),
        }),
    };
    legacy.write_board(&board).await.unwrap();
    let dtbs = DtbFiles::new(config.dtb_dir.clone());
    dtbs.write("legacy.dtb", b"legacy payload").await.unwrap();
    let old_ota = OtaStore::open(&config.data_dir).unwrap();
    let old_image = old_ota
        .put_image(&image(1), Some("legacy".into()))
        .await
        .unwrap();
    let job = old_ota
        .queue(
            board.id.clone(),
            "02:00:00:00:00:01".parse().unwrap(),
            &old_image.sha256,
        )
        .await
        .unwrap();
    let state = build_app_state(
        dir.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    let app = build_router(state.clone());
    assert_eq!(
        call(&app, "GET", "/api/v1/admin/boards", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    for path in [
        "/api/v1/auth/me",
        "/api/v1/admin/users",
        "/api/v1/admin/rentals",
        "/api/v1/account",
    ] {
        assert_eq!(
            call(&app, "GET", path, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, session) = call(
        &app,
        "POST",
        "/api/v1/sessions",
        json!({"board_type":"fixture"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    state
        .remove_session(session["session_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        state.dtb_store.read("legacy.dtb").await.unwrap(),
        b"legacy payload"
    );
    state
        .dtb_store
        .write("new.dtb", b"database payload")
        .await
        .unwrap();
    state
        .dtb_store
        .rename("new.dtb", "renamed.dtb")
        .await
        .unwrap();
    state.dtb_store.delete("legacy.dtb").await.unwrap();
    let new_image = state
        .ota
        .put_image(&image(2), Some("database".into()))
        .await
        .unwrap();
    state
        .config
        .write()
        .await
        .upload_limits
        .session_file_max_mib = 17;
    state.save_config().await.unwrap();
    assert!(!dir.path().join("config.toml").exists());
    // Stale retained sources must neither override nor resurrect PostgreSQL data.
    tokio::fs::write(
        legacy.path_for_id(&board.id),
        "this backup is no longer authoritative",
    )
    .await
    .unwrap();
    tokio::fs::remove_dir_all(&config.dtb_dir).await.unwrap();
    tokio::fs::create_dir_all(&config.dtb_dir).await.unwrap();
    tokio::fs::write(config.dtb_dir.join("legacy.dtb"), b"stale deleted payload")
        .await
        .unwrap();
    tokio::fs::remove_dir_all(config.data_dir.join("loader-ota"))
        .await
        .unwrap();
    config.edition = Edition::Public;
    config.auth.bootstrap_admin_email = Some("admin@storage.test".into());
    config.auth.bootstrap_admin_password = Some("storage-admin-password".into());
    config.upload_limits.session_file_max_mib = 99;
    let public = build_app_state(
        dir.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    assert_eq!(public.boards.read().await.len(), 1);
    assert_eq!(
        public
            .config
            .read()
            .await
            .upload_limits
            .session_file_max_mib,
        17
    );
    assert_eq!(
        public.dtb_store.read("renamed.dtb").await.unwrap(),
        b"database payload"
    );
    assert!(public.dtb_store.get("legacy.dtb").await.unwrap().is_none());
    assert_eq!(
        public.ota.image_bytes(&old_image.sha256).await.unwrap(),
        image(1)
    );
    assert_eq!(
        public.ota.image_bytes(&new_image.sha256).await.unwrap(),
        image(2)
    );
    assert_eq!(
        public.ota.job(&board.id).await.unwrap().update_id,
        job.update_id
    );
    assert!(!config.data_dir.join("loader-ota").exists());
    let app = build_router(public.clone());
    assert_eq!(
        call(&app, "GET", "/api/v1/admin/boards", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/login",
            json!({"email":"admin@storage.test","password":"storage-admin-password"})
        )
        .await
        .0,
        StatusCode::OK
    );
    public.ota.cancel(&board.id, &job.update_id).await.unwrap();
    public.ota.delete_image(&new_image.sha256).await.unwrap();
    config.edition = Edition::Local;
    let local = build_app_state(
        dir.path().join("config.toml"),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    assert_eq!(
        local.ota.job(&board.id).await.unwrap().phase,
        ostool_server::ota::Phase::Cancelled
    );
    assert!(
        local
            .ota
            .images()
            .await
            .unwrap()
            .iter()
            .all(|image| image.sha256 != new_image.sha256)
    );
    assert_eq!(
        call(
            &build_router(local.clone()),
            "GET",
            "/api/v1/board-types",
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let metadata: Value = local
        .database
        .as_ref()
        .unwrap()
        .record("settings")
        .await
        .unwrap()
        .unwrap();
    assert!(metadata.get("auth").is_none());
    assert!(metadata.get("database").is_none());
    assert!(metadata.get("edition").is_none());
    local.database.as_ref().unwrap().pool().close().await;
    assert!(local.dtb_store.read("renamed.dtb").await.is_err());
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&root)
        .await
        .unwrap();
    root.close().await;
}
