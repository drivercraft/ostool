pub mod client;
pub mod config;
pub mod config_tui;
pub mod global_config;
pub mod request;
pub mod serial_stream;
pub mod session;
mod shared_files;
pub mod terminal;

pub use request::BoardRunRequest;

use std::{collections::BTreeMap, path::Path};

use anyhow::Context as _;

use crate::board::{
    client::{BoardServerClient, BoardTypeSummary},
    config::BoardRunConfig,
    config_tui::run_board_config_tui,
    global_config::{BoardEndpoint, LoadedBoardGlobalConfig},
    session::BoardSession,
    shared_files::expand_board_session_variables,
};
use crate::{
    build::config::{BuildConfig, BuildSystem, Cargo},
    invocation::Invocation,
    project::variables::{self, VariableScope},
    utils::format_local_time,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunBoardOptions {
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

pub async fn fetch_board_types_endpoint(
    endpoint: BoardEndpoint,
) -> anyhow::Result<Vec<BoardTypeSummary>> {
    let client = BoardServerClient::new_with_endpoint(endpoint)?;
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
    let tags_width = boards
        .iter()
        .map(|item| {
            if item.tags.is_empty() {
                1
            } else {
                item.tags.join(",").len()
            }
        })
        .max()
        .unwrap_or(1)
        .max("TAGS".len());
    let board_id_width = boards
        .iter()
        .filter_map(|item| item.leases.as_ref())
        .flatten()
        .map(|lease| lease.board_id.len())
        .max()
        .unwrap_or(1)
        .max("BOARD ID".len());
    let date_begin_width = boards
        .iter()
        .filter_map(|item| item.leases.as_ref())
        .flatten()
        .map(|lease| lease.date_begin.len())
        .max()
        .unwrap_or(1)
        .max("DATE BEGIN".len());

    let row_count = boards
        .iter()
        .map(|item| item.leases.as_ref().map_or(1, |leases| leases.len().max(1)))
        .sum::<usize>();
    let mut lines = Vec::with_capacity(row_count + 1);
    lines.push(format!(
        "{:<type_width$}  {:>avail_width$}  {:>total_width$}  {:<tags_width$}  {:<board_id_width$}  {:<date_begin_width$}  DATE END",
        "BOARD TYPE",
        "AVAILABLE",
        "TOTAL",
        "TAGS",
        "BOARD ID",
        "DATE BEGIN",
        type_width = type_width,
        avail_width = avail_width,
        total_width = total_width,
        tags_width = tags_width,
        board_id_width = board_id_width,
        date_begin_width = date_begin_width,
    ));

    for item in boards {
        let tags = if item.tags.is_empty() {
            "-".to_string()
        } else {
            item.tags.join(",")
        };
        let mut push_row = |board_id: &str, date_begin: &str, date_end: &str| {
            lines.push(format!(
                "{:<type_width$}  {:>avail_width$}  {:>total_width$}  {:<tags_width$}  {:<board_id_width$}  {:<date_begin_width$}  {}",
                item.board_type,
                item.available,
                item.total,
                tags,
                board_id,
                date_begin,
                date_end,
                type_width = type_width,
                avail_width = avail_width,
                total_width = total_width,
                tags_width = tags_width,
                board_id_width = board_id_width,
                date_begin_width = date_begin_width,
            ));
        };

        match item.leases.as_deref() {
            Some(leases) if !leases.is_empty() => {
                for lease in leases {
                    push_row(&lease.board_id, &lease.date_begin, &lease.date_end);
                }
            }
            Some(_) | None => push_row("-", "-", "-"),
        }
    }

    lines.join("\n")
}

pub async fn list_boards(server: &str, port: u16) -> anyhow::Result<()> {
    let boards = fetch_board_types(server, port).await?;
    println!("{}", render_board_table(&boards));
    Ok(())
}

pub async fn list_boards_endpoint(endpoint: BoardEndpoint) -> anyhow::Result<()> {
    let boards = fetch_board_types_endpoint(endpoint).await?;
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
    board_id: Option<&str>,
) -> anyhow::Result<(BoardServerClient, BoardSession)> {
    let client = BoardServerClient::new(server, port)?;
    let session = BoardSession::acquire(client.clone(), board_type, board_id)
        .await
        .with_context(|| match board_id {
            Some(board_id) => {
                format!("failed to acquire board type `{board_type}` with board id `{board_id}`")
            }
            None => format!("failed to acquire board type `{board_type}`"),
        })?;
    Ok((client, session))
}

pub async fn acquire_board_session_endpoint(
    endpoint: BoardEndpoint,
    board_type: &str,
    board_id: Option<&str>,
) -> anyhow::Result<(BoardServerClient, BoardSession)> {
    let client = BoardServerClient::new_with_endpoint(endpoint)?;
    let session = BoardSession::acquire(client.clone(), board_type, board_id)
        .await
        .with_context(|| match board_id {
            Some(board_id) => {
                format!("failed to acquire board type `{board_type}` with board id `{board_id}`")
            }
            None => format!("failed to acquire board type `{board_type}`"),
        })?;
    Ok((client, session))
}

pub async fn connect_board(
    server: &str,
    port: u16,
    board_type: &str,
    board_id: Option<&str>,
) -> anyhow::Result<()> {
    let (client, session) = acquire_board_session(server, port, board_type, board_id).await?;
    connect_allocated_board(client, session, board_type).await
}

pub async fn connect_board_endpoint(
    endpoint: BoardEndpoint,
    board_type: &str,
    board_id: Option<&str>,
) -> anyhow::Result<()> {
    let (client, session) = acquire_board_session_endpoint(endpoint, board_type, board_id).await?;
    connect_allocated_board(client, session, board_type).await
}

async fn connect_allocated_board(
    client: BoardServerClient,
    session: BoardSession,
    board_type: &str,
) -> anyhow::Result<()> {
    print_allocated_board_session(&session, board_type);

    let result = if session.info().serial_available {
        let ws_path = session
            .info()
            .ws_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("server did not return a serial websocket URL"))?;
        let ws_url = client.resolve_ws_url(ws_path)?;
        terminal::run_serial_terminal(ws_url, client.websocket_authorization().await?).await
    } else {
        let lease_expires_at = session.current_lease_expires_at().await;
        println!("Board has no serial configuration; keeping session alive until Ctrl+C.");
        println!(
            "  lease_expires_at: {}",
            format_local_time(lease_expires_at)
        );
        tokio::signal::ctrl_c()
            .await
            .context("failed to wait for Ctrl+C")?;
        Ok(())
    };

    finalize_session(session, result).await
}

pub(crate) fn print_allocated_board_session(session: &BoardSession, board_type: &str) {
    println!("Allocated board session:");
    println!("  board_type: {board_type}");
    println!("  board_id: {}", session.info().board_id);
    println!("  session_id: {}", session.info().session_id);
    println!(
        "  lease_expires_at: {}",
        format_local_time(session.info().lease_expires_at)
    );
    println!("  boot_mode: {}", session.info().boot_mode);
}

pub(crate) async fn finalize_session(
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

pub(crate) async fn read_board_run_config_from_path(
    scope: &VariableScope,
    path: &Path,
) -> anyhow::Result<BoardRunConfig> {
    let path = variables::expand_path_variables(path, scope)?;
    BoardRunConfig::read_from_path(scope, path)
}

/// Reads a board-run configuration from an explicit path.
pub async fn read_run_config_from_path(
    invocation: &Invocation,
    path: &Path,
) -> anyhow::Result<BoardRunConfig> {
    let scope = invocation.variable_scope()?;
    read_board_run_config_from_path(&scope, path).await
}

/// Reads a board-run configuration using the Cargo package variable scope.
pub async fn read_run_config_from_path_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
    path: &Path,
) -> anyhow::Result<BoardRunConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    read_board_run_config_from_path(&scope, path).await
}

pub(crate) async fn ensure_board_run_config_in_dir(
    scope: &VariableScope,
    dir: &Path,
) -> anyhow::Result<BoardRunConfig> {
    let dir = variables::expand_path_variables(dir, scope)?;
    BoardRunConfig::load_or_create(scope, Some(dir.join(".board.toml"))).await
}

/// Loads or creates a board-run configuration from a directory.
pub async fn ensure_run_config_in_dir(
    invocation: &Invocation,
    dir: &Path,
) -> anyhow::Result<BoardRunConfig> {
    let scope = invocation.variable_scope()?;
    ensure_board_run_config_in_dir(&scope, dir).await
}

/// Loads or creates a board-run configuration using the Cargo package variable scope.
pub async fn ensure_run_config_in_dir_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
    dir: &Path,
) -> anyhow::Result<BoardRunConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    ensure_board_run_config_in_dir(&scope, dir).await
}

/// Builds/imports artifacts and runs them on a remote board.
///
/// `build_config_path` is the optional source path for `build_config`.
pub async fn run_board(
    invocation: &mut Invocation,
    build_config: &BuildConfig,
    build_config_path: Option<&Path>,
    board_config: &BoardRunConfig,
    options: RunBoardOptions,
) -> anyhow::Result<()> {
    crate::build::prepare_runtime_artifacts(invocation, build_config, build_config_path, false)
        .await?;
    run_prepared_board(invocation, board_config, options).await
}

/// Builds a Cargo artifact and runs it on a remote board.
///
/// `build_config_path` is the optional `.build.toml` source path for `cargo`.
pub async fn cargo_run_board(
    invocation: &mut Invocation,
    cargo: &Cargo,
    build_config_path: Option<&Path>,
    board_config: &BoardRunConfig,
    options: RunBoardOptions,
) -> anyhow::Result<()> {
    run_board(
        invocation,
        &BuildConfig {
            system: BuildSystem::Cargo(Box::new(cargo.clone())),
        },
        build_config_path,
        board_config,
        options,
    )
    .await
}

/// Runs already prepared runtime artifacts on a remote board.
///
/// The invocation must have runtime artifacts prepared by a previous build or by
/// `ostool::build::prepare_runtime_artifact`.
pub async fn run_prepared_board(
    invocation: &mut Invocation,
    board_config: &BoardRunConfig,
    options: RunBoardOptions,
) -> anyhow::Result<()> {
    if !board_config.session_files.is_empty() {
        anyhow::bail!(
            "board config contains `session_files`; use BoardRunRequest::with_session_files to provide the configuration directory"
        );
    }
    run_prepared_board_with_request(
        invocation,
        BoardRunRequest::new(board_config.clone(), options),
    )
    .await
}

pub async fn run_prepared_board_with_request(
    invocation: &mut Invocation,
    request: BoardRunRequest,
) -> anyhow::Result<()> {
    let scope = invocation.variable_scope()?;
    let global_config = load_board_global_config_with_notice()?;
    let (mut board_config, options, session_files) = request.into_parts();
    board_config.apply_overrides(
        &scope,
        options.board_type.as_deref(),
        options.server.as_deref(),
        options.port,
    )?;

    let endpoint = board_config.resolve_endpoint(None, None, &global_config.board)?;
    let (client, session) =
        acquire_board_session_endpoint(endpoint, &board_config.board_type, None).await?;
    print_allocated_board_session(&session, &board_config.board_type);

    let setup_result = async {
        if !board_session_setup_required(&board_config, !session_files.is_empty()) {
            return Ok(());
        }
        let context = session.context().await?;
        let mut uploaded_files = BTreeMap::new();
        for upload in session_files {
            let relative_path = upload.relative_path().to_string();
            let bytes = upload.read().await?;
            let shared_file = session.upload_shared_file(&relative_path, bytes).await?;
            uploaded_files.insert(relative_path, shared_file.http_url);
        }
        expand_board_session_variables(&mut board_config, &context, &uploaded_files)
    }
    .await;

    let run_result = match setup_result {
        Err(error) => Err(error),
        Ok(()) => run_allocated_board(invocation, &board_config, client, &session).await,
    };

    finalize_session(session, run_result).await
}

fn board_session_setup_required(board_config: &BoardRunConfig, has_session_files: bool) -> bool {
    has_session_files
        || board_config.shell_check_steps.iter().any(|step| {
            step.shell_cmd.as_deref().is_some_and(|command| {
                command.contains("${boardServer") || command.contains("${sessionFile")
            })
        })
}

async fn run_allocated_board(
    invocation: &mut Invocation,
    board_config: &BoardRunConfig,
    client: BoardServerClient,
    session: &BoardSession,
) -> anyhow::Result<()> {
    match session.info().boot_mode.as_str() {
        "uboot" => {
            invocation.ensure_runtime_bin()?;
            let input = crate::run::uboot::uboot_run_input(invocation)?;
            crate::run::uboot::run_uboot_remote(input, board_config, client, session.info().clone())
                .await
        }
        "httpboot" | "uefi_http" => {
            let input = crate::run::uboot::uboot_run_input(invocation)?;
            crate::run::httpboot_board::run_httpboot_remote(
                input,
                board_config,
                client,
                session.info().clone(),
            )
            .await
        }
        other => Err(anyhow!(
            "unsupported board boot mode `{other}`; supported modes are `uboot` and `httpboot`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{RunBoardOptions, board_session_setup_required, render_board_table};
    use crate::board::client::{BoardLease, BoardTypeSummary};
    use crate::board::config::BoardRunConfig;

    #[test]
    fn run_board_args_default_to_no_overrides() {
        assert_eq!(RunBoardOptions::default().board_type, None);
    }

    #[test]
    fn legacy_board_runs_do_not_require_session_network_context() {
        assert!(!board_session_setup_required(
            &BoardRunConfig::default(),
            false
        ));
    }

    #[test]
    fn shared_files_and_reserved_variables_require_session_setup() {
        assert!(board_session_setup_required(
            &BoardRunConfig::default(),
            true
        ));
        assert!(board_session_setup_required(
            &BoardRunConfig {
                shell_check_steps: vec![crate::run::ShellCheckStep {
                    shell_prefix: Some("root#".into()),
                    shell_cmd: Some("echo ${boardServerIp}".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            false
        ));
    }

    #[test]
    fn render_board_table_formats_rows() {
        let rendered = render_board_table(&[BoardTypeSummary {
            board_type: "rk3568".into(),
            tags: vec!["arm64".into(), "lab".into()],
            total: 3,
            available: 2,
            leases: None,
        }]);

        assert!(rendered.contains("BOARD TYPE"));
        assert!(rendered.contains("BOARD ID"));
        assert!(rendered.contains("DATE BEGIN"));
        assert!(rendered.contains("DATE END"));
        assert!(rendered.contains("rk3568"));
        assert!(rendered.contains("arm64,lab"));
        assert_eq!(
            rendered
                .lines()
                .nth(1)
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>(),
            vec!["rk3568", "2", "3", "arm64,lab", "-", "-", "-"]
        );
    }

    #[test]
    fn render_board_table_places_each_lease_on_its_own_row() {
        let rendered = render_board_table(&[BoardTypeSummary {
            board_type: "Rock-4D".into(),
            tags: vec![],
            total: 2,
            available: 0,
            leases: Some(vec![
                BoardLease {
                    board_id: "Rock-4D-1".into(),
                    date_begin: "2026-07-31 15:40:00".into(),
                    date_end: "2026-08-03 15:40:00".into(),
                },
                BoardLease {
                    board_id: "Rock-4D-2".into(),
                    date_begin: "2026-08-01 10:00:00".into(),
                    date_end: "2026-08-02 10:00:00".into(),
                },
            ]),
        }]);

        let rows = rendered.lines().skip(1).collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("Rock-4D-1"));
        assert!(rows[0].contains("2026-07-31 15:40:00"));
        assert!(rows[0].contains("2026-08-03 15:40:00"));
        assert!(rows[1].contains("Rock-4D-2"));
        assert!(rows[1].contains("2026-08-01 10:00:00"));
        assert!(rows[1].contains("2026-08-02 10:00:00"));
    }

    #[test]
    fn render_board_table_handles_empty_leases() {
        let rendered = render_board_table(&[BoardTypeSummary {
            board_type: "Rock-4D".into(),
            tags: vec![],
            total: 1,
            available: 1,
            leases: Some(vec![]),
        }]);

        let rows = rendered.lines().skip(1).collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].split_whitespace().collect::<Vec<_>>(),
            vec!["Rock-4D", "1", "1", "-", "-", "-", "-"]
        );
    }

    #[test]
    fn render_board_table_handles_empty_results() {
        assert_eq!(render_board_table(&[]), "No board types found.");
    }
}
