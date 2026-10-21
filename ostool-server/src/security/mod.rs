//! Public-edition authentication protections. Local edition remains anonymous.
pub mod cache;
pub mod captcha;
pub mod config;
pub mod crypto;
pub mod mail;
pub mod mfa;
pub mod sessions;
pub mod store;
pub mod web;

pub async fn projection(
    state: &crate::AppState,
    user: uuid::Uuid,
    session: Option<uuid::Uuid>,
) -> Result<serde_json::Value, crate::api::error::ApiError> {
    Ok(
        serde_json::json!({"security":mfa::status_projection(state,user).await?,"security_sessions":sessions::projection(state,user,session).await?}),
    )
}
