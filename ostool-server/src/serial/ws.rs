use std::time::Duration;

use anyhow::Context;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::SerialPortBuilderExt;

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
    let port = tokio_serial::new(&serial.port, serial.baud_rate)
        .timeout(Duration::from_millis(200))
        .open_native_async()
        .with_context(|| format!("failed to open serial port {}", serial.port))?;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (mut serial_rx, mut serial_tx) = tokio::io::split(port);
    let mut serial_buffer = [0u8; 1024];

    ws_sender
        .send(Message::Text(r#"{"type":"opened"}"#.to_string().into()))
        .await
        .ok();

    loop {
        tokio::select! {
            maybe_message = ws_receiver.next() => {
                let Some(message) = maybe_message else {
                    break;
                };
                match message {
                    Ok(Message::Binary(bytes)) => {
                        write_serial_payload(&mut serial_tx, &bytes).await?;
                    }
                    Ok(Message::Text(text)) => {
                        let control: ClientControlMessage = serde_json::from_str(&text)?;
                        match control.kind.as_str() {
                            "close" => {
                                let _ = ws_sender
                                    .send(Message::Text(r#"{"type":"closed"}"#.to_string().into()))
                                    .await;
                                break;
                            }
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
                                write_serial_payload(&mut serial_tx, &payload).await?;
                            }
                            other => anyhow::bail!("unsupported websocket control type `{other}`"),
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(payload)) => {
                        ws_sender.send(Message::Pong(payload)).await.ok();
                    }
                    Ok(Message::Pong(_)) => {}
                    Err(err) => return Err(err.into()),
                }
                let _ = state.touch_session(session_id).await;
            }
            read = serial_rx.read(&mut serial_buffer) => {
                let read = read.context("serial read failed")?;
                if read == 0 {
                    break;
                }
                ws_sender
                    .send(Message::Binary(serial_buffer[..read].to_vec().into()))
                    .await
                    .context("failed to send serial output over websocket")?;
                let _ = state.touch_session(session_id).await;
            }
        }
    }

    let _ = ws_sender.send(Message::Close(None)).await;
    Ok(())
}

async fn write_serial_payload(
    port: &mut tokio::io::WriteHalf<tokio_serial::SerialStream>,
    payload: &[u8],
) -> anyhow::Result<()> {
    port.write_all(payload).await?;
    port.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ClientControlMessage;

    #[test]
    fn control_message_parses_close_type() {
        let message: ClientControlMessage = serde_json::from_str(r#"{"type":"close"}"#).unwrap();
        assert_eq!(message.kind, "close");
    }
}
