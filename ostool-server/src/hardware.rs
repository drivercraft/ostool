//! The device-network listener exposes no user, admin or board-control API.
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

/// Reuse the configured device interface/port. An explicit address supports a routed device LAN.
pub fn listen_address(config: &ServerConfig) -> anyhow::Result<Option<SocketAddr>> {
    if !config.edition.is_public() {
        return Ok(None);
    }
    if let Some(address) = config.hardware_listen_addr {
        return Ok(Some(address));
    }
    let interface = NetworkInterface::show()?
        .into_iter()
        .find(|i| i.name == config.network.interface);
    Ok(interface.and_then(|i| {
        i.addr.into_iter().find_map(|a| match a {
            Addr::V4(v) if v.ip.is_private() => {
                Some(SocketAddr::new(v.ip.into(), config.listen_addr.port()))
            }
            _ => None,
        })
    }))
}
pub fn restrict_tftp(config: &ServerConfig, tftp: &mut crate::TftpConfig) -> anyhow::Result<()> {
    if config.edition.is_public()
        && let crate::TftpConfig::Builtin(tftp) = tftp
        && tftp.enabled
    {
        if tftp.bind_addr.ip().is_unspecified() {
            let address = listen_address(config)?
                .ok_or_else(|| anyhow::anyhow!("启用公网版 TFTP 前请配置设备网络接口"))?;
            tftp.bind_addr.set_ip(address.ip());
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
    let address = config.hardware_listen_addr.map(|a| a.ip());
    let mut networks = vec!["127.0.0.1/32".to_string(), "::1/128".to_string()];
    for interface in NetworkInterface::show()? {
        for a in interface.addr {
            match a {
                Addr::V4(v) if address == Some(IpAddr::V4(v.ip)) && v.ip.is_private() => {
                    if let Some(network) = v
                        .netmask
                        .and_then(|mask| ipnet::Ipv4Net::with_netmask(v.ip, mask).ok())
                    {
                        networks.push(network.trunc().to_string());
                    }
                }
                Addr::V6(v) if address == Some(IpAddr::V6(v.ip)) && v.ip.is_unique_local() => {
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

pub fn build_router(state: AppState) -> Router {
    protect(
        crate::api::router::hardware_routes().with_state(state.clone()),
        state,
    )
}
pub fn protect(router: Router, state: AppState) -> Router {
    router.layer(axum::middleware::from_fn_with_state(state, admit))
}
async fn admit(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    let config = state.config.read().await;
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|p| p.0.ip());
    let allowed = peer.is_some_and(|ip| allowed(&config, ip));
    drop(config);
    if !allowed {
        return ApiError::forbidden("设备入口仅允许配置的设备网段访问").into_response();
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
