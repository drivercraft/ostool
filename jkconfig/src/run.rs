use std::path::Path;

use anyhow::Context;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::{
    data::{AppState, ConfigDocument, ElementHook},
    ui::run_tui,
};

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
    element_hooks: &[ElementHook],
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

    let app = get_content_by_ui(config_path, &content, &schema_json, element_hooks).await?;
    if !app.needs_save {
        return Ok(None);
    }
    app.validate_before_save()?;
    let val = app.document.as_json();

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
    element_hooks: &[ElementHook],
) -> anyhow::Result<AppState> {
    let document = ConfigDocument::new_with_init_and_schema(content, config.as_ref(), schema)?;
    let mut app_state = AppState::new(document);
    app_state.element_hooks = element_hooks.to_vec();
    run_tui(app_state)
        .map_err(|err| anyhow::anyhow!("failed to launch interactive config UI: {err}"))
}
