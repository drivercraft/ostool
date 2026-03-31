use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};

use crate::api::models::NetworkInterfaceSummary;

pub fn list_network_interfaces() -> anyhow::Result<Vec<NetworkInterfaceSummary>> {
    let mut interfaces = NetworkInterface::show()?
        .into_iter()
        .map(|interface| {
            let primary_ipv4 = interface.addr.iter().find_map(|addr| match addr {
                Addr::V4(v4) => Some(*v4),
                Addr::V6(_) => None,
            });
            let ipv4_addresses = interface
                .addr
                .iter()
                .filter_map(|addr| match addr {
                    Addr::V4(v4) => Some(v4.ip.to_string()),
                    Addr::V6(_) => None,
                })
                .collect::<Vec<_>>();
            let loopback = primary_ipv4.is_some_and(|v4| v4.ip.is_loopback());

            let label = if let Some(primary_ipv4) = primary_ipv4 {
                let netmask = primary_ipv4
                    .netmask
                    .map(|netmask| format!("/{netmask}"))
                    .unwrap_or_default();
                format!("{} ({}{})", interface.name, primary_ipv4.ip, netmask)
            } else {
                interface.name.clone()
            };

            NetworkInterfaceSummary {
                name: interface.name,
                label,
                ipv4_addresses,
                netmask: primary_ipv4
                    .and_then(|v4| v4.netmask)
                    .map(|netmask| netmask.to_string()),
                loopback,
            }
        })
        .collect::<Vec<_>>();

    interfaces.sort_by(|a, b| {
        a.loopback
            .cmp(&b.loopback)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(interfaces)
}

pub fn default_non_loopback_interface_name() -> Option<String> {
    list_network_interfaces()
        .ok()?
        .into_iter()
        .find(|interface| !interface.loopback && !interface.ipv4_addresses.is_empty())
        .map(|interface| interface.name)
}
