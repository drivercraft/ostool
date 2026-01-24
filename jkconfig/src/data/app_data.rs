use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use anyhow::bail;
use cursive::Cursive;

use crate::data::{menu::MenuRoot, types::ElementType};

/// Callback used to provide the list of available features.
pub type FeaturesCallback = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Callback invoked when a menu element is entered.
pub type HockCallback = Arc<dyn Fn(&mut Cursive, &str) + Send + Sync>;

/// Hook registration for a specific menu path.
#[derive(Clone)]
pub struct ElemHock {
    /// Menu path string (dot-separated).
    pub path: String,
    /// Callback executed when entering the path.
    pub callback: HockCallback,
}

/// Application state container for schema-driven config editing.
#[derive(Clone)]
pub struct AppData {
    /// Root menu parsed from JSON Schema.
    pub root: MenuRoot,
    /// Current menu path as a list of keys.
    pub current_key: Vec<String>,
    /// Whether configuration has pending changes.
    pub needs_save: bool,
    /// Path to the configuration file.
    pub config: PathBuf,
    /// Custom user data storage.
    pub user_data: HashMap<String, String>,
    /// Temporary data used by editors.
    pub temp_data: Option<(String, serde_json::Value)>,
    /// Registered element hooks.
    pub elem_hocks: Vec<ElemHock>,
}

const DEFAULT_CONFIG_PATH: &str = ".config.toml";

/// Derive a default schema path from a config path.
pub fn default_schema_by_init(config: &Path) -> PathBuf {
    let binding = config.file_name().unwrap().to_string_lossy();
    let mut name_split = binding.split(".").collect::<Vec<_>>();
    if name_split.len() > 1 {
        name_split.pop();
    }

    let name = format!("{}-schema.json", name_split.join("."));

    if let Some(parent) = config.parent() {
        parent.join(name)
    } else {
        PathBuf::from(name)
    }
}

impl AppData {
    /// Build `AppData` from optional config and schema paths.
    ///
    /// When schema is not provided, it is auto-derived from the config path.
    pub fn new(
        config: Option<impl AsRef<Path>>,
        schema: Option<impl AsRef<Path>>,
    ) -> anyhow::Result<Self> {
        let init_value_path = Self::init_value_path(config);

        let schema_path = if let Some(sch) = schema {
            sch.as_ref().to_path_buf()
        } else {
            default_schema_by_init(&init_value_path)
        };

        if !schema_path.exists() {
            bail!("Schema file does not exist: {}", schema_path.display());
        }

        let schema_content = fs::read_to_string(&schema_path)?;
        let schema_json: serde_json::Value = serde_json::from_str(&schema_content)?;
        Self::new_with_schema(Some(init_value_path), &schema_json)
    }

    fn init_value_path(config: Option<impl AsRef<Path>>) -> PathBuf {
        let mut init_value_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        if let Some(cfg) = config {
            init_value_path = cfg.as_ref().to_path_buf();
        }
        init_value_path
    }

    /// Build `AppData` from an initial content string and a schema value.
    ///
    /// This is useful when config content has already been loaded.
    pub fn new_with_init_and_schema(
        init: &str,
        init_value_path: &Path,
        schema: &serde_json::Value,
    ) -> anyhow::Result<Self> {
        let mut root = MenuRoot::try_from(schema)?;

        if !init.trim().is_empty() {
            let init_json: serde_json::Value = match init_value_path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
            {
                "json" => serde_json::from_str(init)?,
                "toml" => {
                    let v: toml::Value = toml::from_str(init)?;
                    serde_json::to_value(v)?
                }
                ext => {
                    bail!("Unsupported config file extension: {ext:?}");
                }
            };
            root.update_by_value(&init_json)?;
        }

        Ok(AppData {
            root,
            current_key: Vec::new(),
            needs_save: false,
            config: init_value_path.into(),
            temp_data: None,
            elem_hocks: Vec::new(),
            user_data: HashMap::new(),
        })
    }

    /// Build `AppData` from a schema value and an optional config path.
    ///
    /// If the config file exists, it is loaded to initialize values.
    pub fn new_with_schema(
        config: Option<impl AsRef<Path>>,
        schema: &serde_json::Value,
    ) -> anyhow::Result<Self> {
        let init_value_path = Self::init_value_path(config);

        let mut root = MenuRoot::try_from(schema)?;

        if init_value_path.exists() {
            let init_content = fs::read_to_string(&init_value_path)?;
            if !init_content.trim().is_empty() {
                let ext = init_value_path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let init_json: serde_json::Value = match ext {
                    "json" => serde_json::from_str(&init_content)?,
                    "toml" => {
                        let v: toml::Value = toml::from_str(&init_content)?;
                        serde_json::to_value(v)?
                    }
                    _ => {
                        bail!("Unsupported config file extension: {ext:?}");
                    }
                };
                root.update_by_value(&init_json)?;
            }
        }

        Ok(AppData {
            root,
            current_key: Vec::new(),
            needs_save: false,
            config: init_value_path,
            temp_data: None,
            elem_hocks: Vec::new(),
            user_data: HashMap::new(),
        })
    }

    /// Persist changes and create a timestamped backup when needed.
    pub fn on_exit(&mut self) -> anyhow::Result<()> {
        if !self.needs_save {
            return Ok(());
        }
        let ext = self
            .config
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let json_value = self.root.as_json();

        println!("value to save:\n {:?}", json_value);

        let s = match ext {
            "toml" | "tml" => toml::to_string_pretty(&json_value)?,
            "json" => serde_json::to_string_pretty(&json_value)?,
            _ => {
                bail!("Unsupported config file extension: {}", ext);
            }
        };

        if self.config.exists() {
            let bk = format!(
                "bk-{:?}.{ext}",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .as_secs()
            );

            let backup_path = self.config.with_extension(bk);
            fs::copy(&self.config, &backup_path)?;
        }
        fs::write(&self.config, s)?;
        Ok(())
    }

    /// Enter a submenu path (dot-separated).
    pub fn enter(&mut self, key: &str) {
        if key.is_empty() {
            return;
        }
        self.current_key = key.split(".").map(|s| s.to_string()).collect();
    }

    /// Push a field name onto the current path.
    pub fn push_field(&mut self, f: &str) {
        self.current_key.push(f.to_string());
    }

    /// Navigate back to the parent path.
    pub fn navigate_back(&mut self) {
        if !self.current_key.is_empty() {
            self.current_key.pop();
        }
    }

    /// Return the current path as a dot-separated string.
    pub fn key_string(&self) -> String {
        if self.current_key.is_empty() {
            return String::new();
        }

        self.current_key.join(".")
    }

    /// Get the element at the current path.
    pub fn current(&self) -> Option<&ElementType> {
        self.root.get_by_key(&self.key_string())
    }

    /// Get the mutable element at the current path.
    pub fn current_mut(&mut self) -> Option<&mut ElementType> {
        self.root.get_mut_by_key(&self.key_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_default() {
        let name = "config.toml";
        let expected_schema_name = "config-schema.json";
        let schema_path = default_schema_by_init(Path::new(name));
        assert_eq!(schema_path, PathBuf::from(expected_schema_name));
    }
}
