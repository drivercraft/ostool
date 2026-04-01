use anyhow::Context as _;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::sterm::{AsyncTerminal, TerminalConfig};

#[derive(Debug, Deserialize)]
pub(crate) struct ServerControlMessage {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: Option<String>,
}

pub async fn run_serial_terminal(ws_url: reqwest::Url) -> anyhow::Result<()> {
    let (stream, _) = tokio_tungstenite::connect_async(ws_url.as_str())
        .await
        .with_context(|| format!("failed to connect serial websocket {}", ws_url))?;
    let (mut sink, mut stream) = stream.split();

    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let read_task = tokio::spawn(async move {
        while let Some(message) = stream.next().await {
            match message.context("serial websocket read failed")? {
                Message::Binary(bytes) => {
                    if inbound_tx.send(bytes.to_vec()).is_err() {
                        break;
                    }
                }
                Message::Text(text) => {
                    if let Ok(control) = serde_json::from_str::<ServerControlMessage>(&text) {
                        match control.kind.as_str() {
                            "opened" | "closed" => continue,
                            "error" => {
                                let message = control
                                    .message
                                    .unwrap_or_else(|| "serial websocket error".to_string());
                                let formatted = format!("\n[ostool-server] {message}\n");
                                if inbound_tx.send(formatted.into_bytes()).is_err() {
                                    break;
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                    if inbound_tx.send(text.bytes().collect()).is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }

        Ok::<(), anyhow::Error>(())
    });

    let write_task = tokio::spawn(async move {
        while let Some(bytes) = outbound_rx.recv().await {
            sink.send(Message::Binary(bytes.into()))
                .await
                .context("serial websocket write failed")?;
        }

        let _ = sink
            .send(Message::Text(r#"{"type":"close"}"#.to_string().into()))
            .await;
        let _ = sink.send(Message::Close(None)).await;
        Ok::<(), anyhow::Error>(())
    });

    let terminal = AsyncTerminal::new(TerminalConfig {
        intercept_exit_sequence: true,
        timeout: None,
        timeout_label: "remote serial terminal".to_string(),
    });
    let run_result = terminal
        .run(inbound_rx, outbound_tx, |_handle, _byte| {})
        .await;

    read_task.abort();
    if let Err(err) = write_task.await
        && !err.is_cancelled()
    {
        log::debug!("serial websocket writer join error: {err}");
    }
    if let Err(err) = read_task.await
        && !err.is_cancelled()
    {
        log::debug!("serial websocket reader join error: {err}");
    }

    run_result
}

#[cfg(test)]
mod tests {
    use super::ServerControlMessage;

    #[test]
    fn parse_server_control_message() {
        let opened: ServerControlMessage = serde_json::from_str(r#"{"type":"opened"}"#).unwrap();
        assert_eq!(opened.kind, "opened");
    }

    #[test]
    fn parse_server_error_control_message() {
        let error: ServerControlMessage =
            serde_json::from_str(r#"{"type":"error","message":"power failed"}"#).unwrap();
        assert_eq!(error.kind, "error");
        assert_eq!(error.message.as_deref(), Some("power failed"));
    }
}
