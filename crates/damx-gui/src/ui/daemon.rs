use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::ipc::{Request, ResetSection};
use std::cell::Cell;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

/// Runs a systemctl subcommand through pkexec and reports the outcome.
fn run_systemctl(ctx: &Rc<Ctx>, args: &[&str], success: &str) {
    let ctx = ctx.clone();
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let success = success.to_string();
    spawn(
        move || {
            Command::new("pkexec")
                .arg("systemctl")
                .args(&args)
                .output()
        },
        move |result| match result {
            Ok(output) if output.status.success() => ctx.toast(success),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let message = stderr.trim();
                if message.is_empty() {
                    ctx.toast("systemctl failed");
                } else {
                    ctx.toast(format!("systemctl failed: {message}"));
                }
            }
            Err(err) => ctx.toast(format!("could not run pkexec: {err}")),
        },
    );
}

fn refresh_daemon_status(row: &adw::ActionRow) {
    let row = row.clone();
    spawn(
        || {
            let reachable = super::super::backend::snapshot().daemon;
            let active = Command::new("systemctl")
                .args(["is-active", "damxd.service"])
                .output()
                .is_ok_and(|output| output.status.success());
            (reachable, active)
        },
        move |(reachable, active)| {
            row.set_subtitle(if reachable {
                "running and reachable"
            } else if active {
                "running, but the app cannot access its socket; check linuwu_sense group membership"
            } else {
                "stopped"
            });
        },
    );
}

pub fn build(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    let status_group = adw::PreferencesGroup::builder()
        .title("Daemon status")
        .description("damxd is the privileged helper that talks to the kernel module.")
        .build();

    let daemon_row = adw::ActionRow::builder()
        .title("damxd")
        .subtitle("checking…")
        .build();
    status_group.add(&daemon_row);

    refresh_daemon_status(&daemon_row);

    let refresh = gtk::Button::with_label("Refresh");
    refresh.set_valign(gtk::Align::Center);
    let refresh_row = adw::ActionRow::builder().title("Refresh status").build();
    refresh_row.add_suffix(&refresh);
    refresh_row.set_activatable(false);
    status_group.add(&refresh_row);
    {
        let daemon_row = daemon_row.clone();
        refresh.connect_clicked(move |_| {
            refresh_daemon_status(&daemon_row);
        });
    }
    page.add(&status_group);

    let control_group = adw::PreferencesGroup::builder()
        .title("Control")
        .description("Start, stop or restart the daemon. These actions need administrator rights.")
        .build();

    let start = gtk::Button::with_label("Start");
    start.set_valign(gtk::Align::Center);
    let start_row = adw::ActionRow::builder().title("Start damxd").build();
    start_row.add_suffix(&start);
    start_row.set_activatable(false);
    control_group.add(&start_row);
    {
        let ctx = ctx.clone();
        start.connect_clicked(move |_| {
            run_systemctl(&ctx, &["start", "damxd.service"], "damxd started");
        });
    }

    let stop = gtk::Button::with_label("Stop");
    stop.set_valign(gtk::Align::Center);
    let stop_row = adw::ActionRow::builder().title("Stop damxd").build();
    stop_row.add_suffix(&stop);
    stop_row.set_activatable(false);
    control_group.add(&stop_row);
    {
        let ctx = ctx.clone();
        stop.connect_clicked(move |_| {
            run_systemctl(&ctx, &["stop", "damxd.service"], "damxd stopped");
        });
    }

    let restart = gtk::Button::with_label("Restart");
    restart.add_css_class("suggested-action");
    restart.set_valign(gtk::Align::Center);
    let restart_row = adw::ActionRow::builder().title("Restart damxd").build();
    restart_row.add_suffix(&restart);
    restart_row.set_activatable(false);
    control_group.add(&restart_row);
    {
        let ctx = ctx.clone();
        restart.connect_clicked(move |_| {
            run_systemctl(&ctx, &["restart", "damxd.service"], "damxd restarted");
        });
    }
    page.add(&control_group);

    let boot_group = adw::PreferencesGroup::builder()
        .title("Boot behaviour")
        .description("Whether damxd starts automatically when the machine boots.")
        .build();

    let autostart = adw::SwitchRow::builder()
        .title("Start damxd automatically at boot")
        .subtitle("Enable or disable the system service")
        .build();
    autostart.set_sensitive(false);
    boot_group.add(&autostart);

    let syncing = Rc::new(Cell::new(true));
    let enabled_at_boot = Rc::new(Cell::new(false));
    {
        let autostart = autostart.clone();
        let syncing = syncing.clone();
        let enabled_at_boot = enabled_at_boot.clone();
        spawn(
            || Command::new("systemctl").args(["is-enabled", "damxd.service"]).output(),
            move |result| {
                let enabled = result.is_ok_and(|output| output.status.success());
                enabled_at_boot.set(enabled);
                syncing.set(false);
                autostart.set_active(enabled);
                autostart.set_sensitive(true);
            },
        );
    }
    {
        let ctx = ctx.clone();
        let syncing = syncing.clone();
        let enabled_at_boot = enabled_at_boot.clone();
        autostart.connect_active_notify(move |row| {
            if syncing.get() {
                return;
            }

            let enable = row.is_active();
            row.set_sensitive(false);
            let ctx = ctx.clone();
            let row = row.clone();
            let syncing = syncing.clone();
            let enabled_at_boot = enabled_at_boot.clone();
            spawn(
                move || {
                    let mut command = Command::new("pkexec");
                    command.arg("systemctl");
                    if enable {
                        command.args(["enable", "--now", "damxd.service"]);
                    } else {
                        command.args(["disable", "damxd.service"]);
                    }
                    command.output()
                },
                move |result| {
                    match result {
                        Ok(output) if output.status.success() => {
                            enabled_at_boot.set(enable);
                            ctx.toast(if enable {
                                "damxd started and enabled at boot"
                            } else {
                                "damxd disabled at boot"
                            });
                        }
                        Ok(output) => {
                            syncing.set(true);
                            row.set_active(enabled_at_boot.get());
                            syncing.set(false);
                            let message = String::from_utf8_lossy(&output.stderr);
                            let message = message.trim();
                            let message = if message.is_empty() {
                                "systemctl failed".to_string()
                            } else {
                                format!("systemctl failed: {message}")
                            };
                            ctx.toast(message);
                        }
                        Err(err) => {
                            syncing.set(true);
                            row.set_active(enabled_at_boot.get());
                            syncing.set(false);
                            ctx.toast(format!("could not run pkexec: {err}"));
                        }
                    }
                    row.set_sensitive(true);
                },
            );
        });
    }

    page.add(&boot_group);

    let reset_group = adw::PreferencesGroup::builder()
        .title("Restore defaults")
        .description("Restore a section or return all DAMX settings to their defaults.")
        .build();
    let reset_choices = adw::ComboRow::builder()
        .title("Section")
        .model(&gtk::StringList::new(&[
            "Performance",
            "Keyboard lighting",
            "Boot preset",
            "Daemon",
        ]))
        .build();
    reset_group.add(&reset_choices);

    let reset_section = gtk::Button::with_label("Restore section defaults");
    reset_section.set_valign(gtk::Align::Center);
    let reset_section_row = adw::ActionRow::builder()
        .title("Selected section")
        .build();
    reset_section_row.add_suffix(&reset_section);
    reset_section_row.set_activatable(false);
    reset_group.add(&reset_section_row);
    {
        let ctx = ctx.clone();
        let reset_choices = reset_choices.clone();
        let autostart = autostart.clone();
        let syncing = syncing.clone();
        let enabled_at_boot = enabled_at_boot.clone();
        reset_section.connect_clicked(move |_| {
            let section = match reset_choices.selected() {
                0 => ResetSection::Performance,
                1 => ResetSection::KeyboardLighting,
                2 => ResetSection::BootPreset,
                _ => ResetSection::Daemon,
            };
            send(&ctx, Request::ResetDefaults { section }, "Section defaults restored");
            refresh_autostart(&autostart, &syncing, &enabled_at_boot);
        });
    }

    let reset_all = gtk::Button::with_label("Reset all to defaults");
    reset_all.add_css_class("destructive-action");
    reset_all.set_valign(gtk::Align::Center);
    let reset_all_row = adw::ActionRow::builder()
        .title("All settings")
        .build();
    reset_all_row.add_suffix(&reset_all);
    reset_all_row.set_activatable(false);
    reset_group.add(&reset_all_row);
    {
        let ctx = ctx.clone();
        let autostart = autostart.clone();
        let syncing = syncing.clone();
        let enabled_at_boot = enabled_at_boot.clone();
        reset_all.connect_clicked(move |_| {
            send(
                &ctx,
                Request::ResetDefaults {
                    section: ResetSection::All,
                },
                "All defaults restored",
            );
            refresh_autostart(&autostart, &syncing, &enabled_at_boot);
        });
    }
    page.add(&reset_group);

    page.upcast()
}

fn refresh_autostart(
    row: &adw::SwitchRow,
    syncing: &Rc<Cell<bool>>,
    enabled_at_boot: &Rc<Cell<bool>>,
) {
    let row = row.clone();
    let syncing = syncing.clone();
    let enabled_at_boot = enabled_at_boot.clone();
    gtk::glib::timeout_add_local_once(Duration::from_millis(500), move || {
        spawn(
            || Command::new("systemctl").args(["is-enabled", "damxd.service"]).output(),
            move |result| {
                let enabled = result.is_ok_and(|output| output.status.success());
                enabled_at_boot.set(enabled);
                syncing.set(true);
                row.set_active(enabled);
                syncing.set(false);
            },
        );
    });
}
