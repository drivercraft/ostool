use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use httpboot_protocol::{
    LEGACY_PROTOCOL_VERSION, LoaderDiscoveryOffer, LoaderDiscoveryProbe, LoaderHardwareInfo,
    LoaderPollRequest, LoaderStatusReport, MAX_DISCOVERY_DATAGRAM_BYTES, MacAddress,
    PROTOCOL_VERSION,
};
use serde::Serialize;
use tokio::{net::UdpSocket, sync::Mutex, task::JoinHandle};

use crate::state::AppState;

const REGISTRATION_TTL: Duration = Duration::seconds(30);
const ONLINE_TTL: Duration = Duration::seconds(10);
const RECORD_RETENTION: Duration = Duration::hours(24);

#[derive(Debug, Clone, Serialize)]
pub struct LoaderDeviceSnapshot {
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    pub ip_address: String,
    pub arch: httpboot_protocol::BootArch,
    pub loader_version: String,
    pub hardware: LoaderHardwareInfo,
    pub last_seen_at: DateTime<Utc>,
    pub online: bool,
    pub conflict: bool,
    pub current_registration_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Registration {
    mac_address: MacAddress,
    protocol_version: u16,
    issued_at: DateTime<Utc>,
    last_seen_at: Option<DateTime<Utc>>,
    superseded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct DeviceRecord {
    current_mac_address: MacAddress,
    ip_address: String,
    arch: httpboot_protocol::BootArch,
    loader_version: String,
    hardware: LoaderHardwareInfo,
    last_seen_at: DateTime<Utc>,
    current_registration_id: String,
    conflict: bool,
}

#[derive(Debug, Default)]
struct LoaderRegistryState {
    registrations: BTreeMap<String, Registration>,
    devices: BTreeMap<MacAddress, DeviceRecord>,
}

#[derive(Debug, Clone)]
pub struct LoaderRegistry {
    events: crate::admin_events::AdminEvents,
    deadline_changed: Arc<tokio::sync::Notify>,
    server_id: Arc<str>,
    state: Arc<Mutex<LoaderRegistryState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationError {
    ProtocolVersion,
    Unknown,
    Expired,
    MacMismatch,
    Replaced,
    Conflict,
}

impl LoaderRegistry {
    pub fn new() -> Self {
        Self {
            events: crate::admin_events::AdminEvents::default(),
            deadline_changed: Arc::new(tokio::sync::Notify::new()),
            server_id: uuid::Uuid::new_v4().to_string().into(),
            state: Arc::new(Mutex::new(LoaderRegistryState::default())),
        }
    }

    pub(crate) fn with_events(events: crate::admin_events::AdminEvents) -> Self {
        Self {
            events,
            ..Self::new()
        }
    }
    pub(crate) fn start_deadlines(&self) {
        let registry = self.clone();
        tokio::spawn(async move {
            loop {
                let now = Utc::now();
                let next = {
                    let state = registry.state.lock().await;
                    state
                        .devices
                        .values()
                        .flat_map(|d| {
                            [
                                d.last_seen_at + ONLINE_TTL,
                                d.last_seen_at + RECORD_RETENTION,
                            ]
                        })
                        .chain(state.registrations.values().flat_map(|r| {
                            [
                                r.issued_at + REGISTRATION_TTL,
                                r.last_seen_at.unwrap_or(r.issued_at) + ONLINE_TTL,
                            ]
                        }))
                        .filter(|deadline| *deadline >= now)
                        .min()
                };
                match next {
                    Some(deadline) => {
                        let wait = (deadline - now).to_std().unwrap_or_default()
                            + std::time::Duration::from_millis(1);
                        tokio::select! {
                            _ = registry.deadline_changed.notified() => {},
                            _ = tokio::time::sleep(wait) => {
                                registry.events.invalidate(&["loaders"]);
                            }
                        }
                    }
                    None => registry.deadline_changed.notified().await,
                }
            }
        });
    }
    pub async fn accept_poll(
        &self,
        request: &LoaderPollRequest,
    ) -> Result<bool, RegistrationError> {
        let result = self.accept_poll_inner(request).await;
        self.events.invalidate(&["loaders"]);
        self.deadline_changed.notify_one();
        result
    }
    pub async fn accept_status(
        &self,
        report: &LoaderStatusReport,
    ) -> Result<(), RegistrationError> {
        let result = self.accept_status_inner(report).await;
        self.events.invalidate(&["loaders"]);
        self.deadline_changed.notify_one();
        result
    }
    pub async fn offer(
        &self,
        probe: &LoaderDiscoveryProbe,
        control_base_url: String,
    ) -> Result<LoaderDiscoveryOffer, RegistrationError> {
        if !matches!(
            probe.protocol_version,
            LEGACY_PROTOCOL_VERSION | PROTOCOL_VERSION
        ) {
            return Err(RegistrationError::ProtocolVersion);
        }
        let now = Utc::now();
        let registration_id = uuid::Uuid::new_v4().to_string();
        let mut state = self.state.lock().await;
        prune(&mut state, now);
        state.registrations.insert(
            registration_id.clone(),
            Registration {
                mac_address: probe.mac_address,
                protocol_version: probe.protocol_version,
                issued_at: now,
                last_seen_at: None,
                superseded_at: None,
            },
        );
        Ok(LoaderDiscoveryOffer {
            protocol_version: probe.protocol_version,
            server_id: self.server_id.to_string(),
            control_base_url,
            registration_id,
            expires_in_ms: REGISTRATION_TTL.num_milliseconds() as u64,
        })
    }

    async fn accept_poll_inner(
        &self,
        request: &LoaderPollRequest,
    ) -> Result<bool, RegistrationError> {
        if !matches!(
            request.protocol_version,
            LEGACY_PROTOCOL_VERSION | PROTOCOL_VERSION
        ) {
            return Err(RegistrationError::ProtocolVersion);
        }
        let now = Utc::now();
        let mut state = self.state.lock().await;
        prune(&mut state, now);

        let registration = state
            .registrations
            .get(&request.registration_id)
            .ok_or(RegistrationError::Unknown)?;
        if registration.mac_address != request.mac_address {
            return Err(RegistrationError::MacMismatch);
        }
        if registration.protocol_version != request.protocol_version {
            return Err(RegistrationError::ProtocolVersion);
        }
        if registration.last_seen_at.is_none() && now - registration.issued_at > REGISTRATION_TTL {
            return Err(RegistrationError::Expired);
        }
        let issued_at = registration.issued_at;
        let was_superseded = registration.superseded_at.is_some();
        state
            .registrations
            .get_mut(&request.registration_id)
            .expect("registration was validated above")
            .last_seen_at = Some(now);

        if was_superseded {
            if let Some(device) = state.devices.get_mut(&request.mac_address) {
                device.conflict = true;
            }
            return Ok(true);
        }

        if let Some(previous_id) = state
            .devices
            .get(&request.mac_address)
            .map(|device| device.current_registration_id.clone())
            .filter(|previous_id| previous_id != &request.registration_id)
        {
            let previous_issued_at = state
                .registrations
                .get(&previous_id)
                .map(|registration| registration.issued_at)
                .unwrap_or(DateTime::<Utc>::MIN_UTC);
            if issued_at > previous_issued_at {
                if let Some(previous) = state.registrations.get_mut(&previous_id) {
                    previous.superseded_at = Some(now);
                }
            } else {
                state
                    .registrations
                    .get_mut(&request.registration_id)
                    .expect("registration was validated above")
                    .superseded_at = Some(now);
                if let Some(device) = state.devices.get_mut(&request.mac_address) {
                    device.conflict = true;
                }
                return Ok(true);
            }
        }

        let conflict = has_live_superseded_report(&state, request.mac_address, now);

        state.devices.insert(
            request.mac_address,
            DeviceRecord {
                current_mac_address: request.current_mac_address,
                ip_address: request.ip_address.clone(),
                arch: request.arch,
                loader_version: request.loader_version.clone(),
                hardware: request.hardware.clone(),
                last_seen_at: now,
                current_registration_id: request.registration_id.clone(),
                conflict,
            },
        );
        Ok(conflict)
    }

    async fn accept_status_inner(
        &self,
        report: &LoaderStatusReport,
    ) -> Result<(), RegistrationError> {
        if !matches!(
            report.protocol_version,
            LEGACY_PROTOCOL_VERSION | PROTOCOL_VERSION
        ) {
            return Err(RegistrationError::ProtocolVersion);
        }
        let now = Utc::now();
        let mut state = self.state.lock().await;
        prune(&mut state, now);
        let registration = state
            .registrations
            .get(&report.registration_id)
            .ok_or(RegistrationError::Unknown)?;
        if registration.mac_address != report.mac_address {
            return Err(RegistrationError::MacMismatch);
        }
        if registration.protocol_version != report.protocol_version {
            return Err(RegistrationError::ProtocolVersion);
        }
        if registration.superseded_at.is_some() {
            state
                .registrations
                .get_mut(&report.registration_id)
                .expect("registration was validated above")
                .last_seen_at = Some(now);
            if let Some(device) = state.devices.get_mut(&report.mac_address) {
                device.conflict = true;
            }
            return Err(RegistrationError::Replaced);
        }
        let (conflict, current_registration_id) = state
            .devices
            .get(&report.mac_address)
            .map(|device| (device.conflict, device.current_registration_id.clone()))
            .ok_or(RegistrationError::Unknown)?;
        if conflict {
            return Err(RegistrationError::Conflict);
        }
        if current_registration_id != report.registration_id {
            return Err(RegistrationError::Replaced);
        }
        state
            .registrations
            .get_mut(&report.registration_id)
            .expect("registration was validated above")
            .last_seen_at = Some(now);
        state
            .devices
            .get_mut(&report.mac_address)
            .expect("device was validated above")
            .last_seen_at = now;
        Ok(())
    }

    pub async fn snapshots(&self) -> Vec<LoaderDeviceSnapshot> {
        let now = Utc::now();
        let mut state = self.state.lock().await;
        prune(&mut state, now);
        state
            .devices
            .iter()
            .map(|(mac_address, record)| LoaderDeviceSnapshot {
                mac_address: *mac_address,
                current_mac_address: record.current_mac_address,
                ip_address: record.ip_address.clone(),
                arch: record.arch,
                loader_version: record.loader_version.clone(),
                hardware: record.hardware.clone(),
                last_seen_at: record.last_seen_at,
                online: now - record.last_seen_at <= ONLINE_TTL,
                conflict: record.conflict,
                current_registration_id: Some(record.current_registration_id.clone()),
            })
            .collect()
    }
}

impl Default for LoaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn prune(state: &mut LoaderRegistryState, now: DateTime<Utc>) {
    state.registrations.retain(|_, registration| {
        registration.last_seen_at.map_or(
            now - registration.issued_at <= REGISTRATION_TTL,
            |last_seen| now - last_seen <= RECORD_RETENTION,
        )
    });
    state
        .devices
        .retain(|_, device| now - device.last_seen_at <= RECORD_RETENTION);

    let conflicts = state
        .devices
        .keys()
        .copied()
        .map(|mac| (mac, has_live_superseded_report(state, mac, now)))
        .collect::<BTreeMap<_, _>>();
    for (mac, device) in &mut state.devices {
        device.conflict = conflicts.get(mac).copied().unwrap_or(false);
    }
}

fn has_live_superseded_report(
    state: &LoaderRegistryState,
    mac: MacAddress,
    now: DateTime<Utc>,
) -> bool {
    state.registrations.values().any(|registration| {
        registration.mac_address == mac
            && registration.superseded_at.is_some_and(|superseded_at| {
                registration.last_seen_at.is_some_and(|last_seen| {
                    last_seen > superseded_at && now - last_seen <= ONLINE_TTL
                })
            })
    })
}

pub async fn start_udp_discovery(state: AppState) -> anyhow::Result<Option<JoinHandle<()>>> {
    let server_config = state.config.read().await.clone();
    if !server_config.loader_network.enabled {
        return Ok(None);
    }
    let socket = UdpSocket::bind(server_config.loader_network.bind_addr)
        .await
        .with_context(|| {
            format!(
                "failed to bind loader discovery UDP {}",
                server_config.loader_network.bind_addr
            )
        })?;
    socket.set_broadcast(true)?;
    Ok(Some(tokio::spawn(async move {
        if let Err(error) = serve_udp_discovery(state, server_config, socket).await {
            log::error!("loader discovery stopped: {error:#}");
        }
    })))
}

async fn serve_udp_discovery(
    state: AppState,
    server_config: crate::config::ServerConfig,
    socket: UdpSocket,
) -> anyhow::Result<()> {
    let mut buffer = [0_u8; MAX_DISCOVERY_DATAGRAM_BYTES + 1];
    loop {
        let (length, peer) = socket.recv_from(&mut buffer).await?;
        if length > MAX_DISCOVERY_DATAGRAM_BYTES {
            continue;
        }
        let Ok(probe) = serde_json::from_slice::<LoaderDiscoveryProbe>(&buffer[..length]) else {
            continue;
        };
        let base_url = advertised_base_url(&server_config)?;
        let Ok(offer) = state.loader_registry.offer(&probe, base_url).await else {
            continue;
        };
        let response = serde_json::to_vec(&offer)?;
        if response.len() <= MAX_DISCOVERY_DATAGRAM_BYTES {
            socket.send_to(&response, peer).await?;
        }
    }
}

fn advertised_base_url(config: &crate::config::ServerConfig) -> anyhow::Result<String> {
    if let Some(public_base_url) = config.loader_network.public_base_url.as_deref() {
        return Ok(public_base_url.trim_end_matches('/').to_string());
    }
    crate::api::router::http_boot_public_base_url(config)
        .map(|url| url.as_str().trim_end_matches('/').to_string())
        .map_err(|error| anyhow::anyhow!(error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpboot_protocol::{BootArch, LoaderStatusPhase, PROTOCOL_VERSION};

    fn probe(mac: MacAddress) -> LoaderDiscoveryProbe {
        LoaderDiscoveryProbe {
            protocol_version: PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: BootArch::X86_64,
            loader_version: "test".into(),
        }
    }

    fn poll(mac: MacAddress, registration_id: String) -> LoaderPollRequest {
        LoaderPollRequest {
            protocol_version: PROTOCOL_VERSION,
            registration_id,
            mac_address: mac,
            current_mac_address: mac,
            ip_address: "10.77.0.2".into(),
            arch: BootArch::X86_64,
            loader_version: "test".into(),
            hardware: LoaderHardwareInfo::default(),
        }
    }

    #[tokio::test]
    async fn a_new_generation_replaces_the_previous_one_until_it_reports_again() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let first = registry
            .offer(&probe(mac), "http://host".into())
            .await
            .unwrap();
        assert!(
            !registry
                .accept_poll(&poll(mac, first.registration_id.clone()))
                .await
                .unwrap()
        );
        let second = registry
            .offer(&probe(mac), "http://host".into())
            .await
            .unwrap();
        assert!(
            !registry
                .accept_poll(&poll(mac, second.registration_id.clone()))
                .await
                .unwrap()
        );
        assert!(!registry.snapshots().await[0].conflict);
        assert!(
            registry
                .accept_poll(&poll(mac, first.registration_id))
                .await
                .unwrap()
        );
        assert!(registry.snapshots().await[0].conflict);
    }

    #[tokio::test]
    async fn registration_cannot_claim_another_mac() {
        let registry = LoaderRegistry::new();
        let first_mac = "02:00:00:00:00:01".parse().unwrap();
        let second_mac = "02:00:00:00:00:02".parse().unwrap();
        let offer = registry
            .offer(&probe(first_mac), "http://host".into())
            .await
            .unwrap();
        assert_eq!(
            registry
                .accept_poll(&poll(second_mac, offer.registration_id))
                .await,
            Err(RegistrationError::MacMismatch)
        );
    }

    #[tokio::test]
    async fn rejects_an_incompatible_protocol_version() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let mut incompatible = probe(mac);
        incompatible.protocol_version = PROTOCOL_VERSION + 1;
        assert_eq!(
            registry.offer(&incompatible, "http://host".into()).await,
            Err(RegistrationError::ProtocolVersion)
        );
    }

    #[tokio::test]
    async fn status_reports_refresh_the_current_device_heartbeat() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let offer = registry
            .offer(&probe(mac), "http://host".into())
            .await
            .unwrap();
        registry
            .accept_poll(&poll(mac, offer.registration_id.clone()))
            .await
            .unwrap();
        registry
            .state
            .lock()
            .await
            .devices
            .get_mut(&mac)
            .unwrap()
            .last_seen_at = Utc::now() - ONLINE_TTL - Duration::seconds(1);

        registry
            .accept_status(&LoaderStatusReport {
                protocol_version: PROTOCOL_VERSION,
                registration_id: offer.registration_id,
                mac_address: mac,
                session_id: "session".into(),
                boot_id: "boot".into(),
                status: LoaderStatusPhase::Downloading {
                    received: 1,
                    total: 2,
                },
            })
            .await
            .unwrap();

        assert!(registry.snapshots().await[0].online);
    }
}
