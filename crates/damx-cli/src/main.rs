//! `damx` — command line control for the Linuwu Sense driver.
//!
//! Writes go through `damxd` so they work without sudo. When the daemon is not
//! running, root can still use the tool directly (useful in a recovery shell).

use anyhow::{anyhow, bail, Context, Result};
use damx_core::config;
use damx_core::driver::{Driver, Lighting, Status};
use damx_core::ipc::{self, Request, Response};
use damx_core::policy::ProfilePreferences;
use damx_core::preset::BootPreset;
use damx_core::update;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    // Behave like a normal Unix tool when piped into `head`/`less`.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("damx: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(());
    };
    let rest = &args[1..];

    match command {
        "status" => cmd_status(rest),
        "profile" => cmd_profile(rest),
        "profiles" => cmd_profiles(rest),
        "fan" => cmd_fan(rest),
        "light" => cmd_light(rest),
        "boot" => cmd_boot(rest),
        "startup-apply" => cmd_startup_apply(),
        "startup-profile" => cmd_startup_profile(),
        "update" => cmd_update(rest),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "version" | "-V" | "--version" => {
            println!("damx {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => bail!("unknown command {other:?} (try `damx help`)"),
    }
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn send(request: Request) -> Result<Response> {
    match ipc::call(&request) {
        Ok(response) => {
            if response.ok {
                Ok(response)
            } else {
                bail!(
                    "{}",
                    response
                        .error
                        .unwrap_or_else(|| "damxd reported an unknown error".to_string())
                )
            }
        }
        Err(err) if is_root() => direct(request).with_context(|| format!("{err:#}")),
        Err(err) => Err(err),
    }
}

/// Privileged fallback used when `damxd` is not running and we are root.
fn direct(request: Request) -> Result<Response> {
    let driver = || {
        Driver::discover().ok_or_else(|| anyhow!("the linuwu_sense driver is not loaded"))
    };
    Ok(match request {
        Request::Ping => Response::ok(),
        Request::Status => Response::with_status(read_status()),
        Request::BootGet => Response::with_preset(BootPreset::load()),
        Request::ProfilePreferencesGet => {
            Response::with_profile_preferences(ProfilePreferences::load())
        }
        Request::ProfilePreferencesSet {
            ac_profile,
            battery_profile,
        } => {
            let preferences = ProfilePreferences {
                ac_profile,
                battery_profile,
            };
            if let Some(driver) = Driver::discover() {
                preferences.validate(&driver.profile_choices())?;
            }
            preferences.save()?;
            if let Some(profile) = preferences.for_source(damx_core::driver::on_ac()) {
                if let Some(driver) = Driver::discover() {
                    driver.set_profile(profile)?;
                }
            }
            Response::with_profile_preferences(preferences)
        }
        Request::SetProfile { value } => {
            let driver = driver()?;
            driver.set_profile(&value)?;
            let mut preferences = ProfilePreferences::load();
            match damx_core::driver::on_ac() {
                Some(true) => preferences.ac_profile = value,
                Some(false) => preferences.battery_profile = value,
                None => {}
            }
            preferences.save()?;
            Response::with_status(driver.status())
        }
        Request::SetFan { cpu, gpu } => {
            driver()?.set_fan(cpu, gpu)?;
            Response::with_status(driver()?.status())
        }
        Request::SetLighting { zones, brightness } => {
            let driver = driver()?;
            let lighting = Lighting { zones, brightness };
            driver.set_lighting(&lighting)?;
            let mut preset = BootPreset::load_checked()?;
            if preset.lighting.is_some() {
                preset.lighting = Some(lighting);
                preset.save()?;
            }
            Response::with_status(driver.status())
        }
        Request::SetBacklightTimeout { seconds } => {
            driver()?.set_backlight_timeout(seconds)?;
            Response::with_status(driver()?.status())
        }
        Request::SetBatteryLimiter { value } => {
            driver()?.set_battery_limiter(value)?;
            Response::with_status(driver()?.status())
        }
        Request::BootSave { preset } => {
            preset.save()?;
            Response::with_preset(preset)
        }
        Request::BootApply => {
            let preset = BootPreset::load();
            if !preset.enabled {
                bail!("the boot preset is disabled");
            }
            let errors = preset.apply();
            if !errors.is_empty() {
                bail!("{}", errors.join("; "));
            }
            Response::with_status(read_status())
        }
        Request::ResetDefaults { .. } => {
            bail!("reset defaults requires the damxd service")
        }
    })
}

/// Runs before the daemon opens its socket. RGB readiness is independent of
/// the Acer WMI fan and profile attributes.
fn cmd_startup_apply() -> Result<()> {
    if !is_root() {
        bail!("startup-apply requires root");
    }
    let preset = BootPreset::load_checked()?;
    let mut lighting = preset.lighting.as_ref().filter(|_| preset.enabled);
    let mut fan = preset.fan.filter(|_| preset.enabled);
    let mut profile_pending = true;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_lighting_error = None;
    let mut last_fan_error = None;
    let mut last_profile_error = None;

    loop {
        if let Some(saved) = lighting {
            match Driver::restore_lighting(saved) {
                Ok(backend) => {
                    println!("damx: restored boot keyboard lighting via {backend}");
                    lighting = None;
                    last_lighting_error = None;
                }
                Err(err) => last_lighting_error = Some(err),
            }
        }

        if profile_pending {
            match apply_startup_profile(&preset) {
                Ok(()) => {
                    profile_pending = false;
                    last_profile_error = None;
                }
                Err(err) => last_profile_error = Some(err),
            }
        }

        if let Some(saved) = fan {
            match Driver::discover()
                .context("Linuwu Sense fan controls are unavailable")
                .and_then(|driver| driver.set_fan(saved.cpu, saved.gpu))
            {
                Ok(()) => {
                    fan = None;
                    last_fan_error = None;
                }
                Err(err) => last_fan_error = Some(err),
            }
        }

        if (lighting.is_none() && fan.is_none() && !profile_pending)
            || Instant::now() >= deadline
        {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    for (setting, error) in [
        ("keyboard lighting", last_lighting_error),
        ("power profile", last_profile_error),
        ("fan speed", last_fan_error),
    ] {
        if let Some(error) = error {
            eprintln!("damx: skipped startup {setting}: {error:#}");
        }
    }
    Ok(())
}

/// Reapply the source profile after an optional power-profiles-daemon starts.
fn cmd_startup_profile() -> Result<()> {
    if !is_root() {
        bail!("startup-profile requires root");
    }
    apply_startup_profile(&BootPreset::load_checked()?)
}

fn apply_startup_profile(preset: &BootPreset) -> Result<()> {
    let Some(driver) = Driver::discover() else {
        // This is also used on machines without Linuwu Sense.
        if preset.enabled && preset.profile.is_some() {
            bail!("Linuwu Sense profile controls are unavailable");
        }
        return Ok(());
    };
    let choices = driver.profile_choices();
    if choices.is_empty() {
        if preset.enabled && preset.profile.is_some() {
            bail!("no Acer platform profiles are available");
        }
        return Ok(());
    }
    let preferences = ProfilePreferences::load().for_choices(&choices);
    let fallback = preset.profile.as_deref().filter(|_| preset.enabled);
    if let Some(profile) = preferences.for_source(damx_core::driver::on_ac()).or(fallback) {
        if driver.profile().as_deref() != Some(profile) {
            driver.set_profile(profile)?;
        }
        println!("damx: startup power profile is {profile}");
    }
    Ok(())
}

fn read_status() -> Status {
    match Driver::discover() {
        Some(driver) => driver.status(),
        None => Status {
            driver_loaded: false,
            kernel: damx_core::driver::kernel_release(),
            ..Default::default()
        },
    }
}

fn get_status() -> Status {
    match ipc::call(&Request::Status) {
        Ok(response) if response.ok && response.status.is_some() => response.status.unwrap(),
        _ => read_status(),
    }
}

fn cmd_status(args: &[String]) -> Result<()> {
    let status = get_status();
    let unit = config::load().ui.temperature_unit;
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    println!(
        "Driver:            {}",
        match (&status.driver_version, status.driver_loaded) {
            (Some(v), true) => format!("linuwu_sense {v} (loaded)"),
            (None, true) => "linuwu_sense (loaded, version unknown)".to_string(),
            (_, false) => "not loaded".to_string(),
        }
    );
    if let Some(model) = &status.model {
        println!("Model:             {model}");
    }
    println!("Kernel:            {}", status.kernel);
    println!(
        "Power source:      {}",
        match status.on_ac {
            Some(true) => "plugged in",
            Some(false) => "on battery",
            None => "unknown",
        }
    );
    println!(
        "Temperatures:      CPU {} · GPU {}",
        unit.format(status.cpu_temp_c),
        unit.format(status.gpu_temp_c)
    );
    println!(
        "Power profile:     {}{}",
        status.profile.as_deref().unwrap_or("-"),
        if status.profile_choices.is_empty() {
            String::new()
        } else {
            format!("  (choices: {})", status.profile_choices.join(", "))
        }
    );
    if let Some((cpu, gpu)) = status.fan_rpm {
        println!("Fan RPM:           CPU {cpu} · GPU {gpu}");
    }
    if let Some(fan) = status.fan {
        println!(
            "Fan speed:         {}",
            if fan.is_auto() {
                "auto".to_string()
            } else {
                format!("manual, CPU {}%, GPU {}%", fan.cpu, fan.gpu)
            }
        );
    }
    println!(
        "Configured profiles: AC {} · battery {}",
        status.ac_profile, status.battery_profile
    );
    if let Some(lighting) = &status.lighting {
        println!(
            "Keyboard lighting: {} brightness {}%",
            lighting
                .zones
                .iter()
                .map(|z| format!("#{z}"))
                .collect::<Vec<_>>()
                .join(" "),
            lighting.brightness
        );
    }
    if let Some(timeout) = status.backlight_timeout {
        println!("Backlight timeout: {}", if timeout == 0 { "off".to_string() } else { format!("{timeout}s") });
    }
    if let Some(limiter) = status.battery_limiter {
        println!("Battery limiter:   {}", if limiter { "on" } else { "off" });
    }
    if status.capabilities.is_empty() {
        println!("Capabilities:      none detected");
    } else {
        println!("Capabilities:      {}", status.capabilities.join(", "));
    }

    let preset = match ipc::call(&Request::BootGet) {
        Ok(response) => response.preset,
        Err(_) => Some(BootPreset::load()),
    };
    if let Some(preset) = preset {
        println!("Boot preset:       {}", describe_preset(&preset));
    }
    Ok(())
}

fn describe_preset(preset: &BootPreset) -> String {
    if !preset.enabled {
        return "disabled".to_string();
    }
    let mut parts = Vec::new();
    if let Some(profile) = &preset.profile {
        parts.push(format!("profile {profile}"));
    }
    if let Some(fan) = preset.fan {
        parts.push(if fan.is_auto() {
            "fan auto".to_string()
        } else {
            format!("fan {},{}", fan.cpu, fan.gpu)
        });
    }
    if let Some(lighting) = &preset.lighting {
        parts.push(format!("lighting {}%", lighting.brightness));
    }
    if parts.is_empty() {
        "disabled (nothing selected)".to_string()
    } else {
        parts.join(", ")
    }
}

fn cmd_profile(args: &[String]) -> Result<()> {
    match args.first() {
        None => {
            println!("{}", get_status().profile.unwrap_or_default());
            Ok(())
        }
        Some(value) => {
            send(Request::SetProfile {
                value: value.to_string(),
            })?;
            println!("power profile set to {value}");
            Ok(())
        }
    }
}

fn cmd_fan(args: &[String]) -> Result<()> {
    match args {
        [] => {
            match get_status().fan {
                Some(fan) if fan.is_auto() => println!("auto"),
                Some(fan) => println!("{},{}", fan.cpu, fan.gpu),
                None => println!("unsupported"),
            }
            Ok(())
        }
        [mode] if mode == "auto" => {
            send(Request::SetFan { cpu: 0, gpu: 0 })?;
            println!("fan control set to auto");
            Ok(())
        }
        [cpu, gpu] => {
            let cpu: u8 = cpu.parse().context("CPU speed must be 0-100")?;
            let gpu: u8 = gpu.parse().context("GPU speed must be 0-100")?;
            if cpu > 100 || gpu > 100 {
                bail!("fan speed must be between 0 and 100");
            }
            send(Request::SetFan { cpu, gpu })?;
            println!("fan speed set to CPU {cpu}%, GPU {gpu}%");
            Ok(())
        }
        _ => bail!("usage: damx fan auto | damx fan <cpu> <gpu>"),
    }
}

fn cmd_light(args: &[String]) -> Result<()> {
    match args {
        [] => {
            match get_status().lighting {
                Some(lighting) => println!("{}", lighting.to_sysfs()),
                None => println!("unsupported"),
            }
            Ok(())
        }
        [z1, z2, z3, z4, brightness] => {
            let brightness: u8 = brightness.parse().context("brightness must be 0-100")?;
            let lighting = Lighting {
                zones: [z1.clone(), z2.clone(), z3.clone(), z4.clone()],
                brightness: brightness.min(100),
            };
            send(Request::SetLighting {
                zones: lighting.zones.clone(),
                brightness: lighting.brightness,
            })?;
            println!("keyboard lighting set to {}", lighting.to_sysfs());
            Ok(())
        }
        _ => bail!("usage: damx light <zone1> <zone2> <zone3> <zone4> <brightness>"),
    }
}

fn cmd_boot(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None | Some("show") => {
            let preset = match ipc::call(&Request::BootGet) {
                Ok(response) => response.preset.unwrap_or_default(),
                Err(_) => BootPreset::load(),
            };
            println!("{}", toml::to_string_pretty(&preset)?);
            Ok(())
        }
        Some("save") => {
            let status = get_status();
            let preset = BootPreset::from_status(
                &status,
                status.profile.is_some(),
                status.fan.is_some(),
                status.lighting.is_some(),
            );
            send(Request::BootSave {
                preset: preset.clone(),
            })?;
            println!("boot preset saved: {}", describe_preset(&preset));
            Ok(())
        }
        Some("disable") => {
            let mut preset = match ipc::call(&Request::BootGet) {
                Ok(response) => response.preset.unwrap_or_default(),
                Err(_) => BootPreset::load(),
            };
            preset.enabled = false;
            send(Request::BootSave { preset })?;
            println!("boot preset disabled");
            Ok(())
        }
        Some("enable") => {
            let mut preset = match ipc::call(&Request::BootGet) {
                Ok(response) => response.preset.unwrap_or_default(),
                Err(_) => BootPreset::load(),
            };
            if preset.is_empty() {
                bail!("no boot preset saved yet; run `damx boot save` first");
            }
            preset.enabled = true;
            send(Request::BootSave { preset })?;
            println!("boot preset enabled");
            Ok(())
        }
        Some("apply") => {
            send(Request::BootApply)?;
            println!("boot preset applied");
            Ok(())
        }
        Some(other) => bail!("unknown boot command {other:?} (show, save, enable, disable, apply)"),
    }
}

fn cmd_profiles(args: &[String]) -> Result<()> {
    let mut preferences = match ipc::call(&Request::ProfilePreferencesGet) {
        Ok(response) => response.profile_preferences,
        Err(_) => Some(ProfilePreferences::load()),
    }
    .unwrap_or_default();

    match args {
        [] => {
            println!("Plugged in: {}", preferences.ac_profile);
            println!("On battery: {}", preferences.battery_profile);
            Ok(())
        }
        [show] if show == "show" => {
            println!("Plugged in: {}", preferences.ac_profile);
            println!("On battery: {}", preferences.battery_profile);
            Ok(())
        }
        [source, profile] if source == "ac" || source == "plugged-in" => {
            preferences.ac_profile = profile.clone();
            send(Request::ProfilePreferencesSet {
                ac_profile: preferences.ac_profile,
                battery_profile: preferences.battery_profile,
            })?;
            println!("plugged-in profile set to {profile}");
            Ok(())
        }
        [source, profile] if source == "battery" => {
            preferences.battery_profile = profile.clone();
            send(Request::ProfilePreferencesSet {
                ac_profile: preferences.ac_profile,
                battery_profile: preferences.battery_profile,
            })?;
            println!("battery profile set to {profile}");
            Ok(())
        }
        _ => bail!("usage: damx profiles [show] | ac <profile> | battery <profile>"),
    }
}

fn cmd_update(args: &[String]) -> Result<()> {
    config::ensure_config_file();
    let should_check = !args.iter().any(|a| a == "--cached");
    let config = config::load();
    let report = if should_check {
        println!("Checking GitHub for new releases...");
        let report = update::check_all(&config, env!("CARGO_PKG_VERSION"));
        if let Err(err) = update::save_report(&report) {
            eprintln!("damx: could not cache the update check: {err:#}");
        }
        report
    } else {
        update::load_report().ok_or_else(|| anyhow!("no cached update check yet"))?
    };

    print_component(&report.driver);
    print_component(&report.gui);
    Ok(())
}

fn print_component(component: &update::ComponentStatus) {
    println!();
    println!("{}", component.name);
    println!("  repository: {}", component.repo);
    println!("  installed:  {}", component.current.as_deref().unwrap_or("unknown"));
    match &component.latest {
        Some(release) => {
            println!("  latest:     {}", release.tag);
            if component.available {
                println!("  status:     update available: {}", release.url);
            } else {
                println!("  status:     up to date");
            }
        }
        None => println!(
            "  status:     {}",
            component.note.as_deref().unwrap_or("no release information")
        ),
    }
}

fn print_help() {
    println!(
        "damx {version} - control the Acer WMI driver on Linux

Usage:
  damx status [--json]                 Show driver and settings state
  damx profile [name]                  Show or set the power profile
  damx profiles [show]                 Show AC and battery profile choices
  damx profiles ac|battery <profile>   Set a profile for that power source
  damx fan auto | <cpu> <gpu>          Show or set fan speeds (0-100)
  damx light <z1> <z2> <z3> <z4> <b>   Set the four keyboard zones and brightness
  damx boot show|save|enable|disable   Manage the preset applied at boot
  damx boot apply                      Apply the saved boot preset now
  damx update [--cached]               Check GitHub for driver/GUI updates
  damx version                         Print the version

Power profiles: low-power, quiet, balanced, balanced-performance, performance
",
        version = env!("CARGO_PKG_VERSION")
    );
}
