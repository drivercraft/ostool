use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{anyhow, bail};

use crate::data::{
    hook::ElementHook,
    item::ItemType,
    menu::{Menu, MenuRoot},
    path::ElementPath,
    resolver::ElementResolver,
    types::ElementType,
};

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

/// Runtime application state shared by TUI and web workflows.
#[derive(Clone)]
pub struct AppState {
    pub document: ConfigDocument,
    pub nav_stack: Vec<MenuState>,
    pub needs_save: bool,
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
                bail!("Unsupported config file extension: {ext}");
            }
        };

        if self.config.exists() {
            let backup_ext = format!(
                "bk-{}.{ext}",
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
            element_hooks: Vec::new(),
        }
    }

    pub fn current_path(&self) -> &ElementPath {
        &self.nav_stack.last().expect("root menu state").path
    }

    pub fn current_path_string(&self) -> String {
        self.current_path().display()
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

    pub fn move_selection(&mut self, delta: isize) {
        let Some(menu) = self.current_menu() else {
            return;
        };
        if menu.children.is_empty() {
            return;
        }

        let current = self.selected_index() as isize;
        let next = (current + delta).clamp(0, menu.children.len() as isize - 1) as usize;
        self.set_selected_index(next);
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

    pub fn discard_changes(&mut self) {
        self.needs_save = false;
    }

    pub fn persist_if_needed(&mut self) -> anyhow::Result<()> {
        if self.needs_save {
            self.validate_before_save()?;
            self.document.persist()?;
        }
        Ok(())
    }

    pub fn missing_required_paths(&self) -> Vec<String> {
        let mut missing = Vec::new();
        collect_missing_required_element(&self.document.root.menu, &mut missing);
        missing
    }

    pub fn validate_before_save(&self) -> anyhow::Result<()> {
        let missing = self.missing_required_paths();
        if missing.is_empty() {
            return Ok(());
        }

        bail!(
            "Cannot save config; required fields are missing:\n- {}",
            missing.join("\n- ")
        );
    }

    pub fn find_selected_hook(&self) -> Option<ElementHook> {
        let selected_path = self.selected_path()?;
        self.element_hooks
            .iter()
            .find(|hook| hook.path == selected_path)
            .cloned()
    }

    pub fn get_string(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<String>> {
        let path = path.into();
        let element = self
            .document
            .get(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::String { value, .. } => Ok(value.clone()),
                _ => bail!("element is not a string: {}", path.display()),
            },
            _ => bail!("element is not a string: {}", path.display()),
        }
    }

    pub fn get_strings(&self, path: impl Into<ElementPath>) -> anyhow::Result<Vec<String>> {
        let path = path.into();
        let element = self
            .document
            .get(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::Array(array) => Ok(array.values.clone()),
                _ => bail!("element is not a string array: {}", path.display()),
            },
            _ => bail!("element is not a string array: {}", path.display()),
        }
    }

    pub fn get_integer(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<i64>> {
        let path = path.into();
        let element = self
            .document
            .get(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::Integer { value, .. } => Ok(*value),
                _ => bail!("element is not an integer: {}", path.display()),
            },
            _ => bail!("element is not an integer: {}", path.display()),
        }
    }

    pub fn get_number(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<f64>> {
        let path = path.into();
        let element = self
            .document
            .get(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::Number { value, .. } => Ok(*value),
                _ => bail!("element is not a number: {}", path.display()),
            },
            _ => bail!("element is not a number: {}", path.display()),
        }
    }

    pub fn get_bool(&self, path: impl Into<ElementPath>) -> anyhow::Result<Option<bool>> {
        let path = path.into();
        let element = self
            .document
            .get(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &item.item_type {
                ItemType::Boolean { value, .. } => Ok(Some(*value)),
                _ => bail!("element is not a bool: {}", path.display()),
            },
            _ => bail!("element is not a bool: {}", path.display()),
        }
    }

    pub fn toggle_bool(&mut self, path: impl Into<ElementPath>) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &mut item.item_type {
                ItemType::Boolean { value, .. } => {
                    *value = !*value;
                    self.mark_dirty();
                    Ok(())
                }
                _ => bail!("element is not a bool: {}", path.display()),
            },
            _ => bail!("element is not a bool: {}", path.display()),
        }
    }

    pub fn set_string(
        &mut self,
        path: impl Into<ElementPath>,
        value: String,
    ) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &mut item.item_type {
                ItemType::String { value: current, .. } => {
                    *current = Some(value);
                    self.mark_dirty();
                    Ok(())
                }
                _ => bail!("element is not a string: {}", path.display()),
            },
            _ => bail!("element is not a string: {}", path.display()),
        }
    }

    pub fn set_optional_string(
        &mut self,
        path: impl Into<ElementPath>,
        value: Option<String>,
    ) -> anyhow::Result<()> {
        match value {
            Some(value) => self.set_string(path, value),
            None => self.clear_optional(path),
        }
    }

    pub fn set_string_array(
        &mut self,
        path: impl Into<ElementPath>,
        values: Vec<String>,
    ) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &mut item.item_type {
                ItemType::Array(array) => {
                    array.values = values;
                    self.mark_dirty();
                    Ok(())
                }
                _ => bail!("element is not an array: {}", path.display()),
            },
            _ => bail!("element is not an array: {}", path.display()),
        }
    }

    pub fn set_integer(&mut self, path: impl Into<ElementPath>, value: i64) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &mut item.item_type {
                ItemType::Integer { value: current, .. } => {
                    *current = Some(value);
                    self.mark_dirty();
                    Ok(())
                }
                _ => bail!("element is not an integer: {}", path.display()),
            },
            _ => bail!("element is not an integer: {}", path.display()),
        }
    }

    pub fn set_optional_integer(
        &mut self,
        path: impl Into<ElementPath>,
        value: Option<i64>,
    ) -> anyhow::Result<()> {
        match value {
            Some(value) => self.set_integer(path, value),
            None => self.clear_optional(path),
        }
    }

    pub fn set_number(&mut self, path: impl Into<ElementPath>, value: f64) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &mut item.item_type {
                ItemType::Number { value: current, .. } => {
                    *current = Some(value);
                    self.mark_dirty();
                    Ok(())
                }
                _ => bail!("element is not a number: {}", path.display()),
            },
            _ => bail!("element is not a number: {}", path.display()),
        }
    }

    pub fn set_optional_number(
        &mut self,
        path: impl Into<ElementPath>,
        value: Option<f64>,
    ) -> anyhow::Result<()> {
        match value {
            Some(value) => self.set_number(path, value),
            None => self.clear_optional(path),
        }
    }

    pub fn set_enum_variant(
        &mut self,
        path: impl Into<ElementPath>,
        variant: &str,
    ) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Item(item) => match &mut item.item_type {
                ItemType::Enum(enum_item) => {
                    let idx = enum_item
                        .variants
                        .iter()
                        .position(|candidate| candidate == variant)
                        .ok_or_else(|| anyhow!("invalid enum variant '{variant}'"))?;
                    enum_item.value = Some(idx);
                    self.mark_dirty();
                    Ok(())
                }
                _ => bail!("element is not an enum: {}", path.display()),
            },
            _ => bail!("element is not an enum: {}", path.display()),
        }
    }

    pub fn set_oneof_index(
        &mut self,
        path: impl Into<ElementPath>,
        index: usize,
    ) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::OneOf(one_of) => {
                one_of.set_selected_index(index)?;
                self.mark_dirty();
                Ok(())
            }
            _ => bail!("element is not a oneOf: {}", path.display()),
        }
    }

    pub fn clear_optional(&mut self, path: impl Into<ElementPath>) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        if element.is_required {
            bail!(
                "element is required and cannot be cleared: {}",
                path.display()
            );
        }
        element.set_none();
        self.mark_dirty();
        Ok(())
    }

    pub fn toggle_menu_set(&mut self, path: impl Into<ElementPath>) -> anyhow::Result<()> {
        let path = path.into();
        let element = self
            .document
            .get_mut(&path)
            .ok_or_else(|| anyhow!("missing element: {}", path.display()))?;
        match element {
            ElementType::Menu(menu) => {
                if menu.is_required {
                    bail!("menu is required and cannot be toggled: {}", path.display());
                }
                menu.is_set = !menu.is_set;
                self.mark_dirty();
                Ok(())
            }
            _ => bail!("element is not a menu: {}", path.display()),
        }
    }

    pub fn element_kind(element: &ElementType) -> &'static str {
        element_kind_and_tag(element).0
    }

    pub fn element_tag(element: &ElementType) -> &'static str {
        element_kind_and_tag(element).1
    }

    pub fn element_status(element: &ElementType) -> &'static str {
        if element.is_required {
            if element.is_none() {
                "required / NOT SET"
            } else {
                "required / set"
            }
        } else if element.is_none() {
            "optional / unset"
        } else {
            "optional / set"
        }
    }

    pub fn element_summary(element: &ElementType) -> String {
        match element {
            ElementType::Menu(menu) => {
                let set_label = if menu.is_required {
                    "required".to_string()
                } else if menu.is_set {
                    "set".to_string()
                } else {
                    "unset".to_string()
                };
                format!("{} fields, {set_label}", menu.children.len())
            }
            ElementType::OneOf(one_of) => one_of
                .selected_index
                .map(|idx| one_of.variant_display(idx))
                .unwrap_or_else(|| "unset".to_string()),
            ElementType::Item(item) => match &item.item_type {
                ItemType::String { value, .. } => value.clone().unwrap_or_else(|| "<empty>".into()),
                ItemType::Number { value, .. } => value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<empty>".into()),
                ItemType::Integer { value, .. } => value
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<empty>".into()),
                ItemType::Boolean { value, .. } => value.to_string(),
                ItemType::Enum(enum_item) => enum_item
                    .value_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<unset>".into()),
                ItemType::Array(array_item) => {
                    if array_item.values.is_empty() {
                        "[]".to_string()
                    } else {
                        format!("{} item(s)", array_item.values.len())
                    }
                }
            },
        }
    }

    pub fn selected_detail_text(&self) -> String {
        let Some(element) = self.current() else {
            return "No items in this menu.".to_string();
        };

        let mut lines = vec![
            format!("Title: {}", element.title),
            format!("Path: {}", element.key()),
            format!("Type: {}", Self::element_kind(element)),
            format!("State: {}", Self::element_status(element)),
        ];
        if element.is_required && element.is_none() {
            lines.insert(0, ">>> REQUIRED FIELD (NOT SET) <<<".to_string());
        }

        if let Some(help) = &element.help {
            lines.push(String::new());
            lines.push(help.clone());
        }

        match element {
            ElementType::Menu(menu) => {
                lines.push(String::new());
                lines.push(format!("Children: {}", menu.children.len()));
            }
            ElementType::OneOf(one_of) => {
                lines.push(String::new());
                lines.push(format!("Variants: {}", one_of.variants.len()));
                lines.push(format!(
                    "Selected: {}",
                    one_of
                        .selected_index
                        .map(|idx| one_of.variant_display(idx))
                        .unwrap_or_else(|| "unset".to_string())
                ));
            }
            ElementType::Item(item) => match &item.item_type {
                ItemType::String { value, default } => {
                    push_current_and_default(&mut lines, value.as_deref(), default.as_deref());
                }
                ItemType::Number { value, default } => {
                    push_current_and_default(&mut lines, value.as_ref(), default.as_ref());
                }
                ItemType::Integer { value, default } => {
                    push_current_and_default(&mut lines, value.as_ref(), default.as_ref());
                }
                ItemType::Boolean { value, default } => {
                    lines.push(String::new());
                    lines.push(format!("Current: {value}"));
                    lines.push(format!("Default: {default}"));
                }
                ItemType::Enum(enum_item) => {
                    lines.push(String::new());
                    lines.push(format!(
                        "Current: {}",
                        enum_item.value_str().unwrap_or("<unset>")
                    ));
                    lines.push(format!("Variants: {}", enum_item.variants.join(", ")));
                }
                ItemType::Array(array_item) => {
                    lines.push(String::new());
                    lines.push(format!("Element type: {}", array_item.element_type));
                    lines.push(format!("Items: {}", array_item.values.len()));
                    if !array_item.values.is_empty() {
                        lines.push(String::new());
                        lines.extend(
                            array_item
                                .values
                                .iter()
                                .enumerate()
                                .map(|(idx, value)| format!("[{idx}] {value}")),
                        );
                    }
                }
            },
        }

        lines.join("\n")
    }
}

fn element_kind_and_tag(element: &ElementType) -> (&'static str, &'static str) {
    match element {
        ElementType::Menu(_) => ("Object", "OBJ"),
        ElementType::OneOf(_) => ("OneOf", "ALT"),
        ElementType::Item(item) => match &item.item_type {
            ItemType::String { .. } => ("String", "TXT"),
            ItemType::Number { .. } => ("Number", "NUM"),
            ItemType::Integer { .. } => ("Integer", "INT"),
            ItemType::Boolean { .. } => ("Boolean", "BOL"),
            ItemType::Enum(_) => ("Enum", "ENU"),
            ItemType::Array(_) => ("Array", "ARR"),
        },
    }
}

fn push_current_and_default<T: std::fmt::Display>(
    lines: &mut Vec<String>,
    value: Option<T>,
    default: Option<T>,
) {
    lines.push(String::new());
    lines.push(format!(
        "Current: {}",
        value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "<empty>".to_string())
    ));
    if let Some(default) = default {
        lines.push(format!("Default: {default}"));
    }
}

fn collect_missing_required_element(element: &ElementType, missing: &mut Vec<String>) {
    match element {
        ElementType::Menu(menu) => {
            if !menu.is_required && !menu.is_set {
                return;
            }
            for child in &menu.children {
                collect_missing_required_element(child, missing);
            }
        }
        ElementType::OneOf(one_of) => {
            if let Some(selected) = one_of.selected() {
                collect_missing_required_element(selected, missing);
            } else if one_of.is_required {
                missing.push(one_of.key());
            }
        }
        ElementType::Item(item) => {
            if item.base.is_required && element.is_none() {
                missing.push(item.base.key());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{
        item::{Item, ItemType},
        menu::Menu,
        types::ElementBase,
    };

    #[test]
    fn test_schema_default() {
        let name = "config.toml";
        let expected_schema_name = "config-schema.json";
        let schema_path = default_schema_by_init(Path::new(name));
        assert_eq!(schema_path, PathBuf::from(expected_schema_name));
    }

    #[test]
    fn clear_optional_rejects_required_element() {
        let document = ConfigDocument {
            root: MenuRoot {
                schema_version: "test".into(),
                title: "root".into(),
                menu: ElementType::Menu(Menu {
                    base: ElementBase {
                        path: PathBuf::new(),
                        title: "root".into(),
                        help: None,
                        is_required: true,
                        struct_name: "Root".into(),
                    },
                    children: vec![ElementType::Item(Item {
                        base: ElementBase {
                            path: PathBuf::from("name"),
                            title: "name".into(),
                            help: None,
                            is_required: true,
                            struct_name: "string".into(),
                        },
                        item_type: ItemType::String {
                            value: Some("value".into()),
                            default: None,
                        },
                    })],
                    is_set: true,
                }),
            },
            config: PathBuf::from("config.toml"),
        };
        let mut app = AppState::new(document);
        let err = app.clear_optional("name").unwrap_err().to_string();
        assert!(err.contains("required"));
    }

    #[test]
    fn missing_required_paths_reports_missing_required_string() {
        let document = ConfigDocument {
            root: MenuRoot {
                schema_version: "test".into(),
                title: "root".into(),
                menu: ElementType::Menu(Menu {
                    base: ElementBase {
                        path: PathBuf::new(),
                        title: "root".into(),
                        help: None,
                        is_required: true,
                        struct_name: "Root".into(),
                    },
                    children: vec![ElementType::Item(Item {
                        base: ElementBase {
                            path: PathBuf::from("system").join("package"),
                            title: "package".into(),
                            help: None,
                            is_required: true,
                            struct_name: "string".into(),
                        },
                        item_type: ItemType::String {
                            value: None,
                            default: None,
                        },
                    })],
                    is_set: true,
                }),
            },
            config: PathBuf::from("config.toml"),
        };
        let app = AppState::new(document);
        assert_eq!(app.missing_required_paths(), vec!["system.package"]);
    }

    #[test]
    fn missing_required_paths_skips_unset_optional_menu() {
        let document = ConfigDocument {
            root: MenuRoot {
                schema_version: "test".into(),
                title: "root".into(),
                menu: ElementType::Menu(Menu {
                    base: ElementBase {
                        path: PathBuf::new(),
                        title: "root".into(),
                        help: None,
                        is_required: true,
                        struct_name: "Root".into(),
                    },
                    children: vec![ElementType::Menu(Menu {
                        base: ElementBase {
                            path: PathBuf::from("optional"),
                            title: "optional".into(),
                            help: None,
                            is_required: false,
                            struct_name: "Optional".into(),
                        },
                        children: vec![ElementType::Item(Item {
                            base: ElementBase {
                                path: PathBuf::from("optional").join("package"),
                                title: "package".into(),
                                help: None,
                                is_required: true,
                                struct_name: "string".into(),
                            },
                            item_type: ItemType::String {
                                value: None,
                                default: None,
                            },
                        })],
                        is_set: false,
                    })],
                    is_set: true,
                }),
            },
            config: PathBuf::from("config.toml"),
        };
        let app = AppState::new(document);
        assert!(app.missing_required_paths().is_empty());
    }
}
