use clap::{Parser, Subcommand};
use std::path::PathBuf;

use jkconfig::{
    data::{AppState, ConfigDocument},
    ui::run_tui as launch_tui,
};

// mod menu_view;
// use menu_view::MenuView;

/// 命令行参数结构体
#[derive(Parser)]
#[command(name = "jkconfig")]
#[command(author = "周睿 <zrufo747@outlook.com>")]
#[command(about = "配置编辑器", long_about = None)]
struct Cli {
    /// config file path
    #[arg(short = 'c', long = "config", default_value = ".config.toml")]
    config: PathBuf,

    /// schema file path, default is config file name with '-schema.json' suffix
    #[arg(short = 's', long = "schema")]
    schema: Option<PathBuf>,

    /// 子命令
    #[command(subcommand)]
    command: Option<Commands>,
}

/// 子命令枚举
#[derive(Subcommand)]
enum Commands {
    /// TUI (default)
    Tui,
    /// Web UI mode
    Web {
        /// server port
        #[arg(short = 'p', long = "port", default_value = "3000")]
        port: u16,
    },
}

/// 主函数
fn main() -> anyhow::Result<()> {
    // 解析命令行参数
    let cli = Cli::parse();

    // 提取命令行参数
    let config_path = cli.config.to_string_lossy().to_string();
    let schema_path = cli.schema.as_ref().map(|p| p.to_string_lossy().to_string());

    let config_file = Some(config_path.as_str());
    let schema_file = schema_path.as_deref();

    // 初始化AppData
    let document = ConfigDocument::new(config_file, schema_file)?;
    let app_state = AppState::new(document);

    // 根据子命令决定运行模式
    match cli.command {
        Some(Commands::Web { port }) => {
            tokio::runtime::Runtime::new()?.block_on(jkconfig::web::run_server(app_state, port))?;
        }
        Some(Commands::Tui) | None => {
            // 运行TUI界面（默认行为）
            run_tui(app_state)?;
        }
    }

    Ok(())
}

/// 运行TUI界面
fn run_tui(app_state: AppState) -> anyhow::Result<()> {
    let mut app = launch_tui(app_state)?;
    app.persist_if_needed()?;
    Ok(())
}
