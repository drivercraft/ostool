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
            .env("OSTOOL_NETWORK_INTERFACE", "lo")
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
