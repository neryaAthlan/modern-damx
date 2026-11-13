use super::{spawn, Ctx};
use adw::prelude::*;
use damx_core::driver::Status;
use gtk::glib;
use std::rc::Rc;

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let banner = adw::Banner::builder()
        .title("damxd is not running, so settings are read-only. Start it with `sudo systemctl start damxd`.")
        .revealed(false)
        .build();

    let page = adw::PreferencesPage::new();
    page.set_vexpand(true);

    let device = adw::PreferencesGroup::builder().title("Device").build();
    let driver_row = adw::ActionRow::builder()
        .title("Driver")
        .subtitle("checking…")
        .build();
    let profile_row = adw::ActionRow::builder()
        .title("Power profile")
        .subtitle("—")
        .build();
    let fan_row = adw::ActionRow::builder().title("Fan speed").subtitle("—").build();
    device.add(&driver_row);
    device.add(&profile_row);
    device.add(&fan_row);

    let peripherals = adw::PreferencesGroup::builder().title("Settings").build();
    let keyboard_row = adw::ActionRow::builder()
        .title("Keyboard lighting")
        .subtitle("—")
        .build();
    let backlight_row = adw::ActionRow::builder()
        .title("Backlight timeout")
        .subtitle("—")
        .build();
    let battery_row = adw::ActionRow::builder()
        .title("Battery limiter")
        .subtitle("—")
        .build();
    let usb_row = adw::ActionRow::builder()
        .title("USB charging")
        .subtitle("—")
        .build();
    peripherals.add(&keyboard_row);
    peripherals.add(&backlight_row);
    peripherals.add(&battery_row);
    peripherals.add(&usb_row);

    page.add(&device);
    page.add(&peripherals);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.append(&banner);
    container.append(&page);

    let refresh = {
        let banner = banner.clone();
        let driver_row = driver_row.clone();
        let profile_row = profile_row.clone();
        let fan_row = fan_row.clone();
        let keyboard_row = keyboard_row.clone();
        let backlight_row = backlight_row.clone();
        let battery_row = battery_row.clone();
        let usb_row = usb_row.clone();

        Rc::new(move || {
            let banner = banner.clone();
            let driver_row = driver_row.clone();
            let profile_row = profile_row.clone();
            let fan_row = fan_row.clone();
            let keyboard_row = keyboard_row.clone();
            let backlight_row = backlight_row.clone();
            let battery_row = battery_row.clone();
            let usb_row = usb_row.clone();

            spawn(
                super::super::backend::snapshot,
                move |snapshot| {
                    banner.set_revealed(!snapshot.daemon);
                    render(
                        &snapshot.status,
                        &driver_row,
                        &profile_row,
                        &fan_row,
                        &keyboard_row,
                        &backlight_row,
                        &battery_row,
                        &usb_row,
                    );
                },
            );
        })
    };

    refresh();
    {
        let refresh = refresh.clone();
        glib::timeout_add_seconds_local(3, move || {
            refresh();
            glib::ControlFlow::Continue
        });
    }
    let _ = ctx;

    container.upcast()
}

#[allow(clippy::too_many_arguments)]
fn render(
    status: &Status,
    driver_row: &adw::ActionRow,
    profile_row: &adw::ActionRow,
    fan_row: &adw::ActionRow,
    keyboard_row: &adw::ActionRow,
    backlight_row: &adw::ActionRow,
    battery_row: &adw::ActionRow,
    usb_row: &adw::ActionRow,
) {
    if status.driver_loaded {
        let version = status.driver_version.as_deref().unwrap_or("version unknown");
        let model = status.model.as_deref().unwrap_or("unknown model");
        driver_row.set_subtitle(&format!("linuwu_sense {version} · {model}"));
    } else {
        driver_row.set_subtitle("linuwu_sense is not loaded");
    }

    profile_row.set_subtitle(status.profile.as_deref().unwrap_or("not available"));

    fan_row.set_subtitle(&match status.fan {
        Some(fan) if fan.is_auto() => "automatic".to_string(),
        Some(fan) => format!("CPU {}% · GPU {}%", fan.cpu, fan.gpu),
        None => "not available".to_string(),
    });

    keyboard_row.set_subtitle(&match &status.lighting {
        Some(lighting) => {
            let zones: Vec<String> = lighting
                .zones
                .iter()
                .map(|zone| format!("#{zone}"))
                .collect();
            format!("{} · {}%", zones.join(" "), lighting.brightness)
        }
        None => "four-zone keyboard not detected".to_string(),
    });

    backlight_row.set_subtitle(match status.backlight_timeout {
        Some(true) => "turns off after 30 seconds",
        Some(false) => "always on",
        None => "not available",
    });

    battery_row.set_subtitle(match status.battery_limiter {
        Some(true) => "limited to 80%",
        Some(false) => "full charge",
        None => "not available",
    });

    usb_row.set_subtitle(&match status.usb_charging {
        Some(0) => "off while sleeping".to_string(),
        Some(value) => format!("on for {value} minutes after shutdown"),
        None => "not available".to_string(),
    });
}
