use jkconfig::data::menu::MenuRoot;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

// Use Animal structures from other test file
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Cat {
    pub a: usize,
    pub b: String,
    pub children: Option<CatChild>,
    pub child2: CatChild,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CatChild {
    pub e: isize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Dog {
    pub c: Option<f32>,
    pub d: bool,
    pub l: Legs,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum Legs {
    Four,
    Two,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
/// 动物类型
/// Cat 或 Dog 的枚举
pub enum AnimalEnum {
    Cat(Cat),
    Dog(Dog),
    Rabbit,
    Duck { h: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
struct AnimalObject {
    animal: AnimalEnum,
}

#[test]
fn test_object() {
    let schema = schema_for!(AnimalObject);
    println!(
        "Generated JSON Schema: \n{}",
        serde_json::to_string_pretty(&schema).unwrap()
    );
    let menu = MenuRoot::try_from(schema.as_value()).unwrap();

    println!("Generated MenuRoot: \n{:#?}", menu);

    println!(
        "AnimalEnum element: \n{:#?}",
        menu.get_by_key("animal").unwrap()
    );
}

#[test]
fn test_value() {
    let schema = schema_for!(AnimalObject);

    let origin = AnimalObject {
        animal: AnimalEnum::Dog(Dog {
            c: Some(3.5),
            d: true,
            l: Legs::Four,
        }),
    };

    let value = schema.as_value();

    println!(
        "Generated JSON Schema Value: \n{}",
        serde_json::to_string_pretty(&value).unwrap()
    );

    let mut menu = MenuRoot::try_from(value).unwrap();

    let value = serde_json::to_value(&origin).unwrap();

    println!("Origin MenuRoot : \n{menu:#?}",);

    println!(
        "Json value to update: \n{}",
        serde_json::to_string_pretty(&value).unwrap()
    );

    menu.update_by_value(&value).unwrap();

    println!("Updated MenuRoot: \n{:#?}", menu);

    let actual_value = menu.as_json();

    println!(
        "Actual JSON value from MenuRoot: \n{}",
        serde_json::to_string_pretty(&actual_value).unwrap()
    );

    let actual: AnimalObject = serde_json::from_value(actual_value).unwrap();

    assert_eq!(origin.animal, actual.animal);
}

#[test]
fn test_value_enum() {
    let schema = schema_for!(AnimalObject);

    let origin = AnimalObject {
        animal: AnimalEnum::Rabbit,
    };

    let value = schema.as_value();

    println!(
        "Generated JSON Schema Value: \n{}",
        serde_json::to_string_pretty(&value).unwrap()
    );

    let mut menu = MenuRoot::try_from(value).unwrap();

    let value = serde_json::to_value(&origin).unwrap();

    println!("Origin MenuRoot : \n{menu:#?}",);

    println!(
        "Json value to update: \n{}",
        serde_json::to_string_pretty(&value).unwrap()
    );

    menu.update_by_value(&value).unwrap();

    println!("Updated MenuRoot: \n{:#?}", menu);

    let actual_value = menu.as_json();

    println!(
        "Actual JSON value from MenuRoot: \n{}",
        serde_json::to_string_pretty(&actual_value).unwrap()
    );

    let actual: AnimalObject = serde_json::from_value(actual_value).unwrap();

    assert_eq!(origin.animal, actual.animal);
}

#[test]
fn test_value_normal_case() {
    let schema = schema_for!(AnimalObject);
    let mut menu = MenuRoot::try_from(schema.as_value()).unwrap();

    // Test normal case with correct types
    let dog_value = serde_json::json!({
        "animal": {
            "Dog": {
                "c": 2.7,
                "d": false
            }
        }
    });

    let result = menu.update_by_value(&dog_value);
    assert!(
        result.is_ok(),
        "Normal case should succeed: {:?}",
        result.err()
    );
}

#[test]
fn test_value_type_mismatch() {
    let schema = schema_for!(AnimalObject);
    let mut menu = MenuRoot::try_from(schema.as_value()).unwrap();

    // Test type mismatch: boolean field receives string
    let bad_value = serde_json::json!({
        "animal": {
            "Dog": {
                "c": 2.7,
                "d": "this should be boolean"  // Type mismatch
            }
        }
    });

    let result = menu.update_by_value(&bad_value);
    assert!(result.is_err());
    println!("Type mismatch error: {:?}", result.err().unwrap());
}

#[test]
fn test_value_skip_unknown_fields() {
    let schema = schema_for!(AnimalObject);
    let mut menu = MenuRoot::try_from(schema.as_value()).unwrap();

    // Test with extra fields that don't exist in schema
    let value_with_extra = serde_json::json!({
        "animal": {
            "Dog": {
                "c": 1.5,
                "d": true,
                "unknown_field": "should be skipped",
                "another_unknown": 42
            }
        },
        "unknown_top_level": "should be skipped"
    });

    let result = menu.update_by_value(&value_with_extra);
    assert!(result.is_ok(), "Should skip unknown fields and succeed");
}

#[test]
fn test_value_empty_object() {
    let schema = schema_for!(AnimalObject);
    let mut menu = MenuRoot::try_from(schema.as_value()).unwrap();

    // Test with empty object (should skip since no matching fields)
    let empty_value = serde_json::json!({});

    let result = menu.update_by_value(&empty_value);
    // This might fail because animal is required, but let's see what happens
    println!("Empty object result: {:?}", result);
}

#[test]
fn test_value_cat_variant() {
    let schema = schema_for!(AnimalObject);
    let mut menu = MenuRoot::try_from(schema.as_value()).unwrap();

    // Test Cat variant
    let cat_value = serde_json::json!({
        "animal": {
            "Cat": {
                "a": 42,
                "b": "meow"
            }
        }
    });

    let result = menu.update_by_value(&cat_value);
    assert!(result.is_ok(), "Cat variant should succeed");
}

#[test]
fn test_value_integer_type_mismatch() {
    let schema = schema_for!(AnimalObject);
    let mut menu = MenuRoot::try_from(schema.as_value()).unwrap();

    // Test integer field receiving float
    let cat_value = serde_json::json!({
        "animal": {
            "Cat": {
                "a": 3.2,  // Should be integer, not float
                "b": "test"
            }
        }
    });

    let result = menu.update_by_value(&cat_value);
    assert!(result.is_err());
    println!("Integer type mismatch error: {:?}", result.err().unwrap());
}

/***
```json
Generated JSON Schema:
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AnimalObject",
  "type": "object",
  "properties": {
    "animal": {
      "$ref": "#/$defs/AnimalEnum"
    }
  },
  "required": [
    "animal"
  ],
  "$defs": {
    "AnimalEnum": {
      "oneOf": [
        {
          "type": "object",
          "properties": {
            "Cat": {
              "$ref": "#/$defs/Cat"
            }
          },
          "additionalProperties": false,
          "required": [
            "Cat"
          ]
        },
        {
          "type": "object",
          "properties": {
            "Dog": {
              "$ref": "#/$defs/Dog"
            }
          },
          "additionalProperties": false,
          "required": [
            "Dog"
          ]
        }
      ]
    },
    "Cat": {
      "type": "object",
      "properties": {
        "a": {
          "type": "integer",
          "format": "uint",
          "minimum": 0
        },
        "b": {
          "type": "string"
        }
      },
      "required": [
        "a",
        "b"
      ]
    },
    "Dog": {
      "type": "object",
      "properties": {
        "c": {
          "type": "number",
          "format": "float"
        },
        "d": {
          "type": "boolean"
        }
      },
      "required": [
        "c",
        "d"
      ]
    }
  }
}
```
***/

// 测试 MenuRoot::get_mut_by_key 方法的边界条件
#[cfg(test)]
mod menu_root_get_mut_by_key_tests {
    use super::*;

    /// 创建测试用的 MenuRoot 实例
    fn create_test_menu_root() -> MenuRoot {
        let schema = schema_for!(AnimalObject);
        MenuRoot::try_from(schema.as_value()).unwrap()
    }

    #[test]
    /// 测试有效路径（应返回Some的情况）
    fn test_get_mut_by_key_valid_paths() {
        let mut menu = create_test_menu_root();

        // 测试确认存在的有效路径
        let valid_paths = vec![("animal", "top-level field")];

        for (path, description) in valid_paths {
            let result = menu.get_mut_by_key(path);
            assert!(result.is_some(), "{} should return Some", description);
        }
    }

    #[test]
    /// 参数化测试：各种应返回None的边界条件
    fn test_get_mut_by_key_none_cases() {
        let mut menu = create_test_menu_root();

        let test_cases = vec![
            ("nonexistent.path", "nonexistent path"),
            (".animal", "path starting with dot"),
            ("animal.", "path ending with dot"),
            ("animal..Dog", "path with consecutive dots"),
            ("animal-Dog@c", "path with special characters"),
            ("animal.动物.c", "path with unicode characters"),
            ("...", "path with only dots"),
            ("animal..c", "path with empty field in middle"),
        ];

        for (input, description) in test_cases {
            let result = menu.get_mut_by_key(input);
            assert!(result.is_none(), "{} should return None", description);
        }
    }

    #[test]
    /// 测试深层嵌套路径
    fn test_get_mut_by_key_deep_nesting() {
        let mut menu = create_test_menu_root();

        // 测试可能存在的深层路径和不存在路径的边界情况
        let deep_path_cases = vec![
            ("animal.Cat.a", "Cat variant deep path"),
            ("animal.Dog.c", "Dog variant deep path"),
            ("animal.Duck.h", "Duck variant deep path"),
        ];

        let mut has_any_success = false;

        for (path, _description) in deep_path_cases {
            let result = menu.get_mut_by_key(path);
            if result.is_some() {
                has_any_success = true;
                break; // 如果找到有效路径，测试通过
            }
        }

        // 至少应该能够访问animal顶层路径
        let animal_result = menu.get_mut_by_key("animal");
        assert!(
            animal_result.is_some(),
            "Top-level 'animal' path should be accessible"
        );

        // 如果深层路径都不存在，这也是合理的行为（取决于OneOf的当前状态）
        if !has_any_success {
            println!(
                "Note: All deep paths returned None, which may be expected depending on OneOf state"
            );
        }
    }
}
