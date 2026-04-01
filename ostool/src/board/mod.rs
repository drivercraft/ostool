pub mod client;
pub mod config;
pub mod serial_stream;
pub mod session;
pub mod terminal;

use anyhow::Context as _;

use crate::board::{
    client::{BoardServerClient, BoardTypeSummary},
    session::BoardSession,
};

pub async fn list_boards(server: &str, port: u16) -> anyhow::Result<()> {
    let client = BoardServerClient::new(server, port)?;
    let mut boards = client
        .list_board_types()
        .await
        .context("failed to list board types")?;
    boards.sort_by(|a, b| a.board_type.cmp(&b.board_type));
    print_board_table(&boards);
    Ok(())
}

pub async fn run_board(server: &str, port: u16, board_type: &str) -> anyhow::Result<()> {
    let client = BoardServerClient::new(server, port)?;
    let session = BoardSession::acquire(client.clone(), board_type)
        .await
        .with_context(|| format!("failed to acquire board type `{board_type}`"))?;

    println!("Allocated board session:");
    println!("  board_type: {board_type}");
    println!("  board_id: {}", session.info().board_id);
    println!("  session_id: {}", session.info().session_id);
    println!("  lease_expires_at: {}", session.info().lease_expires_at);
    println!("  boot_mode: {}", session.info().boot_mode);

    let result = if session.info().serial_available {
        let ws_path = session
            .info()
            .ws_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("server did not return a serial websocket URL"))?;
        let ws_url = client.resolve_ws_url(ws_path)?;
        terminal::run_serial_terminal(ws_url).await
    } else {
        let lease_expires_at = session.current_lease_expires_at().await;
        println!("Board has no serial configuration; keeping session alive until Ctrl+C.");
        println!("  lease_expires_at: {lease_expires_at}");
        tokio::signal::ctrl_c()
            .await
            .context("failed to wait for Ctrl+C")?;
        Ok(())
    };

    let release_result = session.release().await;
    match (result, release_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Err(run_err), Err(release_err)) => Err(run_err.context(format!(
            "additionally failed to release board session: {release_err:#}"
        ))),
    }
}

fn print_board_table(boards: &[BoardTypeSummary]) {
    if boards.is_empty() {
        println!("No board types found.");
        return;
    }

    let type_width = boards
        .iter()
        .map(|item| item.board_type.len())
        .max()
        .unwrap_or(10)
        .max("BOARD TYPE".len());
    let avail_width = boards
        .iter()
        .map(|item| item.available.to_string().len())
        .max()
        .unwrap_or(1)
        .max("AVAILABLE".len());
    let total_width = boards
        .iter()
        .map(|item| item.total.to_string().len())
        .max()
        .unwrap_or(1)
        .max("TOTAL".len());

    println!(
        "{:<type_width$}  {:>avail_width$}  {:>total_width$}  TAGS",
        "BOARD TYPE",
        "AVAILABLE",
        "TOTAL",
        type_width = type_width,
        avail_width = avail_width,
        total_width = total_width,
    );
    for item in boards {
        let tags = if item.tags.is_empty() {
            "-".to_string()
        } else {
            item.tags.join(",")
        };
        println!(
            "{:<type_width$}  {:>avail_width$}  {:>total_width$}  {}",
            item.board_type,
            item.available,
            item.total,
            tags,
            type_width = type_width,
            avail_width = avail_width,
            total_width = total_width,
        );
    }
}
