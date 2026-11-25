use anyhow::{bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SecurityConfig {
    pub name_policy: super::names::NamePolicy,
    #[serde(skip_serializing)]
    pub redis_url: Option<String>,
    pub redis_prefix: String,
    pub captcha_provider: CaptchaProvider,
    pub turnstile_site_key: Option<String>,
    #[serde(skip_serializing)]
    pub turnstile_secret: Option<String>,
    pub turnstile_hostnames: Vec<String>,
    #[serde(skip_serializing)]
    pub mfa_encryption_key: Option<String>,
    pub require_admin_mfa: bool,
    pub registration_email_verification: bool,
    /// Runtime policy for local registration testing.
    pub dev_registration_codes: bool,
    pub login_max_failures: u32,
    pub login_lock_secs: u32,
    pub admin_allowed_networks: Vec<String>,
    #[serde(default)]
    pub mail_provider: MailProvider,
    #[serde(default = "default_ses_region")]
    pub ses_region: String,
    #[serde(default = "default_mail_hourly_limit")]
    pub mail_hourly_limit: u32,
    pub smtp_enabled: bool,
    pub password_reset_enabled: bool,
    pub password_reset_ttl_secs: u32,
    pub email_policy: EmailPolicy,
    pub registration_email_policy: EmailPolicy,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    #[serde(skip_serializing)]
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
/// Managed providers use the same encrypted SMTP credential storage as custom SMTP.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailProvider {
    #[default]
    Smtp,
    Brevo,
    Mailjet,
    Ses,
}
fn default_ses_region() -> String {
    "us-east-1".into()
}
fn default_mail_hourly_limit() -> u32 {
    100
}
impl SecurityConfig {
    pub fn mail_endpoint(&self) -> (String, u16, SmtpTls) {
        match self.mail_provider {
            MailProvider::Smtp => (
                self.smtp_host.clone().unwrap_or_default(),
                self.smtp_port,
                self.smtp_tls,
            ),
            MailProvider::Brevo => ("smtp-relay.brevo.com".into(), 587, SmtpTls::Starttls),
            MailProvider::Mailjet => ("in-v3.mailjet.com".into(), 587, SmtpTls::Starttls),
            MailProvider::Ses => (
                format!("email-smtp.{}.amazonaws.com", self.ses_region),
                587,
                SmtpTls::Starttls,
            ),
        }
    }
}
pub fn parse_turnstile_hostnames(value: &str) -> anyhow::Result<Vec<String>> {
    let mut hosts = Vec::new();
    for host in value
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '，'))
        .filter(|v| !v.is_empty())
    {
        let host = host.to_ascii_lowercase();
        validate_turnstile_hostname(&host)?;
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    ensure!(hosts.len() <= 100, "Turnstile 最多允许 100 个域名");
    Ok(hosts)
}
fn validate_turnstile_hostname(host: &str) -> anyhow::Result<()> {
    ensure!(
        !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.starts_with(|c: char| c.is_ascii_alphanumeric())
                    && label.ends_with(|c: char| c.is_ascii_alphanumeric())
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }),
        "Turnstile 须填写明确的域名或 IPv4 地址，不含协议、端口、通配符或空域名段"
    );
    Ok(())
}
impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            name_policy: Default::default(),
            redis_url: None,
            redis_prefix: "ostool".into(),
            captcha_provider: CaptchaProvider::Image,
            turnstile_site_key: None,
            turnstile_secret: None,
            turnstile_hostnames: vec![],
            mfa_encryption_key: None,
            require_admin_mfa: false,
            registration_email_verification: false,
            dev_registration_codes: false,
            login_max_failures: 6,
            login_lock_secs: 900,
            admin_allowed_networks: vec![],
            mail_provider: MailProvider::Smtp,
            ses_region: default_ses_region(),
            mail_hourly_limit: default_mail_hourly_limit(),
            smtp_enabled: true,
            password_reset_enabled: true,
            password_reset_ttl_secs: 600,
            email_policy: EmailPolicy::default(),
            registration_email_policy: EmailPolicy::default(),
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
        self.validate_inner(public, true)
    }
    /// 启动时先校验部署值，再在数据库策略合并后校验依赖关系。
    pub(crate) fn validate_deployment(&self, public: bool) -> anyhow::Result<()> {
        self.validate_inner(public, false)
    }
    fn validate_inner(&self, public: bool, complete: bool) -> anyhow::Result<()> {
        if !public {
            return Ok(());
        }
        self.name_policy.validate()?;
        ensure!(
            !self.redis_prefix.is_empty()
                && self.redis_prefix.len() <= 80
                && self
                    .redis_prefix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid Redis prefix"
        );
        if let Some(value) = &self.redis_url {
            let value = url::Url::parse(value).map_err(|_| anyhow::anyhow!("invalid Redis URL"))?;
            ensure!(
                matches!(value.scheme(), "redis" | "rediss"),
                "Redis URL must use redis or rediss"
            );
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
            !complete || !self.require_admin_mfa || self.mfa_encryption_key.is_some(),
            "administrator MFA requires OSTOOL_MFA_ENCRYPTION_KEY"
        );
        for network in &self.admin_allowed_networks {
            network
                .parse::<ipnet::IpNet>()
                .map_err(|_| anyhow::anyhow!("invalid admin network: {network}"))?;
        }
        if complete && self.captcha_provider == CaptchaProvider::Turnstile {
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
            ensure!(
                self.turnstile_hostnames.len() <= 100,
                "Turnstile 最多允许 100 个域名"
            );
            for host in &self.turnstile_hostnames {
                validate_turnstile_hostname(host)?;
            }
            for key in [&self.turnstile_site_key, &self.turnstile_secret]
                .into_iter()
                .flatten()
            {
                ensure!(
                    (10..=256).contains(&key.len())
                        && key
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')),
                    "Turnstile 密钥格式无效"
                );
            }
        }
        ensure!(
            !complete
                || self.mail_provider != MailProvider::Smtp
                || self.smtp_host.is_some() == self.smtp_from.is_some(),
            "SMTP requires both host and from address"
        );
        ensure!(
            !complete || self.smtp_username.is_some() == self.smtp_password.is_some(),
            "SMTP credentials require both username and password"
        );
        ensure!(
            (1..=100000).contains(&self.mail_hourly_limit),
            "全站邮件小时额度须为 1–100000"
        );
        ensure!(
            self.ses_region.len() <= 40
                && self.ses_region.split('-').count() >= 3
                && self
                    .ses_region
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && self
                    .ses_region
                    .starts_with(|c: char| c.is_ascii_lowercase())
                && self.ses_region.ends_with(|c: char| c.is_ascii_digit()),
            "SES region 格式无效"
        );
        if complete && self.smtp_enabled && self.mail_provider != MailProvider::Smtp {
            ensure!(
                self.smtp_from.is_some()
                    && self.smtp_username.as_ref().is_some_and(|s| !s.is_empty())
                    && self.smtp_password.as_ref().is_some_and(|s| !s.is_empty()),
                "邮件服务商需要发件人、SMTP 用户名和密码"
            );
        }
        ensure!(self.smtp_port > 0, "SMTP port must be nonzero");
        if let Some(host) = &self.smtp_host {
            ensure!(
                !host.is_empty()
                    && host.len() <= 253
                    && !host.chars().any(char::is_whitespace)
                    && !host.contains('/'),
                "SMTP 主机须为主机名或 IP，不含协议或路径"
            );
        }
        if let Some(from) = &self.smtp_from {
            ensure!(
                from.parse::<lettre::message::Mailbox>().is_ok(),
                "发件人须为邮箱或名称 <邮箱> 格式"
            );
        }
        ensure!(
            self.smtp_username
                .as_ref()
                .is_none_or(|value| value.len() <= 320),
            "SMTP 用户名过长"
        );
        self.email_policy.validate(86400)?;
        self.registration_email_policy.validate(3600)?;
        ensure!(
            (300..=7200).contains(&self.password_reset_ttl_secs),
            "密码恢复有效期须为 300–7200 秒"
        );
        if complete && self.registration_email_verification {
            ensure!(
                self.smtp_available() || self.dev_registration_codes,
                "email verification requires a configured mail provider and sender"
            );
        }
        if complete
            && self.mail_provider == MailProvider::Smtp
            && self.smtp_tls == SmtpTls::LoopbackPlaintext
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EmailPolicy {
    pub ttl_secs: u32,
    pub resend_secs: u32,
    pub window_secs: u32,
    pub email_limit: u32,
    pub ip_limit: u32,
    pub max_attempts: u32,
}
impl Default for EmailPolicy {
    fn default() -> Self {
        Self {
            ttl_secs: 600,
            resend_secs: 60,
            window_secs: 3600,
            email_limit: 5,
            ip_limit: 8,
            max_attempts: 5,
        }
    }
}
impl EmailPolicy {
    fn validate(&self, max_ttl: u32) -> anyhow::Result<()> {
        ensure!(
            (60..=max_ttl).contains(&self.ttl_secs),
            "邮件验证码有效期超出允许范围"
        );
        ensure!(
            (30..=3600).contains(&self.resend_secs),
            "邮件重发冷却须为 30–3600 秒"
        );
        ensure!(
            (3600..=86400).contains(&self.window_secs),
            "邮件统计窗口须为 3600–86400 秒"
        );
        ensure!(
            (1..=100).contains(&self.email_limit) && (1..=5000).contains(&self.ip_limit),
            "邮件发送额度超出允许范围"
        );
        ensure!(
            (1..=5).contains(&self.max_attempts),
            "验证码最多允许 1–5 次尝试"
        );
        Ok(())
    }
}
impl SecurityConfig {
    pub fn smtp_available(&self) -> bool {
        self.smtp_enabled
            && self.smtp_from.is_some()
            && match self.mail_provider {
                MailProvider::Smtp => self.smtp_host.is_some(),
                _ => {
                    self.smtp_username.as_ref().is_some_and(|s| !s.is_empty())
                        && self.smtp_password.as_ref().is_some_and(|s| !s.is_empty())
                }
            }
    }
}

/// 后台业务策略；连接与主加密密钥留在启动配置，业务凭据单独加密保存。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AccountSettings {
    #[serde(default)]
    pub dev_registration_codes: bool,
    #[serde(default)]
    pub admin_allowed_networks: Vec<String>,
    #[serde(default)]
    pub name_policy: super::names::NamePolicy,
    pub allow_registration: bool,
    pub access_token_ttl_secs: u64,
    pub refresh_token_ttl_secs: u64,
    pub device_code_ttl_secs: u64,
    pub max_active_sessions: u32,
    pub captcha_provider: CaptchaProvider,
    pub turnstile_site_key: Option<String>,
    pub turnstile_hostnames: Vec<String>,
    pub require_admin_mfa: bool,
    pub registration_email_verification: bool,
    pub login_max_failures: u32,
    pub login_lock_secs: u32,
    #[serde(default)]
    pub mail_provider: MailProvider,
    #[serde(default = "default_ses_region")]
    pub ses_region: String,
    #[serde(default = "default_mail_hourly_limit")]
    pub mail_hourly_limit: u32,
    pub smtp_enabled: bool,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub smtp_from: Option<String>,
    pub smtp_tls: SmtpTls,
    pub password_reset_enabled: bool,
    pub password_reset_ttl_secs: u32,
    pub email_policy: EmailPolicy,
    pub registration_email_policy: EmailPolicy,
}
impl AccountSettings {
    pub fn from_config(config: &crate::ServerConfig) -> Self {
        Self {
            dev_registration_codes: config.security.dev_registration_codes,
            admin_allowed_networks: config.security.admin_allowed_networks.clone(),
            name_policy: config.security.name_policy.clone(),
            allow_registration: config.auth.allow_registration,
            access_token_ttl_secs: config.auth.access_token_ttl_secs,
            refresh_token_ttl_secs: config.auth.refresh_token_ttl_secs,
            device_code_ttl_secs: config.auth.device_code_ttl_secs,
            max_active_sessions: config.auth.max_active_sessions,
            captcha_provider: config.security.captcha_provider,
            turnstile_site_key: config.security.turnstile_site_key.clone(),
            turnstile_hostnames: config.security.turnstile_hostnames.clone(),
            require_admin_mfa: config.security.require_admin_mfa,
            registration_email_verification: config.security.registration_email_verification,
            login_max_failures: config.security.login_max_failures,
            login_lock_secs: config.security.login_lock_secs,
            mail_provider: config.security.mail_provider,
            ses_region: config.security.ses_region.clone(),
            mail_hourly_limit: config.security.mail_hourly_limit,
            smtp_enabled: config.security.smtp_enabled,
            smtp_host: config.security.smtp_host.clone(),
            smtp_port: config.security.smtp_port,
            smtp_from: config.security.smtp_from.clone(),
            smtp_tls: config.security.smtp_tls,
            password_reset_enabled: config.security.password_reset_enabled,
            password_reset_ttl_secs: config.security.password_reset_ttl_secs,
            email_policy: config.security.email_policy.clone(),
            registration_email_policy: config.security.registration_email_policy.clone(),
        }
    }
    pub fn apply(&self, config: &mut crate::ServerConfig) -> anyhow::Result<()> {
        self.name_policy.validate()?;
        config.security.dev_registration_codes = self.dev_registration_codes;
        config.security.admin_allowed_networks = self.admin_allowed_networks.clone();
        config.security.name_policy = self.name_policy.clone();
        config.auth.allow_registration = self.allow_registration;
        config.auth.access_token_ttl_secs = self.access_token_ttl_secs;
        config.auth.refresh_token_ttl_secs = self.refresh_token_ttl_secs;
        config.auth.device_code_ttl_secs = self.device_code_ttl_secs;
        config.auth.max_active_sessions = self.max_active_sessions;
        config.security.captcha_provider = self.captcha_provider;
        config.security.turnstile_site_key = self.turnstile_site_key.clone();
        config.security.turnstile_hostnames =
            parse_turnstile_hostnames(&self.turnstile_hostnames.join(","))?;
        config.security.require_admin_mfa = self.require_admin_mfa;
        config.security.registration_email_verification = self.registration_email_verification;
        config.security.login_max_failures = self.login_max_failures;
        config.security.login_lock_secs = self.login_lock_secs;
        config.security.mail_provider = self.mail_provider;
        config.security.ses_region = self.ses_region.clone();
        config.security.mail_hourly_limit = self.mail_hourly_limit;
        config.security.smtp_enabled = self.smtp_enabled;
        config.security.smtp_host = self.smtp_host.clone();
        config.security.smtp_port = self.smtp_port;
        config.security.smtp_from = self.smtp_from.clone();
        config.security.smtp_tls = self.smtp_tls;
        config.security.password_reset_enabled = self.password_reset_enabled;
        config.security.password_reset_ttl_secs = self.password_reset_ttl_secs;
        config.security.email_policy = self.email_policy.clone();
        config.security.registration_email_policy = self.registration_email_policy.clone();
        config.security.validate(config.auth.enabled)?;
        config.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turnstile_hostnames_are_normalized_deduplicated_and_reject_ambiguous_domains() {
        assert_eq!(
            parse_turnstile_hostnames(
                " BOARDS.Example.test，localhost\nboards.example.test 127.0.0.1"
            )
            .unwrap(),
            vec!["boards.example.test", "localhost", "127.0.0.1"]
        );
        for value in [
            "https://example.test",
            "*.example.test",
            "example.test:443",
            ".example.test",
            "example..test",
            "example.test.",
            "-example.test",
            "example-.test",
            "bad_host.test",
        ] {
            assert!(parse_turnstile_hostnames(value).is_err(), "{value}");
        }
    }
    #[test]
    fn development_registration_codes_are_explicit_runtime_policy_with_loopback_requirement() {
        let mut config = SecurityConfig {
            registration_email_verification: true,
            ..Default::default()
        };
        assert!(!config.dev_registration_codes);
        assert!(config.validate(true).is_err());
        config.dev_registration_codes = true;
        config.validate(true).unwrap();
        assert!(!config.smtp_available());
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("dev_registration_codes")
                .is_some()
        );
        let server = crate::ServerConfig {
            security: config,
            listen_addr: "0.0.0.0:2999".parse().unwrap(),
            ..Default::default()
        };
        assert!(
            server
                .validate()
                .unwrap_err()
                .to_string()
                .contains("loopback")
        );
    }
    #[test]
    fn managed_mail_providers_require_credentials_and_pin_encrypted_endpoints() {
        for (provider, host) in [
            (MailProvider::Brevo, "smtp-relay.brevo.com"),
            (MailProvider::Mailjet, "in-v3.mailjet.com"),
            (MailProvider::Ses, "email-smtp.eu-west-1.amazonaws.com"),
        ] {
            let mut config = SecurityConfig {
                mail_provider: provider,
                ses_region: "eu-west-1".into(),
                ..Default::default()
            };
            assert!(config.validate(true).is_err());
            assert!(!config.smtp_available());
            config.smtp_username = Some("provider-login".into());
            config.smtp_password = Some("secret-smtp-key".into());
            config.smtp_from = Some("ostool <noreply@example.test>".into());
            config.registration_email_verification = true;
            config.validate(true).unwrap();
            assert!(config.smtp_available());
            assert_eq!(
                config.mail_endpoint(),
                (host.into(), 587, SmtpTls::Starttls)
            );
            assert!(
                !serde_json::to_string(&config)
                    .unwrap()
                    .contains("secret-smtp-key")
            );
            config.smtp_enabled = false;
            assert!(!config.smtp_available());
        }
    }
    #[test]
    fn old_account_settings_default_to_custom_smtp_and_bounded_mail_budget() {
        let settings = AccountSettings::from_config(&crate::ServerConfig::default());
        let mut value = serde_json::to_value(&settings).unwrap();
        for key in ["mail_provider", "ses_region", "mail_hourly_limit"] {
            value.as_object_mut().unwrap().remove(key);
        }
        let decoded: AccountSettings = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, settings);
        let mut config = SecurityConfig {
            mail_hourly_limit: 0,
            ..Default::default()
        };
        assert!(config.validate(true).is_err());
        config.mail_hourly_limit = 100;
        config.ses_region = "us-east-1.evil.test/".into();
        assert!(config.validate(true).is_err());
    }
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
