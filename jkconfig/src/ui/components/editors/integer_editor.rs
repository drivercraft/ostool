use cursive::{
    Cursive,
    view::{Nameable, Resizable},
    views::{Dialog, DummyView, EditView, LinearLayout, TextView},
};

use crate::{
    data::{AppState, item::ItemType, types::ElementType},
    ui::{components::menu::refresh_menu, handle_back},
};

/// 显示整数编辑对话框
pub fn show_integer_edit(
    s: &mut Cursive,
    key: &str,
    title: &str,
    value: Option<i64>,
    default: Option<i64>,
) {
    let initial = value.or(default).map(|v| v.to_string()).unwrap_or_default();
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
                        .fixed_width(30),
                ),
        )
        .title("Edit Integer")
        .button("OK", move |s| {
            let content = s
                .call_on_name("edit_value", |v: &mut EditView| v.get_content())
                .unwrap();

            match content.parse::<i64>() {
                Ok(num) => {
                    if let Some(app) = s.user_data::<AppState>()
                        && let Some(ElementType::Item(item)) = app.get_mut_by_key(&key)
                        && let ItemType::Integer { value, .. } = &mut item.item_type
                    {
                        *value = Some(num);
                        app.mark_dirty();
                    }
                    handle_back(s);
                    refresh_menu(s);
                }
                Err(_) => {
                    s.add_layer(Dialog::info("Invalid integer format!").dismiss_button("Ok"));
                }
            }
        })
        .button("Cancel", handle_back),
    );
}
