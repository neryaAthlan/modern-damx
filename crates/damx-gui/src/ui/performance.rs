use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::ipc::Request;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    let profile_group = adw::PreferencesGroup::builder()
        .title("Power profile")
        .description("The profile the Acer embedded controller runs. GNOME's power mode can reset it when the power source changes.")
        .build();
    let profile_row = adw::ComboRow::builder().title("Mode").build();
    profile_group.add(&profile_row);
    page.add(&profile_group);

    let fan_group = adw::PreferencesGroup::builder()
        .title("Fan speed")
        .description("0% hands the fans back to the embedded controller. Manual values are a percentage of full speed.")
        .build();

    let mode_row = adw::ComboRow::builder().title("Mode").build();
    mode_row.set_model(Some(&gtk::StringList::new(&["Automatic", "Manual"])));
    mode_row.set_selected(0);
    fan_group.add(&mode_row);

    let cpu_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    let gpu_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    for scale in [&cpu_scale, &gpu_scale] {
        scale.set_draw_value(true);
        scale.set_value_pos(gtk::PositionType::Right);
        scale.set_digits(0);
        scale.set_size_request(260, -1);
        scale.set_valign(gtk::Align::Center);
        scale.set_sensitive(false);
        scale.set_value(50.0);
    }

    let cpu_row = adw::ActionRow::builder().title("CPU fan").build();
    cpu_row.add_suffix(&cpu_scale);
    cpu_row.set_activatable(false);
    let gpu_row = adw::ActionRow::builder().title("GPU fan").build();
    gpu_row.add_suffix(&gpu_scale);
    gpu_row.set_activatable(false);
    fan_group.add(&cpu_row);
    fan_group.add(&gpu_row);
    page.add(&fan_group);

    let apply = gtk::Button::with_label("Apply fan speed");
    apply.add_css_class("suggested-action");
    apply.set_valign(gtk::Align::Center);
    let apply_row = adw::ActionRow::builder().title("Manual speed").build();
    apply_row.add_suffix(&apply);
    apply_row.set_activatable(false);
    fan_group.add(&apply_row);

    let battery_group = adw::PreferencesGroup::builder()
        .title("Battery")
        .description("The battery limiter stops charging at 80% to extend the battery's life.")
        .build();
    let battery_row = adw::SwitchRow::builder()
        .title("Limit charging to 80%")
        .build();
    battery_group.add(&battery_row);
    page.add(&battery_group);

    let syncing = Rc::new(Cell::new(false));
    let choices = Rc::new(RefCell::new(Vec::<String>::new()));

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let choices = choices.clone();
        profile_row.connect_selected_notify(move |row| {
            if syncing.get() {
                return;
            }
            let index = row.selected() as usize;
            let Some(profile) = choices.borrow().get(index).cloned() else {
                return;
            };
            send(
                &ctx,
                Request::SetProfile {
                    value: profile.clone(),
                },
                &format!("Power profile set to {profile}"),
            );
        });
    }

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let cpu_scale = cpu_scale.clone();
        let gpu_scale = gpu_scale.clone();
        let cpu_row = cpu_row.clone();
        let gpu_row = gpu_row.clone();
        mode_row.connect_selected_notify(move |row| {
            let manual = row.selected() == 1;
            cpu_scale.set_sensitive(manual);
            gpu_scale.set_sensitive(manual);
            cpu_row.set_activatable(manual);
            gpu_row.set_activatable(manual);
            if syncing.get() {
                return;
            }
            if manual {
                send(
                    &ctx,
                    Request::SetFan {
                        cpu: cpu_scale.value() as u8,
                        gpu: gpu_scale.value() as u8,
                    },
                    "Manual fan speed applied",
                );
            } else {
                send(
                    &ctx,
                    Request::SetFan { cpu: 0, gpu: 0 },
                    "Fans handed back to the embedded controller",
                );
            }
        });
    }

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        battery_row.connect_active_notify(move |row| {
            if syncing.get() {
                return;
            }
            send(
                &ctx,
                Request::SetBatteryLimiter {
                    value: row.is_active(),
                },
                if row.is_active() {
                    "Battery charging limited to 80%"
                } else {
                    "Battery limiter disabled"
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        let cpu_scale = cpu_scale.clone();
        let gpu_scale = gpu_scale.clone();
        apply.connect_clicked(move |_| {
            send(
                &ctx,
                Request::SetFan {
                    cpu: cpu_scale.value().round() as u8,
                    gpu: gpu_scale.value().round() as u8,
                },
                "Fan speed applied",
            );
        });
    }

    // Initial state.
    {
        let syncing = syncing.clone();
        let choices = choices.clone();
        let profile_row = profile_row.clone();
        let mode_row = mode_row.clone();
        let battery_row = battery_row.clone();
        let cpu_scale = cpu_scale.clone();
        let gpu_scale = gpu_scale.clone();
        spawn(super::super::backend::snapshot, move |snapshot| {
            let status = snapshot.status;
            *choices.borrow_mut() = status.profile_choices.clone();
            let labels: Vec<&str> = status.profile_choices.iter().map(String::as_str).collect();
            syncing.set(true);
            if !labels.is_empty() {
                profile_row.set_model(Some(&gtk::StringList::new(&labels)));
                if let Some(current) = status.profile.as_deref() {
                    if let Some(index) = status.profile_choices.iter().position(|p| p == current) {
                        profile_row.set_selected(index as u32);
                    }
                }
            }
            if let Some(fan) = status.fan {
                let manual = !fan.is_auto();
                mode_row.set_selected(if manual { 1 } else { 0 });
                cpu_scale.set_sensitive(manual);
                gpu_scale.set_sensitive(manual);
                if manual {
                    cpu_scale.set_value(fan.cpu as f64);
                    gpu_scale.set_value(fan.gpu as f64);
                }
            }
            if let Some(limiter) = status.battery_limiter {
                battery_row.set_active(limiter);
            }
            syncing.set(false);
        });
    }

    page.upcast()
}
