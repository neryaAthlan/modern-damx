//! `damxd` — small privileged helper for DAMX.
//!
//! The driver's sysfs attributes are owned by root and sysfs permissions cannot
//! be changed at runtime, so the GUI and the CLI send their writes here over a
//! Unix socket that only members of the `linuwu_sense` group can open.
//!
//! The systemd service runs the boot preset before starting the daemon.

use anyhow::{Context, Result};
use damx_core::driver::{self, Driver, Lighting, Status};
use damx_core::ipc::{self, Request, ResetSection, Response};
use damx_core::policy::ProfilePreferences;
use damx_core::preset::BootPreset;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::collections::HashMap;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

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

    watch_power_source();
    watch_keyboard_activity();

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
        Request::ProfilePreferencesGet => {
            Response::with_profile_preferences(ProfilePreferences::load())
        }
        Request::ProfilePreferencesSet {
            ac_profile,
            battery_profile,
        } => set_profile_preferences(peer, ac_profile, battery_profile),
        Request::SetProfile { value } => set_profile(peer, value),
        Request::SetFan { cpu, gpu } => set_fan(peer, cpu, gpu),
        Request::SetLighting { zones, brightness } => {
            set_lighting(peer, zones, brightness)
        }
        Request::SetBacklightTimeout { seconds } => {
            with_driver(peer, "set backlight timeout", |driver| {
                driver.set_backlight_timeout(seconds)
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
        Request::ResetDefaults { section } => reset_defaults(peer, section),
    }
}

fn reset_defaults(peer: u32, section: ResetSection) -> Response {
    let mut errors = Vec::new();
    match section {
        ResetSection::Performance => reset_performance(&mut errors),
        ResetSection::KeyboardLighting => reset_keyboard_lighting(&mut errors),
        ResetSection::BootPreset => reset_boot_preset(&mut errors),
        ResetSection::Daemon => reset_daemon(&mut errors),
        ResetSection::All => {
            reset_performance(&mut errors);
            reset_keyboard_lighting(&mut errors);
            reset_boot_preset(&mut errors);
            reset_daemon(&mut errors);
        }
    }
    if !errors.is_empty() {
        return Response::error(errors.join("; "));
    }
    println!("damxd: uid {peer} restored {section:?} defaults");
    Response {
        ok: true,
        message: Some("Defaults restored".to_string()),
        status: Some(current_status()),
        preset: Some(BootPreset::load()),
        profile_preferences: Some(ProfilePreferences::load()),
        ..Default::default()
    }
}

fn reset_performance(errors: &mut Vec<String>) {
    let preferences = ProfilePreferences::default();
    if let Err(err) = preferences.save() {
        errors.push(format!("saving default profiles: {err:#}"));
    }
    let Some(driver) = Driver::discover() else {
        errors.push("performance defaults need the Linuwu Sense driver".to_string());
        return;
    };
    let preferences = preferences.for_choices(&driver.profile_choices());
    if let Some(profile) = preferences.for_source(driver::on_ac()) {
        if let Err(err) = driver.set_profile(profile) {
            errors.push(format!("power profile: {err:#}"));
        } else {
            sync_boot_profile(profile);
        }
    }
    if let Err(err) = driver.set_fan(0, 0) {
        errors.push(format!("fan control: {err:#}"));
    }
    if let Err(err) = driver.set_battery_limiter(false) {
        errors.push(format!("battery limiter: {err:#}"));
    }
    sync_boot_fan(0, 0);
}

fn reset_keyboard_lighting(errors: &mut Vec<String>) {
    let Some(driver) = Driver::discover() else {
        errors.push("keyboard defaults need the Linuwu Sense driver".to_string());
        return;
    };
    if let Err(err) = driver.set_lighting(&Lighting::default()) {
        errors.push(format!("keyboard lighting: {err:#}"));
    } else {
        sync_boot_lighting(&Lighting::default());
    }
    if let Err(err) = driver.set_backlight_timeout(30) {
        errors.push(format!("keyboard timeout: {err:#}"));
    }
}

fn reset_boot_preset(errors: &mut Vec<String>) {
    if let Err(err) = BootPreset::default().save() {
        errors.push(format!("boot preset: {err:#}"));
    }
}

fn reset_daemon(errors: &mut Vec<String>) {
    match Command::new("systemctl")
        .args(["enable", "damxd.service"])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => errors.push(format!("enabling damxd at boot failed ({status})")),
        Err(err) => errors.push(format!("enabling damxd at boot failed: {err}")),
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
        Err(err) => {
            eprintln!("damxd: uid {peer} {action} failed: {err:#}");
            Response::error(format!("{err:#}"))
        }
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
            damx_core::policy::decorate(&mut status);
            status
        }
    }
}

fn set_profile(peer: u32, value: String) -> Response {
    let Some(driver) = Driver::discover() else {
        return Response::error("the linuwu_sense driver is not loaded");
    };
    let mut preferences = ProfilePreferences::load();
    if let Err(err) = driver.set_profile(&value) {
        return Response::error(format!("{err:#}"));
    }
    match driver::on_ac() {
        Some(true) => preferences.ac_profile = value.clone(),
        Some(false) => preferences.battery_profile = value.clone(),
        None => {}
    }
    if let Err(err) = preferences.save() {
        return Response::error(format!("power profile changed, but saving its preference failed: {err:#}"));
    }
    println!("damxd: uid {peer} set power profile to {value}");
    sync_boot_profile(&value);
    Response::with_status(driver.status())
}

fn set_fan(peer: u32, cpu: u8, gpu: u8) -> Response {
    let response = with_driver(peer, "set fan speed", |driver| driver.set_fan(cpu, gpu));
    if response.ok {
        sync_boot_fan(cpu, gpu);
    }
    response
}

fn set_lighting(peer: u32, zones: [String; 4], brightness: u8) -> Response {
    let lighting = Lighting { zones, brightness };
    let response = with_driver(peer, "set keyboard lighting", |driver| {
        driver.set_lighting(&lighting)
    });
    if response.ok {
        sync_boot_lighting(&lighting);
    }
    response
}

/// Keep selected boot fields in step with changes made through DAMX controls.
/// Fields omitted from the preset stay unmanaged.
fn sync_boot_profile(value: &str) {
    let mut preset = BootPreset::load();
    if preset.profile.is_none() {
        return;
    }
    preset.profile = Some(value.to_string());
    if let Err(err) = preset.save() {
        eprintln!("damxd: could not sync boot profile: {err:#}");
    }
}

fn sync_boot_fan(cpu: u8, gpu: u8) {
    let mut preset = BootPreset::load();
    if preset.fan.is_none() {
        return;
    }
    preset.fan = Some(damx_core::driver::Fan { cpu, gpu });
    if let Err(err) = preset.save() {
        eprintln!("damxd: could not sync boot fan setting: {err:#}");
    }
}

fn sync_boot_lighting(lighting: &Lighting) {
    let mut preset = BootPreset::load();
    if preset.lighting.is_none() {
        return;
    }
    preset.lighting = Some(lighting.clone());
    if let Err(err) = preset.save() {
        eprintln!("damxd: could not sync boot lighting: {err:#}");
    }
}

fn set_profile_preferences(
    peer: u32,
    ac_profile: String,
    battery_profile: String,
) -> Response {
    let preferences = ProfilePreferences {
        ac_profile,
        battery_profile,
    };
    let driver = Driver::discover();
    if let Some(driver) = &driver {
        if let Err(err) = preferences.validate(&driver.profile_choices()) {
            return Response::error(format!("{err:#}"));
        }
    }
    if let Err(err) = preferences.save() {
        return Response::error(format!("{err:#}"));
    }
    let active_profile = preferences.for_source(driver::on_ac()).map(str::to_string);
    if let (Some(driver), Some(profile)) = (&driver, active_profile.as_deref()) {
        if let Err(err) = driver.set_profile(profile) {
            return Response::error(format!("preferences saved, but applying {profile} failed: {err:#}"));
        }
        sync_boot_profile(profile);
    }
    println!("damxd: uid {peer} set source profiles (AC={}, battery={})", preferences.ac_profile, preferences.battery_profile);
    Response::with_status(current_status())
}

/// Apply the configured profile for the currently connected power source.
fn apply_source_profile() -> Result<Option<String>> {
    let Some(driver) = Driver::discover() else {
        return Ok(None);
    };
    let preferences = ProfilePreferences::load().for_choices(&driver.profile_choices());
    let Some(profile) = preferences.for_source(driver::on_ac()) else {
        return Ok(None);
    };
    driver.set_profile(profile)?;
    sync_boot_profile(profile);
    println!("damxd: selected {profile} for current power source");
    Ok(Some(profile.to_string()))
}

/// Watches for power-source changes and applies the matching saved profile.
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
            match current {
                Some(false) => println!("damxd: charger unplugged"),
                Some(true) => println!("damxd: charger plugged in"),
                None => continue,
            }
            if let Err(err) = apply_source_profile() {
                eprintln!("damxd: applying the source profile failed: {err:#}");
            }
        }
    });
}

/// Acer's firmware timeout can switch the RGB keyboard off without waking it
/// again on keypress under Linux. Restore the driver's current lighting after
/// keyboard activity while the firmware timeout is enabled.
fn watch_keyboard_activity() {
    thread::spawn(|| {
        let mut devices: HashMap<PathBuf, fs::File> = HashMap::new();
        let mut last_restore = None;
        let mut last_device_scan = None;
        let event_size = if cfg!(target_pointer_width = "64") { 24 } else { 16 };
        let mut event = vec![0_u8; event_size];

        loop {
            let should_scan = last_device_scan
                .map(|last: Instant| last.elapsed() >= Duration::from_secs(2))
                .unwrap_or(true);
            if should_scan {
                for path in keyboard_event_paths() {
                    if devices.contains_key(&path) {
                        continue;
                    }
                    match fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(&path)
                    {
                        Ok(file) => {
                            println!("damxd: monitoring keyboard input at {}", path.display());
                            devices.insert(path, file);
                        }
                        Err(err) => eprintln!("damxd: cannot monitor keyboard input at {}: {err}", path.display()),
                    }
                }
                last_device_scan = Some(Instant::now());
            }

            devices.retain(|path, _| path.exists());
            for file in devices.values_mut() {
                loop {
                    match file.read(&mut event) {
                        Ok(count) if count == event_size => {
                            let type_offset = if event_size == 24 { 16 } else { 8 };
                            let value_offset = type_offset + 4;
                            let event_type = u16::from_ne_bytes([event[type_offset], event[type_offset + 1]]);
                            let value = i32::from_ne_bytes([
                                event[value_offset], event[value_offset + 1],
                                event[value_offset + 2], event[value_offset + 3],
                            ]);
                            // EV_KEY, value 1 is a fresh key press (ignore releases/repeats).
                            if event_type == 1 && value == 1 {
                                let now = Instant::now();
                                let due = last_restore
                                    .map(|last: Instant| now.duration_since(last) >= Duration::from_secs(2))
                                    .unwrap_or(true);
                                if due {
                                    last_restore = Some(now);
                                    if let Some(driver) = Driver::discover() {
                                        if driver.backlight_timeout().is_some_and(|seconds| seconds > 0) {
                                            if let Some(lighting) = driver.lighting() {
                                                if let Err(err) = driver.set_lighting(&lighting) {
                                                    eprintln!("damxd: restoring keyboard lighting after keypress failed: {err:#}");
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(_) => break,
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
}

/// Return event devices marked by the kernel as keyboards in its input-device list.
fn keyboard_event_paths() -> Vec<PathBuf> {
    let Ok(devices) = fs::read_to_string("/proc/bus/input/devices") else {
        return Vec::new();
    };
    devices
        .split("\n\n")
        .filter(|device| {
            device.lines().any(|line| {
                line.strip_prefix("H: Handlers=")
                    .is_some_and(|handlers| handlers.split_whitespace().any(|handler| handler == "kbd"))
            })
        })
        .flat_map(|device| {
            device.lines().flat_map(|line| {
                line.strip_prefix("H: Handlers=")
                    .into_iter()
                    .flat_map(|handlers| handlers.split_whitespace())
                    .filter_map(|handler| handler.strip_prefix("event"))
                    .filter_map(|number| number.parse::<u32>().ok())
                    .map(|number| PathBuf::from(format!("/dev/input/event{number}")))
                    .collect::<Vec<_>>()
            })
        })
        .collect()
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
