use std::{collections::BTreeMap, net::Ipv4Addr, sync::Arc};

use anyhow::{Context, bail};
use tokio::net::UdpSocket;

use crate::{
    config::{BoardConfig, BootConfig, ServerConfig, UefiBootArch},
    state::AppState,
};

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const DHCP_OPTION_PAD: u8 = 0;
const DHCP_OPTION_SUBNET_MASK: u8 = 1;
const DHCP_OPTION_ROUTER: u8 = 3;
const DHCP_OPTION_DNS: u8 = 6;
const DHCP_OPTION_REQUESTED_IP: u8 = 50;
const DHCP_OPTION_LEASE_TIME: u8 = 51;
const DHCP_OPTION_MESSAGE_TYPE: u8 = 53;
const DHCP_OPTION_SERVER_ID: u8 = 54;
const DHCP_OPTION_VENDOR_CLASS: u8 = 60;
const DHCP_OPTION_BOOTFILE_NAME: u8 = 67;
const DHCP_OPTION_ARCH: u8 = 93;
const DHCP_OPTION_END: u8 = 255;
const BOOTP_FIXED_HEADER_LEN: usize = 236;
const DHCP_OPTIONS_OFFSET: usize = BOOTP_FIXED_HEADER_LEN + DHCP_MAGIC_COOKIE.len();
const BOOTFILE_OFFSET: usize = 108;
const BOOTFILE_LEN: usize = 128;
const CHADDR_OFFSET: usize = 28;
const CHADDR_LEN: usize = 16;

#[derive(Debug)]
struct DhcpRequest<'a> {
    xid: [u8; 4],
    secs: [u8; 2],
    flags: [u8; 2],
    chaddr: [u8; CHADDR_LEN],
    vendor_class: Option<&'a [u8]>,
    arch: Option<u16>,
    message_type: Option<u8>,
    requested_ip: Option<Ipv4Addr>,
}

pub async fn spawn_proxy_dhcp(
    state: AppState,
) -> anyhow::Result<Option<tokio::task::JoinHandle<()>>> {
    let config = state.config.read().await.clone();
    if !config.proxy_dhcp.enabled {
        return Ok(None);
    }

    let plan = build_boot_plan(&config, &state).await?;
    let socket = Arc::new(
        UdpSocket::bind(config.proxy_dhcp.bind_addr)
            .await
            .with_context(|| {
                format!(
                    "failed to bind ProxyDHCP on {}",
                    config.proxy_dhcp.bind_addr
                )
            })?,
    );
    socket
        .set_broadcast(true)
        .context("failed to enable ProxyDHCP broadcast replies")?;

    log::info!(
        "ProxyDHCP listening on {}, board={}, bootfile={}",
        config.proxy_dhcp.bind_addr,
        plan.board_id,
        plan.boot_url
    );

    Ok(Some(tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, _peer)) => {
                    if let Err(err) = handle_packet(&socket, &buf[..len], &plan).await {
                        log::debug!("ProxyDHCP ignored packet: {err:#}");
                    }
                }
                Err(err) => log::warn!("ProxyDHCP receive failed: {err}"),
            }
        }
    })))
}

async fn handle_packet(socket: &UdpSocket, packet: &[u8], plan: &BootPlan) -> anyhow::Result<()> {
    let request = parse_dhcp_request(packet)?;
    let response_type = match request.message_type {
        Some(DHCP_DISCOVER) => DHCP_OFFER,
        Some(DHCP_REQUEST) => DHCP_ACK,
        _ => bail!("not a DHCP Discover/Request"),
    };
    if !is_http_client(request.vendor_class) {
        bail!("not a UEFI HTTP client");
    }
    if !arch_matches(request.arch, plan.arch.as_ref()) {
        bail!("UEFI HTTP client arch does not match board");
    }
    if let Some(mac) = plan.client_mac.as_deref() {
        let request_mac = format_mac(&request.chaddr[..6]);
        if !mac.eq_ignore_ascii_case(&request_mac) {
            bail!("UEFI HTTP client MAC does not match board");
        }
    }
    if request.message_type == Some(DHCP_REQUEST)
        && let Some(requested_ip) = request.requested_ip
        && requested_ip != plan.client_ip
    {
        bail!("DHCP Request is for a different client IP");
    }

    let response = build_dhcp_response(packet, &request, plan, response_type)?;
    socket
        .send_to(&response, (Ipv4Addr::BROADCAST, DHCP_CLIENT_PORT))
        .await
        .context("failed to send HTTP Boot DHCP response")?;

    log::info!(
        "HTTP Boot DHCP sent {} with {} to {}",
        if response_type == DHCP_ACK {
            "ACK"
        } else {
            "OFFER"
        },
        plan.boot_url,
        format_mac(&request.chaddr[..6])
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct BootPlan {
    board_id: String,
    arch: Option<UefiBootArch>,
    server_ip: Ipv4Addr,
    client_ip: Ipv4Addr,
    client_mac: Option<String>,
    subnet_mask: Ipv4Addr,
    router: Ipv4Addr,
    dns_server: Ipv4Addr,
    lease_time_secs: u32,
    boot_url: String,
}

async fn build_boot_plan(config: &ServerConfig, state: &AppState) -> anyhow::Result<BootPlan> {
    if !config.http_boot.enabled {
        bail!("ProxyDHCP requires http_boot.enabled = true");
    }

    let boards = state.boards.read().await;
    let (board_id, board) = select_board(config, &boards)?;
    let BootConfig::UefiHttp(profile) = &board.boot else {
        bail!("ProxyDHCP board `{board_id}` is not a uefi_http board");
    };

    let loader_file = profile
        .loader_file
        .clone()
        .unwrap_or_else(|| default_loader_file(profile.boot_arch.as_ref()));
    let boot_url = http_boot_url(config, board_id, &loader_file)?;
    let server_ip = proxy_server_ip(config)?;
    let client_ip = profile.client_ip.ok_or_else(|| {
        anyhow::anyhow!("ProxyDHCP requires boot.client_ip for board `{board_id}`")
    })?;
    let router = config.proxy_dhcp.router.unwrap_or(server_ip);
    let dns_server = config.proxy_dhcp.dns_server.unwrap_or(router);

    Ok(BootPlan {
        board_id: board_id.to_string(),
        arch: profile.boot_arch.clone(),
        server_ip,
        client_ip,
        client_mac: profile.mac_address.clone(),
        subnet_mask: config.proxy_dhcp.subnet_mask,
        router,
        dns_server,
        lease_time_secs: config.proxy_dhcp.lease_time_secs,
        boot_url,
    })
}

fn select_board<'a>(
    config: &ServerConfig,
    boards: &'a BTreeMap<String, BoardConfig>,
) -> anyhow::Result<(&'a str, &'a BoardConfig)> {
    if let Some(board_id) = config.proxy_dhcp.board_id.as_deref() {
        let board = boards
            .get(board_id)
            .ok_or_else(|| anyhow::anyhow!("ProxyDHCP board `{board_id}` not found"))?;
        return Ok((board.id.as_str(), board));
    }

    let mut matches = boards
        .iter()
        .filter(|(_, board)| !board.disabled && matches!(board.boot, BootConfig::UefiHttp(_)));
    let Some((board_id, board)) = matches.next() else {
        bail!("ProxyDHCP enabled but no enabled uefi_http board exists");
    };
    if matches.next().is_some() {
        bail!("ProxyDHCP needs proxy_dhcp.board_id when multiple uefi_http boards exist");
    }
    Ok((board_id, board))
}

fn proxy_server_ip(config: &ServerConfig) -> anyhow::Result<Ipv4Addr> {
    if let Some(base) = config.http_boot.public_base_url.as_deref()
        && let Some(rest) = base.strip_prefix("http://")
    {
        let host = rest.split([':', '/']).next().unwrap_or_default();
        if let Ok(ip) = host.parse() {
            return Ok(ip);
        }
    }
    match config.listen_addr.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() => Ok(ip),
        _ => bail!("ProxyDHCP requires http_boot.public_base_url with an IPv4 host"),
    }
}

fn http_boot_url(
    config: &ServerConfig,
    board_id: &str,
    relative_path: &str,
) -> anyhow::Result<String> {
    let base_url = config
        .http_boot
        .public_base_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", config.listen_addr));
    let base_url = base_url.trim_end_matches('/');
    Ok(format!(
        "{base_url}/boot/boards/{board_id}/current/{}",
        relative_path.trim_start_matches('/')
    ))
}

fn default_loader_file(arch: Option<&UefiBootArch>) -> String {
    match arch {
        Some(UefiBootArch::X86_64) => "BOOTX64.EFI",
        Some(UefiBootArch::Aarch64) => "BOOTAA64.EFI",
        Some(UefiBootArch::Loongarch64) => "BOOTLOONGARCH64.EFI",
        Some(UefiBootArch::Riscv64) => "BOOTRISCV64.EFI",
        Some(UefiBootArch::Other) | None => "BOOT.EFI",
    }
    .to_string()
}

fn parse_dhcp_request(packet: &[u8]) -> anyhow::Result<DhcpRequest<'_>> {
    if packet.len() < DHCP_OPTIONS_OFFSET {
        bail!("packet too short");
    }
    if packet[0] != 1 {
        bail!("not a BOOTREQUEST");
    }
    if packet[236..240] != DHCP_MAGIC_COOKIE {
        bail!("missing DHCP magic cookie");
    }

    let mut request = DhcpRequest {
        xid: packet[4..8].try_into().unwrap(),
        secs: packet[8..10].try_into().unwrap(),
        flags: packet[10..12].try_into().unwrap(),
        chaddr: packet[CHADDR_OFFSET..CHADDR_OFFSET + CHADDR_LEN]
            .try_into()
            .unwrap(),
        vendor_class: None,
        arch: None,
        message_type: None,
        requested_ip: None,
    };

    for option in DhcpOptions::new(&packet[DHCP_OPTIONS_OFFSET..]) {
        let (code, value) = option?;
        match code {
            DHCP_OPTION_MESSAGE_TYPE if value.len() == 1 => request.message_type = Some(value[0]),
            DHCP_OPTION_VENDOR_CLASS => request.vendor_class = Some(value),
            DHCP_OPTION_REQUESTED_IP if value.len() == 4 => {
                request.requested_ip = Some(Ipv4Addr::new(value[0], value[1], value[2], value[3]));
            }
            DHCP_OPTION_ARCH if value.len() >= 2 => {
                request.arch = Some(u16::from_be_bytes([value[0], value[1]]));
            }
            _ => {}
        }
    }

    Ok(request)
}

struct DhcpOptions<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> DhcpOptions<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl<'a> Iterator for DhcpOptions<'a> {
    type Item = anyhow::Result<(u8, &'a [u8])>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.offset < self.bytes.len() {
            let code = self.bytes[self.offset];
            self.offset += 1;
            if code == DHCP_OPTION_PAD {
                continue;
            }
            if code == DHCP_OPTION_END {
                return None;
            }
            if self.offset >= self.bytes.len() {
                return Some(Err(anyhow::anyhow!("truncated DHCP option length")));
            }
            let len = self.bytes[self.offset] as usize;
            self.offset += 1;
            if self.offset + len > self.bytes.len() {
                return Some(Err(anyhow::anyhow!("truncated DHCP option value")));
            }
            let value = &self.bytes[self.offset..self.offset + len];
            self.offset += len;
            return Some(Ok((code, value)));
        }
        None
    }
}

fn is_http_client(vendor_class: Option<&[u8]>) -> bool {
    vendor_class
        .and_then(|bytes| core::str::from_utf8(bytes).ok())
        .is_some_and(|value| value.starts_with("HTTPClient:"))
}

fn arch_matches(request_arch: Option<u16>, board_arch: Option<&UefiBootArch>) -> bool {
    match (request_arch, board_arch) {
        (Some(40), Some(UefiBootArch::Loongarch64)) => true,
        (Some(16), Some(UefiBootArch::X86_64)) => true,
        (Some(11), Some(UefiBootArch::Aarch64)) => true,
        (_, None | Some(UefiBootArch::Other)) => true,
        _ => false,
    }
}

fn build_dhcp_response(
    request_packet: &[u8],
    request: &DhcpRequest<'_>,
    plan: &BootPlan,
    response_type: u8,
) -> anyhow::Result<Vec<u8>> {
    if plan.boot_url.len() > BOOTFILE_LEN {
        bail!("ProxyDHCP boot URL is too long for BOOTP file field");
    }

    let mut response = vec![0u8; DHCP_OPTIONS_OFFSET];
    response[0] = 2;
    response[1] = request_packet[1];
    response[2] = request_packet[2];
    response[3] = request_packet[3];
    response[4..8].copy_from_slice(&request.xid);
    response[8..10].copy_from_slice(&request.secs);
    response[10..12].copy_from_slice(&request.flags);
    response[16..20].copy_from_slice(&plan.client_ip.octets());
    response[20..24].copy_from_slice(&plan.server_ip.octets());
    response[CHADDR_OFFSET..CHADDR_OFFSET + CHADDR_LEN].copy_from_slice(&request.chaddr);
    response[BOOTFILE_OFFSET..BOOTFILE_OFFSET + plan.boot_url.len()]
        .copy_from_slice(plan.boot_url.as_bytes());
    response[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE);

    push_option(&mut response, DHCP_OPTION_MESSAGE_TYPE, &[response_type])?;
    if let Some(vendor_class) = request.vendor_class {
        push_option(&mut response, DHCP_OPTION_VENDOR_CLASS, vendor_class)?;
    }
    if let Some(arch) = request.arch {
        push_option(&mut response, DHCP_OPTION_ARCH, &arch.to_be_bytes())?;
    }
    push_option(
        &mut response,
        DHCP_OPTION_SERVER_ID,
        &plan.server_ip.octets(),
    )?;
    push_option(
        &mut response,
        DHCP_OPTION_LEASE_TIME,
        &plan.lease_time_secs.to_be_bytes(),
    )?;
    push_option(
        &mut response,
        DHCP_OPTION_BOOTFILE_NAME,
        plan.boot_url.as_bytes(),
    )?;
    push_option(
        &mut response,
        DHCP_OPTION_SUBNET_MASK,
        &plan.subnet_mask.octets(),
    )?;
    push_option(&mut response, DHCP_OPTION_ROUTER, &plan.router.octets())?;
    push_option(&mut response, DHCP_OPTION_DNS, &plan.dns_server.octets())?;
    response.push(DHCP_OPTION_END);
    Ok(response)
}

fn push_option(response: &mut Vec<u8>, code: u8, value: &[u8]) -> anyhow::Result<()> {
    if value.len() > u8::MAX as usize {
        bail!("DHCP option {code} value too long");
    }
    response.push(code);
    response.push(value.len() as u8);
    response.extend_from_slice(value);
    Ok(())
}

fn format_mac(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}
