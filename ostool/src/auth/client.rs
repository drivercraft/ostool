use std::time::Duration;

use anyhow::{Context as _, bail};
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::Deserialize;

use crate::board::global_config::BoardEndpoint;

use super::credential_store::CredentialRecord;

const CLIENT_ID: &str = "ostool-cli";

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: i64,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    error_description: Option<String>,
}

#[derive(Clone)]
pub struct AuthClient {
    client: Client,
    endpoint: BoardEndpoint,
}

impl AuthClient {
    pub fn new(endpoint: BoardEndpoint) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                // Authentication responses must never be redirected to a host that
                // did not receive the original Device Authorization request.
                .redirect(Policy::none())
                .build()
                .context("failed to build authentication HTTP client")?,
            endpoint,
        })
    }

    pub async fn request_device_code(&self) -> anyhow::Result<DeviceCodeResponse> {
        let response = self
            .client
            .post(self.endpoint.base_url.join("oauth/device/code")?)
            .form(&[
                ("client_id", CLIENT_ID),
                ("scope", "board:operate offline_access"),
            ])
            .send()
            .await
            .context("failed to request device code")?;
        decode_json(response).await
    }

    pub async fn exchange_device_code(
        &self,
        device_code: &str,
    ) -> anyhow::Result<CredentialRecord> {
        let response = self
            .client
            .post(self.endpoint.base_url.join("oauth/token")?)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .context("failed to exchange device code")?;
        token_record(response).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> anyhow::Result<CredentialRecord> {
        let response = self
            .client
            .post(self.endpoint.base_url.join("oauth/token")?)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .context("failed to refresh access token")?;
        token_record(response).await
    }

    pub async fn revoke(&self, refresh_token: &str) -> anyhow::Result<()> {
        let response = self
            .client
            .post(self.endpoint.base_url.join("oauth/revoke")?)
            .form(&[
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .context("failed to revoke OAuth session")?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(oauth_error(response).await)
        }
    }
}

pub async fn complete_device_login(client: &AuthClient) -> anyhow::Result<CredentialRecord> {
    let device = client.request_device_code().await?;
    println!("Open this URL to sign in:");
    println!(
        "  {}",
        device
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&device.verification_uri)
    );
    println!("Code: {}", device.user_code);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = Duration::from_secs(device.interval.max(1));
    loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("device login expired before authorization completed");
        }
        // Device Authorization requires polling at the server-provided rate; a
        // slow_down response below increases this delay instead of busy-looping.
        tokio::time::sleep(interval).await;
        match client.exchange_device_code(&device.device_code).await {
            Ok(record) => return Ok(record),
            Err(error) if error.to_string().contains("authorization_pending") => continue,
            Err(error) if error.to_string().contains("slow_down") => {
                interval += Duration::from_secs(5);
            }
            Err(error) => return Err(error),
        }
    }
}

async fn token_record(response: reqwest::Response) -> anyhow::Result<CredentialRecord> {
    let token: TokenResponse = decode_json(response).await?;
    if !token.token_type.eq_ignore_ascii_case("bearer") {
        bail!(
            "authentication server returned unsupported token type `{}`",
            token.token_type
        );
    }
    if token.expires_in <= 0 {
        bail!("authentication server returned a non-positive token lifetime");
    }
    let refresh_token = token.refresh_token.filter(|value| !value.is_empty());
    if refresh_token.is_none() {
        bail!("authentication server did not return a refresh token");
    }
    Ok(CredentialRecord::OAuthRefresh {
        refresh_token: refresh_token.expect("required refresh token checked"),
        access_token: token.access_token,
        access_expires_at: Utc::now() + ChronoDuration::seconds(token.expires_in),
        scope: token.scope,
    })
}

async fn decode_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    if response.status().is_success() {
        response
            .json()
            .await
            .context("failed to decode authentication response")
    } else {
        Err(oauth_error(response).await)
    }
}

async fn oauth_error(response: reqwest::Response) -> anyhow::Error {
    let status = response.status();
    match response.json::<OAuthErrorResponse>().await {
        Ok(error) => anyhow::anyhow!(
            "authentication request failed with {status}: {}{}",
            error.error,
            error
                .error_description
                .as_deref()
                .map(|description| format!(" ({description})"))
                .unwrap_or_default()
        ),
        Err(_) => anyhow::anyhow!("authentication request failed with {status}"),
    }
}
