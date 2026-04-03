pub mod client;
pub mod config;
pub mod config_tui;
pub mod global_config;
pub mod serial_stream;
pub mod session;
pub mod terminal;

use std::path::PathBuf;

use anyhow::Context as _;
use log::info;

use crate::board::{
    client::{BoardServerClient, BoardTypeSummary},
    config::BoardRunConfig,
    config_tui::run_board_config_tui,
    global_config::LoadedBoardGlobalConfig,
    session::BoardSession,
};
use crate::{
    Tool,
    build::{
        cargo_builder::CargoBuilder,
        config::{BuildConfig, BuildSystem, Cargo},
    },
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunBoardArgs {
    pub config: Option<PathBuf>,
    pub board_config: Option<PathBuf>,
    pub board_type: Option<String>,
    pub server: Option<String>,
    pub port: Option<u16>,
}

pub async fn fetch_board_types(server: &str, port: u16) -> anyhow::Result<Vec<BoardTypeSummary>> {
    let client = BoardServerClient::new(server, port)?;
    let mut boards = client
        .list_board_types()
        .await
        .context("failed to list board types")?;
    boards.sort_by(|a, b| a.board_type.cmp(&b.board_type));
    Ok(boards)
}

pub fn render_board_table(boards: &[BoardTypeSummary]) -> String {
    if boards.is_empty() {
        return "No board types found.".to_string();
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

    let mut lines = Vec::with_capacity(boards.len() + 1);
    lines.push(format!(
        "{:<type_width$}  {:>avail_width$}  {:>total_width$}  TAGS",
        "BOARD TYPE",
        "AVAILABLE",
        "TOTAL",
        type_width = type_width,
        avail_width = avail_width,
        total_width = total_width,
    ));

    for item in boards {
        let tags = if item.tags.is_empty() {
            "-".to_string()
        } else {
            item.tags.join(",")
        };
        lines.push(format!(
            "{:<type_width$}  {:>avail_width$}  {:>total_width$}  {}",
            item.board_type,
            item.available,
            item.total,
            tags,
            type_width = type_width,
            avail_width = avail_width,
            total_width = total_width,
        ));
    }

    lines.join("\n")
}

pub async fn list_boards(server: &str, port: u16) -> anyhow::Result<()> {
    let boards = fetch_board_types(server, port).await?;
    println!("{}", render_board_table(&boards));
    Ok(())
}

pub fn config() -> anyhow::Result<()> {
    run_board_config_tui()
}

pub fn load_board_global_config_with_notice() -> anyhow::Result<LoadedBoardGlobalConfig> {
    let loaded = LoadedBoardGlobalConfig::load_or_create()?;
    if loaded.created {
        println!("Created default board config: {}", loaded.path.display());
    }
    Ok(loaded)
}

pub async fn acquire_board_session(
    server: &str,
    port: u16,
    board_type: &str,
) -> anyhow::Result<(BoardServerClient, BoardSession)> {
    let client = BoardServerClient::new(server, port)?;
    let session = BoardSession::acquire(client.clone(), board_type)
        .await
        .with_context(|| format!("failed to acquire board type `{board_type}`"))?;
    Ok((client, session))
}

pub async fn connect_board(server: &str, port: u16, board_type: &str) -> anyhow::Result<()> {
    let (client, session) = acquire_board_session(server, port, board_type).await?;
    print_allocated_board_session(&session, board_type);

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

    finalize_session(session, result).await
}

fn print_allocated_board_session(session: &BoardSession, board_type: &str) {
    println!("Allocated board session:");
    println!("  board_type: {board_type}");
    println!("  board_id: {}", session.info().board_id);
    println!("  session_id: {}", session.info().session_id);
    println!("  lease_expires_at: {}", session.info().lease_expires_at);
    println!("  boot_mode: {}", session.info().boot_mode);
}

async fn finalize_session(
    session: BoardSession,
    run_result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    let release_result = session.release().await;
    match (run_result, release_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Err(run_err), Err(release_err)) => Err(run_err.context(format!(
            "additionally failed to release board session: {release_err:#}"
        ))),
    }
}

impl Tool {
    pub async fn run_board(&mut self, args: RunBoardArgs) -> anyhow::Result<()> {
        let build_config = self
            .prepare_build_config(args.config.clone(), false)
            .await?;
        self.run_board_with_build_config(&build_config, args).await
    }

    pub async fn cargo_run_board(
        &mut self,
        cargo: &Cargo,
        args: RunBoardArgs,
    ) -> anyhow::Result<()> {
        let build_config_path = self.ctx.build_config_path.clone();
        CargoBuilder::build(self, cargo, build_config_path)
            .skip_objcopy(true)
            .resolve_artifact_from_json(true)
            .execute()
            .await?;

        self.run_prepared_board(args).await
    }

    async fn run_board_with_build_config(
        &mut self,
        build_config: &BuildConfig,
        args: RunBoardArgs,
    ) -> anyhow::Result<()> {
        match &build_config.system {
            BuildSystem::Cargo(cargo) => self.cargo_run_board(cargo, args).await,
            BuildSystem::Custom(custom_cfg) => {
                self.shell_run_cmd(&custom_cfg.build_cmd)?;
                self.set_elf_path(custom_cfg.elf_path.clone().into())
                    .await?;
                info!(
                    "ELF {:?}: {}",
                    self.ctx().arch,
                    self.ctx().artifacts.elf.as_ref().unwrap().display()
                );

                if custom_cfg.to_bin {
                    self.objcopy_output_bin()?;
                }

                self.run_prepared_board(args).await
            }
        }
    }

    async fn run_prepared_board(&mut self, args: RunBoardArgs) -> anyhow::Result<()> {
        let global_config = load_board_global_config_with_notice()?;
        let explicit_board_config = args
            .board_config
            .map(|path| self.replace_path_variables(path))
            .transpose()?;
        let mut board_config = BoardRunConfig::load_or_create(self, explicit_board_config).await?;
        board_config.apply_overrides(
            self,
            args.board_type.as_deref(),
            args.server.as_deref(),
            args.port,
        )?;

        let (server, port) = board_config.resolve_server(None, None, &global_config.board);
        let (client, session) =
            acquire_board_session(&server, port, &board_config.board_type).await?;
        print_allocated_board_session(&session, &board_config.board_type);

        let run_result = match session.info().boot_mode.as_str() {
            "uboot" => {
                self.run_uboot_remote(&board_config, client, session.info().clone())
                    .await
            }
            other => Err(anyhow!(
                "unsupported board boot mode `{other}`; only `uboot` is supported"
            )),
        };

        finalize_session(session, run_result).await
    }
}

#[cfg(test)]
mod tests {
    use super::{RunBoardArgs, render_board_table};
    use crate::board::client::BoardTypeSummary;

    #[test]
    fn run_board_args_default_to_no_overrides() {
        assert_eq!(RunBoardArgs::default().board_type, None);
    }

    #[test]
    fn render_board_table_formats_rows() {
        let rendered = render_board_table(&[BoardTypeSummary {
            board_type: "rk3568".into(),
            tags: vec!["arm64".into(), "lab".into()],
            total: 3,
            available: 2,
        }]);

        assert!(rendered.contains("BOARD TYPE"));
        assert!(rendered.contains("rk3568"));
        assert!(rendered.contains("arm64,lab"));
    }

    #[test]
    fn render_board_table_handles_empty_results() {
        assert_eq!(render_board_table(&[]), "No board types found.");
    }
}
