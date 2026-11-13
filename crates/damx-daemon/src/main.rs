//! `damxd` — small privileged helper for DAMX.
//!
//! The driver's sysfs attributes are owned by root and sysfs permissions cannot
//! be changed at runtime, so the GUI and the CLI send their writes here over a
//! Unix socket that only members of the `linuwu_sense` group can open.
//!
//! The daemon also applies the boot preset after startup, and it is ordered
//! after power-profiles-daemon so the preset wins over GNOME's default.

use anyhow::{Context, Result};
use damx_core::driver::{self, Driver, Lighting, Status};
use damx_core::ipc::{self, Request, Response};
use damx_core::policy::{self, ProfilePolicy};
use damx_core::preset::BootPreset;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const DEFAULT_GROUP: &str = "linuwu_sense";

fn main() -> Result<()> {
    let mut socket = ipc::socket_path();
    let mut group = std::env::var("DAMX_GROUP").unwrap_or_else(|_| DEFAULT_GROUP.to_string());

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" if i + 1 < args.len() => {
                socket = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--group" if i + 1 < args.len() => {
                group = args[i + 1].clone();
                i += 2;
            }
            "-h" | "--help" => {
                println!("damxd [--socket PATH] [--group GROUP]");
                return Ok(());
            }
            other => {
                eprintln!("damxd: unknown argument {other:?}");
                std::process::exit(2);
            }
        }
    }

    if let Some(dir) = socket.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if socket.exists() {
        fs::remove_file(&socket).with_context(|| format!("removing {}", socket.display()))?;
    }

    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("binding {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o660))?;
    match group_gid(&group) {
        Some(gid) => {
            if let Err(err) = std::os::unix::fs::chown(&socket, None, Some(gid)) {
                eprintln!("damxd: could not give {socket:?} to group {group}: {err}");
            }
        }
        None => eprintln!(
            "damxd: group {group} does not exist; only root can talk to the daemon"
        ),
    }
    println!(
        "damxd {} listening on {} (group {group})",
        env!("CARGO_PKG_VERSION"),
        socket.display()
    );

    apply_boot_preset_later();
    watch_power_source();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(err) = serve(stream) {
                        eprintln!("damxd: connection error: {err:#}");
                    }
                });
            }
            Err(err) => eprintln!("damxd: accept failed: {err}"),
        }
    }
    Ok(())
}

fn serve(stream: UnixStream) -> Result<()> {
    let peer = peer_uid(&stream);
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if line.trim().is_empty() {
            line.clear();
            continue;
        }
        let response = match serde_json::from_str::<Request>(line.trim()) {
            Ok(request) => handle(request, peer),
            Err(err) => Response::error(format!("invalid request: {err}")),
        };
        writeln!(writer, "{}", serde_json::to_string(&response)?)?;
        writer.flush()?;
        line.clear();
    }
    Ok(())
}

fn handle(request: Request, peer: u32) -> Response {
    match request {
        Request::Ping => Response::ok(),
        Request::Status => Response::with_status(current_status()),
        Request::BootGet => Response::with_preset(BootPreset::load()),
        Request::PolicyGet => Response::with_policy(ProfilePolicy::load()),
        Request::PolicySet {
            limit_on_battery,
            battery_profile,
        } => set_policy(peer, limit_on_battery, battery_profile),
        Request::SetProfile { value } => set_profile(peer, value),
        Request::SetFan { cpu, gpu } => with_driver(peer, "set fan speed", |driver| {
            driver.set_fan(cpu, gpu)
        }),
        Request::SetLighting { zones, brightness } => {
            with_driver(peer, "set keyboard lighting", |driver| {
                driver.set_lighting(&Lighting { zones, brightness })
            })
        }
        Request::SetBacklightTimeout { value } => {
            with_driver(peer, "set backlight timeout", |driver| {
                driver.set_backlight_timeout(value)
            })
        }
        Request::SetBatteryLimiter { value } => {
            with_driver(peer, "set battery limiter", |driver| {
                driver.set_battery_limiter(value)
            })
        }
        Request::BootSave { preset } => {
            println!("damxd: saving boot preset for uid {peer}: {preset:?}");
            match preset.save() {
                Ok(()) => Response {
                    ok: true,
                    message: Some("Boot preset saved".to_string()),
                    preset: Some(preset),
                    ..Default::default()
                },
                Err(err) => Response::error(format!("{err:#}")),
            }
        }
        Request::BootApply => {
            let preset = BootPreset::load();
            if !preset.enabled {
                return Response::error("the boot preset is disabled");
            }
            let errors = preset.apply();
            if errors.is_empty() {
                Response {
                    ok: true,
                    message: Some("Boot preset applied".to_string()),
                    status: Some(current_status()),
                    ..Default::default()
                }
            } else {
                Response::error(errors.join("; "))
            }
        }
    }
}

fn with_driver<F>(peer: u32, action: &str, f: F) -> Response
where
    F: FnOnce(&Driver) -> Result<()>,
{
    let Some(driver) = Driver::discover() else {
        return Response::error("the linuwu_sense driver is not loaded");
    };
    match f(&driver) {
        Ok(()) => {
            println!("damxd: uid {peer} {action}");
            Response::with_status(driver.status())
        }
        Err(err) => Response::error(format!("{err:#}")),
    }
}

fn current_status() -> Status {
    match Driver::discover() {
        Some(driver) => driver.status(),
        None => {
            let mut status = Status {
                driver_loaded: false,
                kernel: driver::kernel_release(),
                model: fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
                    .ok()
                    .map(|s| s.trim().to_string()),
                on_ac: driver::on_ac(),
                ..Default::default()
            };
            policy::decorate(&mut status);
            status
        }
    }
}

fn set_profile(peer: u32, value: String) -> Response {
    let Some(driver) = Driver::discover() else {
        return Response::error("the linuwu_sense driver is not loaded");
    };
    let policy = ProfilePolicy::load();
    if let Some(fallback) = policy::clamp_profile(
        &value,
        &driver.profile_choices(),
        driver::on_ac(),
        &policy,
    ) {
        return Response::error(format!(
            "on battery the highest available profile is {fallback}; plug in the charger to use {value}"
        ));
    }
    match driver.set_profile(&value) {
        Ok(()) => {
            println!("damxd: uid {peer} set power profile to {value}");
            Response::with_status(driver.status())
        }
        Err(err) => Response::error(format!("{err:#}")),
    }
}

fn set_policy(peer: u32, limit_on_battery: bool, battery_profile: String) -> Response {
    let policy = ProfilePolicy {
        limit_on_battery,
        battery_profile,
    };
    if let Some(driver) = Driver::discover() {
        let choices = driver.profile_choices();
        if policy.limit_on_battery
            && !choices.is_empty()
            && !choices.iter().any(|c| c == &policy.battery_profile)
        {
            return Response::error(format!(
                "unknown battery profile {:?}",
                policy.battery_profile
            ));
        }
    }
    if let Err(err) = policy.save() {
        return Response::error(format!("{err:#}"));
    }
    println!(
        "damxd: uid {peer} set battery policy (limit={}, profile={})",
        policy.limit_on_battery, policy.battery_profile
    );
    let message = match enforce_policy(&policy) {
        Ok(message) => message,
        Err(err) => return Response::error(format!("{err:#}")),
    };
    Response {
        ok: true,
        message,
        policy: Some(policy),
        status: Some(current_status()),
        ..Default::default()
    }
}

/// Re-applies the battery cap to the current profile when needed.
fn enforce_policy(policy: &ProfilePolicy) -> Result<Option<String>> {
    let Some(driver) = Driver::discover() else {
        return Ok(None);
    };
    if !policy.limit_on_battery || driver::on_ac() != Some(false) {
        return Ok(None);
    }
    let Some(current) = driver.profile() else {
        return Ok(None);
    };
    let Some(fallback) = policy::clamp_profile(
        &current,
        &driver.profile_choices(),
        Some(false),
        policy,
    ) else {
        return Ok(None);
    };
    driver.set_profile(&fallback)?;
    println!("damxd: on battery, profile {current} -> {fallback}");
    Ok(Some(format!(
        "On battery, the profile was limited to {fallback}"
    )))
}

/// Watches the charger so unplugging it can never leave a profile above the
/// battery limit selected. This is what keeps the policy global.
fn watch_power_source() {
    thread::spawn(|| {
        let mut last = driver::on_ac();
        loop {
            thread::sleep(Duration::from_secs(2));
            let current = driver::on_ac();
            if current == last {
                continue;
            }
            last = current;
            let policy = ProfilePolicy::load();
            match current {
                Some(false) => println!("damxd: charger unplugged"),
                Some(true) => println!("damxd: charger plugged in"),
                None => continue,
            }
            if let Err(err) = enforce_policy(&policy) {
                eprintln!("damxd: applying the battery profile failed: {err:#}");
            }
        }
    });
}

/// The module may load after us at boot, so retry for a while before applying.
fn apply_boot_preset_later() {
    thread::spawn(|| {
        let preset = BootPreset::load();
        if !preset.enabled || preset.is_empty() {
            println!("damxd: no boot preset configured");
            return;
        }
        for attempt in 0..60 {
            if Driver::discover().is_some() {
                let errors = preset.apply();
                if errors.is_empty() {
                    println!("damxd: boot preset applied on attempt {}", attempt + 1);
                } else {
                    eprintln!("damxd: boot preset failed: {}", errors.join("; "));
                }
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
        eprintln!("damxd: driver did not appear within 60s; boot preset skipped");
    });
}

/// UID of the process on the other end of the socket (SO_PEERCRED).
fn peer_uid(stream: &UnixStream) -> u32 {
    use std::os::unix::io::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc == 0 {
        cred.uid
    } else {
        u32::MAX
    }
}

fn group_gid(group: &str) -> Option<u32> {
    let text = fs::read_to_string("/etc/group").ok()?;
    for line in text.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 3 && fields[0] == group {
            return fields[2].parse().ok();
        }
    }
    None
}
