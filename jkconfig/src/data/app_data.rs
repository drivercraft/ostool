use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use anyhow::bail;
use cursive::Cursive;

use crate::data::{
    menu::{Menu, MenuRoot},
    path::ElementPath,
    resolver::ElementResolver,
    types::ElementType,
};

/// Callback used to provide the list of available features.
pub type FeaturesCallback = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Callback invoked when a menu element is entered.
pub type HookCallback = Arc<dyn Fn(&mut Cursive, &ElementPath) + Send + Sync>;

/// Hook registration for a specific element path.
#[derive(Clone)]
pub struct ElementHook {
    pub path: ElementPath,
    pub callback: HookCallback,
}

impl ElementHook {
    pub fn new(path: impl Into<ElementPath>, callback: HookCallback) -> Self {
        Self {
            path: path.into(),
            callback,
        }
    }
}

/// Persisted configuration document plus schema-derived tree.
#[derive(Clone)]
pub struct ConfigDocument {
    pub root: MenuRoot,
    pub config: PathBuf,
}

/// Navigation state for a single menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub path: ElementPath,
    pub selected_index: usize,
}

/// Runtime application state for TUI and web workflows.
#[derive(Clone)]
pub struct AppState {
    pub document: ConfigDocument,
    pub nav_stack: Vec<MenuState>,
    pub needs_save: bool,
    pub user_data: HashMap<String, String>,
    pub temp_data: Option<(String, serde_json::Value)>,
    pub element_hooks: Vec<ElementHook>,
}

const DEFAULT_CONFIG_PATH: &str = ".config.toml";

/// Derive a default schema path from a config path.
pub fn default_schema_by_init(config: &Path) -> PathBuf {
    let binding = config.file_name().unwrap().to_string_lossy();
    let mut name_split = binding.split('.').collect::<Vec<_>>();
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

impl ConfigDocument {
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
                    let value: toml::Value = toml::from_str(init)?;
                    serde_json::to_value(value)?
                }
                ext => {
                    bail!("Unsupported config file extension: {ext:?}");
                }
            };
            root.update_by_value(&init_json)?;
        }

        Ok(Self {
            root,
            config: init_value_path.into(),
        })
    }

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
                        let value: toml::Value = toml::from_str(&init_content)?;
                        serde_json::to_value(value)?
                    }
                    _ => {
                        bail!("Unsupported config file extension: {ext:?}");
                    }
                };
                root.update_by_value(&init_json)?;
            }
        }

        Ok(Self {
            root,
            config: init_value_path,
        })
    }

    pub fn title(&self) -> &str {
        &self.root.title
    }

    pub fn as_json(&self) -> serde_json::Value {
        self.root.as_json()
    }

    pub fn get(&self, path: &ElementPath) -> Option<&ElementType> {
        ElementResolver::resolve(&self.root, path).ok()
    }

    pub fn get_mut(&mut self, path: &ElementPath) -> Option<&mut ElementType> {
        ElementResolver::resolve_mut(&mut self.root, path).ok()
    }

    pub fn menu(&self, path: &ElementPath) -> Option<&Menu> {
        ElementResolver::menu(&self.root, path).ok()
    }

    pub fn menu_mut(&mut self, path: &ElementPath) -> Option<&mut Menu> {
        ElementResolver::menu_mut(&mut self.root, path).ok()
    }

    pub fn persist(&mut self) -> anyhow::Result<()> {
        let ext = self
            .config
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let json_value = self.root.as_json();

        let content = match ext {
            "toml" | "tml" => toml::to_string_pretty(&json_value)?,
            "json" => serde_json::to_string_pretty(&json_value)?,
            _ => {
                bail!("Unsupported config file extension: {}", ext);
            }
        };

        if self.config.exists() {
            let backup_ext = format!(
                "bk-{:?}.{ext}",
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .as_secs()
            );
            let backup_path = self.config.with_extension(backup_ext);
            fs::copy(&self.config, &backup_path)?;
        }

        fs::write(&self.config, content)?;
        Ok(())
    }
}

impl AppState {
    pub fn new(document: ConfigDocument) -> Self {
        Self {
            document,
            nav_stack: vec![MenuState {
                path: ElementPath::root(),
                selected_index: 0,
            }],
            needs_save: false,
            user_data: HashMap::new(),
            temp_data: None,
            element_hooks: Vec::new(),
        }
    }

    pub fn current_path(&self) -> &ElementPath {
        &self.nav_stack.last().expect("root menu state").path
    }

    pub fn current_path_string(&self) -> String {
        self.current_path().as_key()
    }

    pub fn current_menu(&self) -> Option<&Menu> {
        self.document.menu(self.current_path())
    }

    pub fn current_menu_mut(&mut self) -> Option<&mut Menu> {
        let path = self.current_path().clone();
        self.document.menu_mut(&path)
    }

    pub fn selected_index(&self) -> usize {
        self.nav_stack
            .last()
            .expect("root menu state")
            .selected_index
    }

    pub fn set_selected_index(&mut self, index: usize) {
        if let Some(menu) = self.current_menu() {
            let max_index = menu.children.len().saturating_sub(1);
            if let Some(state) = self.nav_stack.last_mut() {
                state.selected_index = index.min(max_index);
            }
        }
    }

    pub fn set_selected_by_key(&mut self, key: &str) {
        let Some(menu) = self.current_menu() else {
            return;
        };
        let index = menu
            .children
            .iter()
            .position(|element| element.key() == key);
        if let Some(index) = index {
            self.set_selected_index(index);
        }
    }

    pub fn clamp_selection(&mut self) {
        let selected = self.selected_index();
        self.set_selected_index(selected);
    }

    pub fn current(&self) -> Option<&ElementType> {
        self.current_menu()?.children.get(self.selected_index())
    }

    pub fn current_mut(&mut self) -> Option<&mut ElementType> {
        let index = self.selected_index();
        self.current_menu_mut()?.children.get_mut(index)
    }

    pub fn selected_path(&self) -> Option<ElementPath> {
        self.current()
            .map(|element| ElementPath::parse(&element.key()))
    }

    pub fn enter_menu(&mut self, path: impl Into<ElementPath>) {
        self.nav_stack.push(MenuState {
            path: path.into(),
            selected_index: 0,
        });
    }

    pub fn navigate_back(&mut self) -> bool {
        if self.nav_stack.len() <= 1 {
            return false;
        }
        self.nav_stack.pop();
        true
    }

    pub fn get_by_key(&self, key: &str) -> Option<&ElementType> {
        self.document.get(&ElementPath::parse(key))
    }

    pub fn get_mut_by_key(&mut self, key: &str) -> Option<&mut ElementType> {
        self.document.get_mut(&ElementPath::parse(key))
    }

    pub fn mark_dirty(&mut self) {
        self.needs_save = true;
    }

    pub fn persist_if_needed(&mut self) -> anyhow::Result<()> {
        if self.needs_save {
            self.document.persist()?;
        }
        Ok(())
    }

    pub fn find_selected_hook(&self) -> Option<ElementHook> {
        let selected_path = self.selected_path()?;
        self.element_hooks
            .iter()
            .find(|hook| hook.path == selected_path)
            .cloned()
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
