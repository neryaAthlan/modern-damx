use super::{spawn, Ctx};
use adw::prelude::*;
use damx_core::update::{self, ComponentStatus, UpdateReport};
use gtk::glib;
use std::rc::Rc;

struct View {
    driver_current: adw::ActionRow,
    driver_latest: adw::ActionRow,
    driver_status: adw::ActionRow,
    driver_link: gtk::LinkButton,
    gui_current: adw::ActionRow,
    gui_latest: adw::ActionRow,
    gui_status: adw::ActionRow,
    gui_link: gtk::LinkButton,
    checked: gtk::Label,
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    let make_group = |title: &str| {
        let group = adw::PreferencesGroup::builder().title(title).build();
        (
            group.clone(),
            linked_row(&group),
            linked_row(&group),
            linked_row(&group),
        )
    };

    let (driver_group, driver_current, driver_latest, driver_status) =
        make_group("Linuwu Sense driver");
    let driver_link = gtk::LinkButton::with_label("", "Open release");
    driver_link.set_valign(gtk::Align::Center);
    driver_link.set_visible(false);
    driver_status.add_suffix(&driver_link);
    page.add(&driver_group);

    let (gui_group, gui_current, gui_latest, gui_status) = make_group("DAMX");
    let gui_link = gtk::LinkButton::with_label("", "Open release");
    gui_link.set_valign(gtk::Align::Center);
    gui_link.set_visible(false);
    gui_status.add_suffix(&gui_link);
    page.add(&gui_group);

    driver_current.set_title("Installed version");
    driver_latest.set_title("Latest release");
    driver_status.set_title("Status");
    gui_current.set_title("Installed version");
    gui_latest.set_title("Latest release");
    gui_status.set_title("Status");

    let check_group = adw::PreferencesGroup::builder()
        .title("Automatic checks")
        .description("A background timer checks GitHub once a day; this page also checks whenever you open it. Repositories are set in ~/.config/damx/config.toml.")
        .build();
    let checked_row = adw::ActionRow::builder().title("Last checked").build();
    let checked = gtk::Label::new(Some("never"));
    checked.add_css_class("dim-label");
    checked_row.add_suffix(&checked);
    check_group.add(&checked_row);

    let check_button = gtk::Button::with_label("Check now");
    check_button.add_css_class("suggested-action");
    check_button.set_valign(gtk::Align::Center);
    let check_row = adw::ActionRow::builder()
        .title("Check for new versions")
        .build();
    check_row.add_suffix(&check_button);
    check_row.set_activatable(false);
    check_group.add(&check_row);
    page.add(&check_group);

    let view = Rc::new(View {
        driver_current,
        driver_latest,
        driver_status,
        driver_link,
        gui_current,
        gui_latest,
        gui_status,
        gui_link,
        checked,
    });

    if let Some(report) = update::load_report() {
        render(&view, &report);
    }

    {
        let view = view.clone();
        let button = check_button.clone();
        let ctx = ctx.clone();
        let run_check = Rc::new(move || {
            button.set_sensitive(false);
            let view = view.clone();
            let button = button.clone();
            let ctx = ctx.clone();
            spawn(super::super::backend::check_updates, move |report| {
                button.set_sensitive(true);
                render(&view, &report);
                if report.any_available() {
                    ctx.toast("A newer release is available");
                } else {
                    ctx.toast("Everything is up to date");
                }
            });
        });

        let run_check_button = run_check.clone();
        check_button.connect_clicked(move |_| run_check_button());

        // If nothing has been cached yet, check once on first open.
        if update::load_report().is_none() {
            run_check();
        }
    }

    page.upcast()
}

fn linked_row(group: &adw::PreferencesGroup) -> adw::ActionRow {
    let row = adw::ActionRow::builder().build();
    group.add(&row);
    row
}

fn render(view: &View, report: &UpdateReport) {
    render_component(
        &report.driver,
        &view.driver_current,
        &view.driver_latest,
        &view.driver_status,
        &view.driver_link,
    );
    render_component(
        &report.gui,
        &view.gui_current,
        &view.gui_latest,
        &view.gui_status,
        &view.gui_link,
    );
    let when = glib::DateTime::from_unix_local(report.checked_at as i64)
        .ok()
        .and_then(|dt| dt.format("%e %b %Y, %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    view.checked.set_text(&when);
}

fn render_component(
    component: &ComponentStatus,
    current: &adw::ActionRow,
    latest: &adw::ActionRow,
    status: &adw::ActionRow,
    link: &gtk::LinkButton,
) {
    current.set_subtitle(component.current.as_deref().unwrap_or("unknown"));
    match &component.latest {
        Some(release) => {
            latest.set_subtitle(&release.tag);
            if component.available {
                status.set_subtitle("Update available");
                link.set_uri(&release.url);
                link.set_visible(true);
            } else {
                status.set_subtitle("Up to date");
                link.set_uri("");
                link.set_visible(false);
            }
        }
        None => {
            latest.set_subtitle("—");
            status.set_subtitle(
                component
                    .note
                    .as_deref()
                    .unwrap_or("No release information"),
            );
            link.set_uri("");
            link.set_visible(false);
        }
    }
}
