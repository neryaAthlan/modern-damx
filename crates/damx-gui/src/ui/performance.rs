use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::config;
use damx_core::ipc::Request;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct State {
    /// Every power profile reported by this laptop's driver.
    choices: RefCell<Vec<String>>,
    ac_profile: RefCell<String>,
    battery_profile: RefCell<String>,
    fan_loaded: Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    page.set_vexpand(true);

    let profile_group = adw::PreferencesGroup::builder()
        .title("Power profiles")
        .description("Choose the profile to use while plugged in and while on battery. DAMX applies the matching choice when the power source changes.")
        .build();
    let ac_profile_row = adw::ComboRow::builder().title("Plugged in").build();
    let battery_profile_row = adw::ComboRow::builder().title("On battery").build();
    let current_profile_row = adw::ActionRow::builder()
        .title("Current profile")
        .subtitle("—")
        .build();
    profile_group.add(&ac_profile_row);
    profile_group.add(&battery_profile_row);
    profile_group.add(&current_profile_row);
    page.add(&profile_group);

    let fan_group = adw::PreferencesGroup::builder()
        .title("Fan speed")
        .description("0% hands the fans back to the embedded controller. Manual values are a percentage of full speed.")
        .build();

    let cpu_temp_row = adw::ActionRow::builder()
        .title("CPU temperature")
        .subtitle("—")
        .build();
    let gpu_temp_row = adw::ActionRow::builder()
        .title("GPU temperature")
        .subtitle("—")
        .build();
    fan_group.add(&cpu_temp_row);
    fan_group.add(&gpu_temp_row);

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

    let cpu_row = adw::ActionRow::builder()
        .title("CPU fan")
        .subtitle("—")
        .build();
    cpu_row.add_suffix(&cpu_scale);
    cpu_row.set_activatable(false);
    let gpu_row = adw::ActionRow::builder()
        .title("GPU fan")
        .subtitle("—")
        .build();
    gpu_row.add_suffix(&gpu_scale);
    gpu_row.set_activatable(false);
    fan_group.add(&cpu_row);
    fan_group.add(&gpu_row);

    let apply = gtk::Button::with_label("Apply fan speed");
    apply.add_css_class("suggested-action");
    apply.set_valign(gtk::Align::Center);
    let apply_row = adw::ActionRow::builder().title("Manual speed").build();
    apply_row.add_suffix(&apply);
    apply_row.set_activatable(false);
    fan_group.add(&apply_row);
    page.add(&fan_group);

    let charging_group = adw::PreferencesGroup::builder()
        .title("Battery")
        .description("The battery limiter stops charging at 80% to extend the battery's life.")
        .build();
    let battery_row = adw::SwitchRow::builder()
        .title("Limit charging to 80%")
        .build();
    charging_group.add(&battery_row);
    page.add(&charging_group);

    let syncing = Rc::new(Cell::new(false));
    let state = Rc::new(State {
        choices: RefCell::new(Vec::new()),
        ac_profile: RefCell::new(String::new()),
        battery_profile: RefCell::new(String::new()),
        fan_loaded: Cell::new(false),
    });

    {
        let ac_ctx = ctx.clone();
        let ac_syncing = syncing.clone();
        let ac_state = state.clone();
        ac_profile_row.connect_selected_notify(move |row| {
            if ac_syncing.get() {
                return;
            }
            let index = row.selected() as usize;
            let Some(profile) = ac_state.choices.borrow().get(index).cloned() else {
                return;
            };
            *ac_state.ac_profile.borrow_mut() = profile.clone();
            let battery_profile = ac_state.battery_profile.borrow().clone();
            send(
                &ac_ctx,
                Request::ProfilePreferencesSet {
                    ac_profile: profile.clone(),
                    battery_profile,
                },
                &format!("Plugged-in profile set to {profile}"),
            );
        });
        let battery_ctx = ctx.clone();
        let battery_syncing = syncing.clone();
        let battery_state = state.clone();
        battery_profile_row.connect_selected_notify(move |row| {
            if battery_syncing.get() {
                return;
            }
            let index = row.selected() as usize;
            let Some(profile) = battery_state.choices.borrow().get(index).cloned() else {
                return;
            };
            *battery_state.battery_profile.borrow_mut() = profile.clone();
            let ac_profile = battery_state.ac_profile.borrow().clone();
            send(
                &battery_ctx,
                Request::ProfilePreferencesSet {
                    ac_profile,
                    battery_profile: profile.clone(),
                },
                &format!("Battery profile set to {profile}"),
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

    let refresh: Rc<dyn Fn()> = {
        let state = state.clone();
        let syncing = syncing.clone();
        let ac_profile_row = ac_profile_row.clone();
        let battery_profile_row = battery_profile_row.clone();
        let current_profile_row = current_profile_row.clone();
        let mode_row = mode_row.clone();
        let cpu_scale = cpu_scale.clone();
        let gpu_scale = gpu_scale.clone();
        let battery_row = battery_row.clone();
        let cpu_temp_row = cpu_temp_row.clone();
        let gpu_temp_row = gpu_temp_row.clone();
        let cpu_row = cpu_row.clone();
        let gpu_row = gpu_row.clone();

        Rc::new(move || {
            let state = state.clone();
            let syncing = syncing.clone();
            let ac_profile_row = ac_profile_row.clone();
            let battery_profile_row = battery_profile_row.clone();
            let current_profile_row = current_profile_row.clone();
            let mode_row = mode_row.clone();
            let cpu_scale = cpu_scale.clone();
            let gpu_scale = gpu_scale.clone();
            let battery_row = battery_row.clone();
            let cpu_temp_row = cpu_temp_row.clone();
            let gpu_temp_row = gpu_temp_row.clone();
            let cpu_row = cpu_row.clone();
            let gpu_row = gpu_row.clone();

            spawn(super::super::backend::snapshot, move |snapshot| {
                let status = snapshot.status;
                let unit = config::load().ui.temperature_unit;

                syncing.set(true);
                cpu_temp_row.set_subtitle(&unit.format(status.cpu_temp_c));
                gpu_temp_row.set_subtitle(&unit.format(status.gpu_temp_c));
                if let Some((cpu, gpu)) = status.fan_rpm {
                    cpu_row.set_subtitle(&format!("{cpu} RPM"));
                    gpu_row.set_subtitle(&format!("{gpu} RPM"));
                } else {
                    cpu_row.set_subtitle("RPM not available");
                    gpu_row.set_subtitle("RPM not available");
                }

                if *state.choices.borrow() != status.profile_choices
                    && !status.profile_choices.is_empty()
                {
                    *state.choices.borrow_mut() = status.profile_choices.clone();
                    let labels: Vec<&str> = status.profile_choices.iter().map(String::as_str).collect();
                    let model = gtk::StringList::new(&labels);
                    ac_profile_row.set_model(Some(&model));
                    battery_profile_row.set_model(Some(&model));
                }
                *state.ac_profile.borrow_mut() = status.ac_profile.clone();
                *state.battery_profile.borrow_mut() = status.battery_profile.clone();
                if let Some(index) = state
                    .choices
                    .borrow()
                    .iter()
                    .position(|p| p == &status.ac_profile)
                {
                    ac_profile_row.set_selected(index as u32);
                }
                if let Some(index) = state
                    .choices
                    .borrow()
                    .iter()
                    .position(|p| p == &status.battery_profile)
                {
                    battery_profile_row.set_selected(index as u32);
                }
                current_profile_row.set_subtitle(&format!(
                    "{} · {}",
                    status.profile.as_deref().unwrap_or("unavailable"),
                    match status.on_ac {
                        Some(true) => "plugged in",
                        Some(false) => "on battery",
                        None => "power source unknown",
                    }
                ));
                let profiles_available = !status.profile_choices.is_empty();
                ac_profile_row.set_sensitive(profiles_available);
                battery_profile_row.set_sensitive(profiles_available);
                if let Some(limiter) = status.battery_limiter {
                    battery_row.set_active(limiter);
                }

                if !state.fan_loaded.get() {
                    if let Some(fan) = status.fan {
                        let manual = !fan.is_auto();
                        mode_row.set_selected(if manual { 1 } else { 0 });
                        cpu_scale.set_sensitive(manual);
                        gpu_scale.set_sensitive(manual);
                        if manual {
                            cpu_scale.set_value(fan.cpu as f64);
                            gpu_scale.set_value(fan.gpu as f64);
                        }
                        state.fan_loaded.set(true);
                    }
                }
                syncing.set(false);
            });
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
    page.upcast()
}
