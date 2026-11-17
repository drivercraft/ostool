//! Device LAN transport is independent of the authenticated public website.
use std::net::IpAddr;

pub fn is_lan_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback() || ip.is_unicast_link_local(),
    }
}

pub fn http_origin(value: &str) -> anyhow::Result<url::Url> {
    let url = url::Url::parse(value)?;
    anyhow::ensure!(
        url.scheme() == "http"
            && url.host_str().is_some_and(|host| {
                host.trim_matches(['[', ']'])
                    .parse()
                    .is_ok_and(is_lan_address)
            })
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "device HTTP origin must use an explicit LAN IP without credentials, path or query"
    );
    Ok(url)
}

pub fn file_base_url(config: &crate::ServerConfig) -> anyhow::Result<url::Url> {
    if config.http_boot.listen_addr.is_some() {
        http_origin(
            config
                .http_boot
                .public_base_url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing device HTTP origin"))?,
        )
    } else {
        config.auth.public_url()
    }
}

/// Only short-lived, immutable file capabilities are served on the device LAN.
/// User/admin APIs, login, pages and serial WebSockets are absent from this router.
pub fn router(state: crate::AppState) -> axum::Router {
    axum::Router::new()
        .route(
            "/device-files/{id}",
            axum::routing::get(crate::device_security::download),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_http_requires_a_specific_lan_origin() {
        for origin in [
            "http://192.168.10.2:3001",
            "http://10.1.0.2:3001",
            "http://[fd00::1]:3001",
            "http://127.0.0.1:3001",
        ] {
            assert!(http_origin(origin).is_ok(), "{origin}");
        }
        for origin in [
            "http://0.0.0.0:3001",
            "http://8.8.8.8",
            "http://example.com",
            "http://192.168.10.2/path",
            "http://user:pass@192.168.10.2",
            "http://192.168.10.2?token=secret",
        ] {
            assert!(http_origin(origin).is_err(), "{origin}");
        }
    }

    #[test]
    fn public_website_and_device_lan_use_independent_origins() {
        let mut config = crate::ServerConfig::default();
        config.edition = crate::config::Edition::Public;
        config.auth.enabled = true;
        config.auth.public_base_url = Some("https://ostool.muxai.net:14321".into());
        config.network.interface = "lo".into();
        config.http_boot.listen_addr = Some("192.168.10.2:3001".parse().unwrap());
        config.http_boot.public_base_url = Some("http://192.168.10.2:3001".into());
        config.validate().unwrap();
        assert_eq!(
            file_base_url(&config).unwrap().as_str(),
            "http://192.168.10.2:3001/"
        );
        assert_eq!(
            config.auth.public_url().unwrap().as_str(),
            "https://ostool.muxai.net:14321/"
        );
        config.http_boot.public_base_url = Some("http://192.168.10.3:3001".into());
        assert!(config.validate().is_err());
    }
}
