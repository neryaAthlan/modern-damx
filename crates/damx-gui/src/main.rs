mod backend;
mod ui;

use adw::prelude::*;

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("io.github.neryaathlan.Damx")
        .build();
    app.connect_activate(|app| {
        ui::build(app).present();
    });
    app.run()
}
