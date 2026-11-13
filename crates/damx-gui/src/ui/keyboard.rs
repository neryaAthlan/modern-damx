use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::ipc::Request;
use gtk::gdk;
use std::cell::Cell;
use std::rc::Rc;

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let syncing = Rc::new(Cell::new(false));

    let lighting_group = adw::PreferencesGroup::builder()
        .title("Four-zone lighting")
        .description("Colour each keyboard zone separately. Brightness and colours are restored by the driver across reboots.")
        .build();

    let dialog = gtk::ColorDialog::builder().with_alpha(false).build();
    let colors: Rc<Vec<gtk::ColorDialogButton>> = Rc::new(
        (1..=4)
            .map(|_| {
                let button = gtk::ColorDialogButton::new(Some(dialog.clone()));
                button.set_rgba(&gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
                button.set_valign(gtk::Align::Center);
                button
            })
            .collect(),
    );

    for (index, button) in colors.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .title(format!("Zone {}", index + 1))
            .build();
        row.add_suffix(button);
        row.set_activatable(false);
        lighting_group.add(&row);
    }

    let brightness = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    brightness.set_draw_value(true);
    brightness.set_value_pos(gtk::PositionType::Right);
    brightness.set_digits(0);
    brightness.set_size_request(260, -1);
    brightness.set_valign(gtk::Align::Center);
    brightness.set_value(100.0);
    let brightness_row = adw::ActionRow::builder().title("Brightness").build();
    brightness_row.add_suffix(&brightness);
    brightness_row.set_activatable(false);
    lighting_group.add(&brightness_row);
    page.add(&lighting_group);

    let apply = gtk::Button::with_label("Apply lighting");
    apply.add_css_class("suggested-action");
    apply.set_valign(gtk::Align::Center);
    let apply_row = adw::ActionRow::builder().title("New colours").build();
    apply_row.add_suffix(&apply);
    apply_row.set_activatable(false);
    lighting_group.add(&apply_row);

    {
        let ctx = ctx.clone();
        let colors = colors.clone();
        let brightness = brightness.clone();
        apply.connect_clicked(move |_| {
            let zones = [
                to_hex(&colors[0].rgba()),
                to_hex(&colors[1].rgba()),
                to_hex(&colors[2].rgba()),
                to_hex(&colors[3].rgba()),
            ];
            send(
                &ctx,
                Request::SetLighting {
                    zones,
                    brightness: brightness.value().round() as u8,
                },
                "Keyboard lighting applied",
            );
        });
    }

    let backlight_group = adw::PreferencesGroup::builder()
        .title("Backlight")
        .description("Behaviour of the keyboard backlight when the laptop is idle.")
        .build();
    let timeout_row = adw::SwitchRow::builder()
        .title("Turn off after 30 seconds")
        .build();
    backlight_group.add(&timeout_row);
    page.add(&backlight_group);

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        timeout_row.connect_active_notify(move |row| {
            if syncing.get() {
                return;
            }
            send(
                &ctx,
                Request::SetBacklightTimeout {
                    value: row.is_active(),
                },
                if row.is_active() {
                    "Backlight timeout enabled"
                } else {
                    "Backlight timeout disabled"
                },
            );
        });
    }

    {
        let syncing = syncing.clone();
        let colors = colors.clone();
        let brightness = brightness.clone();
        let timeout_row = timeout_row.clone();
        spawn(super::super::backend::snapshot, move |snapshot| {
            let status = snapshot.status;
            if let Some(lighting) = status.lighting {
                for (button, zone) in colors.iter().zip(lighting.zones.iter()) {
                    button.set_rgba(&from_hex(zone));
                }
                brightness.set_value(lighting.brightness as f64);
            }
            if let Some(value) = status.backlight_timeout {
                syncing.set(true);
                timeout_row.set_active(value);
                syncing.set(false);
            }
        });
    }

    page.upcast()
}

fn from_hex(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(format!("#{hex}"))
        .unwrap_or_else(|_| gdk::RGBA::new(1.0, 1.0, 1.0, 1.0))
}

fn to_hex(rgba: &gdk::RGBA) -> String {
    format!(
        "{:02x}{:02x}{:02x}",
        (rgba.red() * 255.0).round() as u8,
        (rgba.green() * 255.0).round() as u8,
        (rgba.blue() * 255.0).round() as u8
    )
}
