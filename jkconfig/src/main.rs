use clap::{Parser, Subcommand};
use cursive::{Cursive, CursiveExt, event::Key};
use std::path::PathBuf;

use jkconfig::{
    data::AppData,
    ui::{components::menu::menu_view, handle_back, handle_quit, handle_save},
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
    let app_data = AppData::new(config_file, schema_file)?;

    // 根据子命令决定运行模式
    match cli.command {
        Some(Commands::Web { port }) => {
            tokio::runtime::Runtime::new()?.block_on(jkconfig::web::run_server(app_data, port))?;
        }
        Some(Commands::Tui) | None => {
            // 运行TUI界面（默认行为）
            run_tui(app_data)?;
        }
    }

    Ok(())
}

/// 运行TUI界面
fn run_tui(app_data: AppData) -> anyhow::Result<()> {
    let title = app_data.root.title.clone();
    let fields = app_data.root.menu().fields();

    // 创建Cursive应用
    let mut siv = Cursive::default();

    // 设置AppData为user_data
    siv.set_user_data(app_data);

    // 添加全局键盘事件处理
    siv.add_global_callback('q', handle_quit);
    siv.add_global_callback('Q', handle_quit);
    siv.add_global_callback('s', handle_save);
    siv.add_global_callback('S', handle_save);
    siv.add_global_callback(Key::Esc, handle_back);
    // 初始菜单路径为空
    siv.add_fullscreen_layer(menu_view(&title, "", fields));

    // 运行应用
    siv.run();

    println!("Exiting jkconfig...");
    let mut app = siv.take_user_data::<AppData>().unwrap();
    println!("Data: \n{:#?}", app.root);
    app.on_exit()?;

    Ok(())
}
