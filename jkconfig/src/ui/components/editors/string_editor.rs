use cursive::{
    Cursive,
    view::{Nameable, Resizable},
    views::{Dialog, DummyView, EditView, LinearLayout, TextView},
};

use crate::{
    data::{item::ItemType, types::ElementType},
    ui::handle_back,
};

/// 显示字符串编辑对话框
pub fn show_string_edit(
    s: &mut Cursive,
    key: &str,
    title: &str,
    value: &Option<String>,
    default: &Option<String>,
) {
    let initial = value
        .clone()
        .or_else(|| default.clone())
        .unwrap_or_default();
    let key = key.to_string();

    s.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(format!("Edit: {}", title)))
                .child(DummyView)
                .child(
                    EditView::new()
                        .content(initial)
                        .with_name("edit_value")
                        .fixed_width(50),
                ),
        )
        .title("Edit String")
        .button("OK", move |s| {
            let st = s
                .call_on_name("edit_value", |v: &mut EditView| v.get_content())
                .unwrap();

            if let Some(app) = s.user_data::<crate::data::app_data::AppData>()
                && let Some(ElementType::Item(item)) = app.root.get_mut_by_key(&key)
                && let ItemType::String { value, .. } = &mut item.item_type
            {
                *value = Some(st.to_string());
            }
            handle_back(s);
        })
        .button("Cancel", handle_back),
    );
}
