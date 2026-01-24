use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::data::{
    item::ItemType,
    schema::SchemaError,
    types::{ElementBase, ElementType},
};

use log::trace;
use serde_json::Value;

/// OneOf/AnyOf variant container.
#[derive(Clone)]
pub struct OneOf {
    /// Shared element metadata.
    pub base: ElementBase,
    /// Available variant elements.
    pub variants: Vec<ElementType>,
    /// Selected variant index.
    pub selected_index: Option<usize>,
    /// Default variant index.
    pub default_index: Option<usize>,
}

impl OneOf {
    /// Get the currently selected variant.
    pub fn selected(&self) -> Option<&ElementType> {
        self.selected_index.and_then(|idx| self.variants.get(idx))
    }

    /// Get the currently selected variant mutably.
    pub fn selected_mut(&mut self) -> Option<&mut ElementType> {
        self.selected_index
            .and_then(move |idx| self.variants.get_mut(idx))
    }

    /// Resolve a display name for a variant index.
    pub fn variant_display(&self, idx: usize) -> String {
        if let Some(variant) = self.variants.get(idx) {
            match variant {
                ElementType::Menu(menu) => menu.struct_name.clone(),
                ElementType::Item(item) => match &item.item_type {
                    crate::data::item::ItemType::Enum(enum_item) => enum_item
                        .variants
                        .first()
                        .cloned()
                        .unwrap_or("<Enum Item>".to_string()),
                    _ => "<Simple Item>".to_string(),
                },
                ElementType::OneOf(one_of) => one_of.struct_name.clone(),
            }
        } else {
            "<Invalid Variant>".to_string()
        }
    }

    /// Get an element by path segments from the selected variant.
    pub fn get_by_field_path(&self, field_path: &[&str]) -> Option<&ElementType> {
        if field_path.is_empty() {
            return None;
        }

        let selected = self.selected()?;
        info!(
            "OneOf get by field path: {:?}, selected: {:?}",
            field_path,
            selected.key()
        );

        match selected {
            ElementType::Menu(menu) => {
                return menu.get_by_field_path(field_path);
            }
            ElementType::OneOf(one_of) => {
                return one_of.get_by_field_path(&field_path[1..]);
            }
            _ => {
                if field_path.len() == 1 {
                    return Some(selected);
                }
            }
        }

        None
    }

    /// Get a mutable element by path segments from the selected variant.
    pub fn get_mut_by_field_path(&mut self, field_path: &[&str]) -> Option<&mut ElementType> {
        if field_path.is_empty() {
            return None;
        }

        let selected = self.selected_mut()?;

        info!(
            "OneOf get by field path: {:?}, selected: {:?}",
            field_path,
            selected.key()
        );

        match selected {
            ElementType::Menu(menu) => {
                return menu.get_mut_by_field_path(field_path);
            }
            ElementType::OneOf(one_of) => {
                return one_of.get_mut_by_field_path(&field_path[1..]);
            }
            _ => {
                if field_path.len() == 1 {
                    return Some(selected);
                }
            }
        }

        None
    }

    fn try_update_index(&mut self, index: usize, name: Option<&str>, value: &Value) -> bool {
        let Some(variant) = self.variants.get_mut(index) else {
            return false;
        };
        trace!("Try index {index} , {variant:?}");
        variant.update_from_value(value, name).is_ok()
    }

    /// Update selection and data from a JSON value.
    pub fn update_from_value(&mut self, value: &Value) -> Result<(), SchemaError> {
        let mut name: Option<String> = None;
        let mut value = value;
        if let Some(obj) = value.as_object()
            && let Some((struct_name, inner_value)) = obj.iter().next()
        {
            name = Some(struct_name.clone());
            value = inner_value;
        }

        trace!(
            "Updating OneOf at {} type `{name:?}` with value: {:?}",
            self.key(),
            value
        );
        for idx in 0..self.variants.len() {
            if self.try_update_index(idx, name.as_deref(), value) {
                self.selected_index = Some(idx);
                return Ok(());
            }
        }
        Err(SchemaError::TypeMismatch {
            path: self.key(),
            expected: self
                .variants
                .iter()
                .map(|v| v.struct_name.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            actual: serde_json::to_string_pretty(value).unwrap(),
        })
    }

    /// Serialize the selected variant into JSON.
    pub fn as_json(&self) -> Value {
        if let Some(selected) = self.selected() {
            match selected {
                ElementType::Menu(menu) => {
                    let mut obj = serde_json::Map::new();
                    obj.insert(menu.struct_name.clone(), menu.as_json());

                    let mut result = serde_json::Map::new();
                    // For OneOf, the variant name should be the field name from the menu
                    let variant_name = menu.field_name();
                    result.insert(variant_name, Value::Object(obj));
                    Value::Object(result)
                }
                ElementType::Item(item) => {
                    // For OneOf containing simple items, return the item's value directly
                    item.as_json()
                }
                ElementType::OneOf(nested_oneof) => nested_oneof.as_json(),
            }
        } else {
            // If no variant is selected, return null
            Value::Null
        }
    }

    /// Get the field name for this OneOf element.
    pub fn field_name(&self) -> String {
        self.base.field_name()
    }

    /// Select a variant by index and initialize defaults when needed.
    pub fn set_selected_index(&mut self, index: usize) -> Result<(), SchemaError> {
        let path = self.path.clone();
        let v = self
            .variants
            .get_mut(index)
            .ok_or(SchemaError::SchemaConversionError {
                path,
                reason: "index out of bounds".to_string(),
            })?;
        self.selected_index = Some(index);
        match v {
            ElementType::Menu(menu) => menu.is_set = true,
            ElementType::Item(item) => {
                if let ItemType::Enum(en) = &mut item.item_type {
                    en.value = Some(0)
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Whether no variant is selected.
    pub fn is_none(&self) -> bool {
        self.selected_index.is_none()
    }
}

impl Deref for OneOf {
    type Target = ElementBase;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for OneOf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl Debug for OneOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneOf")
            .field("path", &self.path)
            .field("title", &self.title)
            .field("help", &self.help)
            .field("is_required", &self.is_required)
            .field("variants", &self.variants)
            .field("selected_index", &self.selected_index)
            .field("default_index", &self.default_index)
            .finish()
    }
}
