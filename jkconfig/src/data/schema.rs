use std::{collections::HashSet, ops::Deref, path::PathBuf};

use serde_json::Value;

use crate::data::{
    item::{EnumItem, Item, ItemType},
    menu::{Menu, MenuRoot},
    oneof::OneOf,
    types::{ElementBase, ElementType},
};

/// Errors produced while converting JSON Schema into internal structures.
#[derive(thiserror::Error, Debug)]
pub enum SchemaError {
    #[error("Unsupported schema")]
    UnsupportedSchema,
    #[error("JSON parse error: {0}")]
    JsonParseError(#[from] serde_json::Error),
    #[error("Schema conversion error at {path:?}: {reason}")]
    SchemaConversionError { path: PathBuf, reason: String },
    #[error("Type mismatch at path '{path}': expected {expected}, got {actual}")]
    TypeMismatch {
        path: String,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, Clone)]
struct WalkContext {
    path: PathBuf,
    value: Value,
    defs: Option<Value>,
}

impl Deref for WalkContext {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl WalkContext {
    fn new(value: Value) -> Self {
        Self {
            path: PathBuf::new(),
            value,
            defs: None,
        }
    }

    fn required_field(&self, field_name: &str) -> Result<&Value, SchemaError> {
        self.get(field_name)
            .ok_or(SchemaError::SchemaConversionError {
                path: self.path.clone(),
                reason: format!("Missing required field '{}'", field_name),
            })
    }

    fn required_field_as_string(&self, field_name: &str) -> Result<String, SchemaError> {
        let value = self.required_field(field_name)?;
        Ok(value
            .as_str()
            .map(String::from)
            .ok_or(SchemaError::SchemaConversionError {
                path: self.path.clone(),
                reason: "$schema is not string".into(),
            })?
            .to_string())
    }

    fn get_str(&self, field_name: &str) -> Result<Option<&str>, SchemaError> {
        self.get(field_name)
            .map(|v| {
                v.as_str().ok_or(SchemaError::SchemaConversionError {
                    path: self.path.clone(),
                    reason: format!("Field '{}' is not a string", field_name),
                })
            })
            .transpose()
    }

    fn description(&self) -> Result<Option<String>, SchemaError> {
        let desc = self.get_str("description")?.map(String::from);
        Ok(desc)
    }

    fn handle_object(
        &self,
        is_required: bool,
        field_name: Option<&str>,
    ) -> Result<Option<Menu>, SchemaError> {
        if let Some(ty) = self.get("type")
            && let Some(ty_str) = ty.as_str()
            && ty_str == "object"
        {
            if let Some(name) = field_name {
                let menu = Menu::from_schema(self, is_required, name)?;
                return Ok(Some(menu));
            } else if let Some(props) = self.get("properties")
                && let Some(props) = props.as_object()
            {
                for prop in props.values() {
                    let mut walk = self.clone();
                    walk.value = prop.clone();

                    if let Some(ElementType::Menu(menu)) = walk.as_ref(is_required)? {
                        return Ok(Some(menu));
                    }
                }
            }
        }
        Ok(None)
    }

    fn as_ref(&self, is_required: bool) -> Result<Option<ElementType>, SchemaError> {
        if let Some(ref_value) = self.get("$ref")
            && let Some(ref_str) = ref_value.as_str()
        {
            let def_name = ref_str.trim_start_matches("#/$defs/");
            if let Some(defs) = &self.defs
                && let Some(def_value) = defs.get(def_name)
            {
                let mut walk = self.clone();
                walk.value = def_value.clone();
                return walk.as_element_type(is_required, Some(def_name));
            }
        }
        Ok(None)
    }

    fn handle_oneof(
        &self,
        is_required: bool,
        field_name: Option<&str>,
    ) -> Result<Option<OneOf>, SchemaError> {
        if let Some(one_of) = self.get("oneOf")
            && let Some(variants) = one_of.as_array()
            && let Some(field_name) = field_name
        {
            let mut variant_elements = Vec::new();
            for variant in variants {
                // Process each variant
                let mut walk = self.clone();
                walk.value = variant.clone();
                if let Some(element_type) = walk.as_element_type(false, None)? {
                    variant_elements.push(element_type);
                }
            }

            let one_of = OneOf {
                base: ElementBase::new(&self.path, self.description()?, is_required, field_name),
                variants: variant_elements,
                selected_index: None,
                default_index: None,
            };

            return Ok(Some(one_of));
        }

        Ok(None)
    }

    /// field_name 为 [None] 时，代表是 array 的元素
    fn as_element_type(
        &self,
        is_required: bool,
        field_name: Option<&str>,
    ) -> Result<Option<ElementType>, SchemaError> {
        if let Some(menu) = self.handle_object(is_required, field_name)? {
            return Ok(Some(ElementType::Menu(menu)));
        }

        if let Some(val) = self.as_ref(is_required)? {
            return Ok(Some(val));
        }

        if let Some(one_of) = self.handle_oneof(is_required, field_name)? {
            return Ok(Some(ElementType::OneOf(one_of)));
        }

        if let Some(item) = self.as_item(is_required)? {
            return Ok(Some(item));
        }

        if let Some(anyof) = self.as_anyof(is_required)? {
            return Ok(Some(anyof));
        }
        Ok(None)
    }

    fn _as_item(
        &self,
        ty_str: &str,
        is_required: bool,
    ) -> Result<Option<ElementType>, SchemaError> {
        match ty_str {
            "string" | "number" | "integer" | "boolean" | "array" => {
                // Create Item based on type
                // Placeholder implementation
                let item = Item {
                    base: ElementBase::new(&self.path, self.description()?, is_required, ty_str),
                    item_type: match ty_str {
                        "string" => {
                            if let Some(enum_values) = self.get("enum")
                                && let Some(variants) = enum_values.as_array()
                            {
                                let variant_strings = variants
                                    .iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect::<Vec<_>>();

                                ItemType::Enum(EnumItem {
                                    variants: variant_strings,
                                    value: None,
                                    default: None,
                                })
                            } else {
                                ItemType::String {
                                    value: None,
                                    default: None,
                                }
                            }
                        }
                        "number" => ItemType::Number {
                            value: None,
                            default: None,
                        },
                        "integer" => ItemType::Integer {
                            value: None,
                            default: None,
                        },
                        "boolean" => ItemType::Boolean {
                            value: false,
                            default: false,
                        },
                        "array" => {
                            // Get array element type from items field
                            let element_type = if let Some(items) = self.get("items")
                                && let Some(item_type) = items.get("type")
                                && let Some(type_str) = item_type.as_str()
                            {
                                type_str.to_string()
                            } else {
                                "string".to_string() // default to string
                            };

                            ItemType::Array(crate::data::item::ArrayItem {
                                element_type,
                                values: Vec::new(),
                                default: Vec::new(),
                            })
                        }
                        _ => unreachable!(),
                    },
                };
                return Ok(Some(ElementType::Item(item)));
            }
            _ => {}
        }

        Ok(None)
    }

    fn as_item(&self, is_required: bool) -> Result<Option<ElementType>, SchemaError> {
        if let Some(ty) = self.get("type") {
            if let Some(type_array) = ty.as_array() {
                // Handle multiple types (e.g., ["string", "null"])
                // For simplicity, we will just take the first type here
                if let Some(first_type) = type_array.first()
                    && let Some(ty_str) = first_type.as_str()
                {
                    return self._as_item(ty_str, is_required);
                }
            }

            if let Some(ty_str) = ty.as_str() {
                return self._as_item(ty_str, is_required);
            }
        }

        Ok(None)
    }

    fn as_anyof(&self, _is_required: bool) -> Result<Option<ElementType>, SchemaError> {
        if let Some(one_of) = self.get("anyOf")
            && let Some(variants) = one_of.as_array()
        {
            let var_object = variants[0].clone();
            let mut walk = self.clone();
            walk.value = var_object;
            if let Some(element_type) = walk.as_element_type(false, None)? {
                return Ok(Some(element_type));
            }
        }

        Ok(None)
    }
}

impl TryFrom<&Value> for MenuRoot {
    type Error = SchemaError;

    fn try_from(schema: &Value) -> Result<Self, Self::Error> {
        let mut walk = WalkContext::new(schema.clone());
        let schema_version = walk.required_field_as_string("$schema")?;
        let title = walk.required_field_as_string("title")?;

        walk.defs = walk.get("$defs").cloned();

        let menu = Menu::from_schema(&walk, true, &title)?;

        Ok(MenuRoot {
            schema_version,
            title,
            menu: ElementType::Menu(menu),
        })
    }
}

impl Menu {
    fn from_schema(
        walk: &WalkContext,
        is_required: bool,
        struct_name: &str,
    ) -> Result<Self, SchemaError> {
        let description = walk.description()?;

        let mut menu = Menu {
            base: ElementBase::new(&walk.path, description, is_required, struct_name),
            children: Default::default(),
            is_set: is_required,
        };

        let mut required_fields = HashSet::new();

        if let Some(req) = walk.get("required")
            && let Some(req_array) = req.as_array()
        {
            for item in req_array {
                if let Some(field_name) = item.as_str() {
                    required_fields.insert(field_name.to_string());
                }
            }
        }

        if let Some(properties) = walk.get("properties")
            && let Some(props_map) = properties.as_object()
        {
            for (key, value) in props_map {
                let child_path = walk.path.join(key);
                let is_required = required_fields.contains(key);
                let walk = WalkContext {
                    path: child_path,
                    value: value.clone(),
                    defs: walk.defs.clone(),
                };

                menu.handle_children(&walk, is_required, key)?;
            }
        }

        // Placeholder implementation
        Ok(menu)
    }

    fn handle_children(
        &mut self,
        walk: &WalkContext,
        is_required: bool,
        field_name: &str,
    ) -> Result<(), SchemaError> {
        if let Some(val) = walk.as_element_type(is_required, Some(field_name))? {
            self.children.push(val);
        }
        Ok(())
    }
}
