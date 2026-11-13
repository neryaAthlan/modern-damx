use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::driver::Status;
use damx_core::ipc::Request;
use damx_core::policy::{self, ProfilePolicy};
use damx_core::preset::BootPreset;
use std::cell::RefCell;
use std::rc::Rc;

struct View {
    enabled: adw::SwitchRow,
    profile: adw::SwitchRow,
    fan: adw::SwitchRow,
    lighting: adw::SwitchRow,
}

impl View {
    /// Shows which parts of the saved preset are active. Nothing is written
    /// here: only the save button changes the preset, so merely opening the
    /// page can never clobber it.
    fn render(&self, preset: &BootPreset, status: Option<&Status>) {
        self.enabled.set_active(preset.enabled);
        self.profile.set_active(preset.profile.is_some());
        self.fan.set_active(preset.fan.is_some());
        self.lighting.set_active(preset.lighting.is_some());

        self.profile
            .set_subtitle(&profile_subtitle(preset, status));
        self.fan.set_subtitle(&match preset.fan {
            Some(fan) if fan.is_auto() => "automatic".to_string(),
            Some(fan) => format!("CPU {}% · GPU {}%", fan.cpu, fan.gpu),
            None => "not applied at boot".to_string(),
        });
        self.lighting.set_subtitle(&match &preset.lighting {
            Some(lighting) => format!("{}% brightness", lighting.brightness),
            None => "not applied at boot".to_string(),
        });

        let sensitive = status.is_some();
        self.profile.set_sensitive(sensitive);
        self.fan.set_sensitive(sensitive);
        self.lighting.set_sensitive(sensitive);
    }
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Boot preset")
        .description("Pick what damxd should re-apply at boot, then save. The preset is applied as root once the kernel module is loaded, before you log in.")
        .build();

    let enabled = adw::SwitchRow::builder()
        .title("Apply a preset at boot")
        .build();
    group.add(&enabled);

    let profile = adw::SwitchRow::builder().title("Power profile").build();
    let fan = adw::SwitchRow::builder().title("Fan speed").build();
    let lighting = adw::SwitchRow::builder().title("Keyboard lighting").build();
    group.add(&profile);
    group.add(&fan);
    group.add(&lighting);
    page.add(&group);

    let actions = adw::PreferencesGroup::new();
    let save = gtk::Button::with_label("Save preset from current settings");
    save.add_css_class("suggested-action");
    save.set_valign(gtk::Align::Center);
    let apply_now = gtk::Button::with_label("Apply preset now");
    apply_now.set_valign(gtk::Align::Center);
    let action_row = adw::ActionRow::builder().title("Actions").build();
    action_row.add_suffix(&save);
    action_row.add_suffix(&apply_now);
    action_row.set_activatable(false);
    actions.add(&action_row);
    page.add(&actions);

    let view = Rc::new(View {
        enabled,
        profile,
        fan,
        lighting,
    });
    let preset_state = Rc::new(RefCell::new(backend_boot()));
    let status_state = Rc::new(RefCell::new(None::<Status>));

    view.render(&preset_state.borrow(), None);
    {
        let view = view.clone();
        let preset_state = preset_state.clone();
        let status_state = status_state.clone();
        spawn(super::super::backend::snapshot, move |snapshot| {
            let status = snapshot.status;
            view.render(&preset_state.borrow(), Some(&status));
            *status_state.borrow_mut() = Some(status);
        });
    }

    {
        let ctx = ctx.clone();
        let view = view.clone();
        let preset_state = preset_state.clone();
        let status_state = status_state.clone();
        save.connect_clicked(move |_| {
            let Some(status) = status_state.borrow().clone() else {
                ctx.toast("The driver is not ready yet, try again in a moment");
                return;
            };
            let preset = BootPreset::from_status(
                &status,
                view.profile.is_active(),
                view.fan.is_active(),
                view.lighting.is_active(),
            );
            let preset = BootPreset {
                enabled: view.enabled.is_active(),
                ..preset
            };
            if preset.enabled && preset.is_empty() {
                ctx.toast("Select at least one setting before enabling the preset");
                return;
            }
            *preset_state.borrow_mut() = preset.clone();
            let status = status_state.borrow().clone();
            view.render(&preset, status.as_ref());

            let ctx = ctx.clone();
            let request = Request::BootSave { preset };
            spawn(
                move || super::super::backend::request(request),
                move |result| match result {
                    Ok(_) => ctx.toast("Boot preset saved"),
                    Err(err) => ctx.toast(format!("Could not save the boot preset: {err:#}")),
                },
            );
        });
    }

    {
        let ctx = ctx.clone();
        apply_now.connect_clicked(move |_| {
            send(&ctx, Request::BootApply, "Boot preset applied");
        });
    }

    page.upcast()
}

/// Shows the saved profile and what it becomes on battery when the global
/// policy caps it, so it is clear the preset has two effective profiles.
fn profile_subtitle(preset: &BootPreset, status: Option<&Status>) -> String {
    let Some(profile) = preset.profile.as_deref() else {
        return "not applied at boot".to_string();
    };
    let Some(status) = status else {
        return profile.to_string();
    };
    let policy = ProfilePolicy {
        limit_on_battery: status.limit_on_battery,
        battery_profile: status.battery_profile.clone(),
    };
    let on_battery = policy::clamp_profile(profile, &status.profile_choices, Some(false), &policy)
        .unwrap_or_else(|| profile.to_string());
    if on_battery == profile {
        format!("{profile} plugged in and on battery")
    } else {
        format!("{profile} plugged in · {on_battery} on battery")
    }
}

fn backend_boot() -> BootPreset {
    super::super::backend::boot_get()
}
