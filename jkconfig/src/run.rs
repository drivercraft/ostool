use std::path::Path;

use anyhow::Context;
pub use cursive;
use cursive::{Cursive, CursiveExt, event::Key};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::{
    data::AppData,
    ui::{components::menu::menu_view, handle_back, handle_quit, handle_save},
};

pub use crate::data::app_data::ElemHock;

/// Run the configuration editor workflow for a typed config.
///
/// When `always_use_ui` is false and the config file can be parsed,
/// the parsed config is returned without launching the UI.
///
/// # Errors
///
/// Returns errors when schema generation, parsing, or I/O fails.
pub async fn run<C: JsonSchema + DeserializeOwned>(
    config_path: impl AsRef<Path>,
    always_use_ui: bool,
    elem_hocks: &[ElemHock],
) -> anyhow::Result<Option<C>> {
    let config_path = config_path.as_ref();
    let schema = schemars::schema_for!(C);
    let schema_json = serde_json::to_value(&schema)?;

    let content = tokio::fs::read_to_string(&config_path)
        .await
        .unwrap_or_default();

    let ext = config_path
        .extension()
        .map(|s| format!("{}", s.display()))
        .unwrap_or(String::new());

    if let Ok(c) = to_typed::<C>(&content, &ext)
        && !always_use_ui
    {
        return Ok(Some(c));
    }

    let app = get_content_by_ui(config_path, &content, &schema_json, elem_hocks).await?;
    if !app.needs_save {
        return Ok(None);
    }
    let val = app.root.as_json();

    let c = match ext.as_str() {
        "json" => serde_json::from_value(val.clone())?,
        "toml" => {
            let content = toml::to_string_pretty(&val)?;
            toml::from_str(&content)?
        }
        _ => {
            anyhow::bail!("unsupported config file extension: {ext}",);
        }
    };

    // Write the content based on the format
    match ext.as_str() {
        "json" => {
            let content = serde_json::to_string_pretty(&val)?;
            tokio::fs::write(&config_path, content)
                .await
                .with_context(|| format!("Failed to write {}", config_path.display()))?;
        }
        "toml" => {
            let content = toml::to_string_pretty(&val)?;
            tokio::fs::write(&config_path, content)
                .await
                .with_context(|| format!("Failed to write {}", config_path.display()))?;
        }
        _ => {
            anyhow::bail!("unsupported config file extension: {ext}",);
        }
    }

    Ok(Some(c))
}

fn to_typed<C: JsonSchema + DeserializeOwned>(s: &str, ext: &str) -> anyhow::Result<C> {
    let c = match ext {
        "json" => serde_json::from_str::<C>(s)?,
        "toml" => toml::from_str::<C>(s)?,
        _ => {
            anyhow::bail!("unsupported config file extension: {ext}",);
        }
    };
    Ok(c)
}

async fn get_content_by_ui(
    config: impl AsRef<Path>,
    content: &str,
    schema: &serde_json::Value,
    elem_hocks: &[ElemHock],
) -> anyhow::Result<AppData> {
    let mut app_data = AppData::new_with_init_and_schema(content, config.as_ref(), schema)?;
    app_data.elem_hocks = elem_hocks.to_vec();

    let title = app_data.root.title.clone();
    let fields = app_data.root.menu().fields();

    #[cfg(feature = "logging")]
    {
        cursive::logger::init();
        cursive::logger::set_filter_levels_from_env();
    }
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
    siv.add_global_callback('~', cursive::Cursive::toggle_debug_console);
    // 初始菜单路径为空
    siv.add_fullscreen_layer(menu_view(&title, "", fields));

    // 运行应用
    siv.run();

    let app = siv.take_user_data::<AppData>().unwrap();
    // println!("Data: \n{:#?}", app.root);
    Ok(app)
}
