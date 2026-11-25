//! Deployment settings are resolved at runtime, never baked into web assets.
use crate::{DatabaseConfig, ServerConfig};
use anyhow::{Context, bail};
use std::{collections::BTreeMap, path::Path, str::FromStr};

/// The sole runtime configuration is .env plus process overrides; no TOML is created.
pub fn load(file: Option<&Path>) -> anyhow::Result<ServerConfig> {
    let mut config = ServerConfig::default_for_path(file.unwrap_or_else(|| Path::new(".env")));
    config.listen_addr = "127.0.0.1:2999".parse()?;
    config.tftp = crate::TftpConfig::Builtin(crate::BuiltinTftpConfig {
        enabled: false,
        ..crate::BuiltinTftpConfig::default_with_root(config.data_dir.join("tftp"))
    });
    config.loader_network.enabled = false;
    config.http_boot.enabled = false;
    config.network_test.enabled = false;
    config.network.interface = crate::serial::network::default_non_loopback_interface_name()
        .unwrap_or_else(|| "lo".into());
    apply(&mut config, file)?;
    config.normalize_paths(file.unwrap_or_else(|| Path::new(".env")))?;
    Ok(config)
}

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
    reject_runtime_settings(values)?;
    apply_database_values(&mut config.database, values)?;
    if let Some(path) = values.get("OSTOOL_DATA_DIR") {
        config.data_dir = path.into();
        config.http_boot.root_dir = config.data_dir.join("http-boot");
        config.virtual_qemu.runtime_dir = config.data_dir.join("qemu");
        if let crate::TftpConfig::Builtin(tftp) = &mut config.tftp {
            tftp.root_dir = config.data_dir.join("tftp");
        }
    }
    parsed(
        values,
        "OSTOOL_LOADER_NETWORK_ENABLED",
        &mut config.loader_network.enabled,
    )?;
    parsed(
        values,
        "OSTOOL_LOADER_BIND_ADDR",
        &mut config.loader_network.bind_addr,
    )?;
    parsed(
        values,
        "OSTOOL_HTTP_BOOT_ENABLED",
        &mut config.http_boot.enabled,
    )?;
    if let Some(url) = values.get("OSTOOL_HTTP_BOOT_BASE_URL") {
        config.http_boot.public_base_url = Some(url.clone());
    }
    if let Some(addr) = values.get("OSTOOL_HTTP_BOOT_LISTEN_ADDR") {
        config.http_boot.listen_addr = Some(
            addr.parse()
                .context("invalid OSTOOL_HTTP_BOOT_LISTEN_ADDR")?,
        );
    }
    parsed(
        values,
        "OSTOOL_VIRTUAL_QEMU_ENABLED",
        &mut config.virtual_qemu.enabled,
    )?;
    for (key, target) in [
        ("OSTOOL_QEMU_BINARY", &mut config.virtual_qemu.qemu_binary),
        ("OSTOOL_OVMF_CODE", &mut config.virtual_qemu.ovmf_code),
        ("OSTOOL_OVMF_VARS", &mut config.virtual_qemu.ovmf_vars),
        ("OSTOOL_AXLOADER_EFI", &mut config.virtual_qemu.axloader_efi),
    ] {
        if let Some(value) = values.get(key) {
            *target = value.into();
        }
    }
    for (key, target) in [
        (
            "OSTOOL_QEMU_NAMESPACE",
            &mut config.virtual_qemu.network_namespace,
        ),
        ("OSTOOL_QEMU_BRIDGE", &mut config.virtual_qemu.bridge),
    ] {
        if let Some(value) = values.get(key) {
            *target = value.clone();
        }
    }
    if let Some(value) = values.get("OSTOOL_QEMU_TAP_POOL") {
        config.virtual_qemu.tap_pool = value.split(',').map(|s| s.trim().to_owned()).collect();
    }
    parsed(
        values,
        "OSTOOL_QEMU_MEMORY_MIB",
        &mut config.virtual_qemu.memory_mib,
    )?;
    parsed(values, "OSTOOL_QEMU_CPUS", &mut config.virtual_qemu.cpus)?;

    apply_security_values(&mut config.security, values)?;
    for (key, target) in [
        ("OSTOOL_PUBLIC_BASE_URL", &mut config.auth.public_base_url),
        (
            "OSTOOL_ADMIN_USERNAME",
            &mut config.auth.bootstrap_admin_username,
        ),
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
    if let Some(value) = values.get("OSTOOL_ALLOWED_ORIGINS") {
        config.auth.allowed_origins = value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
    }
    parsed(values, "OSTOOL_EDITION", &mut config.edition)?;
    if let Some(value) = values.get("OSTOOL_TRUSTED_PROXY_IPS") {
        config.audit.trusted_proxies = value
            .split(',')
            .filter(|ip| !ip.trim().is_empty())
            .map(|ip| ip.trim().parse())
            .collect::<Result<_, _>>()
            .context("invalid OSTOOL_TRUSTED_PROXY_IPS")?;
    }
    parsed(values, "OSTOOL_LISTEN_ADDR", &mut config.listen_addr)?;
    let expected_auth = config.edition.is_public();
    config.auth.enabled = expected_auth;
    parsed(
        values,
        "OSTOOL_NETWORK_TEST_ENABLED",
        &mut config.network_test.enabled,
    )?;
    parsed(
        values,
        "OSTOOL_NETWORK_TEST_LISTEN_ADDR",
        &mut config.network_test.listen_addr,
    )?;
    parsed(
        values,
        "OSTOOL_NETWORK_TEST_MAX_ACTIVE",
        &mut config.network_test.max_active_tests,
    )?;
    parsed(
        values,
        "OSTOOL_NETWORK_TEST_MAX_DURATION_SECS",
        &mut config.network_test.max_duration_secs,
    )?;

    config.security.validate_deployment(config.auth.enabled)?;
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
    ] {
        if let Some(value) = values.get(key) {
            anyhow::ensure!(!value.trim().is_empty(), "{key} must not be empty");
            *target = Some(value.clone());
        }
    }
    if let Some(value) = values.get("OSTOOL_REDIS_PREFIX") {
        config.redis_prefix = value.clone();
    }
    Ok(())
}
fn reject_runtime_settings(values: &BTreeMap<String, String>) -> anyhow::Result<()> {
    for key in values.keys() {
        if key.starts_with("OSTOOL_TFTP_")
            || key == "OSTOOL_NETWORK_INTERFACE"
            || key.starts_with("OSTOOL_SMTP_")
            || key.starts_with("OSTOOL_TURNSTILE_")
            || key.starts_with("OSTOOL_MAIL_")
            || key.starts_with("OSTOOL_SES_")
            || matches!(
                key.as_str(),
                "OSTOOL_AUTH_ENABLED"
                    | "OSTOOL_CAPTCHA_PROVIDER"
                    | "OSTOOL_ALLOW_REGISTRATION"
                    | "OSTOOL_DEV_REGISTRATION_CODES"
                    | "OSTOOL_REQUIRE_ADMIN_MFA"
                    | "OSTOOL_REGISTRATION_EMAIL_VERIFICATION"
                    | "OSTOOL_LOGIN_MAX_FAILURES"
                    | "OSTOOL_LOGIN_LOCK_SECS"
                    | "OSTOOL_ACCESS_TOKEN_TTL_SECS"
                    | "OSTOOL_REFRESH_TOKEN_TTL_SECS"
                    | "OSTOOL_DEVICE_CODE_TTL_SECS"
                    | "OSTOOL_ADMIN_ALLOWED_NETWORKS"
                    | "OSTOOL_AUDIT_RETENTION_DAYS"
            )
        {
            bail!("{key} 已移至管理后台系统设置，请从启动环境中移除并在后台配置");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_business_settings_are_rejected_without_disclosing_values() {
        for key in [
            "OSTOOL_SMTP_PASSWORD",
            "OSTOOL_CAPTCHA_PROVIDER",
            "OSTOOL_MAIL_PROVIDER",
            "OSTOOL_DEV_REGISTRATION_CODES",
            "OSTOOL_ADMIN_ALLOWED_NETWORKS",
            "OSTOOL_AUDIT_RETENTION_DAYS",
        ] {
            let error = apply_values(
                &mut ServerConfig::default(),
                &BTreeMap::from([(key.into(), "sensitive-value".into())]),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(key));
            assert!(!error.contains("sensitive-value"));
        }
    }
    #[test]
    fn env_boot_settings_cover_hardware_without_enabling_it_implicitly() {
        let mut config = ServerConfig::default();
        config.network.interface = "lo".into();
        apply_values(
            &mut config,
            &BTreeMap::from([
                ("OSTOOL_DATA_DIR".into(), "/tmp/env-only-fixture".into()),
                ("OSTOOL_LISTEN_ADDR".into(), "127.0.0.1:2999".into()),
                ("OSTOOL_LOADER_NETWORK_ENABLED".into(), "false".into()),
                ("OSTOOL_HTTP_BOOT_ENABLED".into(), "false".into()),
                ("OSTOOL_NETWORK_TEST_ENABLED".into(), "false".into()),
                ("OSTOOL_QEMU_MEMORY_MIB".into(), "1024".into()),
                ("OSTOOL_QEMU_TAP_POOL".into(), "tap-one,tap-two".into()),
            ]),
        )
        .unwrap();
        assert_eq!(
            config.http_boot.root_dir,
            Path::new("/tmp/env-only-fixture/http-boot")
        );
        assert!(!config.loader_network.enabled);
        assert!(!config.http_boot.enabled);
        assert_eq!(config.virtual_qemu.memory_mib, 1024);
        assert_eq!(config.virtual_qemu.tap_pool, vec!["tap-one", "tap-two"]);
    }

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
