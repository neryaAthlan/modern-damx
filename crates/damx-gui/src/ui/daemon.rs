use super::{send, spawn, Ctx};
use adw::prelude::*;
use damx_core::ipc::Request;
use std::process::Command;
use std::rc::Rc;

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

    {
        let daemon_row = daemon_row.clone();
        spawn(super::super::backend::snapshot, move |snapshot| {
            daemon_row.set_subtitle(if snapshot.daemon {
                "running"
            } else {
                "stopped"
            });
        });
    }

    let refresh = gtk::Button::with_label("Refresh");
    refresh.set_valign(gtk::Align::Center);
    let refresh_row = adw::ActionRow::builder().title("Refresh status").build();
    refresh_row.add_suffix(&refresh);
    refresh_row.set_activatable(false);
    status_group.add(&refresh_row);
    {
        let daemon_row = daemon_row.clone();
        refresh.connect_clicked(move |_| {
            let daemon_row = daemon_row.clone();
            spawn(super::super::backend::snapshot, move |snapshot| {
                daemon_row.set_subtitle(if snapshot.daemon {
                    "running"
                } else {
                    "stopped"
                });
            });
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

    let enable = gtk::Button::with_label("Enable");
    enable.set_valign(gtk::Align::Center);
    let enable_row = adw::ActionRow::builder()
        .title("Start damxd at boot")
        .build();
    enable_row.add_suffix(&enable);
    enable_row.set_activatable(false);
    boot_group.add(&enable_row);
    {
        let ctx = ctx.clone();
        enable.connect_clicked(move |_| {
            run_systemctl(&ctx, &["enable", "damxd.service"], "damxd enabled at boot");
        });
    }

    let disable = gtk::Button::with_label("Disable");
    disable.set_valign(gtk::Align::Center);
    let disable_row = adw::ActionRow::builder()
        .title("Do not start damxd at boot")
        .build();
    disable_row.add_suffix(&disable);
    disable_row.set_activatable(false);
    boot_group.add(&disable_row);
    {
        let ctx = ctx.clone();
        disable.connect_clicked(move |_| {
            run_systemctl(&ctx, &["disable", "damxd.service"], "damxd disabled at boot");
        });
    }

    let apply_now = gtk::Button::with_label("Apply preset now");
    apply_now.set_valign(gtk::Align::Center);
    let apply_row = adw::ActionRow::builder()
        .title("Apply the saved boot preset")
        .build();
    apply_row.add_suffix(&apply_now);
    apply_row.set_activatable(false);
    boot_group.add(&apply_row);
    {
        let ctx = ctx.clone();
        apply_now.connect_clicked(move |_| {
            send(&ctx, Request::BootApply, "Boot preset applied");
        });
    }
    page.add(&boot_group);

    page.upcast()
}