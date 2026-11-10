use std::{collections::BTreeMap, sync::Arc};

use chrono::{DateTime, Duration, Utc};
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION, LoaderAnnouncement, LoaderDeviceStatus, LoaderHardwareInfo, MacAddress,
};
use serde::Serialize;
use tokio::sync::Mutex;

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

#[derive(Debug, Clone, Default)]
pub struct LoaderRegistry {
    events: crate::admin_events::AdminEvents,
    deadline_changed: Arc<tokio::sync::Notify>,
    state: Arc<Mutex<BTreeMap<MacAddress, DeviceRecord>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationError {
    Capacity,
    ProtocolVersion,
}

impl LoaderRegistry {
    pub async fn accept_announcement(
        &self,
        announcement: &LoaderAnnouncement,
        device: &LoaderDeviceStatus,
        ip_address: String,
    ) -> Result<bool, ObservationError> {
        if !matches!(
            announcement.protocol_version,
            DEVICE_PROTOCOL_VERSION | httpboot_protocol::PREVIOUS_DEVICE_PROTOCOL_VERSION
        ) || device.protocol_version != announcement.protocol_version
            || announcement.mac_address != device.mac_address
            || announcement.boot_epoch != device.boot_epoch
            || announcement.arch != device.arch
        {
            return Err(ObservationError::ProtocolVersion);
        }
        let now = Utc::now();
        let mut state = self.state.lock().await;
        prune(&mut state, now);
        if state.len() >= 4096 && !state.contains_key(&device.mac_address) {
            return Err(ObservationError::Capacity);
        }
        let conflict = state.get(&device.mac_address).is_some_and(|previous| {
            (previous.ip_address != ip_address || previous.conflict)
                && now - previous.last_seen_at < ONLINE_TTL
        });
        if conflict {
            if let Some(previous) = state.get_mut(&device.mac_address) {
                previous.conflict = true;
            }
        } else {
            state.insert(
                device.mac_address,
                DeviceRecord {
                    current_mac_address: device.current_mac_address,
                    ip_address,
                    arch: device.arch,
                    loader_version: device.loader_version.clone(),
                    hardware: device.hardware.clone(),
                    last_seen_at: now,
                    current_registration_id: device.boot_epoch.clone(),
                    conflict: false,
                },
            );
        }
        drop(state);
        self.events.invalidate(&["loaders"]);
        self.deadline_changed.notify_one();
        Ok(conflict)
    }

    pub fn new() -> Self {
        Self::default()
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
                        .values()
                        .flat_map(|d| {
                            [
                                d.last_seen_at + ONLINE_TTL,
                                d.last_seen_at + RECORD_RETENTION,
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

fn prune(state: &mut BTreeMap<MacAddress, DeviceRecord>, now: DateTime<Utc>) {
    state.retain(|_, device| now - device.last_seen_at <= RECORD_RETENTION);
    for device in state.values_mut() {
        if now - device.last_seen_at >= ONLINE_TTL {
            device.conflict = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpboot_protocol::BootArch;

    #[tokio::test]
    async fn v5_announcement_requires_matching_observed_epoch_and_address() {
        let registry = LoaderRegistry::new();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let announcement = LoaderAnnouncement {
            serial_id: None,
            serial_ready: false,
            protocol_version: DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: BootArch::X86_64,
            loader_version: "test".into(),
            boot_epoch: "generation-1".into(),
            http_port: 2999,
        };
        let mut observed = LoaderDeviceStatus {
            serial: None,
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: "generation-2".into(),
            mac_address: mac,
            current_mac_address: mac,
            arch: BootArch::X86_64,
            loader_version: "test".into(),
            hardware: LoaderHardwareInfo::default(),
            boot: None,
            ota: None,
        };
        assert!(
            registry
                .accept_announcement(&announcement, &observed, "127.0.0.1".into())
                .await
                .is_err()
        );
        assert!(registry.snapshots().await.is_empty());
        observed.boot_epoch = announcement.boot_epoch.clone();
        assert!(
            !registry
                .accept_announcement(&announcement, &observed, "127.0.0.1".into())
                .await
                .unwrap()
        );
        assert_eq!(
            registry.snapshots().await[0]
                .current_registration_id
                .as_deref(),
            Some("generation-1")
        );
        assert!(
            registry
                .accept_announcement(&announcement, &observed, "127.0.0.2".into())
                .await
                .unwrap()
        );
        assert!(registry.snapshots().await[0].conflict);
    }
}
