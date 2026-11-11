use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use httpboot_protocol::{
    DeviceOperationError, LoaderAnnouncement, LoaderDeviceStatus, LoaderHardwareInfo, MacAddress,
    validate_device_protocol_version,
};
use serde::Serialize;
use tokio::{
    net::UdpSocket,
    sync::{Mutex, Notify},
    task::{JoinHandle, JoinSet},
};

const ONLINE_TTL: Duration = Duration::seconds(10);
const RECORD_RETENTION: Duration = Duration::hours(24);
const MAX_ANNOUNCEMENT_BYTES: usize = 1400;

pub async fn start_udp_discovery(state: crate::AppState) -> anyhow::Result<Option<JoinHandle<()>>> {
    let config = state.config.read().await.clone();
    if !config.loader_network.enabled {
        return Ok(None);
    }
    anyhow::ensure!(!config.edition.is_public(), "UDP discovery is local-only");
    let socket = UdpSocket::bind(config.loader_network.bind_addr)
        .await
        .with_context(|| {
            format!(
                "failed to bind loader discovery UDP {}",
                config.loader_network.bind_addr
            )
        })?;
    Ok(Some(tokio::spawn(async move {
        if let Err(error) = serve_udp_announcements(state, socket).await {
            log::error!("loader discovery stopped: {error:#}");
        }
    })))
}

async fn serve_udp_announcements(state: crate::AppState, socket: UdpSocket) -> anyhow::Result<()> {
    let mut buffer = [0; MAX_ANNOUNCEMENT_BYTES + 1];
    let mut tasks = JoinSet::new();
    let mut running = BTreeMap::<MacAddress, (String, tokio::task::AbortHandle)>::new();
    loop {
        let (length, peer) = socket.recv_from(&mut buffer).await?;
        while tasks.try_join_next().is_some() {}
        running.retain(|_, (_, task)| !task.is_finished());
        if length > MAX_ANNOUNCEMENT_BYTES {
            continue;
        }
        let Ok(announcement) = serde_json::from_slice::<LoaderAnnouncement>(&buffer[..length])
        else {
            continue;
        };
        if validate_device_protocol_version(announcement.protocol_version).is_err()
            || announcement.http_port == 0
        {
            continue;
        }
        state
            .loader_registry
            .touch_announcement(&announcement, &peer.ip().to_string())
            .await;
        let mac = announcement.mac_address;
        if let Some((epoch, task)) = running.get(&mac) {
            if *epoch == announcement.boot_epoch {
                continue;
            }
            task.abort();
        } else if running.len() >= 16 {
            continue;
        }
        let epoch = announcement.boot_epoch.clone();
        let device_state = state.clone();
        let task = tasks.spawn(async move {
            if let Err(error) = crate::device::reconcile(device_state, announcement, peer).await {
                log::warn!("device {mac} at {peer}: {error:#}");
            }
        });
        running.insert(mac, (epoch, task));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoaderDeviceSnapshot {
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    pub ip_address: String,
    pub arch: String,
    pub loader_version: String,
    pub hardware: LoaderHardwareInfo,
    pub last_seen_at: DateTime<Utc>,
    pub online: bool,
    pub conflict: bool,
    pub boot_epoch: String,
}

#[derive(Debug, Clone)]
struct DeviceRecord {
    current_mac_address: MacAddress,
    ip_address: String,
    arch: String,
    loader_version: String,
    hardware: LoaderHardwareInfo,
    last_seen_at: DateTime<Utc>,
    boot_epoch: String,
    conflict: bool,
    conflict_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct LoaderRegistryState {
    devices: BTreeMap<MacAddress, DeviceRecord>,
}

#[derive(Debug, Clone)]
pub struct LoaderRegistry {
    events: crate::admin_events::AdminEvents,
    deadline_changed: Arc<Notify>,
    state: Arc<Mutex<LoaderRegistryState>>,
}

impl LoaderRegistry {
    pub fn new() -> Self {
        Self {
            events: crate::admin_events::AdminEvents::default(),
            deadline_changed: Arc::new(Notify::new()),
            state: Arc::new(Mutex::new(LoaderRegistryState::default())),
        }
    }

    pub(crate) fn with_events(events: crate::admin_events::AdminEvents) -> Self {
        Self {
            events,
            ..Self::new()
        }
    }

    pub async fn accept_announcement(
        &self,
        announcement: &LoaderAnnouncement,
        device: &LoaderDeviceStatus,
        ip_address: String,
    ) -> Result<bool, DeviceOperationError> {
        validate_v7(announcement.protocol_version)?;
        validate_v7(device.protocol_version)?;
        if announcement.http_port == 0 {
            return Err(DeviceOperationError {
                code: "invalid_device_announcement".into(),
                message: "announcement has no device HTTP port".into(),
            });
        }
        if announcement.mac_address != device.mac_address
            || announcement.current_mac_address != device.current_mac_address
        {
            return Err(DeviceOperationError {
                code: "device_identity_mismatch".into(),
                message: "announcement and status report different device MAC identities".into(),
            });
        }
        if announcement.boot_epoch != device.boot_epoch {
            return Err(DeviceOperationError {
                code: "boot_epoch_mismatch".into(),
                message: "announcement and status report different boot epochs".into(),
            });
        }

        let now = Utc::now();
        let mut state = self.state.lock().await;
        prune(&mut state, now);
        if state.devices.len() >= 4096 && !state.devices.contains_key(&device.mac_address) {
            return Err(DeviceOperationError {
                code: "device_capacity".into(),
                message: "device registry is full".into(),
            });
        }
        let source_conflict = state
            .devices
            .get(&device.mac_address)
            .is_some_and(|previous| {
                previous.ip_address != ip_address && now - previous.last_seen_at < ONLINE_TTL
            });
        if source_conflict {
            if let Some(previous) = state.devices.get_mut(&device.mac_address) {
                previous.conflict = true;
                previous.conflict_until = Some(now + ONLINE_TTL);
            }
        } else {
            let conflict_until = state
                .devices
                .get(&device.mac_address)
                .and_then(|previous| previous.conflict_until)
                .filter(|deadline| *deadline > now);
            state.devices.insert(
                device.mac_address,
                DeviceRecord {
                    current_mac_address: device.current_mac_address,
                    ip_address,
                    arch: device.arch.clone(),
                    loader_version: device.loader_version.clone(),
                    hardware: device.hardware.clone(),
                    last_seen_at: now,
                    boot_epoch: device.boot_epoch.clone(),
                    conflict: conflict_until.is_some(),
                    conflict_until,
                },
            );
        }
        let conflict = state
            .devices
            .get(&device.mac_address)
            .is_some_and(|record| record.conflict);
        drop(state);
        self.events.invalidate(&["loaders"]);
        self.deadline_changed.notify_one();
        Ok(conflict)
    }

    pub async fn touch_announcement(&self, announcement: &LoaderAnnouncement, ip: &str) {
        let mut state = self.state.lock().await;
        if let Some(device) = state.devices.get_mut(&announcement.mac_address)
            && device.boot_epoch == announcement.boot_epoch
            && device.ip_address == ip
        {
            device.last_seen_at = Utc::now();
        }
        self.deadline_changed.notify_one();
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
                        .flat_map(|device| {
                            [
                                device.last_seen_at + ONLINE_TTL,
                                device.last_seen_at + RECORD_RETENTION,
                            ]
                        })
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
                arch: record.arch.clone(),
                loader_version: record.loader_version.clone(),
                hardware: record.hardware.clone(),
                last_seen_at: record.last_seen_at,
                online: now - record.last_seen_at <= ONLINE_TTL,
                conflict: record.conflict,
                boot_epoch: record.boot_epoch.clone(),
            })
            .collect()
    }
}

impl Default for LoaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_v7(version: u16) -> Result<(), DeviceOperationError> {
    validate_device_protocol_version(version)
}

fn prune(state: &mut LoaderRegistryState, now: DateTime<Utc>) {
    state
        .devices
        .retain(|_, device| now - device.last_seen_at <= RECORD_RETENTION);
    for device in state.devices.values_mut() {
        if now - device.last_seen_at >= ONLINE_TTL {
            device.conflict = false;
            device.conflict_until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpboot_protocol::DEVICE_PROTOCOL_VERSION;

    fn announcement(mac: MacAddress, boot_epoch: &str, arch: &str) -> LoaderAnnouncement {
        LoaderAnnouncement {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: arch.into(),
            loader_version: "test".into(),
            boot_epoch: boot_epoch.into(),
            http_port: 2999,
            serial_id: None,
            serial_ready: false,
        }
    }

    fn status(mac: MacAddress, boot_epoch: &str, arch: &str) -> LoaderDeviceStatus {
        LoaderDeviceStatus {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: boot_epoch.into(),
            mac_address: mac,
            current_mac_address: mac,
            arch: arch.into(),
            loader_version: "test".into(),
            hardware: LoaderHardwareInfo::default(),
            boot: None,
            ota: None,
            serial: None,
        }
    }

    #[tokio::test]
    async fn local_udp_discovers_unbound_v7_devices_and_public_refuses_the_listener() {
        let root = tempfile::tempdir().unwrap();
        let mut config = crate::ServerConfig {
            data_dir: root.path().join("data"),
            ..Default::default()
        };
        config.tftp = crate::TftpConfig::Builtin(crate::BuiltinTftpConfig::default_with_root(
            root.path().join("tftp"),
        ));
        let manager = crate::tftp::service::build_tftp_manager(&config.tftp);
        let state = crate::test_support::build_test_app_state(
            root.path().join("test.env"),
            config,
            manager,
        )
        .await
        .unwrap();
        let mac = "02:00:00:00:00:42".parse().unwrap();
        let observed = status(mac, "udp-epoch", "future-isa");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = axum::Router::new().route(
            "/api/v1/status",
            axum::routing::get(move || {
                let observed = observed.clone();
                async move { axum::Json(observed) }
            }),
        );
        let device = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let discovery = tokio::spawn(serve_udp_announcements(state.clone(), socket));
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut announcement = announcement(mac, "udp-epoch", "future-isa");
        announcement.http_port = port;
        sender
            .send_to(&serde_json::to_vec(&announcement).unwrap(), address)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(device) = state.loader_registry.snapshots().await.first() {
                    assert_eq!(device.mac_address, mac);
                    assert_eq!(device.arch, "future-isa");
                    assert!(device.online);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(state.boards.read().await.is_empty());
        {
            let mut config = state.config.write().await;
            config.edition = crate::config::Edition::Public;
            config.loader_network.enabled = true;
        }
        assert!(start_udp_discovery(state).await.is_err());
        discovery.abort();
        device.abort();
    }

    #[tokio::test]
    async fn future_arch_is_displayed_without_affecting_identity() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        registry
            .accept_announcement(
                &announcement(mac, "epoch-1", "future-isa"),
                &status(mac, "epoch-1", "future-isa"),
                "127.0.0.1".into(),
            )
            .await
            .unwrap();
        let snapshots = registry.snapshots().await;
        assert_eq!(snapshots[0].arch, "future-isa");
        assert_eq!(snapshots[0].boot_epoch, "epoch-1");
    }

    #[tokio::test]
    async fn arch_text_is_not_an_admission_condition() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        assert!(
            !registry
                .accept_announcement(
                    &announcement(mac, "epoch-1", "loader-name"),
                    &status(mac, "epoch-1", "current-name"),
                    "127.0.0.1".into(),
                )
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn same_mac_from_two_active_sources_is_a_conflict() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        registry
            .accept_announcement(
                &announcement(mac, "epoch-1", "future-isa"),
                &status(mac, "epoch-1", "future-isa"),
                "127.0.0.1".into(),
            )
            .await
            .unwrap();
        assert!(
            registry
                .accept_announcement(
                    &announcement(mac, "epoch-2", "future-isa"),
                    &status(mac, "epoch-2", "future-isa"),
                    "127.0.0.2".into(),
                )
                .await
                .unwrap()
        );
        assert!(registry.snapshots().await[0].conflict);
        assert!(
            registry
                .accept_announcement(
                    &announcement(mac, "epoch-3", "future-isa"),
                    &status(mac, "epoch-3", "future-isa"),
                    "127.0.0.1".into(),
                )
                .await
                .unwrap()
        );
        assert!(registry.snapshots().await[0].conflict);
    }

    #[tokio::test]
    async fn rejects_non_v7_announcements() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let mut old = announcement(mac, "epoch-1", "x86_64");
        old.protocol_version = DEVICE_PROTOCOL_VERSION - 1;
        let error = registry
            .accept_announcement(&old, &status(mac, "epoch-1", "x86_64"), "127.0.0.1".into())
            .await
            .unwrap_err();
        assert_eq!(error.code, "unsupported_protocol");
    }
}
