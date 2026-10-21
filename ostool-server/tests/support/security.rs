//! Deterministic proofs are inserted only into the test's disposable database.
#![allow(dead_code)]
use axum::http::{HeaderMap, header};
use ostool_server::{
    persistence::Database,
    security::{
        captcha::HumanAction,
        store::{ClientContext, challenge},
    },
};
use serde_json::{Value, json};
pub async fn proof(database: &Database, action: HumanAction, client: &ClientContext) -> Value {
    let id = challenge(
        database.pool(),
        &format!("captcha:{}", action.name()),
        &client.binding(),
        "AB29Q",
        json!({}),
        None,
        120,
    )
    .await
    .unwrap();
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
