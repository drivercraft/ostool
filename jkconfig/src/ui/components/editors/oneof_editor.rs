use cursive::{
    Cursive,
    event::Key,
    view::{Nameable, Resizable},
    views::{Dialog, DummyView, LinearLayout, OnEventView, SelectView, TextView},
};

use crate::{
    data::{AppState, oneof::OneOf, types::ElementType},
    ui::{components::menu::refresh_menu, handle_back},
};

/// 显示 OneOf 选择对话框
pub fn show_oneof_dialog(s: &mut Cursive, path: &str, one_of: &OneOf) {
    let mut select = SelectView::new();
    let path = path.to_string();
    let path_submit = path.clone();

    for (idx, _) in one_of.variants.iter().enumerate() {
        let display = one_of.variant_display(idx);
        let label = if Some(idx) == one_of.selected_index {
            format!("(*) {display}")
        } else {
            format!("( ) {display}")
        };
        select.add_item(label, idx);
    }

    s.add_layer(
        OnEventView::new(
            Dialog::around(
                LinearLayout::vertical()
                    .child(TextView::new(format!("Select variant: {}", one_of.title)))
                    .child(DummyView)
                    .child(select.with_name("oneof_select").fixed_height(10)),
            )
            .title("Select One Of")
            .button("OK", move |s| on_ok(s, &path_submit))
            .button("Cancel", handle_back),
        )
        .on_event(Key::Enter, move |s| on_ok(s, &path)),
    );
}

fn on_ok(s: &mut Cursive, path: &str) {
    let selection = s
        .call_on_name("oneof_select", |v: &mut SelectView<usize>| v.selection())
        .unwrap();

    if let Some(idx) = selection
        && let Some(app) = s.user_data::<AppState>()
        && let Some(ElementType::OneOf(one_of)) = app.get_mut_by_key(path)
    {
        let _ = one_of.set_selected_index(*idx);
        app.mark_dirty();
    }
    handle_back(s);
    refresh_menu(s);
}
