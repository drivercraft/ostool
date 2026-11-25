use super::{
    config::{CaptchaProvider, SecurityConfig},
    crypto,
    store::{self, ClientContext},
};
use crate::{AppState, api::error::ApiError};
use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanAction {
    Login,
    AdminLogin,
    Register,
    RegistrationEmail,
    PasswordReset,
    ConfigurationTest,
}
impl HumanAction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::AdminLogin => "admin_login",
            Self::Register => "register",
            Self::RegistrationEmail => "registration_email",
            Self::PasswordReset => "password_reset",
            Self::ConfigurationTest => "configuration_test",
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cdata: Option<String>,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/captcha", get(create))
        .route(
            "/api/v1/admin/account-settings/captcha",
            get(create_configuration),
        )
        .route(
            "/api/v1/admin/account-settings/captcha/verify",
            post(verify_configuration),
        )
}
fn invalid() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_REQUEST,
        "captcha_invalid",
        "验证码错误或已过期，请刷新后重试",
    )
}
fn unavailable() -> ApiError {
    ApiError::service_unavailable("人机验证服务暂时不可用，请重试")
}
fn binding(config: &SecurityConfig) -> Value {
    // Rotating either key, provider or allowlist invalidates outstanding challenges without persisting secrets.
    json!({"configuration": crypto::hash(&serde_json::to_string(&(
        config.captcha_provider, &config.turnstile_site_key, &config.turnstile_secret, &config.turnstile_hostnames
    )).expect("serializable CAPTCHA configuration"))})
}
async fn create(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Query(query): Query<QueryAction>,
) -> Result<Json<Challenge>, ApiError> {
    if query.action == HumanAction::ConfigurationTest {
        return Err(ApiError::forbidden("请在后台测试人机验证配置"));
    }
    issue(&state, &client, query.action).await
}
async fn create_configuration(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
) -> Result<Json<Challenge>, ApiError> {
    store::limit(
        &state.security_cache,
        "captcha-configuration",
        &client.ip,
        5,
        60,
    )
    .await?;
    issue(&state, &client, HumanAction::ConfigurationTest).await
}
async fn verify_configuration(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Json(proof): Json<HumanProof>,
) -> Result<Json<Value>, ApiError> {
    store::limit(
        &state.security_cache,
        "captcha-configuration-verify",
        &client.ip,
        5,
        60,
    )
    .await?;
    verify(&state, &client, HumanAction::ConfigurationTest, &proof).await?;
    Ok(Json(json!({"verified":true})))
}
async fn issue(
    state: &AppState,
    client: &ClientContext,
    action: HumanAction,
) -> Result<Json<Challenge>, ApiError> {
    let config = state.config.read().await.security.clone();
    store::limit(&state.security_cache, "captcha-total", &client.ip, 60, 60).await?;
    store::limit(
        &state.security_cache,
        "captcha",
        &format!("{}:{}", client.ip, action.name()),
        20,
        60,
    )
    .await?;
    let (code, image) = if config.captcha_provider == CaptchaProvider::Image {
        tokio::task::spawn_blocking(render_image)
            .await
            .map_err(|_| unavailable())??
    } else {
        (String::new(), None)
    };
    let ttl: u32 = if config.captcha_provider == CaptchaProvider::Turnstile {
        300
    } else {
        120
    };
    let id = store::challenge(
        state.database.pool(),
        &format!("captcha:{}", action.name()),
        &client.binding(),
        &code,
        binding(&config),
        None,
        i64::from(ttl),
    )
    .await?;
    let turnstile = config.captcha_provider == CaptchaProvider::Turnstile;
    Ok(Json(Challenge {
        id,
        action,
        provider: config.captcha_provider,
        expires_in: ttl,
        image_data_url: image,
        site_key: if turnstile {
            config.turnstile_site_key
        } else {
            None
        },
        cdata: turnstile.then(|| id.to_string()),
    }))
}
pub async fn verify(
    state: &AppState,
    client: &ClientContext,
    action: HumanAction,
    proof: &HumanProof,
) -> Result<(), ApiError> {
    store::limit(&state.security_cache, "captcha-verify", &client.ip, 60, 60).await?;
    let id = proof.captcha_id.ok_or_else(invalid)?;
    let code = proof.captcha_code.as_deref().unwrap_or_default();
    let well_formed = !code.is_empty() && code.len() <= 2048;
    let config = state.config.read().await.security.clone();
    let answer = if config.captcha_provider == CaptchaProvider::Image && well_formed {
        code.trim().to_ascii_uppercase()
    } else {
        String::new()
    };
    let payload = store::consume(
        state.database.pool(),
        id,
        &format!("captcha:{}", action.name()),
        &client.binding(),
        &answer,
    )
    .await?
    .ok_or_else(invalid)?;
    if !well_formed || payload != binding(&config) {
        return Err(invalid());
    }
    if config.captcha_provider == CaptchaProvider::Turnstile {
        verify_turnstile(
            &config,
            client,
            action,
            id,
            code,
            "https://challenges.cloudflare.com/turnstile/v0/siteverify",
        )
        .await?;
    }
    Ok(())
}
#[derive(Deserialize)]
struct TurnstileResponse {
    success: bool,
    hostname: Option<String>,
    action: Option<String>,
    cdata: Option<String>,
    #[serde(default, rename = "error-codes")]
    errors: Vec<String>,
}
fn validate_turnstile(
    config: &SecurityConfig,
    action: HumanAction,
    id: Uuid,
    status: reqwest::StatusCode,
    result: TurnstileResponse,
) -> Result<(), ApiError> {
    if result.errors.iter().any(|code| {
        matches!(
            code.as_str(),
            "invalid-input-secret" | "missing-input-secret"
        )
    }) {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "captcha_configuration_invalid",
            "Cloudflare 人机验证配置无效，请联系管理员",
        ));
    }
    if !status.is_success() && status != reqwest::StatusCode::BAD_REQUEST {
        return Err(unavailable());
    }
    if !status.is_success()
        || !result.success
        || result.action.as_deref() != Some(action.name())
        || result.cdata.as_deref() != Some(id.to_string().as_str())
        || !result.hostname.is_some_and(|host| {
            config
                .turnstile_hostnames
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&host))
        })
    {
        return Err(invalid());
    }
    Ok(())
}
// Endpoint is internal-only so tests can exercise HTTP failure modes without a production bypass setting.
async fn verify_turnstile(
    config: &SecurityConfig,
    client: &ClientContext,
    action: HumanAction,
    id: Uuid,
    code: &str,
    endpoint: &str,
) -> Result<(), ApiError> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unavailable())?;
    let mut request = json!({"secret":config.turnstile_secret,"response":code});
    if client.ip.parse::<std::net::IpAddr>().is_ok() {
        request["remoteip"] = json!(&client.ip);
    }
    let mut response = http
        .post(endpoint)
        .json(&request)
        .send()
        .await
        .map_err(|_| unavailable())?;
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if body.len() + chunk.len() > 16384 {
            return Err(unavailable());
        }
        body.extend_from_slice(&chunk);
    }
    let result = serde_json::from_slice(&body).map_err(|_| unavailable())?;
    validate_turnstile(config, action, id, status, result)
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
    fn provider_configuration_binding_changes_without_storing_keys() {
        let mut config = SecurityConfig::default();
        let image = binding(&config);
        config.captcha_provider = CaptchaProvider::Turnstile;
        config.turnstile_site_key = Some("public-site-key".into());
        config.turnstile_secret = Some("private-secret-key".into());
        config.turnstile_hostnames = vec!["example.test".into()];
        let cloudflare = binding(&config);
        assert_ne!(image, cloudflare);
        assert!(!cloudflare.to_string().contains("private-secret-key"));
        config.turnstile_secret = Some("rotated-secret-key".into());
        assert_ne!(cloudflare, binding(&config));
    }
    #[test]
    fn custom_image_rendering_is_randomized_for_the_same_answer() {
        let render = || {
            let mut image = captcha::Captcha::new();
            image.set_chars(&['W']).add_chars(5);
            encode_image(image).unwrap()
        };
        let a = render();
        let b = render();
        assert_eq!(a.0, b.0);
        assert_ne!(a.1, b.1);
    }
    #[tokio::test]
    async fn cloudflare_siteverify_checks_binding_and_distinguishes_configuration_and_network_errors()
     {
        use axum::{response::IntoResponse, routing::post};
        use std::sync::Arc;
        let config = SecurityConfig {
            turnstile_secret: Some("private-secret-key".into()),
            turnstile_hostnames: vec!["example.test".into()],
            ..Default::default()
        };
        let client = ClientContext {
            ip: "203.0.113.5".into(),
            user_agent: "test".into(),
        };
        let id = Uuid::new_v4();
        let valid = json!({"success":true,"hostname":"EXAMPLE.TEST","action":"login","cdata":id.to_string()});
        let cases = [
            (200, valid.clone(), None),
            (
                200,
                {
                    let mut value = valid.clone();
                    value["action"] = json!("registration_email");
                    value
                },
                Some((400, "captcha_invalid")),
            ),
            (
                200,
                {
                    let mut value = valid.clone();
                    value["hostname"] = json!("evil.test");
                    value
                },
                Some((400, "captcha_invalid")),
            ),
            (
                200,
                {
                    let mut value = valid.clone();
                    value["cdata"] = json!(Uuid::new_v4());
                    value
                },
                Some((400, "captcha_invalid")),
            ),
            (
                200,
                json!({"success":false,"error-codes":["invalid-input-secret"]}),
                Some((503, "captcha_configuration_invalid")),
            ),
            (
                400,
                json!({"success":false,"error-codes":["invalid-input-response"]}),
                Some((400, "captcha_invalid")),
            ),
            (
                500,
                json!({"success":false}),
                Some((503, "service_unavailable")),
            ),
            (
                200,
                json!({"unexpected":true}),
                Some((503, "service_unavailable")),
            ),
            (
                200,
                json!({"success":false,"padding":"x".repeat(17000)}),
                Some((503, "service_unavailable")),
            ),
        ];
        for (status, response, expected) in cases {
            let requests = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
            let captured = requests.clone();
            let app = Router::new().route(
                "/siteverify",
                post(move |Json(body): Json<Value>| {
                    let response = response.clone();
                    let captured = captured.clone();
                    async move {
                        captured.lock().await.push(body);
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            Json(response),
                        )
                            .into_response()
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/siteverify", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let result = verify_turnstile(
                &config,
                &client,
                HumanAction::Login,
                id,
                "test-token",
                &endpoint,
            )
            .await;
            if let Some((status, code)) = expected {
                let error = result.unwrap_err();
                assert_eq!(error.status.as_u16(), status);
                assert_eq!(error.code, code);
            } else {
                result.unwrap();
            }
            let request = &requests.lock().await[0];
            assert_eq!(request["secret"], "private-secret-key");
            assert_eq!(request["response"], "test-token");
            assert_eq!(request["remoteip"], "203.0.113.5");
            server.abort();
        }
    }
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
