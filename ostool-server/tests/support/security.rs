//! Real CAPTCHA challenges with deterministic answers in disposable test databases.
#![allow(dead_code)]
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode, header},
};
use ostool_server::{
    persistence::Database,
    security::{captcha::HumanAction, crypto, store::ClientContext},
};
use serde_json::{Value, json};
use tower::ServiceExt;

pub async fn proof(
    app: &Router,
    database: &Database,
    action: HumanAction,
    client: &ClientContext,
) -> Value {
    let mut request = Request::builder()
        .uri(format!("/api/v1/auth/captcha?action={}", action.name()))
        .header(header::USER_AGENT, &client.user_agent)
        .body(Body::empty())
        .unwrap();
    if let Ok(ip) = client.ip.parse::<std::net::IpAddr>() {
        request
            .extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::new(ip, 0)));
    }
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let challenge: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "CAPTCHA issuance failed: {challenge}"
    );
    assert_eq!(
        challenge["provider"], "image",
        "deterministic test proofs require the local image provider"
    );
    let id = challenge["id"]
        .as_str()
        .unwrap()
        .parse::<uuid::Uuid>()
        .unwrap();
    // Preserve the production purpose, client binding, configuration fingerprint and expiry.
    let updated = sqlx::query("UPDATE security_challenges SET secret_hash=$2 WHERE id=$1")
        .bind(id)
        .bind(crypto::proof_hash(
            &format!("captcha:{}", action.name()),
            id,
            "AB29Q",
        ))
        .execute(database.pool())
        .await
        .unwrap();
    assert_eq!(updated.rows_affected(), 1);
    json!({"captcha_id":id,"captcha_code":"AB29Q"})
}
pub fn access_cookie(headers: &HeaderMap) -> &str {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next()?.split_once('='))
        .find(|(name, _)| name.ends_with("-access"))
        .map(|(_, value)| value)
        .expect("access cookie; browser JSON must not carry tokens")
}
pub fn cookie_header(headers: &HeaderMap) -> String {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn refresh_cookie(headers: &HeaderMap) -> &str {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next()?.split_once('='))
        .find(|(name, _)| name.ends_with("-refresh"))
        .map(|(_, value)| value)
        .expect("refresh cookie")
}
