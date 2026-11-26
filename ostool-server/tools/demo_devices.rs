//! Device-side simulation for page/rental demonstrations, never a server bypass.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION, DeviceOperationError, LoaderDeviceStatus, LoaderHardwareInfo,
    MacAddress,
};
use std::sync::Arc;
use tokio::{net::TcpListener, sync::Mutex, task::JoinSet};
use uuid::Uuid;

const BASE_PORT: u16 = 31000;

pub fn endpoint(index: usize) -> String {
    format!("http://127.0.0.1:{}", BASE_PORT + index as u16)
}

pub fn mac(index: usize) -> MacAddress {
    MacAddress::new([
        2,
        0x74,
        0x6c,
        (index % 6 + 1) as u8,
        (index / 6 + 1) as u8,
        (0x20 + index) as u8,
    ])
}

struct Device {
    powered: bool,
    status: LoaderDeviceStatus,
}
type SharedDevice = Arc<Mutex<Device>>;

fn router(index: usize) -> Router {
    let state = Arc::new(Mutex::new(Device {
        powered: true,
        status: LoaderDeviceStatus {
            protocol_version: DEVICE_PROTOCOL_VERSION,
            boot_epoch: Uuid::new_v4().to_string(),
            mac_address: mac(index),
            current_mac_address: mac(index),
            arch: [
                "aarch64",
                "aarch64",
                "riscv64",
                "loongarch64",
                "x86_64",
                "aarch64",
            ][index % 6]
                .into(),
            loader_version: "ostool-demo".into(),
            hardware: LoaderHardwareInfo {
                manufacturer: Some("ostool demo".into()),
                product: Some("模拟设备（仅在线与电源）".into()),
                serial: Some(format!("demo-{:02}", index + 1)),
                ..Default::default()
            },
            boot: None,
            ota: None,
            serial: None,
        },
    }));
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/api/v1/status", get(status))
        .route("/demo/power/{action}", post(power))
        .fallback(unsupported)
        .with_state(state)
}

async fn status(State(device): State<SharedDevice>) -> Response {
    let device = device.lock().await;
    if !device.powered {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "device_powered_off",
            "演示设备已断电",
        );
    }
    Json(device.status.clone()).into_response()
}

async fn power(State(device): State<SharedDevice>, Path(action): Path<String>) -> StatusCode {
    let mut device = device.lock().await;
    match action.as_str() {
        "on" => {
            if !device.powered {
                device.status.boot_epoch = Uuid::new_v4().to_string();
            }
            device.powered = true;
        }
        "off" => device.powered = false,
        _ => return StatusCode::NOT_FOUND,
    }
    StatusCode::NO_CONTENT
}

async fn unsupported() -> Response {
    error(
        StatusCode::NOT_IMPLEMENTED,
        "demo_operation_unsupported",
        "模拟器仅支持在线与电源，不能执行内核、串口或 OTA",
    )
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(DeviceOperationError {
            code: code.into(),
            message: message.into(),
        }),
    )
        .into_response()
}

pub async fn serve() -> anyhow::Result<()> {
    // Bind the entire bank first; a port collision must not leave a partial bank.
    let mut listeners = Vec::new();
    for index in 0..super::BOARD_COUNT {
        listeners.push(
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, BASE_PORT + index as u16)).await?,
        );
    }
    let mut tasks = JoinSet::new();
    for (index, listener) in listeners.into_iter().enumerate() {
        tasks.spawn(async move { axum::serve(listener, router(index)).await });
    }
    println!(
        "本机演示设备：127.0.0.1:{BASE_PORT}–{}；仅模拟在线与电源",
        BASE_PORT + super::BOARD_COUNT as u16 - 1
    );
    tokio::select! {
        result = tasks.join_next() => {
            let task = result.ok_or_else(|| anyhow::anyhow!("device bank stopped"))?;
            task??;
            anyhow::bail!("device listener stopped unexpectedly");
        }
        result = tokio::signal::ctrl_c() => result?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, path: &str) -> (StatusCode, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        (
            response.status(),
            to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .to_vec(),
        )
    }

    #[tokio::test]
    async fn power_is_idempotent_independent_and_reboots_change_epoch() {
        let first = router(0);
        let second = router(1);
        let (code, body) = call(&first, "GET", "/api/v1/status").await;
        assert_eq!(code, StatusCode::OK);
        let initial: LoaderDeviceStatus = serde_json::from_slice(&body).unwrap();
        assert_eq!(initial.mac_address, mac(0));
        assert!(initial.serial.is_none() && initial.boot.is_none());
        assert_eq!(
            call(&first, "POST", "/demo/power/on").await.0,
            StatusCode::NO_CONTENT
        );
        let same: LoaderDeviceStatus =
            serde_json::from_slice(&call(&first, "GET", "/api/v1/status").await.1).unwrap();
        assert_eq!(same.boot_epoch, initial.boot_epoch);
        assert_eq!(
            call(&first, "POST", "/demo/power/off").await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(&first, "GET", "/api/v1/status").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            call(&first, "GET", "/health").await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(&second, "GET", "/api/v1/status").await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&first, "POST", "/demo/power/on").await.0,
            StatusCode::NO_CONTENT
        );
        let rebooted: LoaderDeviceStatus =
            serde_json::from_slice(&call(&first, "GET", "/api/v1/status").await.1).unwrap();
        assert_ne!(rebooted.boot_epoch, initial.boot_epoch);
        assert_eq!(
            call(&first, "POST", "/api/v1/boot/jobs").await.0,
            StatusCode::NOT_IMPLEMENTED
        );
    }
}
