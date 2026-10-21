use super::{
    config::CaptchaProvider,
    store::{self, ClientContext},
};
use crate::{AppState, api::error::ApiError};
use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanAction {
    Login,
    AdminLogin,
    Register,
    RegistrationEmail,
    PasswordReset,
}
impl HumanAction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::AdminLogin => "admin_login",
            Self::Register => "register",
            Self::RegistrationEmail => "registration_email",
            Self::PasswordReset => "password_reset",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryAction {
    action: HumanAction,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanProof {
    pub captcha_id: Option<Uuid>,
    pub captcha_code: Option<String>,
}
#[derive(Serialize)]
pub struct Challenge {
    pub id: Uuid,
    pub action: HumanAction,
    pub provider: CaptchaProvider,
    pub expires_in: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_key: Option<String>,
}
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/v1/auth/captcha", get(create))
}
async fn create(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Query(query): Query<QueryAction>,
) -> Result<Json<Challenge>, ApiError> {
    let config = state.config.read().await.security.clone();
    store::limit(
        state.database.pool(),
        "captcha",
        &format!("{}:{}", client.ip, query.action.name()),
        20,
        60,
    )
    .await?;
    let (code, image) = if config.captcha_provider == CaptchaProvider::Image {
        tokio::task::spawn_blocking(render_image)
            .await
            .map_err(|_| ApiError::service_unavailable("验证码暂时不可用"))??
    } else {
        (String::new(), None)
    };
    let purpose = format!("captcha:{}", query.action.name());
    let id = store::challenge(
        state.database.pool(),
        &purpose,
        &client.binding(),
        &code,
        json!({}),
        None,
        120,
    )
    .await?;
    Ok(Json(Challenge {
        id,
        action: query.action,
        provider: config.captcha_provider,
        expires_in: 120,
        image_data_url: image,
        site_key: config.turnstile_site_key,
    }))
}
pub async fn verify(
    state: &AppState,
    client: &ClientContext,
    action: HumanAction,
    proof: &HumanProof,
) -> Result<(), ApiError> {
    let invalid = || {
        ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "captcha_invalid",
            "验证码错误或已过期，请刷新后重试",
        )
    };
    let id = proof.captcha_id.ok_or_else(invalid)?;
    let code = proof
        .captcha_code
        .as_deref()
        .filter(|v| !v.is_empty() && v.len() <= 2048)
        .ok_or_else(invalid)?;
    let config = state.config.read().await.security.clone();
    let purpose = format!("captcha:{}", action.name());
    let answer = if config.captcha_provider == CaptchaProvider::Image {
        code.trim().to_ascii_uppercase()
    } else {
        String::new()
    };
    if store::consume(
        state.database.pool(),
        id,
        &purpose,
        &client.binding(),
        &answer,
    )
    .await?
    .is_none()
    {
        return Err(invalid());
    }
    if config.captcha_provider == CaptchaProvider::Turnstile {
        #[derive(Deserialize)]
        struct Result {
            success: bool,
            hostname: Option<String>,
            action: Option<String>,
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(anyhow::Error::from)?;
        let result=http.post("https://challenges.cloudflare.com/turnstile/v0/siteverify").json(&json!({"secret":config.turnstile_secret,"response":code,"remoteip":if client.ip=="unknown" {None}else{Some(&client.ip)}})).send().await;
        let result = match result {
            Ok(response) if response.status().is_success() => response.json::<Result>().await.ok(),
            _ => None,
        }
        .ok_or_else(|| ApiError::service_unavailable("人机验证服务暂时不可用"))?;
        if !result.success
            || result.action.as_deref() != Some(action.name())
            || !result.hostname.is_some_and(|host| {
                config
                    .turnstile_hostnames
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(&host))
            })
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn render_image() -> anyhow::Result<(String, Option<String>)> {
    use captcha::Captcha;
    let mut image = Captcha::new();
    let supported = image.supported_chars();
    let alphabet: Vec<char> = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
        .chars()
        .filter(|c| supported.contains(c))
        .collect();
    image.set_chars(&alphabet).add_chars(5);
    encode_image(image)
}
fn encode_image(mut image: captcha::Captcha) -> anyhow::Result<(String, Option<String>)> {
    use captcha::filters::{Dots, Noise, Wave};
    // view() crops rather than scales. Measure the entire word before adding padding,
    // otherwise wide characters are cut off and the displayed answer cannot be entered.
    let text = image.text_area();
    image
        .apply_filter(Noise::new(0.06))
        .apply_filter(Wave::new(2.0, 22.0).horizontal())
        .view(text.right - text.left + 20, text.bottom - text.top + 16)
        .apply_filter(Dots::new(8).min_radius(1).max_radius(2));
    let code = image.chars_as_string();
    let png = image
        .as_png()
        .ok_or_else(|| anyhow::anyhow!("captcha rendering failed"))?;
    Ok((
        code,
        Some(format!("data:image/png;base64,{}", STANDARD.encode(png))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_characters_are_not_cropped_to_the_widget_width() {
        let mut image = captcha::Captcha::new();
        image.set_chars(&['W']).add_chars(5);
        let (answer, image) = encode_image(image).unwrap();
        assert_eq!(answer, "WWWWW");
        let png = STANDARD
            .decode(
                image
                    .unwrap()
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        // Five of this font's widest glyphs need more than the old 160px crop.
        assert!(width > 250);
        assert!(height > 45);
    }
}
