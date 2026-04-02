use jkconfig::data::menu::MenuRoot;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct AppData {
    pub cat: CatBBB,
    pub dog: Option<DogAAA>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CatBBB {
    pub name: String,
    pub age: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DogAAA {
    pub name: String,
    pub children: Vec<String>,
}

#[test]
fn test_value() {
    let schema = schema_for!(AppData);

    let origin = AppData {
        cat: CatBBB {
            name: "Kitty".to_string(),
            age: 3,
        },
        dog: Some(DogAAA {
            name: "Doggy".to_string(),
            children: vec!["Puppy1".to_string(), "Puppy2".to_string()],
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

    let actual: AppData = serde_json::from_value(actual_value).unwrap();

    assert_eq!(origin, actual);
}
