//! PostgreSQL is the sole runtime authority in both editions.
#[path = "support/database.rs"]
mod test_database;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ostool_server::{
    BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement, PowerManagementConfig,
    PxeProfile, ServerConfig, TftpConfig, build_app_state, build_router, config::Edition,
    tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn production_builders_require_postgres_in_both_editions() {
    for edition in [Edition::Local, Edition::Public] {
        let config = ServerConfig {
            edition,
            ..Default::default()
        };
        let error = build_app_state(
            "unused.toml".into(),
            config.clone(),
            build_tftp_manager(&config.tftp),
        )
        .await
        .err()
        .expect("database is required");
        assert!(
            error
                .to_string()
                .contains("both local and public editions require PostgreSQL")
        );
    }
}
fn board() -> BoardConfig {
    BoardConfig {
        reset_command: None,
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
    }
}
fn image() -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&[0x64, 0x86]);
    bytes[0x58..0x5a].copy_from_slice(&[0x0b, 0x02]);
    bytes[0x9c..0x9e].copy_from_slice(&[10, 0]);
    bytes
}
#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn postgres_survives_restart_and_edition_switch_without_file_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut config = ServerConfig {
        data_dir: dir.path().join("data"),
        tftp: TftpConfig::Builtin(BuiltinTftpConfig {
            enabled: false,
            ..BuiltinTftpConfig::default_with_root(dir.path().join("tftp"))
        }),
        network: ostool_server::TftpNetworkConfig {
            interface: "lo".into(),
        },
        ..Default::default()
    };
    test_database::prepare(&path, &mut config).await.unwrap();
    // Leftover old data is never discovered or imported by the running service.
    let old_boards = config.data_dir.join("boards");
    std::fs::create_dir_all(&old_boards).unwrap();
    std::fs::write(
        old_boards.join("ignored.toml"),
        toml::to_string(&board()).unwrap(),
    )
    .unwrap();
    let state = build_app_state(
        path.clone(),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    assert!(state.boards.read().await.is_empty());
    state.board_store.write_board(&board()).await.unwrap();
    state
        .dtb_store
        .write("board.dtb", b"database payload")
        .await
        .unwrap();
    let ota_image = state
        .ota
        .put_image(&image(), Some("database".into()))
        .await
        .unwrap();
    let job = state
        .ota
        .queue(
            "board-one".into(),
            "02:00:00:00:00:01".parse().unwrap(),
            &ota_image.sha256,
        )
        .await
        .unwrap();
    state
        .config
        .write()
        .await
        .upload_limits
        .session_file_max_mib = 17;
    state.save_config().await.unwrap();
    assert!(!path.exists());
    std::fs::remove_dir_all(&config.data_dir).unwrap();
    config.edition = Edition::Public;
    config.security.redis_url = Some(std::env::var("OSTOOL_TEST_REDIS_URL").unwrap());
    config.auth.public_base_url = Some("http://127.0.0.1:2999".into());
    config.auth.bootstrap_admin_email = Some("storage-admin@test.local".into());
    config.auth.bootstrap_admin_password = Some("storage-password-123".into());
    let public = build_app_state(
        path.clone(),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    assert_eq!(public.boards.read().await.len(), 1);
    assert_eq!(
        public.dtb_store.read("board.dtb").await.unwrap(),
        b"database payload"
    );
    assert_eq!(
        public.ota.image_bytes(&ota_image.sha256).await.unwrap(),
        image()
    );
    assert_eq!(
        public.ota.job("board-one").await.unwrap().update_id,
        job.update_id
    );
    assert_eq!(
        public
            .config
            .read()
            .await
            .upload_limits
            .session_file_max_mib,
        17
    );
    assert!(!config.data_dir.exists());
    public.board_store.delete_board("board-one").await.unwrap();
    public.dtb_store.delete("board.dtb").await.unwrap();
    public
        .ota
        .cancel("board-one", &job.update_id)
        .await
        .unwrap();
    public.ota.delete_image(&ota_image.sha256).await.unwrap();
    config.edition = Edition::Local;
    let local = build_app_state(path, config.clone(), build_tftp_manager(&config.tftp))
        .await
        .unwrap();
    assert!(local.boards.read().await.is_empty());
    assert!(local.dtb_store.get("board.dtb").await.unwrap().is_none());
    assert!(local.ota.images().await.unwrap().is_empty());
    let response = build_router(local.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/board-types")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let metadata: Value = local.database.record("settings").await.unwrap().unwrap();
    for key in ["auth", "database", "edition"] {
        assert!(metadata.get(key).is_none());
    }
    local.database.pool().close().await;
    assert!(local.dtb_store.read("board.dtb").await.is_err());
    assert!(local.board_store.load_all().await.is_err());
    assert_eq!(
        json!(public.ota.job("board-one").await.unwrap().phase),
        "cancelled"
    );
}
