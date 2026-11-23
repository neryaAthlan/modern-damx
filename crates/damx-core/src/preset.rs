//! The boot preset: the settings `damxd` re-applies at startup, so the laptop
//! comes up with the chosen power profile, fan speed and keyboard lighting
//! without anyone opening the GUI.

use crate::driver::{Fan, Lighting, Status};
use crate::policy::ProfilePreferences;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

/// `Some(value)` means "apply this at boot"; `None` means "leave it alone".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BootPreset {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan: Option<Fan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<Lighting>,
}

impl BootPreset {
    pub fn path() -> PathBuf {
        env::var_os("DAMX_BOOT_PRESET")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/damxd/boot.toml"))
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serialising the boot preset")?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.profile.is_none() && self.fan.is_none() && self.lighting.is_none()
    }

    /// Builds a preset from a live status snapshot. `profile`, `fan` and
    /// `lighting` say which parts the user wants re-applied at boot.
    pub fn from_status(status: &Status, profile: bool, fan: bool, lighting: bool) -> Self {
        Self {
            enabled: true,
            profile: if profile { status.profile.clone() } else { None },
            fan: if fan { status.fan } else { None },
            lighting: if lighting { status.lighting.clone() } else { None },
        }
    }

    /// Applies the enabled parts of the preset to the driver.
    pub fn apply(&self) -> Vec<String> {
        let Some(driver) = crate::driver::Driver::discover() else {
            return vec!["driver is not loaded".to_string()];
        };
        let mut errors = Vec::new();
        if let Some(profile) = &self.profile {
            // Use the configured profile for the current power source.
            let preferences = ProfilePreferences::load().for_choices(&driver.profile_choices());
            let effective = preferences
                .for_source(crate::driver::on_ac())
                .unwrap_or(profile);
            if let Err(err) = driver.set_profile(&effective) {
                errors.push(format!("power profile: {err:#}"));
            }
        }
        if let Some(fan) = self.fan {
            if let Err(err) = driver.set_fan(fan.cpu, fan.gpu) {
                errors.push(format!("fan speed: {err:#}"));
            }
        }
        if let Some(lighting) = &self.lighting {
            if let Err(err) = driver.set_lighting(lighting) {
                errors.push(format!("keyboard lighting: {err:#}"));
            }
        }
        errors
    }
}

impl From<&Status> for BootPreset {
    fn from(status: &Status) -> Self {
        Self::from_status(status, true, true, status.lighting.is_some())
    }
}
