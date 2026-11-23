use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::ipc::Request;
use gtk::gdk;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let syncing = Rc::new(Cell::new(false));
    let pending_lighting = Rc::new(RefCell::new(None::<gtk::glib::SourceId>));

    let lighting_group = adw::PreferencesGroup::builder()
        .title("Four-zone lighting")
        .description("Colour each keyboard zone separately. Changes apply automatically.")
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

    for button in colors.iter() {
        let ctx = ctx.clone();
        let colors = colors.clone();
        let brightness = brightness.clone();
        let syncing = syncing.clone();
        let pending_lighting = pending_lighting.clone();
        button.connect_notify_local(Some("rgba"), move |_, _| {
            schedule_lighting_apply(
                &ctx,
                &colors,
                &brightness,
                &syncing,
                &pending_lighting,
            );
        });
    }

    {
        let ctx = ctx.clone();
        let colors = colors.clone();
        let brightness = brightness.clone();
        let brightness_control = brightness.clone();
        let syncing = syncing.clone();
        let pending_lighting = pending_lighting.clone();
        brightness_control.connect_value_changed(move |_| {
            schedule_lighting_apply(
                &ctx,
                &colors,
                &brightness,
                &syncing,
                &pending_lighting,
            );
        });
    }

    let backlight_group = adw::PreferencesGroup::builder()
        .title("Backlight")
        .description("Behaviour of the keyboard backlight when the laptop is idle.")
        .build();
    let timeout_row = adw::SwitchRow::builder()
        .title("Turn off keyboard lighting when idle")
        .build();
    backlight_group.add(&timeout_row);
    let timeout_choices = adw::ComboRow::builder()
        .title("Idle time")
        .model(&gtk::StringList::new(&[
            "30 seconds",
            "40 seconds",
            "50 seconds",
            "1 minute",
            "2 minutes",
            "Custom",
        ]))
        .build();
    timeout_choices.set_selected(0);
    backlight_group.add(&timeout_choices);

    let custom_timeout = gtk::SpinButton::with_range(1.0, 255.0, 1.0);
    custom_timeout.set_value(90.0);
    custom_timeout.set_size_request(100, -1);
    custom_timeout.set_valign(gtk::Align::Center);
    let custom_timeout_row = adw::ActionRow::builder()
        .title("Custom timeout")
        .subtitle("Timeout in seconds, from 1 to 255")
        .build();
    custom_timeout_row.add_suffix(&custom_timeout);
    custom_timeout_row.set_activatable(false);
    backlight_group.add(&custom_timeout_row);
    {
        let custom_timeout_row = custom_timeout_row.clone();
        timeout_choices.connect_selected_notify(move |row| {
            custom_timeout_row.set_visible(row.selected() == 5);
        });
    }
    page.add(&backlight_group);

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let timeout_choices = timeout_choices.clone();
        let custom_timeout = custom_timeout.clone();
        timeout_row.connect_active_notify(move |row| {
            if syncing.get() {
                return;
            }
            let seconds = if row.is_active() {
                timeout_seconds(timeout_choices.selected(), custom_timeout.value())
            } else {
                0
            };
            send(
                &ctx,
                Request::SetBacklightTimeout { seconds },
                "Backlight timeout updated",
            );
        });
    }

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let timeout_row = timeout_row.clone();
        let custom_timeout = custom_timeout.clone();
        timeout_choices.connect_selected_notify(move |row| {
            if syncing.get() || !timeout_row.is_active() {
                return;
            }
            let seconds = timeout_seconds(row.selected(), custom_timeout.value());
            send(
                &ctx,
                Request::SetBacklightTimeout { seconds },
                "Backlight timeout updated",
            );
        });
    }

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let timeout_choices = timeout_choices.clone();
        let timeout_row = timeout_row.clone();
        custom_timeout.connect_value_changed(move |spin| {
            if syncing.get() || !timeout_row.is_active() || timeout_choices.selected() != 5 {
                return;
            }
            send(
                &ctx,
                Request::SetBacklightTimeout {
                    seconds: spin.value().round() as u16,
                },
                "Backlight timeout updated",
            );
        });
    }

    let sync_controls: Rc<dyn Fn(super::super::backend::Snapshot)> = {
        let syncing = syncing.clone();
        let colors = colors.clone();
        let brightness = brightness.clone();
        let timeout_row = timeout_row.clone();
        let timeout_choices = timeout_choices.clone();
        let custom_timeout = custom_timeout.clone();
        Rc::new(move |snapshot| {
            let status = snapshot.status;
            syncing.set(true);
            if let Some(lighting) = status.lighting {
                for (button, zone) in colors.iter().zip(lighting.zones.iter()) {
                    button.set_rgba(&from_hex(zone));
                }
                brightness.set_value(lighting.brightness as f64);
            }
            if let Some(seconds) = status.backlight_timeout {
                timeout_row.set_active(seconds > 0);
                if seconds > 0 {
                    let selected = match seconds {
                        30 => 0,
                        40 => 1,
                        50 => 2,
                        60 => 3,
                        120 => 4,
                        _ => 5,
                    };
                    timeout_choices.set_selected(selected);
                    if selected == 5 {
                        custom_timeout.set_value(seconds as f64);
                    }
                }
            }
            syncing.set(false);
        })
    };
    {
        let sync_controls = sync_controls.clone();
        spawn(super::super::backend::snapshot, move |snapshot| sync_controls(snapshot));
    }
    {
        let sync_controls = sync_controls.clone();
        gtk::glib::timeout_add_seconds_local(3, move || {
            let sync_controls = sync_controls.clone();
            spawn(super::super::backend::snapshot, move |snapshot| sync_controls(snapshot));
            gtk::glib::ControlFlow::Continue
        });
    }

    page.upcast()
}

fn timeout_seconds(selected: u32, custom: f64) -> u16 {
    match selected {
        0 => 30,
        1 => 40,
        2 => 50,
        3 => 60,
        4 => 120,
        _ => custom.round().clamp(1.0, 255.0) as u16,
    }
}

fn schedule_lighting_apply(
    ctx: &Rc<super::Ctx>,
    colors: &Rc<Vec<gtk::ColorDialogButton>>,
    brightness: &gtk::Scale,
    syncing: &Rc<Cell<bool>>,
    pending: &Rc<RefCell<Option<gtk::glib::SourceId>>>,
) {
    if syncing.get() {
        return;
    }
    if let Some(previous) = pending.borrow_mut().take() {
        previous.remove();
    }

    let ctx = ctx.clone();
    let colors = colors.clone();
    let brightness = brightness.clone();
    let syncing = syncing.clone();
    let pending_in_timeout = pending.clone();
    let source = gtk::glib::timeout_add_local_once(Duration::from_millis(150), move || {
        *pending_in_timeout.borrow_mut() = None;
        if syncing.get() {
            return;
        }
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
            "Keyboard lighting updated",
        );
    });
    *pending.borrow_mut() = Some(source);
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
