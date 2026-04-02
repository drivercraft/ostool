use crate::{
    data::{AppState, item::ItemType, types::ElementType},
    ui::components::icon::ItemDisplay,
};
use cursive::{
    Cursive,
    align::HAlign,
    event::{Event, Key},
    theme::{ColorStyle, Effect, Style},
    utils::markup::StyledString,
    view::{IntoBoxedView, Nameable, Resizable, Scrollable},
    views::{Dialog, DummyView, LinearLayout, OnEventView, Panel, SelectView, TextView},
};

use super::editors::*;

const MENU_SELECT_NAME: &str = "menu_select";
const TITLE_TEXT_NAME: &str = "menu_title";
const PATH_TEXT_NAME: &str = "menu_path";
const DETAIL_TEXT_NAME: &str = "menu_detail";

pub fn install_menu_view(s: &mut Cursive) {
    s.add_fullscreen_layer(menu_view());
    refresh_menu(s);
}

pub fn refresh_menu(s: &mut Cursive) {
    let Some(app) = s.user_data::<AppState>() else {
        return;
    };
    let Some(menu) = app.current_menu() else {
        return;
    };

    let title = menu.title.clone();
    let path = app.current_path().display();
    let fields = menu.fields();
    let selected_index = app.selected_index();
    let detail = app
        .current()
        .map(format_detail)
        .unwrap_or_else(|| "No items in this menu.".to_string());

    s.call_on_name(TITLE_TEXT_NAME, |view: &mut TextView| {
        view.set_content(format!(" {} ", title));
    });
    s.call_on_name(PATH_TEXT_NAME, |view: &mut TextView| {
        view.set_content(path);
    });
    s.call_on_name(DETAIL_TEXT_NAME, |view: &mut TextView| {
        view.set_content(detail);
    });
    s.call_on_name(MENU_SELECT_NAME, |view: &mut SelectView<ElementType>| {
        view.clear();
        for field in &fields {
            view.add_item(format_item_label(field), field.clone());
        }
        if !fields.is_empty() {
            view.set_selection(selected_index.min(fields.len().saturating_sub(1)));
        }
    });
}

fn menu_view() -> impl IntoBoxedView {
    let mut select = SelectView::new();
    select.set_autojump(true);
    select.set_on_select(on_select);
    select.set_on_submit(on_submit);

    OnEventView::new(
        LinearLayout::vertical()
            .child(DummyView.fixed_height(1))
            .child(
                TextView::new("")
                    .center()
                    .with_name(TITLE_TEXT_NAME)
                    .full_width(),
            )
            .child(DummyView.fixed_height(1))
            .child(Panel::new(TextView::new("").with_name(PATH_TEXT_NAME)).title("Current Path"))
            .child(DummyView.fixed_height(1))
            .child(
                LinearLayout::horizontal()
                    .child(
                        Panel::new(select.with_name(MENU_SELECT_NAME).scrollable())
                            .title("Items")
                            .full_width()
                            .full_height(),
                    )
                    .child(DummyView.fixed_width(1))
                    .child(
                        Panel::new(TextView::new("").with_name(DETAIL_TEXT_NAME).scrollable())
                            .title("Details")
                            .fixed_width(44)
                            .full_height(),
                    ),
            )
            .child(DummyView.fixed_height(1))
            .child(
                Panel::new(TextView::new(create_help_text()))
                    .title("Keyboard Shortcuts")
                    .title_position(HAlign::Center),
            ),
    )
    .on_event(Event::Char('m'), on_change_set)
    .on_event(Event::Char('M'), on_change_set)
    .on_event(Event::Char('c'), on_clear)
    .on_event(Event::Char('C'), on_clear)
    .on_event(Event::Char('h'), on_show_help)
    .on_event(Event::Char('H'), on_show_help)
    .on_event(Event::Char('j'), |s| move_selection(s, 1))
    .on_event(Event::Char('k'), |s| move_selection(s, -1))
    .on_event(Key::Tab, on_oneof_switch)
}

fn move_selection(s: &mut Cursive, delta: isize) {
    let next_index = {
        let Some(app) = s.user_data::<AppState>() else {
            return;
        };
        let Some(menu) = app.current_menu() else {
            return;
        };
        if menu.children.is_empty() {
            return;
        }

        let current = app.selected_index() as isize;
        (current + delta).clamp(0, menu.children.len() as isize - 1) as usize
    };

    if let Some(app) = s.user_data::<AppState>() {
        app.set_selected_index(next_index);
    }
    refresh_menu(s);
}

fn on_select(s: &mut Cursive, item: &ElementType) {
    if let Some(app) = s.user_data::<AppState>() {
        app.set_selected_by_key(&item.key());
    }
    s.call_on_name(DETAIL_TEXT_NAME, |view: &mut TextView| {
        view.set_content(format_detail(item));
    });
}

fn on_submit(s: &mut Cursive, _item: &ElementType) {
    open_selected(s);
}

fn on_clear(s: &mut Cursive) {
    let Some(app) = s.user_data::<AppState>() else {
        return;
    };
    let Some(path) = app.selected_path() else {
        return;
    };

    if let Some(app) = s.user_data::<AppState>()
        && let Some(element) = app.get_mut_by_key(&path.as_key())
    {
        element.set_none();
        app.mark_dirty();
    }
    refresh_menu(s);
}

fn on_change_set(s: &mut Cursive) {
    let Some(app) = s.user_data::<AppState>() else {
        return;
    };
    let Some(path) = app.selected_path() else {
        return;
    };

    if let Some(app) = s.user_data::<AppState>()
        && let Some(ElementType::Menu(menu)) = app.get_mut_by_key(&path.as_key())
        && !menu.is_required
    {
        menu.is_set = !menu.is_set;
        app.mark_dirty();
    }
    refresh_menu(s);
}

fn on_oneof_switch(s: &mut Cursive) {
    let Some(app) = s.user_data::<AppState>() else {
        return;
    };
    let Some(path) = app.selected_path() else {
        return;
    };
    let Some(ElementType::OneOf(one_of)) = app.get_by_key(&path.as_key()).cloned() else {
        return;
    };

    show_oneof_dialog(s, &path.as_key(), &one_of);
}

fn on_show_help(s: &mut Cursive) {
    let detail = {
        let Some(app) = s.user_data::<AppState>() else {
            return;
        };
        let Some(item) = app.current() else {
            return;
        };
        format_detail(item)
    };

    s.add_layer(
        Dialog::around(TextView::new(detail).scrollable())
            .title("Item Details")
            .dismiss_button("Close"),
    );
}

fn open_selected(s: &mut Cursive) {
    let Some(app) = s.user_data::<AppState>() else {
        return;
    };
    let Some(path) = app.selected_path() else {
        return;
    };
    let Some(element) = app.get_by_key(&path.as_key()).cloned() else {
        return;
    };

    if let Some(hook) = app.find_selected_hook() {
        (hook.callback)(s, &hook.path);
        return;
    }

    match element {
        ElementType::Menu(menu) => {
            if let Some(app) = s.user_data::<AppState>()
                && let Some(ElementType::Menu(current_menu)) = app.get_mut_by_key(&path.as_key())
                && !current_menu.is_required
                && !current_menu.is_set
            {
                current_menu.is_set = true;
                app.mark_dirty();
            }
            if let Some(app) = s.user_data::<AppState>() {
                app.enter_menu(path);
            }
            refresh_menu(s);
            let _ = menu;
        }
        ElementType::OneOf(one_of) => {
            if matches!(one_of.selected(), Some(ElementType::Menu(_))) {
                if let Some(app) = s.user_data::<AppState>() {
                    app.enter_menu(path);
                }
                refresh_menu(s);
            } else {
                show_oneof_dialog(s, &path.as_key(), &one_of);
            }
        }
        ElementType::Item(item) => match &item.item_type {
            ItemType::Boolean { .. } => {
                if let Some(app) = s.user_data::<AppState>()
                    && let Some(ElementType::Item(item)) = app.get_mut_by_key(&path.as_key())
                    && let ItemType::Boolean { value, .. } = &mut item.item_type
                {
                    *value = !*value;
                    app.mark_dirty();
                }
                refresh_menu(s);
            }
            ItemType::String { value, default } => {
                show_string_edit(s, &path.as_key(), &item.base.title, value, default);
            }
            ItemType::Number { value, default } => {
                show_number_edit(s, &path.as_key(), &item.base.title, *value, *default);
            }
            ItemType::Integer { value, default } => {
                show_integer_edit(s, &path.as_key(), &item.base.title, *value, *default);
            }
            ItemType::Enum(enum_item) => {
                show_enum_select(s, &path.as_key(), &item.base.title, enum_item);
            }
            ItemType::Array(array_item) => {
                show_array_edit(s, &path.as_key(), &item.base.title, &array_item.values);
            }
        },
    }
}

pub fn format_item_label(element: &ElementType) -> StyledString {
    let mut label = StyledString::new();
    label.append_plain(element.icon());
    label.append_plain(" ");
    label.append_styled(&element.title, ColorStyle::title_secondary());

    let value = element.value();
    if !value.is_empty() {
        label.append_plain("  ");
        label.append_styled(value, ColorStyle::secondary());
    }

    label
}

fn create_help_text() -> StyledString {
    let mut text = StyledString::new();
    text.append_styled("↑↓/jk", Style::from(Effect::Bold));
    text.append_plain(" Move  ");
    text.append_styled("Enter", Style::from(Effect::Bold));
    text.append_plain(" Open/Edit  ");
    text.append_styled("Esc", Style::from(Effect::Bold));
    text.append_plain(" Back  ");
    text.append_styled("Tab", Style::from(Effect::Bold));
    text.append_plain(" Switch OneOf\n");
    text.append_styled("M", Style::from(Effect::Bold));
    text.append_plain(" Toggle optional menu  ");
    text.append_styled("C", Style::from(Effect::Bold));
    text.append_plain(" Clear  ");
    text.append_styled("H", Style::from(Effect::Bold));
    text.append_plain(" Help  ");
    text.append_styled("S", Style::from(Effect::Bold));
    text.append_plain(" Save  ");
    text.append_styled("Q", Style::from(Effect::Bold));
    text.append_plain(" Quit");
    text
}

fn format_detail(element: &ElementType) -> String {
    match element {
        ElementType::Menu(menu) => {
            let mut text = String::new();
            text.push_str(&format!("Menu: {}\n", menu.title));
            if let Some(help) = &menu.help {
                text.push_str(help);
                text.push('\n');
            }
            text.push_str(&format!("Items: {}\n", menu.children.len()));
            text.push_str(&format!(
                "Required: {}\nEnabled: {}",
                if menu.is_required { "yes" } else { "no" },
                if menu.is_set { "yes" } else { "no" }
            ));
            text
        }
        ElementType::OneOf(one_of) => {
            let mut text = String::new();
            text.push_str(&format!("OneOf: {}\n", one_of.title));
            if let Some(help) = &one_of.help {
                text.push_str(help);
                text.push('\n');
            }
            if let Some(index) = one_of.selected_index {
                text.push_str(&format!("Selected: {}\n", one_of.variant_display(index)));
            } else {
                text.push_str("Selected: <unset>\n");
            }
            text.push_str("Tab switches the active variant.");
            text
        }
        ElementType::Item(item) => {
            let mut text = String::new();
            text.push_str(&format!("Field: {}\n", item.base.title));
            if let Some(help) = &item.base.help {
                text.push_str(help);
                text.push('\n');
            }
            text.push_str(&format!("Value: {}\n", element.value()));
            text.push_str(match &item.item_type {
                ItemType::String { .. } => "Type: string",
                ItemType::Number { .. } => "Type: number",
                ItemType::Integer { .. } => "Type: integer",
                ItemType::Boolean { .. } => "Type: boolean",
                ItemType::Enum(enum_item) => {
                    return format!(
                        "{}\nType: enum\nOptions: {}",
                        text,
                        enum_item.variants.join(", ")
                    );
                }
                ItemType::Array(array_item) => {
                    return format!("{}\nType: array\nItems: {}", text, array_item.values.len());
                }
            });
            text
        }
    }
}
