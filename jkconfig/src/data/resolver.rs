use crate::data::{
    menu::{Menu, MenuRoot},
    oneof::OneOf,
    path::ElementPath,
    types::ElementType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    MissingPath(String),
    UnselectedVariant(String),
    ExpectedMenu(String),
}

pub struct ElementResolver;

impl ElementResolver {
    pub fn resolve<'a>(
        root: &'a MenuRoot,
        path: &ElementPath,
    ) -> Result<&'a ElementType, ResolveError> {
        if path.is_root() {
            return Ok(&root.menu);
        }

        let mut current = &root.menu;
        let mut index = 0;

        loop {
            if index >= path.len() {
                return Ok(current);
            }

            current = match current {
                ElementType::Menu(menu) => {
                    let key = &path.segments()[index];
                    index += 1;
                    menu.get_child_by_key(key)
                        .ok_or_else(|| ResolveError::MissingPath(path.as_key()))?
                }
                ElementType::OneOf(one_of) => Self::selected_variant(one_of, path)?,
                ElementType::Item(_) => {
                    return Err(ResolveError::MissingPath(path.as_key()));
                }
            };
        }
    }

    pub fn resolve_mut<'a>(
        root: &'a mut MenuRoot,
        path: &ElementPath,
    ) -> Result<&'a mut ElementType, ResolveError> {
        if path.is_root() {
            return Ok(&mut root.menu);
        }

        let mut current = &mut root.menu;
        let mut index = 0;

        loop {
            if index >= path.len() {
                return Ok(current);
            }

            current = match current {
                ElementType::Menu(menu) => {
                    let key = &path.segments()[index];
                    index += 1;
                    menu.get_child_mut_by_key(key)
                        .ok_or_else(|| ResolveError::MissingPath(path.as_key()))?
                }
                ElementType::OneOf(one_of) => Self::selected_variant_mut(one_of, path)?,
                ElementType::Item(_) => {
                    return Err(ResolveError::MissingPath(path.as_key()));
                }
            };
        }
    }

    pub fn menu<'a>(root: &'a MenuRoot, path: &ElementPath) -> Result<&'a Menu, ResolveError> {
        match Self::resolve(root, path)? {
            ElementType::Menu(menu) => Ok(menu),
            ElementType::OneOf(one_of) => match one_of.selected() {
                Some(ElementType::Menu(menu)) => Ok(menu),
                Some(_) => Err(ResolveError::ExpectedMenu(path.as_key())),
                None => Err(ResolveError::UnselectedVariant(path.as_key())),
            },
            ElementType::Item(_) => Err(ResolveError::ExpectedMenu(path.as_key())),
        }
    }

    pub fn menu_mut<'a>(
        root: &'a mut MenuRoot,
        path: &ElementPath,
    ) -> Result<&'a mut Menu, ResolveError> {
        match Self::resolve_mut(root, path)? {
            ElementType::Menu(menu) => Ok(menu),
            ElementType::OneOf(one_of) => match one_of.selected_mut() {
                Some(ElementType::Menu(menu)) => Ok(menu),
                Some(_) => Err(ResolveError::ExpectedMenu(path.as_key())),
                None => Err(ResolveError::UnselectedVariant(path.as_key())),
            },
            ElementType::Item(_) => Err(ResolveError::ExpectedMenu(path.as_key())),
        }
    }

    fn selected_variant<'a>(
        one_of: &'a OneOf,
        path: &ElementPath,
    ) -> Result<&'a ElementType, ResolveError> {
        one_of
            .selected()
            .ok_or_else(|| ResolveError::UnselectedVariant(path.as_key()))
    }

    fn selected_variant_mut<'a>(
        one_of: &'a mut OneOf,
        path: &ElementPath,
    ) -> Result<&'a mut ElementType, ResolveError> {
        one_of
            .selected_mut()
            .ok_or_else(|| ResolveError::UnselectedVariant(path.as_key()))
    }
}

#[cfg(test)]
mod tests {
    use crate::data::{
        item::{Item, ItemType},
        menu::{Menu, MenuRoot},
        oneof::OneOf,
        path::ElementPath,
        resolver::ElementResolver,
        types::{ElementBase, ElementType},
    };

    fn string_item(path: &str) -> ElementType {
        ElementType::Item(Item {
            base: ElementBase {
                path: ElementPath::parse(path).to_path_buf(),
                title: path.to_string(),
                help: None,
                is_required: true,
                struct_name: "string".to_string(),
            },
            item_type: ItemType::String {
                value: None,
                default: None,
            },
        })
    }

    #[test]
    fn resolve_nested_menu_through_oneof() {
        let dog_menu = Menu {
            base: ElementBase {
                path: ElementPath::parse("animal").to_path_buf(),
                title: "animal".to_string(),
                help: None,
                is_required: true,
                struct_name: "Dog".to_string(),
            },
            children: vec![string_item("animal.name")],
            is_set: true,
        };

        let root = MenuRoot {
            schema_version: "test".to_string(),
            title: "root".to_string(),
            menu: ElementType::Menu(Menu {
                base: ElementBase {
                    path: Default::default(),
                    title: "root".to_string(),
                    help: None,
                    is_required: true,
                    struct_name: "Root".to_string(),
                },
                children: vec![ElementType::OneOf(OneOf {
                    base: ElementBase {
                        path: ElementPath::parse("animal").to_path_buf(),
                        title: "animal".to_string(),
                        help: None,
                        is_required: true,
                        struct_name: "Animal".to_string(),
                    },
                    variants: vec![ElementType::Menu(dog_menu)],
                    selected_index: Some(0),
                    default_index: None,
                })],
                is_set: true,
            }),
        };

        let element = ElementResolver::resolve(&root, &ElementPath::parse("animal.name")).unwrap();
        assert_eq!(element.field_name(), "name");
    }
}
