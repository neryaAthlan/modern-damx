//! Access to the Linuwu Sense driver through sysfs.
//!
//! The driver exposes two attribute directories below the `acer-wmi` platform
//! device:
//!
//! * `nitro_sense` / `predator_sense` — fan speed, battery limiter, USB
//!   charging, backlight timeout, boot animation, LCD override.
//! * `four_zoned_kb` — `four_zone_mode` and `per_zone_mode` for the RGB
//!   keyboard.
//!
//! Everything here is plain file I/O. Reading works as an ordinary user;
//! writing needs root, which is why the GUI talks to `damxd`.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";
pub const PLATFORM_PROFILE_CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";

/// Four-zone keyboard colours (lowercase `rrggbb`) plus a 0-100 brightness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lighting {
    pub zones: [String; 4],
    pub brightness: u8,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            zones: [
                "ffffff".into(),
                "ffffff".into(),
                "ffffff".into(),
                "ffffff".into(),
            ],
            brightness: 100,
        }
    }
}

impl Lighting {
    /// Same layout as the driver's `per_zone_mode` attribute.
    pub fn to_sysfs(&self) -> String {
        format!(
            "{},{},{},{},{}",
            normalize_zone(&self.zones[0]),
            normalize_zone(&self.zones[1]),
            normalize_zone(&self.zones[2]),
            normalize_zone(&self.zones[3]),
            self.brightness.min(100)
        )
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let parts: Vec<&str> = raw.trim().split(',').collect();
        if parts.len() != 5 {
            return None;
        }
        let mut zones = ["ffffff".to_string(), "ffffff".to_string(), "ffffff".to_string(), "ffffff".to_string()];
        for (i, part) in parts[..4].iter().enumerate() {
            let hex = part.trim().trim_start_matches('#').to_lowercase();
            if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            zones[i] = hex;
        }
        let brightness = parts[4].trim().parse::<u8>().ok()?.min(100);
        Some(Self { zones, brightness })
    }
}

fn normalize_zone(zone: &str) -> String {
    let hex = zone.trim().trim_start_matches('#').to_lowercase();
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        hex
    } else {
        "ffffff".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fan {
    pub cpu: u8,
    pub gpu: u8,
}

impl Fan {
    pub fn is_auto(&self) -> bool {
        self.cpu == 0 && self.gpu == 0
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let mut it = raw.trim().split(',');
        let cpu = it.next()?.trim().parse::<u8>().ok()?;
        let gpu = it.next()?.trim().parse::<u8>().ok()?;
        Some(Self {
            cpu: cpu.min(100),
            gpu: gpu.min(100),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub driver_loaded: bool,
    pub driver_version: Option<String>,
    pub model: Option<String>,
    pub kernel: String,
    pub profile: Option<String>,
    pub profile_choices: Vec<String>,
    pub fan: Option<Fan>,
    pub lighting: Option<Lighting>,
    pub backlight_timeout: Option<bool>,
    pub battery_limiter: Option<bool>,
    pub usb_charging: Option<u8>,
    pub capabilities: Vec<String>,
}

impl Status {
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// A handle on the driver's sysfs directories.
#[derive(Debug, Clone)]
pub struct Driver {
    /// Either `.../acer-wmi/nitro_sense` or `.../acer-wmi/predator_sense`.
    sense_dir: PathBuf,
    kb_dir: Option<PathBuf>,
}

impl Driver {
    /// Find the driver's attribute directories. Returns `None` when the module
    /// is not loaded (or not bound to an Acer WMI device).
    pub fn discover() -> Option<Self> {
        for sense_dir in sense_candidates() {
            if !sense_dir.is_dir() {
                continue;
            }
            let kb_dir = sense_dir
                .parent()
                .map(|p| p.join("four_zoned_kb"))
                .filter(|p| p.is_dir());
            return Some(Self { sense_dir, kb_dir });
        }
        None
    }

    pub fn sense_dir(&self) -> &Path {
        &self.sense_dir
    }

    pub fn kb_dir(&self) -> Option<&Path> {
        self.kb_dir.as_deref()
    }

    fn attr(&self, name: &str) -> PathBuf {
        self.sense_dir.join(name)
    }

    fn kb_attr(&self, name: &str) -> Option<PathBuf> {
        self.kb_dir.as_ref().map(|d| d.join(name))
    }

    pub fn read_attr(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.attr(name))
            .ok()
            .map(|s| s.trim().to_string())
    }

    fn read_bool(&self, name: &str) -> Option<bool> {
        match self.read_attr(name)?.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }

    /// Writes `value` (with the trailing newline the sysfs parsers expect).
    /// The caller must have write permission — normally root.
    pub fn write_attr(&self, path: &Path, value: &str) -> Result<()> {
        fs::write(path, format!("{value}\n"))
            .with_context(|| format!("writing {value:?} to {}", path.display()))
    }

    pub fn profile(&self) -> Option<String> {
        fs::read_to_string(PLATFORM_PROFILE)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn profile_choices(&self) -> Vec<String> {
        fs::read_to_string(PLATFORM_PROFILE_CHOICES)
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }

    pub fn set_profile(&self, profile: &str) -> Result<()> {
        let choices = self.profile_choices();
        if !choices.is_empty() && !choices.iter().any(|c| c == profile) {
            bail!(
                "unsupported power profile {profile:?} (choices: {})",
                choices.join(", ")
            );
        }
        self.write_attr(Path::new(PLATFORM_PROFILE), profile)
    }

    pub fn fan(&self) -> Option<Fan> {
        Fan::parse(&self.read_attr("fan_speed")?)
    }

    pub fn set_fan(&self, cpu: u8, gpu: u8) -> Result<()> {
        if cpu > 100 || gpu > 100 {
            bail!("fan speed must be between 0 and 100");
        }
        let path = self.attr("fan_speed");
        self.write_attr(&path, &format!("{},{}", cpu, gpu))
    }

    pub fn lighting(&self) -> Option<Lighting> {
        let path = self.kb_attr("per_zone_mode")?;
        Lighting::parse(&fs::read_to_string(path).ok()?)
    }

    pub fn set_lighting(&self, lighting: &Lighting) -> Result<()> {
        let path = self
            .kb_attr("per_zone_mode")
            .ok_or_else(|| anyhow!("this laptop does not expose a four-zone keyboard"))?;
        self.write_attr(&path, &lighting.to_sysfs())
    }

    pub fn backlight_timeout(&self) -> Option<bool> {
        self.read_bool("backlight_timeout")
    }

    pub fn set_backlight_timeout(&self, enabled: bool) -> Result<()> {
        let path = self.attr("backlight_timeout");
        if !path.exists() {
            bail!("backlight_timeout is not available on this laptop");
        }
        self.write_attr(&path, if enabled { "1" } else { "0" })
    }

    pub fn battery_limiter(&self) -> Option<bool> {
        self.read_bool("battery_limiter")
    }

    pub fn set_battery_limiter(&self, enabled: bool) -> Result<()> {
        let path = self.attr("battery_limiter");
        if !path.exists() {
            bail!("battery_limiter is not available on this laptop");
        }
        self.write_attr(&path, if enabled { "1" } else { "0" })
    }

    pub fn usb_charging(&self) -> Option<u8> {
        self.read_attr("usb_charging")?.parse().ok()
    }

    pub fn status(&self) -> Status {
        let mut capabilities = Vec::new();
        for cap in [
            "fan_speed",
            "battery_limiter",
            "usb_charging",
            "backlight_timeout",
            "boot_animation_sound",
            "lcd_override",
            "four_zoned_kb",
        ] {
            let present = if cap == "four_zoned_kb" {
                self.kb_attr("per_zone_mode").is_some()
            } else {
                self.attr(cap).exists()
            };
            if present {
                capabilities.push(cap.to_string());
            }
        }

        Status {
            driver_loaded: true,
            driver_version: dkms_version(),
            model: read_trimmed("/sys/devices/virtual/dmi/id/product_name"),
            kernel: kernel_release(),
            profile: self.profile(),
            profile_choices: self.profile_choices(),
            fan: self.fan(),
            lighting: self.lighting(),
            backlight_timeout: self.backlight_timeout(),
            battery_limiter: self.battery_limiter(),
            usb_charging: self.usb_charging(),
            capabilities,
        }
    }
}

fn sense_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();

    // Preferred: walk the bound platform driver.
    if let Ok(entries) = fs::read_dir("/sys/bus/platform/drivers/acer-wmi") {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            for name in ["nitro_sense", "predator_sense"] {
                let candidate = dir.join(name);
                if candidate.is_dir() {
                    out.push(candidate);
                }
            }
        }
    }

    // Fallback: the module's own driver symlink, in case the bus view is
    // unavailable (for example inside a container).
    if let Ok(entries) = fs::read_dir("/sys/module/linuwu_sense/drivers") {
        for entry in entries.flatten() {
            let wmi = entry.path().join("acer-wmi");
            for name in ["nitro_sense", "predator_sense"] {
                let candidate = wmi.join(name);
                if candidate.is_dir() {
                    out.push(candidate);
                }
            }
        }
    }

    out
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn kernel_release() -> String {
    read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".to_string())
}

/// Version of the installed DKMS package, when the driver was installed by
/// `dkms` (the normal case for the fork).
pub fn dkms_version() -> Option<String> {
    let entries = fs::read_dir("/usr/src").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix("linuwu_sense-") else {
            continue;
        };
        let conf = entry.path().join("dkms.conf");
        if let Ok(text) = fs::read_to_string(conf) {
            for line in text.lines() {
                if let Some(value) = line.trim().strip_prefix("PACKAGE_VERSION=") {
                    let value = value.trim().trim_matches('"');
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    None
}
