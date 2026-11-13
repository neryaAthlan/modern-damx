//! `damx` — command line control for the Linuwu Sense driver.
//!
//! Writes go through `damxd` so they work without sudo. When the daemon is not
//! running, root can still use the tool directly (useful in a recovery shell).

use anyhow::{anyhow, bail, Context, Result};
use damx_core::config;
use damx_core::driver::{Driver, Lighting, Status};
use damx_core::ipc::{self, Request, Response};
use damx_core::policy::{self, ProfilePolicy};
use damx_core::preset::BootPreset;
use damx_core::update;
use std::process::ExitCode;

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
        "fan" => cmd_fan(rest),
        "light" => cmd_light(rest),
        "boot" => cmd_boot(rest),
        "policy" => cmd_policy(rest),
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
        Request::PolicyGet => Response::with_policy(ProfilePolicy::load()),
        Request::PolicySet {
            limit_on_battery,
            battery_profile,
        } => {
            let policy = ProfilePolicy {
                limit_on_battery,
                battery_profile,
            };
            policy.save()?;
            if policy.limit_on_battery && damx_core::driver::on_ac() == Some(false) {
                if let Some(driver) = Driver::discover() {
                    if let Some(current) = driver.profile() {
                        if let Some(fallback) = policy::clamp_profile(
                            &current,
                            &driver.profile_choices(),
                            Some(false),
                            &policy,
                        ) {
                            driver.set_profile(&fallback)?;
                        }
                    }
                }
            }
            Response::with_policy(policy)
        }
        Request::SetProfile { value } => {
            let driver = driver()?;
            let policy = ProfilePolicy::load();
            if let Some(fallback) = policy::clamp_profile(
                &value,
                &driver.profile_choices(),
                damx_core::driver::on_ac(),
                &policy,
            ) {
                bail!(
                    "on battery the highest available profile is {fallback}; plug in the charger to use {value}"
                );
            }
            driver.set_profile(&value)?;
            Response::with_status(driver.status())
        }
        Request::SetFan { cpu, gpu } => {
            driver()?.set_fan(cpu, gpu)?;
            Response::with_status(driver()?.status())
        }
        Request::SetLighting { zones, brightness } => {
            driver()?.set_lighting(&Lighting { zones, brightness })?;
            Response::with_status(driver()?.status())
        }
        Request::SetBacklightTimeout { value } => {
            driver()?.set_backlight_timeout(value)?;
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
    })
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
        "Battery policy:    {}",
        if status.limit_on_battery {
            format!("limit to {}", status.battery_profile)
        } else {
            "no limit on battery".to_string()
        }
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
        println!("Backlight timeout: {}", if timeout { "on (30s)" } else { "off" });
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

fn cmd_policy(args: &[String]) -> Result<()> {
    let policy = match ipc::call(&Request::PolicyGet) {
        Ok(response) => response.policy,
        Err(_) => Some(ProfilePolicy::load()),
    }
    .unwrap_or_default();

    match args.first().map(String::as_str) {
        None | Some("show") => {
            println!(
                "Battery profile limit: {}",
                if policy.limit_on_battery {
                    format!("on (max {})", policy.battery_profile)
                } else {
                    "off".to_string()
                }
            );
            Ok(())
        }
        Some("off") => {
            send(Request::PolicySet {
                limit_on_battery: false,
                battery_profile: policy.battery_profile,
            })?;
            println!("battery profile limit disabled");
            Ok(())
        }
        Some(profile) => {
            send(Request::PolicySet {
                limit_on_battery: true,
                battery_profile: profile.to_string(),
            })?;
            println!("on battery, the profile is limited to {profile}");
            Ok(())
        }
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
  damx fan auto | <cpu> <gpu>          Show or set fan speeds (0-100)
  damx light <z1> <z2> <z3> <z4> <b>   Set the four keyboard zones and brightness
  damx boot show|save|enable|disable   Manage the preset applied at boot
  damx boot apply                      Apply the saved boot preset now
  damx policy [show|off|<profile>]     Show or set the battery profile limit
  damx update [--cached]               Check GitHub for driver/GUI updates
  damx version                         Print the version

Power profiles: low-power, quiet, balanced, balanced-performance, performance
",
        version = env!("CARGO_PKG_VERSION")
    );
}
