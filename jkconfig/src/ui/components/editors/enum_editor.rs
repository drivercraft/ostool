use cursive::{
    Cursive,
    event::Key,
    view::{Nameable, Resizable},
    views::{Dialog, DummyView, LinearLayout, OnEventView, SelectView, TextView},
};

use crate::{
    data::{
        AppState,
        item::{EnumItem, ItemType},
        types::ElementType,
    },
    ui::{components::menu::refresh_menu, handle_back},
};

/// 显示枚举选择对话框
pub fn show_enum_select(s: &mut Cursive, path: &str, title: &str, enum_item: &EnumItem) {
    let mut select = SelectView::new();
    let path = path.to_string();
    let path_submit = path.clone();

    for (idx, variant) in enum_item.variants.iter().enumerate() {
        let label = if Some(idx) == enum_item.value {
            format!("[*] {}", variant)
        } else {
            format!("[ ] {}", variant)
        };
        select.add_item(label, idx);
    }

    s.add_layer(
        OnEventView::new(
            Dialog::around(
                LinearLayout::vertical()
                    .child(TextView::new(format!("Select: {}", title)))
                    .child(DummyView)
                    .child(select.with_name("enum_select").fixed_height(10)),
            )
            .title("Select Option")
            .button("OK", move |s| on_ok(s, &path_submit))
            .button("Cancel", handle_back),
        )
        .on_event(Key::Enter, move |s| on_ok(s, &path)),
    );
}

fn on_ok(s: &mut Cursive, path: &str) {
    let selection = s
        .call_on_name("enum_select", |v: &mut SelectView<usize>| v.selection())
        .unwrap();
    let Some(selection) = selection else {
        return;
    };

    if let Some(app) = s.user_data::<AppState>()
        && let Some(ElementType::Item(item)) = app.get_mut_by_key(path)
        && let ItemType::Enum(en) = &mut item.item_type
    {
        en.value = Some(*selection);
        app.mark_dirty();
    }
    handle_back(s);
    refresh_menu(s);
}

pub fn show_list_select(
    s: &mut Cursive,
    title: &str,
    items: &[String],
    path: &str,
    on_ok: fn(&mut AppState, path: &str, selected: &str),
) {
    let mut select = SelectView::new();

    for (idx, item) in items.iter().enumerate() {
        select.add_item(item.clone(), idx);
    }
    let items1 = items.to_vec(); // 克隆以供闭包使用
    let items2 = items.to_vec();

    let path1 = path.to_string();
    let path2 = path.to_string();

    s.add_layer(
        OnEventView::new(
            Dialog::around(
                LinearLayout::vertical()
                    .child(TextView::new(format!("Select: {}", title)))
                    .child(DummyView)
                    .child(select.with_name("list_select").fixed_height(10)),
            )
            .title("Select Item")
            .button("OK", move |s| {
                on_list_ok(s, &items1, &path1, on_ok);
            })
            .button("Cancel", handle_back),
        )
        .on_event(Key::Enter, move |s| {
            on_list_ok(s, &items2, &path2, on_ok);
        }),
    );
}

fn on_list_ok(
    s: &mut Cursive,
    ls: &[String],
    path: &str,
    on_ok: fn(&mut AppState, path: &str, selected: &str),
) {
    let selection = s
        .call_on_name("list_select", |v: &mut SelectView<usize>| v.selection())
        .unwrap();
    let Some(selection) = selection else {
        return;
    };

    let Some(app) = s.user_data::<AppState>() else {
        return;
    };

    on_ok(app, path, ls[*selection].as_str());
    app.mark_dirty();

    handle_back(s);
    refresh_menu(s);
}
