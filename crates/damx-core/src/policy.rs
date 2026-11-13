//! Global profile policy: which power profiles are allowed while on battery.
//!
//! The setting is stored in `/etc/damxd/policy.toml` and enforced by `damxd`,
//! so the GUI, the CLI and the boot preset all obey the same rule.

use crate::driver::{self, Status};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilePolicy {
    #[serde(default = "default_true")]
    pub limit_on_battery: bool,
    #[serde(default = "default_battery_profile")]
    pub battery_profile: String,
}

impl Default for ProfilePolicy {
    fn default() -> Self {
        Self {
            limit_on_battery: true,
            battery_profile: default_battery_profile(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_battery_profile() -> String {
    "balanced".to_string()
}

impl ProfilePolicy {
    pub fn path() -> PathBuf {
        env::var_os("DAMX_POLICY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/damxd/policy.toml"))
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
        let text = toml::to_string_pretty(self).context("serialising the profile policy")?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }
}

/// Profiles ordered from least to most aggressive, as the driver lists them.
pub fn rank(choices: &[String], name: &str) -> Option<usize> {
    choices.iter().position(|choice| choice == name)
}

/// The profiles a user may pick right now.
///
/// On AC everything is allowed. On battery, when the limit is enabled, only
/// profiles up to `battery_profile` are allowed (by the driver's ordering).
pub fn allowed_profiles(choices: &[String], on_ac: Option<bool>, policy: &ProfilePolicy) -> Vec<String> {
    if on_ac != Some(false) || !policy.limit_on_battery {
        return choices.to_vec();
    }
    let Some(cap) = rank(choices, &policy.battery_profile) else {
        return choices.to_vec();
    };
    choices.iter().take(cap + 1).cloned().collect()
}

/// The profile to use instead of `requested`, when the current power source
/// does not allow it.
pub fn clamp_profile(
    requested: &str,
    choices: &[String],
    on_ac: Option<bool>,
    policy: &ProfilePolicy,
) -> Option<String> {
    if on_ac != Some(false) || !policy.limit_on_battery {
        return None;
    }
    let requested_rank = rank(choices, requested)?;
    let cap = rank(choices, &policy.battery_profile)?;
    if requested_rank > cap {
        Some(policy.battery_profile.clone())
    } else {
        None
    }
}

/// Fills the policy-dependent fields of a status snapshot.
pub fn decorate(status: &mut Status) {
    let policy = ProfilePolicy::load();
    let choices = status.profile_choices.clone();
    status.allowed_profiles = allowed_profiles(&choices, status.on_ac, &policy);
    status.limit_on_battery = policy.limit_on_battery;
    status.battery_profile = policy.battery_profile;
}

/// Convenience helper used by callers that only have a status snapshot.
pub fn status_with_policy() -> Option<Status> {
    let driver = driver::Driver::discover()?;
    let mut status = driver.status();
    decorate(&mut status);
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices() -> Vec<String> {
        ["low-power", "quiet", "balanced", "balanced-performance", "performance"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn on_battery_caps_at_balanced() {
        let policy = ProfilePolicy::default();
        let allowed = allowed_profiles(&choices(), Some(false), &policy);
        assert_eq!(allowed, vec!["low-power", "quiet", "balanced"]);
        assert_eq!(
            clamp_profile("performance", &choices(), Some(false), &policy),
            Some("balanced".to_string())
        );
        assert_eq!(
            clamp_profile("quiet", &choices(), Some(false), &policy),
            None
        );
    }

    #[test]
    fn on_ac_everything_allowed() {
        let policy = ProfilePolicy::default();
        assert_eq!(allowed_profiles(&choices(), Some(true), &policy), choices());
        assert_eq!(
            clamp_profile("performance", &choices(), Some(true), &policy),
            None
        );
    }

    #[test]
    fn limit_can_be_disabled() {
        let policy = ProfilePolicy {
            limit_on_battery: false,
            ..Default::default()
        };
        assert_eq!(allowed_profiles(&choices(), Some(false), &policy), choices());
    }
}
