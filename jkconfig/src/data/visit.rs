use crate::data::{menu::Menu, oneof::OneOf, types::ElementType};

pub trait Visit {
    fn visit_element(&mut self, node: &ElementType) {
        visit_element(self, node);
    }

    fn visit_menu(&mut self, node: &Menu) {
        visit_menu(self, node);
    }

    fn visit_one_of(&mut self, node: &OneOf) {
        visit_one_of(self, node);
    }
}

pub fn visit_element<V>(visitor: &mut V, node: &ElementType)
where
    V: Visit + ?Sized,
{
    match node {
        ElementType::Menu(menu) => visitor.visit_menu(menu),
        ElementType::OneOf(one_of) => visitor.visit_one_of(one_of),
        ElementType::Item(_) => {}
    }
}

pub fn visit_menu<V>(visitor: &mut V, node: &Menu)
where
    V: Visit + ?Sized,
{
    for child in &node.children {
        visitor.visit_element(child);
    }
}

pub fn visit_one_of<V>(visitor: &mut V, node: &OneOf)
where
    V: Visit + ?Sized,
{
    for variant in &node.variants {
        visitor.visit_element(variant);
    }
}
