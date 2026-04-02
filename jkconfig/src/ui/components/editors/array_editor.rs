use cursive::{
    Cursive,
    event::Key,
    theme::{ColorStyle, Effect, Style},
    utils::markup::StyledString,
    view::{Nameable, Resizable, Scrollable},
    views::{Dialog, DummyView, EditView, LinearLayout, OnEventView, Panel, SelectView, TextView},
};

use crate::{
    data::{AppState, item::ItemType, types::ElementType},
    ui::{components::menu::refresh_menu, handle_back},
};

/// 显示数组编辑对话框
pub fn show_array_edit(s: &mut Cursive, key: &str, title: &str, values: &[String]) {
    let key_clone = key.to_string();
    let mut select = SelectView::new();

    // Add existing items to the list
    for (idx, value) in values.iter().enumerate() {
        let mut label = StyledString::new();
        label.append_styled(format!("[{}]", idx), ColorStyle::secondary());
        label.append_plain(" ");
        label.append_plain(value);
        select.add_item(label, idx);
    }

    // Add "Add new item" option
    let mut add_label = StyledString::new();
    add_label.append_styled("➕ ", ColorStyle::tertiary());
    add_label.append_styled("Add new item", Style::from(Effect::Italic));
    select.add_item(add_label, usize::MAX);

    // Create help text
    let mut help_text = StyledString::new();
    help_text.append_styled("Enter", Style::from(Effect::Bold));
    help_text.append_plain(" Edit/Add  ");
    help_text.append_styled("Del", Style::from(Effect::Bold));
    help_text.append_plain(" Delete  ");
    help_text.append_styled("Esc", Style::from(Effect::Bold));
    help_text.append_plain(" Back");

    s.add_layer(
        OnEventView::new(
            Dialog::around(
                LinearLayout::vertical()
                    .child(TextView::new(format!("📋 Array Editor: {}", title)).center())
                    .child(DummyView)
                    .child(
                        Panel::new(
                            select
                                .with_name("array_select")
                                .scrollable()
                                .fixed_height(15),
                        )
                        .title(format!("Items ({})", values.len()))
                        .full_width(),
                    )
                    .child(DummyView)
                    .child(Panel::new(TextView::new(help_text)).full_width()),
            )
            .title("Array Editor")
            .button("Done", move |s| {
                handle_back(s);
            }),
        )
        .on_event(Key::Enter, move |s| {
            on_enter(s, &key_clone);
        })
        .on_event(Key::Del, on_delete),
    );
}

fn on_enter(s: &mut Cursive, key: &str) {
    let selection = s
        .call_on_name("array_select", |v: &mut SelectView<usize>| v.selection())
        .unwrap();

    if let Some(idx) = selection {
        if *idx == usize::MAX {
            // Add new item
            show_add_item_dialog(s, key);
        } else {
            // Edit existing item
            show_edit_item_dialog(s, key, *idx);
        }
    }
}

fn on_delete(s: &mut Cursive) {
    let selection = s
        .call_on_name("array_select", |v: &mut SelectView<usize>| v.selection())
        .unwrap();

    if let Some(idx) = selection
        && *idx != usize::MAX
    {
        // Get the value to display in confirmation
        let value = if let Some(app) = s.user_data::<AppState>()
            && let Some(ElementType::Item(item)) = app.current()
            && let ItemType::Array(array_item) = &item.item_type
            && *idx < array_item.values.len()
        {
            array_item.values[*idx].clone()
        } else {
            return;
        };

        s.add_layer(
            Dialog::around(
                LinearLayout::vertical()
                    .child(TextView::new(
                        "⚠️  Are you sure you want to delete this item?",
                    ))
                    .child(DummyView)
                    .child(TextView::new(format!("  [{}] {}", idx, value))),
            )
            .title("Confirm Delete")
            .button("Yes", move |s| {
                if let Some(app) = s.user_data::<AppState>()
                    && let Some(ElementType::Item(item)) = app.current_mut()
                    && let ItemType::Array(array_item) = &mut item.item_type
                    && *idx < array_item.values.len()
                {
                    array_item.values.remove(*idx);
                    app.mark_dirty();
                    s.pop_layer(); // Close confirm dialog
                    refresh_array_view(s);
                    refresh_menu(s);
                }
            })
            .button("No", |s| {
                s.pop_layer();
            }),
        );
    }
}

fn show_add_item_dialog(s: &mut Cursive, key: &str) {
    let key = key.to_string();
    s.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new("➕ Enter new value:"))
                .child(DummyView)
                .child(
                    Panel::new(EditView::new().with_name("new_item_value").fixed_width(48))
                        .title("Value"),
                ),
        )
        .title("Add Item")
        .button("Add", move |s| {
            let content = s
                .call_on_name("new_item_value", |v: &mut EditView| v.get_content())
                .unwrap();

            if !content.is_empty() {
                if let Some(app) = s.user_data::<AppState>()
                    && let Some(ElementType::Item(item)) = app.get_mut_by_key(&key)
                    && let ItemType::Array(array_item) = &mut item.item_type
                {
                    array_item.values.push(content.to_string());
                    app.mark_dirty();
                    s.pop_layer(); // Close add dialog
                    refresh_array_view(s);
                    refresh_menu(s);
                }
            } else {
                s.add_layer(
                    Dialog::text("⚠️  Value cannot be empty!")
                        .title("Error")
                        .dismiss_button("OK"),
                );
            }
        })
        .button("Cancel", |s| {
            s.pop_layer();
        }),
    );
}

fn show_edit_item_dialog(s: &mut Cursive, key: &str, idx: usize) {
    let key = key.to_string();

    // Get current value
    let current_value = if let Some(app) = s.user_data::<AppState>()
        && let Some(ElementType::Item(item)) = app.get_by_key(&key)
        && let ItemType::Array(array_item) = &item.item_type
        && idx < array_item.values.len()
    {
        array_item.values[idx].clone()
    } else {
        String::new()
    };

    s.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(format!("✏️  Edit item [{}]:", idx)))
                .child(DummyView)
                .child(
                    Panel::new(
                        EditView::new()
                            .content(current_value)
                            .with_name("edit_item_value")
                            .fixed_width(48),
                    )
                    .title("Value"),
                ),
        )
        .title("Edit Item")
        .button("Save", move |s| {
            let content = s
                .call_on_name("edit_item_value", |v: &mut EditView| v.get_content())
                .unwrap();

            if !content.is_empty() {
                if let Some(app) = s.user_data::<AppState>()
                    && let Some(ElementType::Item(item)) = app.get_mut_by_key(&key)
                    && let ItemType::Array(array_item) = &mut item.item_type
                    && idx < array_item.values.len()
                {
                    array_item.values[idx] = content.to_string();
                    app.mark_dirty();
                    s.pop_layer(); // Close edit dialog
                    refresh_array_view(s);
                    refresh_menu(s);
                }
            } else {
                s.add_layer(
                    Dialog::text("⚠️  Value cannot be empty!")
                        .title("Error")
                        .dismiss_button("OK"),
                );
            }
        })
        .button("Cancel", |s| {
            s.pop_layer();
        }),
    );
}

fn refresh_array_view(s: &mut Cursive) {
    // Get current array values
    let values = if let Some(app) = s.user_data::<AppState>()
        && let Some(ElementType::Item(item)) = app.current()
        && let ItemType::Array(array_item) = &item.item_type
    {
        array_item.values.clone()
    } else {
        return;
    };

    // Update the select view
    s.call_on_name("array_select", |view: &mut SelectView<usize>| {
        view.clear();
        for (idx, value) in values.iter().enumerate() {
            let mut label = StyledString::new();
            label.append_styled(format!("[{}]", idx), ColorStyle::secondary());
            label.append_plain(" ");
            label.append_plain(value);
            view.add_item(label, idx);
        }
        // Re-add "Add new item" option
        let mut add_label = StyledString::new();
        add_label.append_styled("➕ ", ColorStyle::tertiary());
        add_label.append_styled("Add new item", Style::from(Effect::Italic));
        view.add_item(add_label, usize::MAX);
    });
}
