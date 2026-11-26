//! Run the actual importer against an isolated schema, never a deployment database.
#[path = "support/database.rs"]
mod test_database;
use ostool_server::ServerConfig;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn public_import_creates_configured_admin_and_preserves_changed_passwords()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let mut config = ServerConfig::default();
    let db = test_database::prepare(&root.path().join("seed.env"), &mut config).await?;
    for password in [
        "initial-admin-password-123",
        "different-bootstrap-password-123",
    ] {
        let result = tokio::process::Command::new(env!("CARGO_BIN_EXE_ostool-server-seed-demo"))
            .args(["--env-file", "/dev/null"])
            .env("OSTOOL_EDITION", "public")
            .env("OSTOOL_DATABASE_URL", config.database.url.as_ref().unwrap())
            .env(
                "OSTOOL_DATABASE_SCHEMA",
                config.database.schema.as_ref().unwrap(),
            )
            .env("OSTOOL_PUBLIC_BASE_URL", "https://boards.example.test")
            .env("OSTOOL_ADMIN_USERNAME", "lab_admin")
            .env("OSTOOL_ADMIN_EMAIL", "admin@example.test")
            .env("OSTOOL_ADMIN_PASSWORD", password)
            .env("OSTOOL_DATA_DIR", root.path().join("data"))
            .output()
            .await?;
        anyhow::ensure!(
            result.status.success(),
            "import failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let user = db
            .authenticate_password("admin@example.test", "initial-admin-password-123")
            .await?
            .expect("configured administrator can log in");
        assert!(user.is_admin);
        assert_eq!(user.username, "lab_admin");
        assert!(
            db.authenticate_password("lab_admin", "initial-admin-password-123")
                .await?
                .is_some()
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE email='admin@example.test'")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(count, 1);
    }
    assert!(
        db.authenticate_password("lab_admin", "different-bootstrap-password-123")
            .await?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn simulated_import_upgrades_only_untouched_drafts_and_keeps_real_connections()
-> anyhow::Result<()> {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let root = tempfile::tempdir()?;
    let mut config = ServerConfig::default();
    let db = test_database::prepare(&root.path().join("seed.env"), &mut config).await?;
    let import = |simulate| {
        let mut command =
            tokio::process::Command::new(env!("CARGO_BIN_EXE_ostool-server-seed-demo"));
        command
            .args(["--env-file", "/dev/null"])
            .env("OSTOOL_EDITION", "public")
            .env("OSTOOL_DATABASE_URL", config.database.url.as_ref().unwrap())
            .env(
                "OSTOOL_DATABASE_SCHEMA",
                config.database.schema.as_ref().unwrap(),
            )
            .env("OSTOOL_PUBLIC_BASE_URL", "https://boards.example.test")
            .env("OSTOOL_LOADER_NETWORK_ENABLED", "false")
            .env("OSTOOL_DATA_DIR", root.path().join("data"));
        if simulate {
            command.arg("--simulate-devices");
        }
        command
    };
    let first = import(false).output().await?;
    anyhow::ensure!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    sqlx::query("UPDATE board_records SET payload=jsonb_set(payload,'{notes}',to_jsonb('user modification'::text)) WHERE id='rk3588-a-01'")
        .execute(db.pool()).await?;
    sqlx::query("INSERT INTO device_connections(board_id,endpoint) VALUES('raspberry-pi-4-a-01','http://127.0.0.1:31999')")
        .execute(db.pool()).await?;
    for _ in 0..2 {
        let result = import(true).output().await?;
        anyhow::ensure!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM board_records WHERE NOT disabled")
                .fetch_one(db.pool())
                .await?;
        assert_eq!(count, 42); // 4 maintenance boards + 2 edited drafts remain disabled.
        let notes: String = sqlx::query_scalar(
            "SELECT payload->>'notes' FROM board_records WHERE id='rk3588-a-01'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(notes, "user modification");
        let endpoint: String = sqlx::query_scalar(
            "SELECT endpoint FROM device_connections WHERE board_id='raspberry-pi-4-a-01'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(endpoint, "http://127.0.0.1:31999");
        let endpoint: String = sqlx::query_scalar(
            "SELECT endpoint FROM device_connections WHERE board_id='visionfive2-b-01'",
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(endpoint, "http://127.0.0.1:31002");
        let automatic: bool = sqlx::query_scalar(
            "SELECT 'board.instant'=ANY(permissions) FROM roles WHERE id='kernel-engineer'",
        )
        .fetch_one(db.pool())
        .await?;
        assert!(automatic);
    }
    // Simulated inventory must pass the real public enable checks, not a demo bypass.
    config.edition = ostool_server::config::Edition::Public;
    config.security.mfa_encryption_key =
        Some("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".into());
    config.data_dir = root.path().join("data");
    config.tftp = ostool_server::TftpConfig::Builtin(
        ostool_server::BuiltinTftpConfig::default_with_root(root.path().join("cache")),
    );
    let state = ostool_server::build_app_state(
        root.path().join("seed.env"),
        config.clone(),
        ostool_server::tftp::service::build_tftp_manager(&config.tftp),
    )
    .await?;
    let app = test_database::authenticated_router(&state).await?;
    for disabled in [true, false] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/admin/boards/visionfive2-b-01")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"disabled":disabled}).to_string(),
                    ))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    Ok(())
}
