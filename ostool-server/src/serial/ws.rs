use std::{
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::{sync::mpsc, task::spawn_blocking};

use crate::{config::SerialConfig, state::AppState};

#[derive(Debug, Deserialize)]
struct ClientControlMessage {
    #[serde(rename = "type")]
    kind: String,
    encoding: Option<String>,
    data: Option<String>,
}

pub async fn run_serial_ws(
    socket: WebSocket,
    state: AppState,
    session_id: String,
    serial: SerialConfig,
) {
    let result = run_serial_ws_inner(socket, &state, &session_id, &serial).await;
    if let Err(err) = result {
        log::warn!("serial websocket ended with error: {err:#}");
    }
    state
        .active_serial_sessions
        .write()
        .await
        .remove(&session_id);
}

async fn run_serial_ws_inner(
    socket: WebSocket,
    state: &AppState,
    session_id: &str,
    serial: &SerialConfig,
) -> anyhow::Result<()> {
    let rx_port = serialport::new(&serial.port, serial.baud_rate)
        .timeout(Duration::from_millis(200))
        .open()
        .with_context(|| format!("failed to open serial port {}", serial.port))?;
    let tx_port = rx_port
        .try_clone()
        .with_context(|| format!("failed to clone serial port {}", serial.port))?;

    let rx_port = Arc::new(Mutex::new(rx_port));
    let tx_port = Arc::new(Mutex::new(tx_port));
    let stop = Arc::new(AtomicBool::new(false));

    let (mut ws_sender, mut ws_receiver) = socket.split();
    ws_sender
        .send(Message::Text(r#"{"type":"opened"}"#.to_string().into()))
        .await
        .ok();

    let (serial_tx, mut serial_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let read_stop = stop.clone();
    let read_rx_port = rx_port.clone();
    let read_task = spawn_blocking(move || {
        let mut buffer = [0u8; 1024];
        while !read_stop.load(Ordering::Acquire) {
            match read_rx_port.lock().unwrap().read(&mut buffer) {
                Ok(read) if read > 0 => {
                    if serial_tx.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {}
                Err(err) => {
                    log::warn!("serial read failed: {err}");
                    break;
                }
            }
        }
    });

    let send_task = tokio::spawn(async move {
        while let Some(bytes) = serial_rx.recv().await {
            if ws_sender.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = ws_receiver.next().await {
        match message {
            Ok(Message::Binary(bytes)) => {
                tx_port.lock().unwrap().write_all(&bytes)?;
            }
            Ok(Message::Text(text)) => {
                let control: ClientControlMessage = serde_json::from_str(&text)?;
                match control.kind.as_str() {
                    "close" => break,
                    "tx" => {
                        let Some(data) = control.data.as_deref() else {
                            anyhow::bail!("missing tx data");
                        };
                        let payload = match control.encoding.as_deref() {
                            Some("base64") => base64::engine::general_purpose::STANDARD
                                .decode(data)
                                .context("invalid base64 payload")?,
                            Some("utf8") | None => data.as_bytes().to_vec(),
                            Some(other) => anyhow::bail!("unsupported encoding `{other}`"),
                        };
                        tx_port.lock().unwrap().write_all(&payload)?;
                    }
                    other => anyhow::bail!("unsupported websocket control type `{other}`"),
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
            Err(err) => return Err(err.into()),
        }
        let _ = state.touch_session(session_id).await;
    }

    stop.store(true, Ordering::Release);
    read_task.await?;
    send_task.abort();
    Ok(())
}
