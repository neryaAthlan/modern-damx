use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::config;
use damx_core::ipc::Request;
use damx_core::policy;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct State {
    /// Profiles currently offered in the combo (filtered on battery).
    shown: RefCell<Vec<String>>,
    /// All profiles the driver supports.
    all: RefCell<Vec<String>>,
    /// Profiles allowed as the battery maximum (up to `balanced`).
    cap_choices: RefCell<Vec<String>>,
    fan_loaded: Cell<bool>,
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    page.set_vexpand(true);

    let profile_group = adw::PreferencesGroup::builder()
        .title("Power profile")
        .description("On battery, profiles above the battery maximum are unavailable and damxd switches down automatically when you unplug the charger.")
        .build();
    let profile_row = adw::ComboRow::builder().title("Mode").build();
    profile_group.add(&profile_row);
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

    let cpu_row = adw::ActionRow::builder().title("CPU fan").build();
    cpu_row.add_suffix(&cpu_scale);
    cpu_row.set_activatable(false);
    let gpu_row = adw::ActionRow::builder().title("GPU fan").build();
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

    let policy_group = adw::PreferencesGroup::builder()
        .title("Battery profile limit")
        .description("Applies everywhere: in the GUI, the CLI and the boot preset. While the charger is connected every profile is available.")
        .build();
    let limit_row = adw::SwitchRow::builder()
        .title("Limit the profile on battery")
        .subtitle("When unplugged, the profile is capped automatically")
        .build();
    let cap_row = adw::ComboRow::builder()
        .title("Highest profile on battery")
        .build();
    policy_group.add(&limit_row);
    policy_group.add(&cap_row);
    page.add(&policy_group);

    let syncing = Rc::new(Cell::new(false));
    let state = Rc::new(State {
        shown: RefCell::new(Vec::new()),
        all: RefCell::new(Vec::new()),
        cap_choices: RefCell::new(Vec::new()),
        fan_loaded: Cell::new(false),
    });

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let state = state.clone();
        profile_row.connect_selected_notify(move |row| {
            if syncing.get() {
                return;
            }
            let index = row.selected() as usize;
            let Some(profile) = state.shown.borrow().get(index).cloned() else {
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
        let syncing = syncing.clone();
        let limit_row_2 = limit_row.clone();
        let state = state.clone();
        cap_row.connect_selected_notify(move |row| {
            if syncing.get() {
                return;
            }
            let index = row.selected() as usize;
            let Some(profile) = state.cap_choices.borrow().get(index).cloned() else {
                return;
            };
            send(
                &ctx,
                Request::PolicySet {
                    limit_on_battery: limit_row_2.is_active(),
                    battery_profile: profile.clone(),
                },
                &format!("On battery, the profile is limited to {profile}"),
            );
        });
    }

    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let cap_row_2 = cap_row.clone();
        let state = state.clone();
        limit_row.connect_active_notify(move |row| {
            if syncing.get() {
                return;
            }
            let profile = state
                .cap_choices
                .borrow()
                .get(cap_row_2.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "balanced".to_string());
            send(
                &ctx,
                Request::PolicySet {
                    limit_on_battery: row.is_active(),
                    battery_profile: profile,
                },
                if row.is_active() {
                    "Profile limit on battery enabled"
                } else {
                    "Profile limit on battery disabled"
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
        let profile_row = profile_row.clone();
        let mode_row = mode_row.clone();
        let cpu_scale = cpu_scale.clone();
        let gpu_scale = gpu_scale.clone();
        let battery_row = battery_row.clone();
        let limit_row = limit_row.clone();
        let cap_row = cap_row.clone();
        let cpu_temp_row = cpu_temp_row.clone();
        let gpu_temp_row = gpu_temp_row.clone();

        Rc::new(move || {
            let state = state.clone();
            let syncing = syncing.clone();
            let profile_row = profile_row.clone();
            let mode_row = mode_row.clone();
            let cpu_scale = cpu_scale.clone();
            let gpu_scale = gpu_scale.clone();
            let battery_row = battery_row.clone();
            let limit_row = limit_row.clone();
            let cap_row = cap_row.clone();
            let cpu_temp_row = cpu_temp_row.clone();
            let gpu_temp_row = gpu_temp_row.clone();

            spawn(super::super::backend::snapshot, move |snapshot| {
                let status = snapshot.status;
                let unit = config::load().ui.temperature_unit;

                syncing.set(true);
                cpu_temp_row.set_subtitle(&unit.format(status.cpu_temp_c));
                gpu_temp_row.set_subtitle(&unit.format(status.gpu_temp_c));

                *state.all.borrow_mut() = status.profile_choices.clone();
                let allowed = status.allowed_profiles.clone();
                if *state.shown.borrow() != allowed && !allowed.is_empty() {
                    *state.shown.borrow_mut() = allowed.clone();
                    let labels: Vec<&str> = allowed.iter().map(String::as_str).collect();
                    profile_row.set_model(Some(&gtk::StringList::new(&labels)));
                }
                if let Some(current) = status.profile.as_deref() {
                    if let Some(index) = state.shown.borrow().iter().position(|p| p == current) {
                        profile_row.set_selected(index as u32);
                    }
                }

                if !status.profile_choices.is_empty() {
                    let cap = policy::rank(&status.profile_choices, "balanced")
                        .unwrap_or(status.profile_choices.len().saturating_sub(1));
                    let cap_choices: Vec<String> = status
                        .profile_choices
                        .iter()
                        .take(cap + 1)
                        .cloned()
                        .collect();
                    if *state.cap_choices.borrow() != cap_choices && !cap_choices.is_empty() {
                        *state.cap_choices.borrow_mut() = cap_choices.clone();
                        let labels: Vec<&str> = cap_choices.iter().map(String::as_str).collect();
                        cap_row.set_model(Some(&gtk::StringList::new(&labels)));
                    }
                    if let Some(index) = state
                        .cap_choices
                        .borrow()
                        .iter()
                        .position(|p| p == &status.battery_profile)
                    {
                        cap_row.set_selected(index as u32);
                    }
                }

                limit_row.set_active(status.limit_on_battery);
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
