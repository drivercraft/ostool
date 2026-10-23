//! Network boundary for server-owned device discovery, TFTP and throughput tests.
use crate::{AppState, ServerConfig, api::error::ApiError};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use std::net::{IpAddr, SocketAddr};

/// Device downloads use the selected private device interface, never the user origin.
pub fn download_address(config: &ServerConfig) -> anyhow::Result<Option<SocketAddr>> {
    let Some(ip) = crate::tftp::status::resolve_interface_ipv4(&config.network.interface)? else {
        return Ok(None);
    };
    let ip: IpAddr = ip.parse()?;
    Ok((private_address(ip) && ip != config.listen_addr.ip())
        .then_some(SocketAddr::new(ip, config.listen_addr.port())))
}
pub fn download_base_url(config: &ServerConfig) -> anyhow::Result<url::Url> {
    let address = if config.edition.is_public() {
        download_address(config)?
            .ok_or_else(|| anyhow::anyhow!("device download interface unavailable"))?
    } else {
        let ip = crate::tftp::status::resolve_interface_ipv4(&config.network.interface)?
            .ok_or_else(|| anyhow::anyhow!("device network unavailable"))?;
        SocketAddr::new(ip.parse()?, config.listen_addr.port())
    };
    Ok(url::Url::parse(&format!("http://{address}/"))?)
}
pub fn download_router(state: AppState) -> Router {
    protect(
        crate::api::router::device_download_routes().with_state(state.clone()),
        state,
    )
}

pub fn restrict_tftp(config: &ServerConfig, tftp: &mut crate::TftpConfig) -> anyhow::Result<()> {
    if config.edition.is_public()
        && let crate::TftpConfig::Builtin(tftp) = tftp
        && tftp.enabled
    {
        if tftp.bind_addr.ip().is_unspecified() {
            let address = crate::tftp::status::resolve_interface_ipv4(&config.network.interface)?
                .ok_or_else(|| anyhow::anyhow!("启用 TFTP 前请配置设备网络接口"))?;
            tftp.bind_addr.set_ip(address.parse()?);
        }
        anyhow::ensure!(
            private_address(tftp.bind_addr.ip()),
            "TFTP 只能绑定私有设备网或回环地址"
        );
    }
    Ok(())
}
pub fn private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
    }
}

pub fn resolve_networks(config: &mut ServerConfig) -> anyhow::Result<()> {
    if !config.security.hardware_allowed_networks.is_empty() {
        return Ok(());
    }
    let mut networks = vec!["127.0.0.1/32".to_string(), "::1/128".to_string()];
    for interface in NetworkInterface::show()? {
        if interface.name != config.network.interface {
            continue;
        }
        for a in interface.addr {
            match a {
                Addr::V4(v) if v.ip.is_private() => {
                    if let Some(network) = v
                        .netmask
                        .and_then(|mask| ipnet::Ipv4Net::with_netmask(v.ip, mask).ok())
                    {
                        networks.push(network.trunc().to_string());
                    }
                }
                Addr::V6(v) if v.ip.is_unique_local() => {
                    if let Some(network) = v
                        .netmask
                        .and_then(|mask| ipnet::Ipv6Net::with_netmask(v.ip, mask).ok())
                    {
                        networks.push(network.trunc().to_string());
                    }
                }
                _ => {}
            }
        }
    }
    config.security.hardware_allowed_networks = networks;
    Ok(())
}
pub fn allowed(config: &ServerConfig, ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    config
        .security
        .hardware_allowed_networks
        .iter()
        .filter_map(|n| n.parse::<ipnet::IpNet>().ok())
        .any(|n| n.contains(&ip))
}

pub fn protect(router: Router, state: AppState) -> Router {
    router.layer(axum::middleware::from_fn_with_state(state, admit))
}
async fn admit(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    let config = state.config.read().await;
    let permitted = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| allowed(&config, peer.0.ip()));
    drop(config);
    if !permitted {
        return ApiError::forbidden("设备接口来源不受信任").into_response();
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "content-security-policy",
        "sandbox; default-src 'none'".parse().unwrap(),
    );
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_source_ranges_cannot_include_internet_addresses() {
        for (network, valid) in [
            ("192.168.50.0/24", true),
            ("127.0.0.1/32", true),
            ("fd00::/8", true),
            ("::1/128", true),
            ("0.0.0.0/0", false),
            ("::/0", false),
            ("8.8.8.0/24", false),
        ] {
            let config = crate::security::config::SecurityConfig {
                hardware_allowed_networks: vec![network.into()],
                ..Default::default()
            };
            assert_eq!(config.validate_deployment(true).is_ok(), valid, "{network}");
        }
    }
}
