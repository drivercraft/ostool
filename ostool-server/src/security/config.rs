use anyhow::{bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SecurityConfig {
    pub captcha_provider: CaptchaProvider,
    pub turnstile_site_key: Option<String>,
    #[serde(skip_serializing)]
    pub turnstile_secret: Option<String>,
    pub turnstile_hostnames: Vec<String>,
    #[serde(skip_serializing)]
    pub mfa_encryption_key: Option<String>,
    pub require_admin_mfa: bool,
    pub registration_email_verification: bool,
    pub login_max_failures: u32,
    pub login_lock_secs: u32,
    pub admin_allowed_networks: Vec<String>,
    pub hardware_allowed_networks: Vec<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub smtp_username: Option<String>,
    #[serde(skip_serializing)]
    pub smtp_password: Option<String>,
    pub smtp_from: Option<String>,
    pub smtp_tls: SmtpTls,
}
impl std::fmt::Debug for SecurityConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecurityConfig").finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptchaProvider {
    #[default]
    Image,
    Turnstile,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTls {
    #[default]
    Starttls,
    Tls,
    LoopbackPlaintext,
}
impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            captcha_provider: CaptchaProvider::Image,
            turnstile_site_key: None,
            turnstile_secret: None,
            turnstile_hostnames: vec![],
            mfa_encryption_key: None,
            require_admin_mfa: false,
            registration_email_verification: false,
            login_max_failures: 6,
            login_lock_secs: 900,
            admin_allowed_networks: vec![],
            hardware_allowed_networks: [
                "127.0.0.0/8",
                "10.0.0.0/8",
                "172.16.0.0/12",
                "192.168.0.0/16",
                "::1/128",
                "fc00::/7",
                "fe80::/10",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            smtp_host: None,
            smtp_port: 587,
            smtp_username: None,
            smtp_password: None,
            smtp_from: None,
            smtp_tls: SmtpTls::Starttls,
        }
    }
}
impl SecurityConfig {
    pub fn validate(&self, public: bool) -> anyhow::Result<()> {
        if !public {
            return Ok(());
        }
        ensure!(
            (3..=20).contains(&self.login_max_failures),
            "security.login_max_failures must be 3..20"
        );
        ensure!(
            (60..=86400).contains(&self.login_lock_secs),
            "security.login_lock_secs must be 60..86400"
        );
        if let Some(key) = &self.mfa_encryption_key {
            ensure!(
                STANDARD.decode(key).is_ok_and(|key| key.len() == 32),
                "OSTOOL_MFA_ENCRYPTION_KEY must be a base64-encoded random 32-byte key"
            );
        }
        ensure!(
            !self.require_admin_mfa || self.mfa_encryption_key.is_some(),
            "administrator MFA requires OSTOOL_MFA_ENCRYPTION_KEY"
        );
        for network in self
            .admin_allowed_networks
            .iter()
            .chain(&self.hardware_allowed_networks)
        {
            network
                .parse::<ipnet::IpNet>()
                .map_err(|_| anyhow::anyhow!("invalid admin network: {network}"))?;
        }
        if self.captcha_provider == CaptchaProvider::Turnstile {
            ensure!(
                self.turnstile_site_key
                    .as_ref()
                    .is_some_and(|v| !v.is_empty())
                    && self
                        .turnstile_secret
                        .as_ref()
                        .is_some_and(|v| !v.is_empty()),
                "Turnstile requires site and secret keys"
            );
            ensure!(
                !self.turnstile_hostnames.is_empty(),
                "Turnstile requires explicit hostnames"
            );
            for host in &self.turnstile_hostnames {
                ensure!(
                    !host.is_empty()
                        && !host.contains(['/', ':', '*'])
                        && host
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
                    "invalid Turnstile hostname"
                );
            }
        }
        ensure!(
            self.smtp_host.is_some() == self.smtp_from.is_some(),
            "SMTP requires both host and from address"
        );
        ensure!(
            self.smtp_username.is_some() == self.smtp_password.is_some(),
            "SMTP credentials require both username and password"
        );
        ensure!(self.smtp_port > 0, "SMTP port must be nonzero");
        if self.registration_email_verification {
            ensure!(
                self.smtp_host.is_some() && self.smtp_from.is_some(),
                "email verification requires SMTP host and from address"
            );
        }
        if self.smtp_tls == SmtpTls::LoopbackPlaintext
            && !matches!(
                self.smtp_host.as_deref(),
                Some("127.0.0.1" | "::1" | "localhost")
            )
        {
            bail!("plaintext SMTP is restricted to loopback");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_public_uses_image_without_email_or_mfa_dependencies() {
        let config = SecurityConfig::default();
        config.validate(true).unwrap();
        assert_eq!(config.captcha_provider, CaptchaProvider::Image);
        assert!(!config.registration_email_verification);
        assert!(!config.require_admin_mfa);
    }
    #[test]
    fn deployment_security_fails_closed_on_partial_settings() {
        let mut config = SecurityConfig {
            require_admin_mfa: true,
            ..Default::default()
        };
        assert!(config.validate(true).is_err());
        config.mfa_encryption_key = Some(STANDARD.encode([9u8; 32]));
        config.validate(true).unwrap();
        config.registration_email_verification = true;
        assert!(config.validate(true).is_err());
        config.smtp_host = Some("smtp.example.test".into());
        config.smtp_from = Some("noreply@example.test".into());
        config.validate(true).unwrap();
        config.smtp_tls = SmtpTls::LoopbackPlaintext;
        assert!(config.validate(true).is_err());
        config.validate(false).unwrap();
    }
    #[test]
    fn turnstile_needs_secret_and_explicit_hostnames_and_secrets_are_redacted() {
        let mut config = SecurityConfig {
            captcha_provider: CaptchaProvider::Turnstile,
            turnstile_site_key: Some("public-site-key".into()),
            ..Default::default()
        };
        assert!(config.validate(true).is_err());
        config.turnstile_secret = Some("private-turnstile-secret".into());
        config.turnstile_hostnames = vec!["*.example.test".into()];
        assert!(config.validate(true).is_err());
        config.turnstile_hostnames = vec!["boards.example.test".into()];
        config.validate(true).unwrap();
        assert!(
            !serde_json::to_string(&config)
                .unwrap()
                .contains("private-turnstile-secret")
        );
        assert!(!format!("{config:?}").contains("private-turnstile-secret"));
    }
}
