use crate::{
    data::{AppData, item::ItemType, menu::Menu, types::ElementType},
    ui::{components::icon::ItemDisplay, handle_edit},
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

/// 创建菜单视图
pub fn menu_view(title: &str, path: &str, fields: Vec<ElementType>) -> impl IntoBoxedView {
    let menu_select_name = menu_view_name(path);
    let mut select = SelectView::new();
    select.set_autojump(true);

    select.set_on_select(on_select);
    select.set_on_submit(on_submit);
    menu_select_flush_fields(&mut select, &fields);
    let select = select.with_name(&menu_select_name);

    // 创建标题栏
    let mut title_text = StyledString::new();
    title_text.append_styled("╔═══", ColorStyle::title_primary());
    title_text.append_styled(format!(" {} ", title), Style::from(Effect::Bold));
    title_text.append_styled("═══╗", ColorStyle::title_primary());
    let title_view = TextView::new(title_text).center();

    // 创建路径显示面板
    let path_text = if path.is_empty() {
        let mut styled = StyledString::new();
        styled.append_styled("📂 ", ColorStyle::tertiary());
        styled.append_styled("/ ", Style::from(Effect::Bold));
        styled.append_styled("(Root)", ColorStyle::secondary());
        styled
    } else {
        let mut styled = StyledString::new();
        styled.append_styled("📂 ", ColorStyle::tertiary());
        styled.append_styled(path, Style::from(Effect::Bold));
        styled
    };
    let path_view = TextView::new(path_text).with_name("path_text");

    // 创建帮助信息显示区域
    let help_view = TextView::new(create_help_text()).with_name("help_text");

    // 构建主布局 - 使用更灵活的布局来适应窗口大小
    OnEventView::new(
        LinearLayout::vertical()
            .child(DummyView.fixed_height(1))
            .child(title_view)
            .child(DummyView.fixed_height(1))
            .child(Panel::new(path_view).title("Current Path").full_width())
            .child(DummyView.fixed_height(1))
            // 列表区域占据大部分空间，自动滚动
            .child(
                Panel::new(select.scrollable())
                    .title("Items")
                    .full_width()
                    .full_height(), // 使用 full_height 让列表占据剩余空间
            )
            .child(DummyView.fixed_height(1))
            // 帮助区域固定高度，确保完全显示
            .child(
                Panel::new(help_view)
                    .title("Keyboard Shortcuts")
                    .full_width()
                    .fixed_height(7), // 固定高度确保按键提示完全显示
            )
            .child(DummyView.fixed_height(1)),
    )
    .on_event(Event::Char('m'), on_change_set)
    .on_event(Event::Char('M'), on_change_set)
    .on_event(Key::Tab, on_oneof_switch)
    .on_event(Event::Char('c'), on_clear)
    .on_event(Event::Char('C'), on_clear)
    .on_event(Event::Char('h'), on_show_help)
    .on_event(Event::Char('H'), on_show_help)
}

fn on_clear(s: &mut Cursive) {
    let Some(_selected) = menu_selected(s) else {
        return;
    };

    update_selected(s, |elem| elem.set_none());
}

fn update_selected(s: &mut Cursive, f: impl Fn(&mut ElementType)) {
    let Some(selected) = menu_selected(s) else {
        return;
    };

    if let Some(app) = s.user_data::<AppData>()
        && let Some(elem) = app.root.get_mut_by_key(&selected.key())
    {
        f(elem);
        menu_flush(s);
    }
}

fn menu_selected(s: &mut Cursive) -> Option<ElementType> {
    let mut selected = None;
    let name = menu_view_name(&menu_key(s));
    s.call_on_name(&name, |view: &mut SelectView<ElementType>| {
        if let Some(elem) = view.selection() {
            selected = Some(elem.as_ref().clone());
        }
    });

    selected
}

fn on_change_set(s: &mut Cursive) {
    update_selected(s, |elem| {
        if let ElementType::Menu(menu) = elem
            && !menu.is_required
        {
            menu.is_set = !menu.is_set;
        }
    });
}

fn menu_key(s: &mut Cursive) -> String {
    let app = s.user_data::<AppData>().unwrap();
    app.key_string()
}

fn menu_flush(s: &mut Cursive) {
    let key = menu_key(s);
    menu_select_flush(s, &key);
}

pub fn menu_view_name(path: &str) -> String {
    format!("menu_view_{path}")
}

pub fn menu_select_flush(s: &mut Cursive, path: &str) {
    let Some(app) = s.user_data::<AppData>() else {
        return;
    };

    let menu = match app.root.get_by_key(path) {
        Some(ElementType::Menu(menu)) => menu,
        Some(ElementType::OneOf(oneof)) => {
            if let Some(selected) = oneof.selected()
                && let ElementType::Menu(menu) = selected
            {
                menu
            } else {
                return;
            }
        }
        _ => {
            return;
        }
    };

    let name = menu_view_name(path);
    let fields = menu.fields();
    s.call_on_name(&name, |view: &mut SelectView<ElementType>| {
        menu_select_flush_fields(view, &fields);
    });
}

fn menu_select_flush_fields(view: &mut SelectView<ElementType>, fields: &[ElementType]) {
    let select_old = view.selected_id();
    view.clear();
    // 为每个字段添加带格式的项
    for field in fields {
        let label = format_item_label(field);
        view.add_item(label, field.clone());
    }
    // 恢复之前的选择位置
    if let Some(idx) = select_old
        && idx < view.len()
    {
        view.set_selection(idx);
    }
}

/// 格式化项目标签，显示类型和当前值
pub fn format_item_label(element: &ElementType) -> StyledString {
    let mut label = StyledString::new();
    label.append_plain(element.icon());
    label.append_plain(" ");
    label.append_styled(&element.title, ColorStyle::title_secondary());
    label.append_plain("  ");
    label.append_styled(element.value(), ColorStyle::secondary());

    label
}

/// 创建帮助文本（在底部状态栏显示）
fn create_help_text() -> StyledString {
    let mut text = StyledString::new();

    // 紧凑型三行布局
    // 第一行：导航
    text.append_styled("▶ ", ColorStyle::tertiary());
    text.append_styled("↑↓/jk", Style::from(Effect::Bold));
    text.append_plain(" Move  ");
    text.append_styled("Enter", Style::from(Effect::Bold));
    text.append_plain(" Select  ");
    text.append_styled("Esc", Style::from(Effect::Bold));
    text.append_plain(" Back  ");
    text.append_styled("H", Style::from(Effect::Bold));
    text.append_plain(" Help\n");

    // 第二行：编辑
    text.append_styled("▶ ", ColorStyle::tertiary());
    text.append_styled("C", Style::from(Effect::Bold));
    text.append_plain(" Clear  ");
    text.append_styled("M", Style::from(Effect::Bold));
    text.append_plain(" Toggle  ");
    text.append_styled("Tab", Style::from(Effect::Bold));
    text.append_plain(" Switch\n");

    // 第三行：全局
    text.append_styled("▶ ", ColorStyle::tertiary());
    text.append_styled("S", Style::from(Effect::Bold));
    text.append_plain(" Save & Exit  ");
    text.append_styled("Q", Style::from(Effect::Bold));
    text.append_plain(" Quit");

    text
}

/// 显示帮助对话框，展示当前选中项的详细信息
fn on_show_help(s: &mut Cursive) {
    // 获取当前选中的项
    let element = match menu_selected(s) {
        Some(e) => e,
        None => return,
    };

    // 根据元素类型格式化详情
    let details = match element {
        ElementType::Menu(menu) => {
            let mut text = StyledString::new();
            text.append_styled(
                "📁 Menu\n",
                Style::from(Effect::Bold).combine(ColorStyle::title_primary()),
            );
            text.append_plain("\n");
            text.append_styled("Title: ", Style::from(Effect::Bold));
            text.append_plain(&menu.title);
            text.append_plain("\n\n");

            if let Some(help) = &menu.help {
                text.append_styled("Description:\n", Style::from(Effect::Bold));
                text.append_plain(help);
                text.append_plain("\n\n");
            }

            let item_count = menu.children.len();
            text.append_styled("Items: ", Style::from(Effect::Bold));
            text.append_plain(format!("{} items\n", item_count));

            text
        }
        ElementType::OneOf(oneof) => {
            let mut text = StyledString::new();
            text.append_styled(
                "🔀 OneOf Selector\n",
                Style::from(Effect::Bold).combine(ColorStyle::title_primary()),
            );
            text.append_plain("\n");
            text.append_styled("Property: ", Style::from(Effect::Bold));
            text.append_plain(&oneof.title);
            text.append_plain("\n\n");

            if let Some(help) = &oneof.help {
                text.append_styled("Description:\n", Style::from(Effect::Bold));
                text.append_plain(help);
                text.append_plain("\n\n");
            }

            text.append_styled("Current Variant: ", Style::from(Effect::Bold));
            if let Some(idx) = oneof.selected_index {
                text.append_plain(format!("{}\n\n", idx));
            } else {
                text.append_plain("(none)\n\n");
            }

            text.append_styled("Available Variants:\n", Style::from(Effect::Bold));
            for (i, variant) in oneof.variants.iter().enumerate() {
                let prefix = if Some(i) == oneof.selected_index {
                    "→ "
                } else {
                    "  "
                };
                text.append_plain(format!("{}[{}] {}\n", prefix, i, variant.title));
                if let Some(help) = &variant.help {
                    text.append_plain(format!("    {}\n", help));
                }
            }

            text
        }
        ElementType::Item(item) => {
            let mut text = StyledString::new();

            // 标题和类型
            text.append_styled(
                format!("{}\n", item.base.title),
                Style::from(Effect::Bold).combine(ColorStyle::title_primary()),
            );
            text.append_plain("\n");

            // 类型信息
            text.append_styled("Type: ", Style::from(Effect::Bold));
            match &item.item_type {
                ItemType::String { .. } => text.append_plain("String"),
                ItemType::Integer { .. } => text.append_plain("Integer"),
                ItemType::Number { .. } => text.append_plain("Number"),
                ItemType::Boolean { .. } => text.append_plain("Boolean"),
                ItemType::Enum(_) => text.append_plain("Enum"),
                ItemType::Array(_) => text.append_plain("Array"),
            }
            text.append_plain("\n\n");

            // 描述
            if let Some(help) = &item.base.help {
                text.append_styled("Description:\n", Style::from(Effect::Bold));
                text.append_plain(help);
                text.append_plain("\n\n");
            }

            // 当前值
            text.append_styled("Current Value:\n", Style::from(Effect::Bold));
            match &item.item_type {
                ItemType::String { value, .. } => {
                    text.append_plain(value.as_ref().unwrap_or(&"(none)".to_string()));
                }
                ItemType::Integer { value, .. } => {
                    text.append_plain(format!("{}", value.unwrap_or(0)));
                }
                ItemType::Number { value, .. } => {
                    text.append_plain(format!("{}", value.unwrap_or(0.0)));
                }
                ItemType::Boolean { value, .. } => {
                    text.append_plain(if *value { "true" } else { "false" });
                }
                ItemType::Enum(v) => {
                    if let Some(idx) = v.value {
                        if let Some(variant) = v.variants.get(idx) {
                            text.append_plain(variant);
                        } else {
                            text.append_plain("(invalid)");
                        }
                    } else {
                        text.append_plain("(none)");
                    }
                }
                ItemType::Array(v) => {
                    text.append_plain(format!("[{} items]", v.values.len()));
                }
            }
            text.append_plain("\n\n");

            // 额外信息
            match &item.item_type {
                ItemType::String { default, .. } => {
                    if let Some(default) = default {
                        text.append_styled("Default: ", Style::from(Effect::Bold));
                        text.append_plain(default);
                        text.append_plain("\n");
                    }
                }
                ItemType::Integer { default, .. } => {
                    if let Some(default) = default {
                        text.append_styled("Default: ", Style::from(Effect::Bold));
                        text.append_plain(format!("{}\n", default));
                    }
                }
                ItemType::Number { default, .. } => {
                    if let Some(default) = default {
                        text.append_styled("Default: ", Style::from(Effect::Bold));
                        text.append_plain(format!("{}\n", default));
                    }
                }
                ItemType::Boolean { default, .. } => {
                    text.append_styled("Default: ", Style::from(Effect::Bold));
                    text.append_plain(if *default { "true" } else { "false" });
                    text.append_plain("\n");
                }
                ItemType::Enum(v) => {
                    if let Some(default_idx) = v.default
                        && let Some(default) = v.variants.get(default_idx)
                    {
                        text.append_styled("Default: ", Style::from(Effect::Bold));
                        text.append_plain(default);
                        text.append_plain("\n");
                    }
                    text.append_styled("Options:\n", Style::from(Effect::Bold));
                    for opt in &v.variants {
                        text.append_plain(format!("  • {}\n", opt));
                    }
                }
                ItemType::Array(v) => {
                    text.append_styled("Element Type: ", Style::from(Effect::Bold));
                    text.append_plain(format!("{}\n", v.element_type));
                    if !v.default.is_empty() {
                        text.append_styled("Default: ", Style::from(Effect::Bold));
                        text.append_plain(format!("[{:?}]\n", v.default));
                    }
                }
            }

            text
        }
    };

    // 创建漂亮的对话框
    s.add_layer(
        Dialog::around(
            Panel::new(
                TextView::new(details)
                    .scrollable()
                    .scroll_x(true)
                    .max_width(80)
                    .max_height(25),
            )
            .title("╔═══ Item Details ═══╗")
            .title_position(HAlign::Center),
        )
        .dismiss_button("Close")
        .button("OK", |s| {
            s.pop_layer();
        }),
    );
}

/// 当选择项改变时更新详细信息
fn on_select(s: &mut Cursive, item: &ElementType) {
    let detail = match item {
        ElementType::Menu(menu) => {
            let mut text = String::new();
            text.push_str("╔═ Menu ═══════════════════════════════════════\n");
            text.push_str(&format!("║ Title: {}\n", menu.title));
            if let Some(help) = &menu.help {
                text.push_str("║\n");
                for line in help.lines() {
                    text.push_str(&format!("║ {}\n", line));
                }
            }
            text.push_str("║\n");
            text.push_str(&format!("║ Contains {} items\n", menu.children.len()));
            text.push_str("║ Required: ");
            text.push_str(if menu.is_required { "Yes" } else { "No" });
            text.push_str("\n╚═══════════════════════════════════════════════");
            text
        }
        ElementType::OneOf(one_of) => {
            let mut text = String::new();
            text.push_str("╔═ OneOf ══════════════════════════════════════\n");
            text.push_str(&format!("║ Title: {}\n", one_of.title));
            if let Some(help) = &one_of.help {
                text.push_str("║\n");
                for line in help.lines() {
                    text.push_str(&format!("║ {}\n", line));
                }
            }
            text.push_str("║\n");
            text.push_str(&format!("║ Variants: {}\n", one_of.variants.len()));
            if let Some(selected) = one_of.selected() {
                text.push_str(&format!("║ Current: {}\n", selected.title));
            } else {
                text.push_str("║ Current: <Unset>\n");
            }
            text.push_str("║ Tip: Press Tab to switch variants\n");
            text.push_str("╚═══════════════════════════════════════════════");
            text
        }
        ElementType::Item(item) => {
            let mut text = String::new();
            text.push_str("╔═ Item ═══════════════════════════════════════\n");
            text.push_str(&format!("║ Name: {}\n", item.base.title));

            if let Some(help) = &item.base.help {
                text.push_str("║\n");
                for line in help.lines() {
                    text.push_str(&format!("║ {}\n", line));
                }
            }
            text.push_str("║\n");

            match &item.item_type {
                ItemType::Boolean { value, default } => {
                    text.push_str("║ Type: Boolean\n");
                    text.push_str(&format!(
                        "║ Current: {}\n",
                        if *value { "✓ True" } else { "✗ False" }
                    ));
                    text.push_str(&format!(
                        "║ Default: {}\n",
                        if *default { "True" } else { "False" }
                    ));
                    text.push_str("║\n║ Tip: Press Enter to toggle");
                }
                ItemType::String { value, default } => {
                    text.push_str("║ Type: String\n");
                    text.push_str(&format!(
                        "║ Current: {}\n",
                        value
                            .as_ref()
                            .map(|v| format!("\"{}\"", v))
                            .unwrap_or_else(|| "<Empty>".to_string())
                    ));
                    if let Some(d) = default {
                        text.push_str(&format!("║ Default: \"{}\"\n", d));
                    }
                    text.push_str("║\n║ Tip: Press Enter to edit");
                }
                ItemType::Number { value, default } => {
                    text.push_str("║ Type: Number (float)\n");
                    text.push_str(&format!(
                        "║ Current: {}\n",
                        value
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "<Empty>".to_string())
                    ));
                    if let Some(d) = default {
                        text.push_str(&format!("║ Default: {}\n", d));
                    }
                    text.push_str("║\n║ Tip: Press Enter to edit");
                }
                ItemType::Integer { value, default } => {
                    text.push_str("║ Type: Integer\n");
                    text.push_str(&format!(
                        "║ Current: {}\n",
                        value
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "<Empty>".to_string())
                    ));
                    if let Some(d) = default {
                        text.push_str(&format!("║ Default: {}\n", d));
                    }
                    text.push_str("║\n║ Tip: Press Enter to edit");
                }
                ItemType::Enum(enum_item) => {
                    text.push_str("║ Type: Enum\n");
                    text.push_str(&format!("║ Options: {}\n", enum_item.variants.join(", ")));
                    if let Some(val) = enum_item.value_str() {
                        text.push_str(&format!("║ Current: {}\n", val));
                    } else {
                        text.push_str("║ Current: <Unset>\n");
                    }
                    text.push_str("║\n║ Tip: Press Enter to select");
                }
                ItemType::Array(array_item) => {
                    text.push_str("║ Type: Array\n");
                    text.push_str(&format!("║ Element Type: {}\n", array_item.element_type));
                    text.push_str(&format!("║ Count: {}\n", array_item.values.len()));
                    if !array_item.values.is_empty() {
                        text.push_str("║ Values:\n");
                        let max_display = 5;
                        for (idx, val) in array_item.values.iter().take(max_display).enumerate() {
                            text.push_str(&format!("║   [{}] {}\n", idx, val));
                        }
                        if array_item.values.len() > max_display {
                            text.push_str(&format!(
                                "║   ... and {} more\n",
                                array_item.values.len() - max_display
                            ));
                        }
                    } else {
                        text.push_str("║ Values: <Empty>\n");
                    }
                    text.push_str("║\n║ Tip: Enter=Edit, Del=Delete item");
                }
            }
            text.push_str("\n╚═══════════════════════════════════════════════");
            text
        }
    };

    s.call_on_name("detail_text", |v: &mut TextView| {
        v.set_content(detail);
    });
}

pub fn enter_menu(s: &mut Cursive, menu: &Menu) {
    let mut path = String::new();

    if let Some(app) = s.user_data::<AppData>() {
        path = app.key_string();
    }

    let title = menu.title.clone();
    let fields = menu.fields();

    s.add_fullscreen_layer(menu_view(&title, &path, fields));
}

fn enter_elem(s: &mut Cursive, elem: &ElementType) {
    let key = elem.key();
    let mut path = String::new();

    if let Some(app) = s.user_data::<AppData>() {
        path = app.key_string();
    }

    let mut hocked = false;
    if let Some(app_data) = s.user_data::<AppData>() {
        for hook in app_data.elem_hocks.iter().cloned() {
            if hook.path == path {
                (hook.callback)(s, &path);
                hocked = true;
                break;
            }
        }
    }
    if hocked {
        return;
    }

    match elem {
        ElementType::Menu(menu) => {
            // 进入子菜单
            if menu.is_none() {
                if let Some(ElementType::Menu(m)) =
                    s.user_data::<AppData>().unwrap().root.get_mut_by_key(&key)
                {
                    m.is_set = true;
                }
                handle_edit(s);
            } else {
                enter_menu(s, menu);
            }
        }
        ElementType::OneOf(one_of) => {
            if let Some(selected) = one_of.selected()
                && let ElementType::Menu(menu) = selected
            {
                // 进入子菜单
                enter_menu(s, menu);
                return;
            }

            // 显示 OneOf 选择对话框
            show_oneof_dialog(s, one_of);
        }
        ElementType::Item(item) => {
            // 根据类型显示编辑对话框
            match &item.item_type {
                ItemType::Boolean { .. } => {
                    // Boolean 类型直接切换
                    if let Some(ElementType::Item(b)) =
                        s.user_data::<AppData>().unwrap().root.get_mut_by_key(&key)
                        && let ItemType::Boolean { value, .. } = &mut b.item_type
                    {
                        *value = !*value;
                    }
                    handle_edit(s);
                }
                ItemType::String { value, default } => {
                    show_string_edit(s, &item.base.key(), &item.base.title, value, default);
                }
                ItemType::Number { value, default } => {
                    show_number_edit(s, &item.base.key(), &item.base.title, *value, *default);
                }
                ItemType::Integer { value, default } => {
                    show_integer_edit(s, &item.base.key(), &item.base.title, *value, *default);
                }
                ItemType::Enum(enum_item) => {
                    show_enum_select(s, &item.base.title, enum_item);
                }
                ItemType::Array(array_item) => {
                    show_array_edit(s, &item.base.key(), &item.base.title, &array_item.values);
                }
            }
        }
    }
}

pub fn enter_key(s: &mut Cursive, key: &str) {
    if let Some(app) = s.user_data::<AppData>()
        && let Some(item) = app.root.get_by_key(key).cloned()
    {
        app.enter(key);
        enter_elem(s, &item);
    }
}

fn on_oneof_switch(s: &mut Cursive) {
    let Some(selected) = menu_selected(s) else {
        return;
    };

    let ElementType::OneOf(oneof) = selected else {
        return;
    };

    if let Some(app) = s.user_data::<AppData>() {
        let key = oneof.key();
        app.enter(&key);
    }
    show_oneof_dialog(s, &oneof);
}

/// 处理项目选择
fn on_submit(s: &mut Cursive, item: &ElementType) {
    enter_key(s, &item.key());
}
