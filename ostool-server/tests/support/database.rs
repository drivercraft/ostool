//! Disposable PostgreSQL schemas for tests; no alternate storage backend.
#![allow(dead_code)]
use ostool_server::{AppState, ServerConfig, persistence::Database, tftp::service::TftpManager};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub async fn prepare(config_path: &Path, config: &mut ServerConfig) -> anyhow::Result<Database> {
    if config.edition.is_public() {
        config.security.redis_url = Some(
            std::env::var("OSTOOL_TEST_REDIS_URL")
                .map_err(|_| anyhow::anyhow!("public tests require OSTOOL_TEST_REDIS_URL"))?,
        );
    }
    if config.edition.is_public() && config.auth.public_base_url.is_none() {
        config.auth.public_base_url = Some("http://127.0.0.1:2999".into());
    }
    if config.network.interface.is_empty() {
        config.network.interface = "lo".into();
    }
    if config.database.url.is_none() {
        config.database.url = Some(std::env::var("OSTOOL_TEST_DATABASE_URL").map_err(|_| {
            anyhow::anyhow!("tests require disposable PostgreSQL: set OSTOOL_TEST_DATABASE_URL")
        })?);
        let identity = format!("{}:{}", config_path.display(), config.data_dir.display());
        let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
        config.database.schema = Some(format!("ostool_test_{}", &digest[..32]));
        config.database.max_connections = 4;
    }
    Database::connect(&config.database).await
}
pub async fn database(path: &Path) -> anyhow::Result<Database> {
    let mut config = ServerConfig {
        data_dir: path.to_path_buf(),
        ..Default::default()
    };
    prepare(path, &mut config).await
}
pub async fn build_test_app_state(
    path: PathBuf,
    mut config: ServerConfig,
    manager: Arc<dyn TftpManager>,
) -> anyhow::Result<AppState> {
    prepare(&path, &mut config).await?;
    ostool_server::build_app_state(path, config, manager).await
}
