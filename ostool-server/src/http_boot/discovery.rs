use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket as StdUdpSocket},
    time::Duration,
};

use anyhow::Context;
use httpboot_protocol::{
    DISCOVERY_ADVERTISE_TYPE, DISCOVERY_PROTOCOL_VERSION, DISCOVERY_SOLICIT_TYPE,
    DiscoveryAdvertise, DiscoverySolicit,
};
use tokio::net::UdpSocket;
use uuid::Uuid;

use crate::{AppState, ServerConfig, tftp::status::resolve_interface_ipv4};

const MAX_DISCOVERY_DATAGRAM: usize = 2048;
const MIN_ADVERTISE_INTERVAL: Duration = Duration::from_millis(100);

pub async fn run_discovery_service(state: AppState) -> anyhow::Result<()> {
    let initial_config = state.config.read().await.clone();
    if !initial_config.http_boot.enabled || !initial_config.http_boot.discovery.enabled {
        log::info!("HTTP Boot discovery service disabled");
        return Ok(());
    }

    let udp_port = initial_config.http_boot.discovery.udp_port;
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, udp_port);
    let std_socket = StdUdpSocket::bind(bind_addr)
        .with_context(|| format!("failed to bind HTTP Boot discovery UDP socket {bind_addr}"))?;
    std_socket.set_nonblocking(true)?;
    std_socket.set_broadcast(true)?;
    let socket = UdpSocket::from_std(std_socket)?;
    let server_id = Uuid::new_v4().to_string();
    let broadcast_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, udp_port));
    let mut interval = tokio::time::interval(advertise_interval(&initial_config));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut buf = [0u8; MAX_DISCOVERY_DATAGRAM];

    log::info!("HTTP Boot discovery listening on UDP {bind_addr}");
    loop {
        tokio::select! {
            recv = socket.recv_from(&mut buf) => {
                let (len, peer) = recv?;
                handle_discovery_datagram(&socket, &state, &server_id, &buf[..len], peer).await;
            }
            _ = interval.tick() => {
                send_advertise(&socket, &state, &server_id, broadcast_addr).await;
            }
        }
    }
}

async fn handle_discovery_datagram(
    socket: &UdpSocket,
    state: &AppState,
    server_id: &str,
    datagram: &[u8],
    peer: SocketAddr,
) {
    let Ok(solicit) = serde_json::from_slice::<DiscoverySolicit>(datagram) else {
        return;
    };
    if solicit.r#type != DISCOVERY_SOLICIT_TYPE || solicit.version != DISCOVERY_PROTOCOL_VERSION {
        return;
    }

    log::info!(
        "HTTP Boot discovery solicit from {peer}: arch={:?}, board={:?}, mac={}",
        solicit.arch,
        solicit.board,
        solicit.mac
    );
    send_advertise(socket, state, server_id, peer).await;
}

async fn send_advertise(socket: &UdpSocket, state: &AppState, server_id: &str, peer: SocketAddr) {
    let config = state.config.read().await.clone();
    if !config.http_boot.enabled || !config.http_boot.discovery.enabled {
        return;
    }

    let advertise = match build_discovery_advertise(&config, server_id) {
        Ok(advertise) => advertise,
        Err(err) => {
            log::warn!("failed to build HTTP Boot discovery advertise: {err:#}");
            return;
        }
    };
    let payload = match serde_json::to_vec(&advertise) {
        Ok(payload) => payload,
        Err(err) => {
            log::warn!("failed to encode HTTP Boot discovery advertise: {err}");
            return;
        }
    };
    if let Err(err) = socket.send_to(&payload, peer).await {
        log::debug!("failed to send HTTP Boot discovery advertise to {peer}: {err}");
    }
}

fn build_discovery_advertise(
    config: &ServerConfig,
    server_id: &str,
) -> anyhow::Result<DiscoveryAdvertise> {
    Ok(DiscoveryAdvertise {
        r#type: DISCOVERY_ADVERTISE_TYPE.into(),
        version: DISCOVERY_PROTOCOL_VERSION,
        server_id: server_id.into(),
        base_url: http_boot_public_base_url(config)?,
        discovery_port: config.http_boot.discovery.udp_port,
    })
}

fn advertise_interval(config: &ServerConfig) -> Duration {
    Duration::from_millis(config.http_boot.discovery.advertise_interval_ms)
        .max(MIN_ADVERTISE_INTERVAL)
}

fn http_boot_public_base_url(config: &ServerConfig) -> anyhow::Result<String> {
    if let Some(public_base_url) = config.http_boot.public_base_url.as_deref()
        && !public_base_url.trim().is_empty()
    {
        return Ok(public_base_url.trim().trim_end_matches('/').to_string());
    }

    let interface = config.network.interface.trim();
    if interface == "lo" {
        return Ok(format!("http://127.0.0.1:{}", config.listen_addr.port()));
    }
    if !interface.is_empty()
        && let Some(server_ip) = resolve_interface_ipv4(interface)?
    {
        return Ok(format!(
            "http://{}:{}",
            server_ip,
            config.listen_addr.port()
        ));
    }

    Ok(format!("http://{}", config.listen_addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertise_uses_configured_public_base_url() {
        let mut config = ServerConfig::default();
        config.http_boot.public_base_url = Some("http://10.3.10.192:2999/".into());

        let advertise = build_discovery_advertise(&config, "server-1").unwrap();

        assert_eq!(advertise.r#type, DISCOVERY_ADVERTISE_TYPE);
        assert_eq!(advertise.version, DISCOVERY_PROTOCOL_VERSION);
        assert_eq!(advertise.server_id, "server-1");
        assert_eq!(advertise.base_url, "http://10.3.10.192:2999");
    }

    #[test]
    fn advertise_defaults_to_loopback_interface_ip() {
        let mut config = ServerConfig::default();
        config.network.interface = "lo".into();

        let advertise = build_discovery_advertise(&config, "server-1").unwrap();

        assert_eq!(advertise.base_url, "http://127.0.0.1:2999");
        assert_eq!(advertise.discovery_port, 2998);
    }

    #[test]
    fn advertise_interval_has_minimum() {
        let mut config = ServerConfig::default();
        config.http_boot.discovery.advertise_interval_ms = 1;

        assert_eq!(advertise_interval(&config), MIN_ADVERTISE_INTERVAL);
    }
}
