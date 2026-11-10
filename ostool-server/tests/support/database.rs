//! Disposable PostgreSQL schemas for tests; no alternate storage backend.
#![allow(dead_code)]
use ostool_server::{AppState, ServerConfig, persistence::Database, tftp::service::TftpManager};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub async fn prepare(config_path: &Path, config: &mut ServerConfig) -> anyhow::Result<Database> {
    {
        config.security.redis_url = Some(
            std::env::var("OSTOOL_TEST_REDIS_URL")
                .map_err(|_| anyhow::anyhow!("public tests require OSTOOL_TEST_REDIS_URL"))?,
        );
    }
    if config.auth.public_base_url.is_none() {
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

/// Handler unit tests use a real administrator credential. Authentication-boundary
/// tests deliberately build the unwrapped production router instead.
pub async fn authenticated_router(state: &AppState) -> anyhow::Result<axum::Router> {
    let user = match state.database.find_user("handler-admin@test.local").await? {
        Some(user) => user,
        None => {
            state
                .database
                .create_user(
                    "handler_admin",
                    "handler-admin@test.local",
                    "handler-test-password-123",
                    None,
                    true,
                )
                .await?
        }
    };
    let grant = state
        .database
        .issue_grant(
            user,
            "admin-web",
            "board:operate",
            &state.config.read().await.auth,
        )
        .await?;
    let value = axum::http::HeaderValue::from_str(&format!("Bearer {}", grant.access_token))?;
    Ok(
        ostool_server::build_router(state.clone()).layer(axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: axum::middleware::Next| {
                let value = value.clone();
                async move {
                    if !request
                        .headers()
                        .contains_key(axum::http::header::AUTHORIZATION)
                    {
                        request
                            .headers_mut()
                            .insert(axum::http::header::AUTHORIZATION, value);
                    }
                    next.run(request).await
                }
            },
        )),
    )
}

/// Provision production TLS/credential validation around a local protocol emulator.
/// The plaintext hop exists only inside the disposable fixture on loopback.
pub async fn provision_tls_proxy(
    state: &AppState,
    board_id: &str,
    target: std::net::SocketAddr,
) -> anyhow::Result<tokio::process::Child> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tower::ServiceExt;
    anyhow::ensure!(target.ip().is_loopback(), "test proxy requires loopback");
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/device-tls");
    let token = ostool_server::security::crypto::random_token();
    let mut child = tokio::process::Command::new("node")
        .arg(fixture.join("proxy.mjs"))
        .arg(fixture.join("cert.pem"))
        .arg(fixture.join("key.pem"))
        .arg(&token)
        .arg(format!("http://{target}"))
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await??;
    let port: u16 = line.trim().parse()?;
    if state
        .config
        .read()
        .await
        .security
        .mfa_encryption_key
        .is_none()
    {
        state.config.write().await.security.mfa_encryption_key =
            Some("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".into());
    }
    let board = state
        .boards
        .read()
        .await
        .get(board_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("test board not found"))?;
    state.board_store.write_board(&board).await?;
    let ca = tokio::fs::read_to_string(fixture.join("ca.pem")).await?;
    let app = authenticated_router(state).await?;
    let request=axum::http::Request::builder().method("PUT").uri(format!("/api/v1/admin/boards/{board_id}/device-connection")).header("content-type","application/json").body(axum::body::Body::from(serde_json::json!({"endpoint":format!("https://127.0.0.1:{port}"),"token":token,"ca_pem":ca}).to_string()))?;
    let response = app.oneshot(request).await?;
    anyhow::ensure!(
        response.status() == axum::http::StatusCode::OK,
        "could not provision test device authentication"
    );
    Ok(child)
}
