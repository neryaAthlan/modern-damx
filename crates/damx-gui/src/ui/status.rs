use super::{spawn, Ctx};
use adw::prelude::*;
use damx_core::config::{self, TemperatureUnit};
use damx_core::driver::Status;
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;

struct Rows {
    driver: adw::ActionRow,
    power: adw::ActionRow,
    profile: adw::ActionRow,
    fan: adw::ActionRow,
    fan_rpm: adw::ActionRow,
    cpu_temp: adw::ActionRow,
    gpu_temp: adw::ActionRow,
    keyboard: adw::ActionRow,
    backlight: adw::ActionRow,
    battery: adw::ActionRow,
    usb: adw::ActionRow,
}

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
    let power_row = adw::ActionRow::builder()
        .title("Power source")
        .subtitle("—")
        .build();
    let profile_row = adw::ActionRow::builder()
        .title("Power profile")
        .subtitle("—")
        .build();
    let fan_row = adw::ActionRow::builder().title("Fan speed").subtitle("—").build();
    let fan_rpm_row = adw::ActionRow::builder()
        .title("Fan RPM")
        .subtitle("—")
        .build();
    device.add(&driver_row);
    device.add(&power_row);
    device.add(&profile_row);
    device.add(&fan_row);
    device.add(&fan_rpm_row);

    let thermal = adw::PreferencesGroup::builder()
        .title("Temperatures")
        .description("Read from the Acer embedded controller, with the NVIDIA driver as a fallback.")
        .build();
    let cpu_temp_row = adw::ActionRow::builder()
        .title("CPU temperature")
        .subtitle("—")
        .build();
    let gpu_temp_row = adw::ActionRow::builder()
        .title("GPU temperature")
        .subtitle("—")
        .build();
    thermal.add(&cpu_temp_row);
    thermal.add(&gpu_temp_row);

    let settings = adw::PreferencesGroup::builder().title("Settings").build();
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
    settings.add(&keyboard_row);
    settings.add(&backlight_row);
    settings.add(&battery_row);
    settings.add(&usb_row);

    let preferences = adw::PreferencesGroup::builder()
        .title("Preferences")
        .description("Saved in ~/.config/damx/config.toml.")
        .build();
    let unit_row = adw::ComboRow::builder().title("Temperature unit").build();
    unit_row.set_model(Some(&gtk::StringList::new(&["Celsius (°C)", "Fahrenheit (°F)"])));
    preferences.add(&unit_row);

    page.add(&device);
    page.add(&thermal);
    page.add(&settings);
    page.add(&preferences);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.append(&banner);
    container.append(&page);

    let units = Rc::new(Cell::new(config::load().ui.temperature_unit));
    let syncing = Rc::new(Cell::new(false));

    let rows = Rc::new(Rows {
        driver: driver_row,
        power: power_row,
        profile: profile_row,
        fan: fan_row,
        fan_rpm: fan_rpm_row,
        cpu_temp: cpu_temp_row,
        gpu_temp: gpu_temp_row,
        keyboard: keyboard_row,
        backlight: backlight_row,
        battery: battery_row,
        usb: usb_row,
    });

    let refresh = {
        let banner = banner.clone();
        let rows = rows.clone();
        let units = units.clone();
        Rc::new(move || {
            let banner = banner.clone();
            let rows = rows.clone();
            let units = units.clone();
            spawn(
                super::super::backend::snapshot,
                move |snapshot| {
                    banner.set_revealed(!snapshot.daemon);
                    render(&snapshot.status, units.get(), &rows);
                },
            );
        })
    };

    syncing.set(true);
    unit_row.set_selected(if units.get() == TemperatureUnit::Fahrenheit {
        1
    } else {
        0
    });
    syncing.set(false);

    {
        let units = units.clone();
        let syncing = syncing.clone();
        let refresh = refresh.clone();
        unit_row.connect_selected_notify(move |row| {
            if syncing.get() {
                return;
            }
            let unit = if row.selected() == 1 {
                TemperatureUnit::Fahrenheit
            } else {
                TemperatureUnit::Celsius
            };
            units.set(unit);
            let mut config = config::load();
            config.ui.temperature_unit = unit;
            if let Err(err) = config::save(&config) {
                eprintln!("could not save the temperature unit: {err:#}");
            }
            refresh();
        });
    }

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

fn render(status: &Status, unit: TemperatureUnit, rows: &Rows) {
    if status.driver_loaded {
        let version = status.driver_version.as_deref().unwrap_or("version unknown");
        let model = status.model.as_deref().unwrap_or("unknown model");
        rows.driver
            .set_subtitle(&format!("linuwu_sense {version} · {model}"));
    } else {
        rows.driver.set_subtitle("linuwu_sense is not loaded");
    }

    rows.power.set_subtitle(match status.on_ac {
        Some(true) => "plugged in",
        Some(false) => "on battery",
        None => "unknown",
    });

    rows.profile
        .set_subtitle(status.profile.as_deref().unwrap_or("not available"));

    rows.fan.set_subtitle(&match status.fan {
        Some(fan) if fan.is_auto() => "automatic".to_string(),
        Some(fan) => format!("CPU {}% · GPU {}%", fan.cpu, fan.gpu),
        None => "not available".to_string(),
    });

    if let Some((cpu, gpu)) = status.fan_rpm {
        rows.fan_rpm.set_subtitle(&format!("CPU {cpu} RPM · GPU {gpu} RPM"));
    } else {
        rows.fan_rpm.set_subtitle("not available");
    }

    rows.cpu_temp.set_subtitle(&unit.format(status.cpu_temp_c));
    rows.gpu_temp.set_subtitle(&unit.format(status.gpu_temp_c));

    rows.keyboard.set_subtitle(&match &status.lighting {
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

    rows.backlight.set_subtitle(match status.backlight_timeout {
        Some(true) => "turns off after 30 seconds",
        Some(false) => "always on",
        None => "not available",
    });

    rows.battery.set_subtitle(match status.battery_limiter {
        Some(true) => "limited to 80%",
        Some(false) => "full charge",
        None => "not available",
    });

    rows.usb.set_subtitle(&match status.usb_charging {
        Some(0) => "off while sleeping".to_string(),
        Some(value) => format!("on for {value} minutes after shutdown"),
        None => "not available".to_string(),
    });
}
