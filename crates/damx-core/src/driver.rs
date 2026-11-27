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
use std::fs::{self, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
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
    /// Keyboard idle timeout in seconds; zero disables the timeout.
    pub backlight_timeout: Option<u16>,
    pub battery_limiter: Option<bool>,
    pub usb_charging: Option<u8>,
    pub capabilities: Vec<String>,
    /// `Some(true)` when the charger is connected.
    pub on_ac: Option<bool>,
    pub cpu_temp_c: Option<f32>,
    pub gpu_temp_c: Option<f32>,
    /// Current fan speeds in RPM, `(cpu, gpu)`.
    pub fan_rpm: Option<(u32, u32)>,
    /// Saved profiles for the two power sources.
    pub ac_profile: String,
    pub battery_profile: String,
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
        if !enek5130_hid_devices().is_empty() {
            if let Some(lighting) = load_hid_lighting() {
                return Some(lighting);
            }
        }
        let path = self.kb_attr("per_zone_mode")?;
        Lighting::parse(&fs::read_to_string(path).ok()?)
    }

    pub fn set_lighting(&self, lighting: &Lighting) -> Result<()> {
        if !enek5130_hid_devices().is_empty() {
            set_enek5130_lighting(lighting)?;
            save_hid_lighting(lighting)?;
            return Ok(());
        }

        self.set_wmi_lighting(lighting)
    }

    /// Boot can reach the WMI attributes before the RGB controller has a
    /// hidraw node. On ENEK5130 hardware, a successful WMI write does not
    /// establish that the LEDs changed, so wait for the matching HID device.
    pub fn restore_lighting(lighting: &Lighting) -> Result<&'static str> {
        if !enek5130_hid_devices().is_empty() || enek5130_acpi_present() {
            set_enek5130_lighting(lighting)?;
            save_hid_lighting(lighting)?;
            return Ok("ENEK5130 HID");
        }
        Self::discover()
            .context("Linuwu Sense keyboard controls are unavailable")?
            .set_wmi_lighting(lighting)?;
        Ok("Linuwu Sense WMI")
    }

    fn set_wmi_lighting(&self, lighting: &Lighting) -> Result<()> {
        let path = self
            .kb_attr("per_zone_mode")
            .ok_or_else(|| anyhow!("this laptop does not expose a four-zone keyboard"))?;
        self.write_attr(&path, &lighting.to_sysfs())
    }

    pub fn backlight_timeout(&self) -> Option<u16> {
        self.read_attr("backlight_timeout")?
            .parse::<u16>()
            .ok()
            .filter(|seconds| *seconds <= 255)
    }

    pub fn set_backlight_timeout(&self, seconds: u16) -> Result<()> {
        if seconds > 255 {
            bail!("keyboard timeout must be between 0 and 255 seconds");
        }
        let path = self.attr("backlight_timeout");
        if !path.exists() {
            bail!("backlight_timeout is not available on this laptop");
        }
        self.write_attr(&path, &seconds.to_string())
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

        let (cpu_temp_c, gpu_temp_c) = temperatures();
        let fan_rpm = fan_rpm();
        let mut status = Status {
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
            on_ac: on_ac(),
            cpu_temp_c,
            gpu_temp_c,
            fan_rpm,
            ac_profile: "balanced".to_string(),
            battery_profile: "balanced".to_string(),
        };
        crate::policy::decorate(&mut status);
        status
    }
}

/// `Some(true)` when a mains adapter is online, `Some(false)` on battery, and
/// `None` when no mains supply is exposed (for example on a desktop).
pub fn on_ac() -> Option<bool> {
    // DAMX_POWER_SUPPLY exists so the battery policy can be tested without
    // unplugging the machine.
    let root = std::env::var_os("DAMX_POWER_SUPPLY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/sys/class/power_supply"));
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let kind = fs::read_to_string(dir.join("type")).ok()?;
        if kind.trim() != "Mains" {
            continue;
        }
        let online = fs::read_to_string(dir.join("online")).ok()?;
        return Some(online.trim() == "1");
    }
    None
}

/// CPU and GPU temperatures in degrees Celsius.
///
/// The driver's own hwmon device exposes the embedded controller sensors; when
/// the GPU sensor reads 0 (the dGPU is asleep) the proprietary NVIDIA driver is
/// asked instead.
pub fn temperatures() -> (Option<f32>, Option<f32>) {
    let acer = hwmon_by_name("acer");
    let cpu = acer
        .as_ref()
        .and_then(|dir| read_millidegrees(&dir.join("temp1_input")))
        .or_else(coretemp_package);
    let gpu = acer
        .as_ref()
        .and_then(|dir| read_millidegrees(&dir.join("temp2_input")))
        .filter(|value| *value > 0.0)
        .or_else(nvidia_temperature)
        .or_else(gpu_hwmon);
    (cpu, gpu)
}

/// Current fan speeds in RPM from the driver's hwmon device: `(CPU, GPU)`.
pub fn fan_rpm() -> Option<(u32, u32)> {
    let dir = hwmon_by_name("acer")?;
    let cpu = read_rpm(&dir.join("fan1_input"))?;
    let gpu = read_rpm(&dir.join("fan2_input")).unwrap_or(0);
    Some((cpu, gpu))
}

fn read_rpm(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn hwmon_by_name(name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(found) = fs::read_to_string(dir.join("name")) else {
            continue;
        };
        if found.trim() == name {
            return Some(dir);
        }
    }
    None
}

fn read_millidegrees(path: &Path) -> Option<f32> {
    let raw = fs::read_to_string(path).ok()?;
    let milli: f32 = raw.trim().parse().ok()?;
    Some((milli / 1000.0 * 10.0).round() / 10.0)
}

fn coretemp_package() -> Option<f32> {
    let dir = hwmon_by_name("coretemp")?;
    let entries = fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().to_string();
        if !name.starts_with("temp") || !name.ends_with("_label") {
            continue;
        }
        if fs::read_to_string(&path).ok()?.trim() == "Package id 0" {
            let input = dir.join(name.replace("_label", "_input"));
            return read_millidegrees(&input);
        }
    }
    read_millidegrees(&dir.join("temp1_input"))
}

fn nvidia_temperature() -> Option<f32> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let value: f32 = text.lines().next()?.trim().parse().ok()?;
    Some((value * 10.0).round() / 10.0)
}

fn gpu_hwmon() -> Option<f32> {
    for name in ["amdgpu", "nouveau", "nvidia"] {
        if let Some(dir) = hwmon_by_name(name) {
            if let Some(value) = read_millidegrees(&dir.join("temp1_input")) {
                return Some(value);
            }
        }
    }
    None
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

fn enek5130_acpi_present() -> bool {
    fs::read_dir("/sys/bus/acpi/devices").is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_name().to_string_lossy().starts_with("ENE51301:")
        })
    })
}

/// Find Acer's ENEK5130 keyboard controller by its stable HID identity.
/// On newer Nitro keyboards, RGB sysfs writes may be accepted without
/// changing the LEDs; those models need direct HID feature reports.
fn enek5130_hid_devices() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("hidraw") {
            continue;
        }
        let uevent = Path::new("/sys/class/hidraw")
            .join(&name)
            .join("device/uevent");
        let Ok(identity) = fs::read_to_string(uevent) else {
            continue;
        };
        if identity.contains("HID_ID=0018:00000CF2:00005130")
            || identity.contains("HID_NAME=ENEK5130:00 0CF2:5130")
        {
            devices.push(entry.path());
        }
    }
    devices.sort();
    devices
}

/// Apply four-zone colors through ENEK5130 HID feature reports, matching the
/// working backend in legacy DAMX (report 0xA4/0x21, static effect 0x02).
fn set_enek5130_lighting(lighting: &Lighting) -> Result<()> {
    let devices = enek5130_hid_devices();
    if devices.is_empty() {
        bail!("ENEK5130 HID keyboard was not found");
    }

    let mut last_error = None;
    for device in devices {
        match set_enek5130_lighting_on_device(&device, lighting) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("no usable ENEK5130 HID device")))
}

fn set_enek5130_lighting_on_device(device: &Path, lighting: &Lighting) -> Result<()> {
    let hid = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(device)
        .with_context(|| format!("opening {}", device.display()))?;

    for (zone, mask) in lighting.zones.iter().zip([0x01u8, 0x02, 0x04, 0x08]) {
        let color = normalize_zone(zone);
        let red = u8::from_str_radix(&color[0..2], 16)?;
        let green = u8::from_str_radix(&color[2..4], 16)?;
        let blue = u8::from_str_radix(&color[4..6], 16)?;
        let mut report = [
            0xA4,
            0x21,
            0x02,
            lighting.brightness.min(100),
            0x00,
            0x00,
            red,
            green,
            blue,
            mask,
            0x00,
        ];
        let request = hid_ioc_set_feature(report.len());
        // SAFETY: report is writable storage of exactly the size encoded in
        // the matching HIDIOCSFEATURE request.
        let result = unsafe {
            libc::ioctl(
                hid.as_raw_fd(),
                request,
                report.as_mut_ptr().cast::<libc::c_void>(),
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("sending ENEK5130 color for zone mask {mask:#04x}"));
        }
    }
    Ok(())
}

fn hid_lighting_path() -> PathBuf {
    PathBuf::from("/etc/damxd/lighting.toml")
}

fn load_hid_lighting() -> Option<Lighting> {
    fs::read_to_string(hid_lighting_path())
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
}

fn save_hid_lighting(lighting: &Lighting) -> Result<()> {
    let path = hid_lighting_path();
    let directory = path
        .parent()
        .ok_or_else(|| anyhow!("invalid ENEK5130 lighting state path"))?;
    fs::create_dir_all(directory)
        .with_context(|| format!("creating {}", directory.display()))?;
    let temporary = path.with_extension("toml.tmp");
    let contents = toml::to_string_pretty(lighting).context("serialising ENEK5130 lighting")?;
    fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    fs::rename(&temporary, &path)
        .with_context(|| format!("replacing {}", path.display()))
}

/// Linux hidraw's HIDIOCSFEATURE(length): _IOC(_IOC_WRITE | _IOC_READ,
/// 'H', 0x06, length).
fn hid_ioc_set_feature(length: usize) -> libc::c_ulong {
    const IOC_NRBITS: u32 = 8;
    const IOC_TYPEBITS: u32 = 8;
    const IOC_SIZEBITS: u32 = 14;
    const IOC_TYPESHIFT: u32 = IOC_NRBITS;
    const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
    const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
    const IOC_READ_WRITE: u32 = 3;
    ((IOC_READ_WRITE << IOC_DIRSHIFT)
        | (u32::from(b'H') << IOC_TYPESHIFT)
        | 0x06
        | ((length as u32) << IOC_SIZESHIFT)) as libc::c_ulong
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
