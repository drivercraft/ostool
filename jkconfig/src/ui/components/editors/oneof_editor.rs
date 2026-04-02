use cursive::{
    Cursive,
    event::Key,
    view::{Nameable, Resizable},
    views::{Dialog, DummyView, LinearLayout, OnEventView, SelectView, TextView},
};

use crate::{data::oneof::OneOf, ui::handle_back};

/// 显示 OneOf 选择对话框
pub fn show_oneof_dialog(s: &mut Cursive, one_of: &OneOf) {
    let mut select = SelectView::new();

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
            .button("OK", on_ok)
            .button("Cancel", handle_back),
        )
        .on_event(Key::Enter, on_ok),
    );
}

fn on_ok(s: &mut Cursive) {
    let selection = s
        .call_on_name("oneof_select", |v: &mut SelectView<usize>| v.selection())
        .unwrap();

    if let Some(idx) = selection
        && let Some(app) = s.user_data::<crate::data::app_data::AppData>()
        && let Some(current) = app.current_mut()
        && let crate::data::types::ElementType::OneOf(one_of) = current
    {
        let _ = one_of.set_selected_index(*idx);

        handle_back(s);
    }
}
