use adw::prelude::*;

pub fn build() -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    let author = adw::PreferencesGroup::builder()
        .title("Modern DAMX")
        .description("A Rust rewrite of DAMX for Debian and recent GNOME desktops.")
        .build();
    author.add(&link_row(
        "Nerya Athlan",
        "Author and maintainer of this rewrite and of the Linuwu Sense fork",
        "https://github.com/neryaAthlan",
        "GitHub",
    ));
    page.add(&author);

    let based_on = adw::PreferencesGroup::builder()
        .title("Built on")
        .description("None of this would exist without these projects.")
        .build();
    based_on.add(&link_row(
        "Div Acer Manager Max",
        "The original GUI this rewrite is based on, by PXDiv",
        "https://github.com/PXDiv/Div-Acer-Manager-Max",
        "Visit",
    ));
    based_on.add(&link_row(
        "Linuwu Sense",
        "The kernel driver that exposes fan, power and keyboard controls, by 0x7375646F",
        "https://github.com/0x7375646F/Linuwu-Sense",
        "Visit",
    ));
    based_on.add(&link_row(
        "Div-Linuwu-Sense fork",
        "The driver fork used and maintained for this project",
        "https://github.com/neryaAthlan/Div-Linuwu-Sense",
        "Visit",
    ));
    page.add(&based_on);

    let license = adw::PreferencesGroup::builder().title("License").build();
    license.add(&link_row(
        "GNU General Public License v3.0",
        "Free software: you can use, study, share and modify it",
        "https://www.gnu.org/licenses/gpl-3.0.html",
        "Read",
    ));
    page.add(&license);

    page.upcast()
}

fn link_row(title: &str, subtitle: &str, url: &str, button_label: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    let link = gtk::LinkButton::with_label(url, button_label);
    link.set_valign(gtk::Align::Center);
    row.add_suffix(&link);
    row.set_activatable(false);
    row
}
