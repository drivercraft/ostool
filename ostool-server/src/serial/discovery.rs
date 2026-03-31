use nusb::MaybeFuture;

use crate::api::models::SerialPortSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsbDeviceRecord {
    vendor_id: u16,
    product_id: u16,
    manufacturer: Option<String>,
    product: Option<String>,
    serial_number: Option<String>,
}

pub fn list_serial_ports() -> anyhow::Result<Vec<SerialPortSummary>> {
    let usb_devices = load_usb_devices().unwrap_or_default();
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

                let mut detail_parts = Vec::new();
                if let Some(manufacturer) = manufacturer.clone() {
                    detail_parts.push(manufacturer);
                }
                if let Some(product) = product.clone() {
                    detail_parts.push(product);
                }
                detail_parts.push(format!("VID:PID {:04x}:{:04x}", info.vid, info.pid));
                if let Some(serial_number) = serial_number.clone() {
                    detail_parts.push(format!("SN {serial_number}"));
                }

                SerialPortSummary {
                    port_name: port.port_name.clone(),
                    port_type: "usb".to_string(),
                    label: if detail_parts.is_empty() {
                        format!("{} (usb)", port.port_name)
                    } else {
                        format!("{} ({})", port.port_name, detail_parts.join(" / "))
                    },
                    usb_vendor_id: Some(info.vid),
                    usb_product_id: Some(info.pid),
                    manufacturer,
                    product,
                    serial_number,
                }
            }
            serialport::SerialPortType::BluetoothPort => SerialPortSummary {
                port_name: port.port_name.clone(),
                port_type: "bluetooth".to_string(),
                label: format!("{} (蓝牙串口)", port.port_name),
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
            serialport::SerialPortType::PciPort => SerialPortSummary {
                port_name: port.port_name.clone(),
                port_type: "pci".to_string(),
                label: format!("{} (PCI 串口)", port.port_name),
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
            serialport::SerialPortType::Unknown => SerialPortSummary {
                port_name: port.port_name.clone(),
                port_type: "unknown".to_string(),
                label: format!("{} (未知类型)", port.port_name),
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
        })
        .collect::<Vec<_>>();

    ports.sort_by(|a, b| a.port_name.cmp(&b.port_name));
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

    if let Some(product) = info.product.as_deref()
        && let Some(exact) = usb_devices.iter().find(|device| {
            device.vendor_id == info.vid
                && device.product_id == info.pid
                && device.product.as_deref() == Some(product)
        })
    {
        return Some(exact);
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
    use super::{UsbDeviceRecord, match_usb_device};

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
}
