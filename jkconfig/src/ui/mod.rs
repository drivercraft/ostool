use cursive::{Cursive, views::Dialog};

use crate::{
    data::AppState,
    ui::components::menu::{install_menu_view, refresh_menu},
};

pub mod components;

pub fn start_ui(siv: &mut Cursive) {
    install_menu_view(siv);
}

pub fn handle_back(siv: &mut Cursive) {
    if siv.screen().len() > 1 {
        siv.pop_layer();
        refresh_menu(siv);
        return;
    }

    if let Some(app) = siv.user_data::<AppState>()
        && app.navigate_back()
    {
        refresh_menu(siv);
        return;
    }

    handle_quit(siv);
}

pub fn handle_quit(siv: &mut Cursive) {
    siv.add_layer(
        Dialog::text("Quit without saving?")
            .title("Quit")
            .button("Back", |s| {
                s.pop_layer();
            })
            .button("Quit", |s| {
                s.quit();
            }),
    );
}

pub fn handle_save(siv: &mut Cursive) {
    siv.add_layer(
        Dialog::text("Save and exit?")
            .title("Save")
            .button("Ok", |s| {
                if let Some(app) = s.user_data::<AppState>() {
                    app.mark_dirty();
                }
                s.quit();
            })
            .button("Cancel", |s| {
                s.pop_layer();
            }),
    );
}
