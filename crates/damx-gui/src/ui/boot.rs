use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::driver::Status;
use damx_core::ipc::Request;
use damx_core::preset::BootPreset;
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

struct View {
    enabled: adw::SwitchRow,
    profile: adw::SwitchRow,
    fan: adw::SwitchRow,
    lighting: adw::SwitchRow,
    syncing: Rc<Cell<bool>>,
    busy: Cell<bool>,
}

impl View {
    /// Shows which parts of the saved preset are active. Nothing is written
    /// here: only explicit switch changes or live control changes update it.
    fn render(&self, preset: &BootPreset, status: Option<&Status>) {
        self.syncing.set(true);
        self.enabled.set_active(preset.enabled);
        self.profile.set_active(preset.profile.is_some());
        self.fan.set_active(preset.fan.is_some());
        self.lighting.set_active(preset.lighting.is_some());

        self.profile.set_subtitle(&profile_subtitle(preset, status));
        self.fan.set_subtitle(&fan_subtitle(preset.fan, status));
        self.lighting
            .set_subtitle(&lighting_subtitle(preset.lighting.as_ref(), status));

        self.profile
            .set_sensitive(status.is_some_and(|status| status.profile.is_some()));
        self.fan
            .set_sensitive(status.is_some_and(|status| status.fan.is_some()));
        self.lighting
            .set_sensitive(status.is_some_and(|status| status.lighting.is_some()));
        self.enabled
            .set_sensitive(status.is_some_and(|status| status.driver_loaded));
        self.syncing.set(false);
        self.busy.set(false);
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        let sensitive = !busy;
        self.enabled.set_sensitive(sensitive);
        self.profile.set_sensitive(sensitive);
        self.fan.set_sensitive(sensitive);
        self.lighting.set_sensitive(sensitive);
    }
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Boot preset")
        .description("Choose which settings damxd should apply at boot. Changes sync automatically with the other DAMX controls.")
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

    let apply_now = gtk::Button::with_label("Apply preset now");
    apply_now.set_valign(gtk::Align::Center);
    let apply_row = adw::ActionRow::builder()
        .title("Apply the saved boot preset")
        .build();
    apply_row.add_suffix(&apply_now);
    apply_row.set_activatable(false);
    group.add(&apply_row);

    let syncing = Rc::new(Cell::new(false));
    let view = Rc::new(View {
        enabled,
        profile,
        fan,
        lighting,
        syncing,
        busy: Cell::new(false),
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

    for row in [
        view.enabled.clone(),
        view.profile.clone(),
        view.fan.clone(),
        view.lighting.clone(),
    ] {
        let ctx = ctx.clone();
        let view = view.clone();
        let preset_state = preset_state.clone();
        let status_state = status_state.clone();
        row.connect_active_notify(move |_| {
            sync_preset_from_current(&ctx, &view, &preset_state, &status_state);
        });
    }

    {
        let page = page.clone();
        let view = view.clone();
        let preset_state = preset_state.clone();
        let status_state = status_state.clone();
        let refreshing = Rc::new(Cell::new(false));
        gtk::glib::timeout_add_local(Duration::from_secs(3), move || {
            if !page.is_mapped() || view.busy.get() || refreshing.replace(true) {
                return gtk::glib::ControlFlow::Continue;
            }
            let refreshing = refreshing.clone();
            let view = view.clone();
            let preset_state = preset_state.clone();
            let status_state = status_state.clone();
            spawn(
                || {
                    (
                        super::super::backend::boot_get(),
                        super::super::backend::snapshot().status,
                    )
                },
                move |(preset, status)| {
                    if !view.busy.get() {
                        *preset_state.borrow_mut() = preset.clone();
                        *status_state.borrow_mut() = Some(status.clone());
                        view.render(&preset, Some(&status));
                    }
                    refreshing.set(false);
                },
            );
            gtk::glib::ControlFlow::Continue
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

fn sync_preset_from_current(
    ctx: &Rc<Ctx>,
    view: &Rc<View>,
    preset_state: &Rc<RefCell<BootPreset>>,
    status_state: &Rc<RefCell<Option<Status>>>,
) {
    if view.syncing.get() {
        return;
    }

    let enabled = view.enabled.is_active();
    let include_profile = view.profile.is_active();
    let include_fan = view.fan.is_active();
    let include_lighting = view.lighting.is_active();
    let old_preset = preset_state.borrow().clone();
    view.set_busy(true);

    let ctx = ctx.clone();
    let view = view.clone();
    let preset_state = preset_state.clone();
    let status_state = status_state.clone();
    spawn(super::super::backend::snapshot, move |snapshot| {
        let status = snapshot.status;
        *status_state.borrow_mut() = Some(status.clone());

        let error = if !status.driver_loaded {
            Some("The Linuwu-Sense driver is not available")
        } else if include_profile && status.profile.is_none() {
            Some("Could not read the current power profile")
        } else if include_fan && status.fan.is_none() {
            Some("Could not read the current fan setting")
        } else if include_lighting && status.lighting.is_none() {
            Some("Could not read the current keyboard lighting")
        } else {
            None
        };
        if let Some(message) = error {
            view.render(&old_preset, Some(&status));
            ctx.toast(message);
            return;
        }

        let preset = BootPreset {
            enabled,
            ..BootPreset::from_status(
                &status,
                include_profile,
                include_fan,
                include_lighting,
            )
        };
        if preset.enabled && preset.is_empty() {
            view.render(&old_preset, Some(&status));
            ctx.toast("Select at least one setting before enabling the preset");
            return;
        }

        let request = Request::BootSave {
            preset: preset.clone(),
        };
        let ctx = ctx.clone();
        spawn(
            move || super::super::backend::request(request),
            move |result| match result {
                Ok(_) => {
                    *preset_state.borrow_mut() = preset.clone();
                    *status_state.borrow_mut() = Some(status.clone());
                    view.render(&preset, Some(&status));
                    ctx.toast("Boot preset synced");
                }
                Err(err) => {
                    view.render(&old_preset, Some(&status));
                    ctx.toast(format!("Could not sync the boot preset: {err:#}"));
                }
            },
        );
    });
}

/// Shows the saved profile and what it becomes on battery when the global
/// policy caps it, so it is clear the preset has two effective profiles.
fn profile_subtitle(preset: &BootPreset, status: Option<&Status>) -> String {
    let saved = match preset.profile.as_deref() {
        Some(profile) => format!("Saved: {profile}"),
        None => "Not saved".to_string(),
    };
    match status {
        Some(status) => format!(
            "{saved} · Current: {} · Plugged in: {} · Battery: {}",
            status.profile.as_deref().unwrap_or("unavailable"),
            status.ac_profile,
            status.battery_profile
        ),
        None => format!("{saved} · Current: unavailable"),
    }
}

fn fan_subtitle(saved: Option<damx_core::driver::Fan>, status: Option<&Status>) -> String {
    let describe = |fan: damx_core::driver::Fan| {
        if fan.is_auto() {
            "automatic".to_string()
        } else {
            format!("CPU {}% · GPU {}%", fan.cpu, fan.gpu)
        }
    };
    let saved = saved
        .map(|fan| format!("Saved: {}", describe(fan)))
        .unwrap_or_else(|| "Not saved".to_string());
    let current = status
        .and_then(|status| status.fan)
        .map(describe)
        .unwrap_or_else(|| "unavailable".to_string());
    format!("{saved} · Current: {current}")
}

fn lighting_subtitle(
    saved: Option<&damx_core::driver::Lighting>,
    status: Option<&Status>,
) -> String {
    let describe = |lighting: &damx_core::driver::Lighting| {
        format!(
            "{} · {}%",
            lighting
                .zones
                .iter()
                .map(|zone| format!("#{zone}"))
                .collect::<Vec<_>>()
                .join(" "),
            lighting.brightness
        )
    };
    let saved = saved
        .map(|lighting| format!("Saved: {}", describe(lighting)))
        .unwrap_or_else(|| "Not saved".to_string());
    let current = status
        .and_then(|status| status.lighting.as_ref())
        .map(describe)
        .unwrap_or_else(|| "unavailable".to_string());
    format!("{saved} · Current: {current}")
}

fn backend_boot() -> BootPreset {
    super::super::backend::boot_get()
}
