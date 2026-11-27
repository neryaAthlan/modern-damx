mod backend;
mod ui;

use adw::prelude::*;

const APP_ID: &str = "io.github.neryaathlan.Damx";

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .build();
    app.connect_activate(|app| {
        gtk::Window::set_default_icon_name(APP_ID);
        ui::build(app).present();
    });
    app.run()
}
