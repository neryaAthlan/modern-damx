//! Saved power profiles for AC and battery operation.

use crate::driver::Status;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfilePreferences {
    pub ac_profile: String,
    pub battery_profile: String,
}

impl Default for ProfilePreferences {
    fn default() -> Self {
        Self {
            ac_profile: "balanced".to_string(),
            battery_profile: "balanced".to_string(),
        }
    }
}

impl ProfilePreferences {
    pub fn path() -> PathBuf {
        env::var_os("DAMX_PROFILES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/damxd/profiles.toml"))
    }

    fn legacy_path() -> PathBuf {
        PathBuf::from("/etc/damxd/policy.toml")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .or_else(|| fs::read_to_string(Self::legacy_path()).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serialising power profiles")?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    pub fn for_source(&self, on_ac: Option<bool>) -> Option<&str> {
        match on_ac {
            Some(true) => Some(&self.ac_profile),
            Some(false) => Some(&self.battery_profile),
            None => None,
        }
    }

    pub fn for_choices(&self, choices: &[String]) -> Self {
        if choices.is_empty() {
            return self.clone();
        }
        let fallback = || {
            choices
                .iter()
                .find(|choice| choice.as_str() == "balanced")
                .cloned()
                .unwrap_or_else(|| choices[0].clone())
        };
        let ac_profile = if choices.iter().any(|choice| choice == &self.ac_profile) {
            self.ac_profile.clone()
        } else {
            choices.last().cloned().unwrap_or_else(|| fallback())
        };
        let battery_profile = if choices
            .iter()
            .any(|choice| choice == &self.battery_profile)
        {
            self.battery_profile.clone()
        } else {
            fallback()
        };
        Self {
            ac_profile,
            battery_profile,
        }
    }

    pub fn validate(&self, choices: &[String]) -> Result<()> {
        for (source, profile) in [
            ("plugged in", self.ac_profile.as_str()),
            ("on battery", self.battery_profile.as_str()),
        ] {
            if !choices.is_empty() && !choices.iter().any(|choice| choice == profile) {
                bail!("unsupported profile {profile:?} for {source} (choices: {})", choices.join(", "));
            }
        }
        Ok(())
    }
}

/// Adds the saved AC and battery choices to the live driver status.
pub fn decorate(status: &mut Status) {
    let preferences = ProfilePreferences::load().for_choices(&status.profile_choices);
    status.ac_profile = preferences.ac_profile;
    status.battery_profile = preferences.battery_profile;
}
