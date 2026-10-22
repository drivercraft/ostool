//! Deployment settings are resolved at runtime, never baked into web assets.
use crate::{DatabaseConfig, ServerConfig};
use anyhow::{Context, bail};
use std::{collections::BTreeMap, path::Path, str::FromStr};

pub fn apply(config: &mut ServerConfig, file: Option<&Path>) -> anyhow::Result<()> {
    apply_values(config, &read_values(file)?)
}

/// Resolve only database settings for offline tools; hardware and auth are irrelevant.
pub fn apply_database(config: &mut DatabaseConfig, file: Option<&Path>) -> anyhow::Result<()> {
    apply_database_values(config, &read_values(file)?)
}

fn read_values(file: Option<&Path>) -> anyhow::Result<BTreeMap<String, String>> {
    let path = file.unwrap_or_else(|| Path::new(".env"));
    let mut values = BTreeMap::new();
    if file.is_some() || path.exists() {
        for entry in dotenvy::from_path_iter(path)
            .with_context(|| format!("failed to read {}", path.display()))?
        {
            let (key, value) = entry.context("invalid environment file")?;
            values.insert(key, value);
        }
    }
    // Process environment (including systemd EnvironmentFile) wins over .env.
    values.extend(std::env::vars());
    Ok(values)
}

pub(crate) fn apply_values(
    config: &mut ServerConfig,
    values: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    apply_database_values(&mut config.database, values)?;
    apply_security_values(&mut config.security, values)?;
    for (key, target) in [
        ("OSTOOL_PUBLIC_BASE_URL", &mut config.auth.public_base_url),
        ("OSTOOL_ADMIN_EMAIL", &mut config.auth.bootstrap_admin_email),
        (
            "OSTOOL_ADMIN_PASSWORD",
            &mut config.auth.bootstrap_admin_password,
        ),
    ] {
        if let Some(value) = values.get(key) {
            if value.trim().is_empty() {
                bail!("{key} must not be empty");
            }
            *target = Some(value.clone());
        }
    }
    parsed(values, "OSTOOL_EDITION", &mut config.edition)?;
    if let Some(value) = values.get("OSTOOL_AUDIT_RETENTION_DAYS") {
        let days: u32 = value
            .parse()
            .context("invalid OSTOOL_AUDIT_RETENTION_DAYS")?;
        config.audit.retention_days = (days > 0).then_some(days);
    }
    if let Some(value) = values.get("OSTOOL_TRUSTED_PROXY_IPS") {
        config.audit.trusted_proxies = value
            .split(',')
            .filter(|ip| !ip.trim().is_empty())
            .map(|ip| ip.trim().parse())
            .collect::<Result<_, _>>()
            .context("invalid OSTOOL_TRUSTED_PROXY_IPS")?;
    }
    parsed(values, "OSTOOL_LISTEN_ADDR", &mut config.listen_addr)?;
    parsed(values, "OSTOOL_AUTH_ENABLED", &mut config.auth.enabled)?;
    if config.edition.is_public()
        && values
            .get("OSTOOL_AUTH_ENABLED")
            .is_some_and(|value| value == "false")
    {
        bail!("public edition cannot disable authentication");
    }
    config.auth.enabled = config.edition.is_public();
    parsed(
        values,
        "OSTOOL_ALLOW_REGISTRATION",
        &mut config.auth.allow_registration,
    )?;
    parsed(
        values,
        "OSTOOL_ACCESS_TOKEN_TTL_SECS",
        &mut config.auth.access_token_ttl_secs,
    )?;
    parsed(
        values,
        "OSTOOL_REFRESH_TOKEN_TTL_SECS",
        &mut config.auth.refresh_token_ttl_secs,
    )?;
    parsed(
        values,
        "OSTOOL_DEVICE_CODE_TTL_SECS",
        &mut config.auth.device_code_ttl_secs,
    )?;
    parsed(
        values,
        "OSTOOL_NETWORK_TEST_ENABLED",
        &mut config.network_test.enabled,
    )?;
    config
        .security
        .validate_deployment(config.edition.is_public())?;
    config.validate()?;
    Ok(())
}

fn parsed<T: FromStr>(
    values: &BTreeMap<String, String>,
    key: &str,
    target: &mut T,
) -> anyhow::Result<()> {
    if let Some(value) = values.get(key) {
        *target = value
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid {key}"))?;
    }
    Ok(())
}

fn apply_database_values(
    config: &mut DatabaseConfig,
    values: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    for (key, target) in [
        ("OSTOOL_DATABASE_URL", &mut config.url),
        ("OSTOOL_DATABASE_SCHEMA", &mut config.schema),
    ] {
        if let Some(value) = values.get(key) {
            if value.trim().is_empty() {
                bail!("{key} must not be empty");
            }
            *target = Some(value.clone());
        }
    }
    parsed(
        values,
        "OSTOOL_DATABASE_MAX_CONNECTIONS",
        &mut config.max_connections,
    )?;
    parsed(values, "OSTOOL_DATABASE_MIGRATE", &mut config.migrate)?;
    Ok(())
}

fn apply_security_values(
    config: &mut crate::security::config::SecurityConfig,
    values: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    for (key, target) in [
        ("OSTOOL_REDIS_URL", &mut config.redis_url),
        ("OSTOOL_MFA_ENCRYPTION_KEY", &mut config.mfa_encryption_key),
        ("OSTOOL_TURNSTILE_SITE_KEY", &mut config.turnstile_site_key),
        ("OSTOOL_TURNSTILE_SECRET", &mut config.turnstile_secret),
        ("OSTOOL_SMTP_HOST", &mut config.smtp_host),
        ("OSTOOL_SMTP_USERNAME", &mut config.smtp_username),
        ("OSTOOL_SMTP_PASSWORD", &mut config.smtp_password),
        ("OSTOOL_SMTP_FROM", &mut config.smtp_from),
    ] {
        if let Some(value) = values.get(key) {
            anyhow::ensure!(!value.trim().is_empty(), "{key} must not be empty");
            *target = Some(value.clone());
        }
    }
    if let Some(value) = values.get("OSTOOL_REDIS_PREFIX") {
        config.redis_prefix = value.clone();
    }
    if let Some(value) = values.get("OSTOOL_CAPTCHA_PROVIDER") {
        config.captcha_provider = match value.as_str() {
            "image" => crate::security::config::CaptchaProvider::Image,
            "turnstile" => crate::security::config::CaptchaProvider::Turnstile,
            _ => bail!("OSTOOL_CAPTCHA_PROVIDER must be image or turnstile"),
        };
    }
    if let Some(value) = values.get("OSTOOL_SMTP_TLS") {
        config.smtp_tls = match value.as_str() {
            "starttls" => crate::security::config::SmtpTls::Starttls,
            "tls" => crate::security::config::SmtpTls::Tls,
            "loopback_plaintext" => crate::security::config::SmtpTls::LoopbackPlaintext,
            _ => bail!("invalid OSTOOL_SMTP_TLS"),
        };
    }
    for (key, target) in [
        (
            "OSTOOL_ADMIN_ALLOWED_NETWORKS",
            &mut config.admin_allowed_networks,
        ),
        (
            "OSTOOL_HARDWARE_ALLOWED_NETWORKS",
            &mut config.hardware_allowed_networks,
        ),
        (
            "OSTOOL_TURNSTILE_HOSTNAMES",
            &mut config.turnstile_hostnames,
        ),
    ] {
        if let Some(value) = values.get(key) {
            *target = value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
        }
    }
    parsed(
        values,
        "OSTOOL_REQUIRE_ADMIN_MFA",
        &mut config.require_admin_mfa,
    )?;
    parsed(
        values,
        "OSTOOL_REGISTRATION_EMAIL_VERIFICATION",
        &mut config.registration_email_verification,
    )?;
    parsed(
        values,
        "OSTOOL_LOGIN_MAX_FAILURES",
        &mut config.login_max_failures,
    )?;
    parsed(
        values,
        "OSTOOL_LOGIN_LOCK_SECS",
        &mut config.login_lock_secs,
    )?;
    parsed(values, "OSTOOL_SMTP_PORT", &mut config.smtp_port)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_database_settings_ignore_hardware_and_auth() {
        let mut config = DatabaseConfig::default();
        apply_database_values(
            &mut config,
            &BTreeMap::from([
                (
                    "OSTOOL_DATABASE_URL".into(),
                    "postgres://localhost/test".into(),
                ),
                ("OSTOOL_DATABASE_MAX_CONNECTIONS".into(), "2".into()),
                ("OSTOOL_AUTH_ENABLED".into(), "irrelevant".into()),
            ]),
        )
        .unwrap();
        assert_eq!(config.max_connections, 2);
        assert!(
            apply_database_values(
                &mut config,
                &BTreeMap::from([("OSTOOL_DATABASE_MIGRATE".into(), "yes".into()),])
            )
            .is_err()
        );
    }
    #[test]
    fn edition_is_explicit_and_public_cannot_disable_authentication() {
        let mut config = ServerConfig::default();
        config.network.interface = "lo".into();
        config.auth.enabled = true;
        apply_values(
            &mut config,
            &BTreeMap::from([(
                "OSTOOL_DATABASE_URL".into(),
                "postgres://localhost/test".into(),
            )]),
        )
        .unwrap();
        assert_eq!(config.edition, crate::config::Edition::Local);
        assert!(!config.auth.enabled);
        assert!(
            apply_values(
                &mut config,
                &BTreeMap::from([
                    ("OSTOOL_EDITION".into(), "public".into()),
                    ("OSTOOL_AUTH_ENABLED".into(), "false".into())
                ])
            )
            .is_err()
        );
        assert!(
            apply_values(
                &mut config,
                &BTreeMap::from([("OSTOOL_EDITION".into(), "pubic".into())])
            )
            .is_err()
        );
    }
    #[test]
    fn overrides_and_invalid_values() {
        let mut config = ServerConfig::default();
        config.network.interface = "lo".into();
        apply_values(
            &mut config,
            &BTreeMap::from([
                ("OSTOOL_EDITION".into(), "public".into()),
                ("OSTOOL_AUTH_ENABLED".into(), "true".into()),
                (
                    "OSTOOL_DATABASE_URL".into(),
                    "postgres://localhost/test".into(),
                ),
                (
                    "OSTOOL_PUBLIC_BASE_URL".into(),
                    "https://boards.example.com".into(),
                ),
            ]),
        )
        .unwrap();
        assert!(config.auth.enabled);
        assert!(
            apply_values(
                &mut config,
                &BTreeMap::from([("OSTOOL_AUTH_ENABLED".into(), "yes".into())])
            )
            .is_err()
        );
    }
}
