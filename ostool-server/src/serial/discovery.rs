use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use nusb::MaybeFuture;

use crate::{
    api::models::SerialPortSummary,
    config::{SerialConfig, SerialPortKey, SerialPortKeyKind},
};

const SERIAL_BY_PATH_DIR: &str = "/dev/serial/by-path";
const SERIAL_BY_ID_DIR: &str = "/dev/serial/by-id";

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsbDeviceRecord {
    vendor_id: u16,
    product_id: u16,
    manufacturer: Option<String>,
    product: Option<String>,
    serial_number: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPortRecord {
    pub current_device_path: String,
    pub port_type: String,
    pub primary_key_kind: Option<SerialPortKeyKind>,
    pub primary_key_value: Option<String>,
    pub usb_path: Option<String>,
    pub usb_id: Option<String>,
    pub stable_identity: bool,
    pub usb_vendor_id: Option<u16>,
    pub usb_product_id: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

pub fn list_serial_ports() -> anyhow::Result<Vec<SerialPortSummary>> {
    let records = discover_serial_ports()?;
    Ok(records
        .into_iter()
        .map(|record| SerialPortSummary {
            label: serial_port_label(&record),
            current_device_path: record.current_device_path,
            port_type: record.port_type,
            primary_key_kind: record.primary_key_kind,
            primary_key_value: record.primary_key_value,
            usb_path: record.usb_path,
            usb_id: record.usb_id,
            stable_identity: record.stable_identity,
            usb_vendor_id: record.usb_vendor_id,
            usb_product_id: record.usb_product_id,
            manufacturer: record.manufacturer,
            product: record.product,
            serial_number: record.serial_number,
        })
        .collect())
}

pub fn resolve_serial_config(serial: &SerialConfig) -> anyhow::Result<SerialPortRecord> {
    resolve_serial_key(&serial.key)
}

pub fn resolve_serial_key(key: &SerialPortKey) -> anyhow::Result<SerialPortRecord> {
    let records = discover_serial_ports()?;
    resolve_serial_key_from_records(&records, key)
}

fn resolve_serial_key_from_records(
    records: &[SerialPortRecord],
    key: &SerialPortKey,
) -> anyhow::Result<SerialPortRecord> {
    let mut matches = records.iter().filter(|record| {
        let value = match key.kind {
            SerialPortKeyKind::SerialNumber => record.serial_number.as_deref(),
            SerialPortKeyKind::UsbId => record.usb_id.as_deref(),
            SerialPortKeyKind::UsbPath => record.usb_path.as_deref(),
            SerialPortKeyKind::Qemu => None,
        };
        value == Some(key.value.as_str())
    });
    if let Some(matched) = matches.next() {
        if matches.next().is_some() {
            return Err(anyhow::anyhow!(
                "ambiguous serial device for {} `{}`; select an interface-specific usb_id or usb_path",
                serial_key_kind_label(&key.kind),
                key.value
            ));
        }
        return Ok(matched.clone());
    }

    if matches!(
        key.kind,
        SerialPortKeyKind::UsbPath | SerialPortKeyKind::UsbId
    ) && Path::new(&key.value).exists()
    {
        return Ok(SerialPortRecord {
            current_device_path: fs::canonicalize(&key.value)
                .unwrap_or_else(|_| PathBuf::from(&key.value))
                .to_string_lossy()
                .into_owned(),
            port_type: "usb".to_string(),
            primary_key_kind: Some(key.kind.clone()),
            primary_key_value: Some(key.value.clone()),
            usb_path: (key.kind == SerialPortKeyKind::UsbPath)
                .then(|| key.value.clone())
                .filter(|value| value.starts_with(SERIAL_BY_PATH_DIR)),
            usb_id: (key.kind == SerialPortKeyKind::UsbId).then(|| key.value.clone()),
            stable_identity: true,
            usb_vendor_id: None,
            usb_product_id: None,
            manufacturer: None,
            product: None,
            serial_number: None,
        });
    }

    Err(anyhow::anyhow!(
        "failed to resolve serial device for {} `{}`",
        serial_key_kind_label(&key.kind),
        key.value
    ))
}

fn discover_serial_ports() -> anyhow::Result<Vec<SerialPortRecord>> {
    let usb_devices = load_usb_devices().unwrap_or_default();
    let by_path_map = load_usb_path_map();
    let by_id_map = load_usb_id_map();
    let mut ports = serialport::available_ports()?
        .into_iter()
        .map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(info) => {
                let usb_match = match_usb_device(&usb_devices, &info);
                let manufacturer = usb_match
                    .and_then(|device| device.manufacturer.clone())
                    .or(info.manufacturer.clone());
                let product = usb_match
                    .and_then(|device| device.product.clone())
                    .or(info.product.clone());
                let serial_number = usb_match
                    .and_then(|device| device.serial_number.clone())
                    .or(info.serial_number.clone());
                let usb_path = by_path_map.get(port.port_name.as_str()).cloned();
                let usb_id = by_id_map.get(port.port_name.as_str()).cloned();
                let (primary_key_kind, primary_key_value) = if let Some(usb_id) = usb_id.clone() {
                    (Some(SerialPortKeyKind::UsbId), Some(usb_id))
                } else if let Some(serial_number) = serial_number.clone() {
                    (Some(SerialPortKeyKind::SerialNumber), Some(serial_number))
                } else if let Some(usb_path) = usb_path.clone() {
                    (Some(SerialPortKeyKind::UsbPath), Some(usb_path))
                } else {
                    (None, None)
                };

                SerialPortRecord {
                    current_device_path: port.port_name,
                    port_type: "usb".to_string(),
                    primary_key_kind,
                    primary_key_value,
                    stable_identity: serial_number.is_some()
                        || usb_path.is_some()
                        || usb_id.is_some(),
                    usb_path,
                    usb_id,
                    usb_vendor_id: Some(info.vid),
                    usb_product_id: Some(info.pid),
                    manufacturer,
                    product,
                    serial_number,
                }
            }
            serialport::SerialPortType::BluetoothPort => SerialPortRecord {
                current_device_path: port.port_name,
                port_type: "bluetooth".to_string(),
                primary_key_kind: None,
                primary_key_value: None,
                stable_identity: false,
                usb_path: None,
                usb_id: None,
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
            serialport::SerialPortType::PciPort => SerialPortRecord {
                current_device_path: port.port_name,
                port_type: "pci".to_string(),
                primary_key_kind: None,
                primary_key_value: None,
                stable_identity: false,
                usb_path: None,
                usb_id: None,
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
            serialport::SerialPortType::Unknown => SerialPortRecord {
                current_device_path: port.port_name,
                port_type: "unknown".to_string(),
                primary_key_kind: None,
                primary_key_value: None,
                stable_identity: false,
                usb_path: None,
                usb_id: None,
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
        })
        .collect::<Vec<_>>();

    ports.sort_by(|a, b| {
        b.stable_identity
            .cmp(&a.stable_identity)
            .then_with(|| a.current_device_path.cmp(&b.current_device_path))
    });
    Ok(ports)
}

fn load_usb_devices() -> anyhow::Result<Vec<UsbDeviceRecord>> {
    let devices = nusb::list_devices()
        .wait()?
        .map(|device| UsbDeviceRecord {
            vendor_id: device.vendor_id(),
            product_id: device.product_id(),
            manufacturer: device.manufacturer_string().map(ToOwned::to_owned),
            product: device.product_string().map(ToOwned::to_owned),
            serial_number: device.serial_number().map(ToOwned::to_owned),
        })
        .collect::<Vec<_>>();
    Ok(devices)
}

fn load_usb_path_map() -> BTreeMap<String, String> {
    let mut by_path = BTreeMap::new();
    let Ok(entries) = fs::read_dir(SERIAL_BY_PATH_DIR) else {
        return by_path;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(target) = fs::read_link(&path) else {
            continue;
        };
        let device_path = resolve_symlink_target(path.parent().unwrap_or(Path::new("/")), &target);
        if let (Some(device_path), Some(usb_path)) =
            (device_path.and_then(path_to_string), path_to_string(path))
        {
            by_path.insert(device_path, usb_path);
        }
    }

    by_path
}

fn load_usb_id_map() -> BTreeMap<String, String> {
    load_serial_link_map(SERIAL_BY_ID_DIR)
}

fn load_serial_link_map(dir: &str) -> BTreeMap<String, String> {
    let mut links = BTreeMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return links;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(target) = fs::read_link(&path) else {
            continue;
        };
        let device_path = resolve_symlink_target(path.parent().unwrap_or(Path::new("/")), &target);
        if let (Some(device_path), Some(link)) =
            (device_path.and_then(path_to_string), path_to_string(path))
        {
            links.insert(device_path, link);
        }
    }
    links
}

fn resolve_symlink_target(base_dir: &Path, target: &Path) -> Option<PathBuf> {
    let candidate = if target.is_absolute() {
        target.to_path_buf()
    } else {
        base_dir.join(target)
    };
    fs::canonicalize(candidate).ok()
}

fn path_to_string(path: impl AsRef<Path>) -> Option<String> {
    path.as_ref().to_str().map(ToOwned::to_owned)
}

fn serial_port_label(record: &SerialPortRecord) -> String {
    let primary = match (&record.primary_key_kind, &record.primary_key_value) {
        (Some(kind), Some(value)) => format!("[{}] {}", serial_key_kind_short_label(kind), value),
        _ => format!("[UNSTABLE] {}", record.current_device_path),
    };

    let mut detail_parts = Vec::new();
    if let Some(usb_path) = record.usb_path.as_deref() {
        detail_parts.push(usb_path.to_string());
    }
    detail_parts.push(record.current_device_path.clone());
    if let Some(manufacturer) = record.manufacturer.as_deref() {
        detail_parts.push(manufacturer.to_string());
    }
    if let Some(product) = record.product.as_deref() {
        detail_parts.push(product.to_string());
    }
    if let (Some(vid), Some(pid)) = (record.usb_vendor_id, record.usb_product_id) {
        detail_parts.push(format!("VID:PID {vid:04x}:{pid:04x}"));
    }

    format!("{primary} ({})", detail_parts.join(" / "))
}

fn serial_key_kind_short_label(kind: &SerialPortKeyKind) -> &'static str {
    match kind {
        SerialPortKeyKind::SerialNumber => "SN",
        SerialPortKeyKind::UsbId => "USB ID",
        SerialPortKeyKind::UsbPath => "USB PATH",
        SerialPortKeyKind::Qemu => "QEMU",
    }
}

fn serial_key_kind_label(kind: &SerialPortKeyKind) -> &'static str {
    match kind {
        SerialPortKeyKind::SerialNumber => "serial number",
        SerialPortKeyKind::UsbId => "usb id",
        SerialPortKeyKind::UsbPath => "usb path",
        SerialPortKeyKind::Qemu => "qemu virtual device",
    }
}

fn match_usb_device<'a>(
    usb_devices: &'a [UsbDeviceRecord],
    info: &serialport::UsbPortInfo,
) -> Option<&'a UsbDeviceRecord> {
    if let Some(serial_number) = info.serial_number.as_deref()
        && let Some(exact) = usb_devices.iter().find(|device| {
            device.vendor_id == info.vid
                && device.product_id == info.pid
                && device.serial_number.as_deref() == Some(serial_number)
        })
    {
        return Some(exact);
    }

    if let Some(product) = info.product.as_deref() {
        let mut matches = usb_devices.iter().filter(|device| {
            device.vendor_id == info.vid
                && device.product_id == info.pid
                && device.product.as_deref() == Some(product)
        });
        if let Some(exact) = matches.next()
            && matches.next().is_none()
        {
            return Some(exact);
        }
    }

    let candidates = usb_devices
        .iter()
        .filter(|device| device.vendor_id == info.vid && device.product_id == info.pid)
        .collect::<Vec<_>>();

    if candidates.len() == 1 {
        candidates.into_iter().next()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        SerialPortRecord, UsbDeviceRecord, match_usb_device, resolve_serial_key_from_records,
        serial_port_label,
    };
    use crate::config::{SerialPortKey, SerialPortKeyKind};

    fn usb_info(
        vid: u16,
        pid: u16,
        manufacturer: Option<&str>,
        product: Option<&str>,
        serial_number: Option<&str>,
    ) -> serialport::UsbPortInfo {
        serialport::UsbPortInfo {
            vid,
            pid,
            serial_number: serial_number.map(ToOwned::to_owned),
            manufacturer: manufacturer.map(ToOwned::to_owned),
            product: product.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn match_usb_device_prefers_serial_number() {
        let devices = vec![
            UsbDeviceRecord {
                vendor_id: 0x1a86,
                product_id: 0x7523,
                manufacturer: Some("QinHeng".into()),
                product: Some("USB2.0-Serial".into()),
                serial_number: Some("A".into()),
            },
            UsbDeviceRecord {
                vendor_id: 0x1a86,
                product_id: 0x7523,
                manufacturer: Some("QinHeng".into()),
                product: Some("USB2.0-Serial".into()),
                serial_number: Some("B".into()),
            },
        ];

        let matched =
            match_usb_device(&devices, &usb_info(0x1a86, 0x7523, None, None, Some("B"))).unwrap();

        assert_eq!(matched.serial_number.as_deref(), Some("B"));
        assert!(
            match_usb_device(
                &devices,
                &usb_info(0x1a86, 0x7523, None, Some("USB2.0-Serial"), None)
            )
            .is_none()
        );
    }

    #[test]
    fn match_usb_device_falls_back_to_unique_vid_pid() {
        let devices = vec![UsbDeviceRecord {
            vendor_id: 0x10c4,
            product_id: 0xea60,
            manufacturer: Some("Silicon Labs".into()),
            product: Some("CP2102".into()),
            serial_number: None,
        }];

        let matched =
            match_usb_device(&devices, &usb_info(0x10c4, 0xea60, None, None, None)).unwrap();

        assert_eq!(matched.product.as_deref(), Some("CP2102"));
    }

    #[test]
    fn serial_port_label_highlights_primary_key() {
        let label = serial_port_label(&SerialPortRecord {
            current_device_path: "/dev/ttyUSB1".into(),
            port_type: "usb".into(),
            primary_key_kind: Some(SerialPortKeyKind::SerialNumber),
            primary_key_value: Some("ABC123".into()),
            usb_path: Some("/dev/serial/by-path/demo".into()),
            usb_id: None,
            stable_identity: true,
            usb_vendor_id: Some(0x0403),
            usb_product_id: Some(0x6001),
            manufacturer: Some("FTDI".into()),
            product: Some("UART".into()),
            serial_number: Some("ABC123".into()),
        });

        assert!(label.contains("[SN] ABC123"));
        assert!(label.contains("/dev/serial/by-path/demo"));
        assert!(label.contains("/dev/ttyUSB1"));
    }

    #[test]
    fn resolve_serial_key_matches_serial_number_record() {
        let resolved = resolve_serial_key_from_records(
            &[SerialPortRecord {
                current_device_path: "/dev/ttyUSB7".into(),
                port_type: "usb".into(),
                primary_key_kind: Some(SerialPortKeyKind::SerialNumber),
                primary_key_value: Some("relay-123".into()),
                usb_path: Some("/dev/serial/by-path/demo".into()),
                usb_id: None,
                stable_identity: true,
                usb_vendor_id: Some(0x1a86),
                usb_product_id: Some(0x7523),
                manufacturer: Some("QinHeng".into()),
                product: Some("USB Serial".into()),
                serial_number: Some("relay-123".into()),
            }],
            &SerialPortKey {
                kind: SerialPortKeyKind::SerialNumber,
                value: "relay-123".into(),
            },
        )
        .unwrap();

        assert_eq!(resolved.current_device_path, "/dev/ttyUSB7");
    }

    #[test]
    fn resolve_serial_key_matches_usb_path_record() {
        let resolved = resolve_serial_key_from_records(
            &[SerialPortRecord {
                current_device_path: "/dev/ttyUSB8".into(),
                port_type: "usb".into(),
                primary_key_kind: Some(SerialPortKeyKind::UsbPath),
                primary_key_value: Some("/dev/serial/by-path/relay".into()),
                usb_path: Some("/dev/serial/by-path/relay".into()),
                usb_id: None,
                stable_identity: true,
                usb_vendor_id: Some(0x0403),
                usb_product_id: Some(0x6001),
                manufacturer: Some("FTDI".into()),
                product: Some("UART".into()),
                serial_number: None,
            }],
            &SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: "/dev/serial/by-path/relay".into(),
            },
        )
        .unwrap();

        assert_eq!(resolved.current_device_path, "/dev/ttyUSB8");
    }

    #[test]
    fn resolve_serial_key_matches_usb_id_record() {
        let resolved = resolve_serial_key_from_records(
            &[SerialPortRecord {
                current_device_path: "/dev/ttyUSB9".into(),
                port_type: "usb".into(),
                primary_key_kind: Some(SerialPortKeyKind::UsbId),
                primary_key_value: Some("/dev/serial/by-id/ftdi-if01".into()),
                usb_path: None,
                usb_id: Some("/dev/serial/by-id/ftdi-if01".into()),
                stable_identity: true,
                usb_vendor_id: Some(0x0403),
                usb_product_id: Some(0x6011),
                manufacturer: Some("FTDI".into()),
                product: Some("FT4232H".into()),
                serial_number: Some("FTB4GC1E".into()),
            }],
            &SerialPortKey {
                kind: SerialPortKeyKind::UsbId,
                value: "/dev/serial/by-id/ftdi-if01".into(),
            },
        )
        .unwrap();

        assert_eq!(resolved.current_device_path, "/dev/ttyUSB9");
    }

    fn multi_port_record(interface: u8) -> SerialPortRecord {
        let usb_id = format!("/dev/serial/by-id/ftdi-if{interface:02}");
        SerialPortRecord {
            current_device_path: format!("/dev/ttyUSB{interface}"),
            port_type: "usb".into(),
            primary_key_kind: Some(SerialPortKeyKind::UsbId),
            primary_key_value: Some(usb_id.clone()),
            usb_path: Some(format!("/dev/serial/by-path/ftdi-if{interface:02}")),
            usb_id: Some(usb_id),
            stable_identity: true,
            usb_vendor_id: Some(0x0403),
            usb_product_id: Some(0x6011),
            manufacturer: Some("FTDI".into()),
            product: Some("FT4232H".into()),
            serial_number: Some("FTB4GC1E".into()),
        }
    }

    #[test]
    fn resolve_serial_key_preserves_serial_number_with_usb_id_primary() {
        let record = multi_port_record(1);
        let resolved = resolve_serial_key_from_records(
            std::slice::from_ref(&record),
            &SerialPortKey {
                kind: SerialPortKeyKind::SerialNumber,
                value: "FTB4GC1E".into(),
            },
        )
        .unwrap();
        assert_eq!(resolved, record);
    }

    #[test]
    fn resolve_serial_key_preserves_usb_path_with_usb_id_primary() {
        let record = multi_port_record(1);
        let resolved = resolve_serial_key_from_records(
            std::slice::from_ref(&record),
            &SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: record.usb_path.clone().unwrap(),
            },
        )
        .unwrap();
        assert_eq!(resolved, record);
    }

    #[test]
    fn resolve_serial_key_rejects_shared_serial_number_regardless_of_primary() {
        let mut first = multi_port_record(0);
        first.primary_key_kind = Some(SerialPortKeyKind::SerialNumber);
        first.primary_key_value = first.serial_number.clone();
        let second = multi_port_record(1);
        for records in [vec![first.clone(), second.clone()], vec![second, first]] {
            let err = resolve_serial_key_from_records(
                &records,
                &SerialPortKey {
                    kind: SerialPortKeyKind::SerialNumber,
                    value: "FTB4GC1E".into(),
                },
            )
            .unwrap_err();
            let message = err.to_string();
            assert!(message.contains("ambiguous"));
            assert!(message.contains("FTB4GC1E"));
            assert!(message.contains("usb_id"));
            assert!(message.contains("usb_path"));
        }
    }

    #[test]
    fn resolve_serial_key_selects_interface_with_shared_serial_number() {
        let records = [multi_port_record(0), multi_port_record(1)];
        for (kind, value) in [
            (SerialPortKeyKind::UsbId, records[1].usb_id.clone().unwrap()),
            (
                SerialPortKeyKind::UsbPath,
                records[1].usb_path.clone().unwrap(),
            ),
        ] {
            let resolved =
                resolve_serial_key_from_records(&records, &SerialPortKey { kind, value }).unwrap();
            assert_eq!(resolved, records[1]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolve_serial_key_falls_back_to_usb_id_symlink() {
        let temp = tempdir().unwrap();
        let device = temp.path().join("ttyUSB9");
        let link = temp.path().join("ftdi-if01");
        std::fs::write(&device, b"").unwrap();
        std::os::unix::fs::symlink("ttyUSB9", &link).unwrap();
        let resolved = resolve_serial_key_from_records(
            &[],
            &SerialPortKey {
                kind: SerialPortKeyKind::UsbId,
                value: link.display().to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            resolved.current_device_path,
            device.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(resolved.usb_id, Some(link.display().to_string()));
        assert_eq!(resolved.primary_key_kind, Some(SerialPortKeyKind::UsbId));
        assert!(resolved.stable_identity);
        std::fs::remove_file(device).unwrap();
        assert!(
            resolve_serial_key_from_records(
                &[],
                &SerialPortKey {
                    kind: SerialPortKeyKind::UsbId,
                    value: link.display().to_string(),
                },
            )
            .is_err()
        );
    }

    #[test]
    fn resolve_serial_key_falls_back_to_existing_usb_path() {
        let temp = tempdir().unwrap();
        let existing_path = temp.path().join("ttyUSB-relay");
        std::fs::write(&existing_path, b"").unwrap();

        let resolved = resolve_serial_key_from_records(
            &[],
            &SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: existing_path.display().to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            resolved.current_device_path,
            existing_path.display().to_string()
        );
        assert_eq!(resolved.primary_key_kind, Some(SerialPortKeyKind::UsbPath));
        assert!(resolved.stable_identity);
    }

    #[test]
    fn resolve_serial_key_returns_error_when_key_is_missing() {
        let err = resolve_serial_key_from_records(
            &[],
            &SerialPortKey {
                kind: SerialPortKeyKind::SerialNumber,
                value: "missing-relay".into(),
            },
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("failed to resolve serial device for serial number `missing-relay`")
        );
    }
}
