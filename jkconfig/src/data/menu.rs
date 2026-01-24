use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use log::trace;
use serde_json::Value;

use crate::data::{
    schema::SchemaError,
    types::{ElementBase, ElementType},
};

/// Root container for schema-derived menu tree.
#[derive(Clone)]
pub struct MenuRoot {
    /// JSON Schema version string.
    pub schema_version: String,
    /// Root title displayed in the UI.
    pub title: String,
    /// Root element (must be a menu).
    pub menu: ElementType,
}

impl MenuRoot {
    /// Get the root menu node (panics if root is not a menu).
    pub fn menu(&self) -> &Menu {
        match &self.menu {
            ElementType::Menu(menu) => menu,
            _ => panic!("Root element is not a Menu"),
        }
    }

    /// Get the mutable root menu node (panics if root is not a menu).
    pub fn menu_mut(&mut self) -> &mut Menu {
        match &mut self.menu {
            ElementType::Menu(menu) => menu,
            _ => panic!("Root element is not a Menu"),
        }
    }

    /// Get an element by its dot-separated key.
    pub fn get_by_key(&self, key: &str) -> Option<&ElementType> {
        if key.is_empty() {
            return Some(&self.menu);
        }

        let ks = key.split(".").collect::<Vec<_>>();
        self.menu().get_by_field_path(&ks)
    }

    /// Get a mutable element by its dot-separated key.
    pub fn get_mut_by_key(&mut self, key: &str) -> Option<&mut ElementType> {
        if key.is_empty() {
            return Some(&mut self.menu);
        }
        let ks = key.split(".").collect::<Vec<_>>();
        self.menu_mut().get_mut_by_field_path(&ks)
    }

    /// Update the menu tree from a JSON object value.
    pub fn update_by_value(&mut self, value: &Value) -> Result<(), SchemaError> {
        self.menu.update_from_value(value, None)
    }

    /// Serialize the menu tree into a JSON value.
    pub fn as_json(&self) -> Value {
        self.menu().as_json()
    }
}

impl Debug for MenuRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuRoot")
            .field("schema_version", &self.schema_version)
            .field("title", &self.title)
            .field("path", &self.menu.path)
            .field("help", &self.menu.help)
            .field("is_required", &self.menu.is_required)
            .field("struct_name", &self.menu.struct_name)
            .field("is_set", &self.menu().is_set)
            .field("children", &self.menu().children)
            .finish()
    }
}

/// Menu node for schema objects (type: object).
#[derive(Clone)]
pub struct Menu {
    /// Shared element metadata.
    pub base: ElementBase,
    /// Child elements for each field.
    pub children: Vec<ElementType>,
    /// Whether this menu has been set by user input.
    pub is_set: bool,
}

impl Menu {
    /// Serialize this menu into a JSON object.
    pub fn as_json(&self) -> Value {
        let mut result = serde_json::Map::new();

        for child_element in &self.children {
            if child_element.is_none() {
                continue;
            }
            let child_key = child_element.field_name();

            match child_element {
                ElementType::Menu(menu) => {
                    let field_name = menu.field_name();
                    result.insert(field_name, menu.as_json());
                }
                ElementType::Item(item) => {
                    let field_name = item.base.field_name();
                    result.insert(field_name, item.as_json());
                }
                ElementType::OneOf(oneof) => {
                    // For OneOf, the as_json() method already generates the correct structure
                    // with the proper field name, so we should merge its result directly
                    match oneof.as_json() {
                        Value::Object(oneof_result) => {
                            // Merge the OneOf result into our result
                            for (key, value) in oneof_result {
                                result.insert(key, value);
                            }
                        }
                        other => {
                            // For non-object results (like simple strings or null),
                            // use the child_key as the key
                            result.insert(child_key.clone(), other);
                        }
                    }
                }
            }
        }

        Value::Object(result)
    }

    /// Get a child element by its path segments.
    pub fn get_by_field_path(&self, field_path: &[&str]) -> Option<&ElementType> {
        if field_path.is_empty() {
            return None;
        }
        info!("menu get by field path: {:?}", field_path);
        let first_field = field_path[0];

        let child = self.get_child_by_key(first_field)?;

        if field_path.len() == 1 {
            return Some(child);
        }

        match child {
            ElementType::Menu(menu) => menu.get_by_field_path(&field_path[1..]),
            ElementType::OneOf(oneof) => oneof.get_by_field_path(&field_path[1..]),
            _ => None,
        }
    }

    /// Get a mutable child element by its path segments.
    pub fn get_mut_by_field_path(&mut self, field_path: &[&str]) -> Option<&mut ElementType> {
        if field_path.is_empty() {
            return None;
        }

        let first_field = field_path[0];

        let child = self.get_child_mut_by_key(first_field)?;

        if field_path.len() == 1 {
            return Some(child);
        }

        match child {
            ElementType::Menu(menu) => menu.get_mut_by_field_path(&field_path[1..]),
            ElementType::OneOf(oneof) => oneof.get_mut_by_field_path(&field_path[1..]),
            _ => None,
        }
    }

    /// Update this menu from a JSON object value.
    pub fn update_from_value(&mut self, value: &Value) -> Result<(), SchemaError> {
        let value = value.as_object().ok_or(SchemaError::TypeMismatch {
            path: self.key(),
            expected: "object".to_string(),
            actual: serde_json::to_string_pretty(value).unwrap(),
        })?;
        trace!("Updating Menu at {} with value: {:?}", self.key(), value);
        for (key, val) in value {
            if let Some(element) = self.get_child_mut_by_key(key) {
                element.update_from_value(val, None)?;
                trace!("Updated child {} of Menu at {}", key, self.key());
            }
            self.is_set = true;
            // If key doesn't exist in menu children, skip it as per requirement
        }

        Ok(())
    }

    /// Whether this menu is considered unset.
    pub fn is_none(&self) -> bool {
        if self.is_required {
            return false;
        }
        !self.is_set
    }

    /// Return a copy of child elements for UI rendering.
    pub fn fields(&self) -> Vec<ElementType> {
        self.children.to_vec()
    }

    /// Get a direct child by field name.
    pub fn get_child_by_key(&self, key: &str) -> Option<&ElementType> {
        self.children
            .iter()
            .find(|&child| child.field_name() == key)
            .map(|v| v as _)
    }

    /// Get a mutable direct child by field name.
    pub fn get_child_mut_by_key(&mut self, key: &str) -> Option<&mut ElementType> {
        self.children
            .iter_mut()
            .find(|child| child.field_name() == key)
            .map(|v| v as _)
    }
}

impl Debug for Menu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Menu")
            .field("path", &self.path)
            .field("title", &self.title)
            .field("help", &self.help)
            .field("is_required", &self.is_required)
            .field("is_set", &self.is_set)
            .field("struct_name", &self.struct_name)
            .field("children", &self.children)
            .finish()
    }
}

impl Deref for Menu {
    type Target = ElementBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for Menu {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
