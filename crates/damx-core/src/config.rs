//! User-level configuration and cache locations.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_DRIVER_REPO: &str = "neryaAthlan/Div-Linuwu-Sense";
/// Change this if you publish the GUI under a different name.
pub const DEFAULT_GUI_REPO: &str = "neryaAthlan/modern-damx";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub updates: UpdatesConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            updates: UpdatesConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub temperature_unit: TemperatureUnit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

impl TemperatureUnit {
    pub fn format(self, celsius: Option<f32>) -> String {
        match celsius {
            Some(value) => match self {
                TemperatureUnit::Celsius => format!("{value:.1} °C"),
                TemperatureUnit::Fahrenheit => {
                    format!("{:.1} °F", value * 9.0 / 5.0 + 32.0)
                }
            },
            None => "—".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesConfig {
    #[serde(default = "default_driver_repo")]
    pub driver_repo: String,
    #[serde(default = "default_gui_repo")]
    pub gui_repo: String,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            driver_repo: default_driver_repo(),
            gui_repo: default_gui_repo(),
        }
    }
}

fn default_driver_repo() -> String {
    DEFAULT_DRIVER_REPO.to_string()
}

fn default_gui_repo() -> String {
    DEFAULT_GUI_REPO.to_string()
}

fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| "/root".to_string()))
}

fn config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
}

fn cache_home() -> PathBuf {
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
}

pub fn config_path() -> PathBuf {
    config_home().join("damx").join("config.toml")
}

pub fn cache_dir() -> PathBuf {
    cache_home().join("damx")
}

pub fn update_cache_path() -> PathBuf {
    cache_dir().join("update.json")
}

/// Loads `~/.config/damx/config.toml`, falling back to the defaults when it is
/// missing or malformed.
pub fn load() -> AppConfig {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Saves the user configuration, used by the temperature-unit preference.
pub fn save(config: &AppConfig) -> Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(config).context("serialising the configuration")?;
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

/// Writes the default configuration file if none exists yet, so users have
/// something to edit.
pub fn ensure_config_file() -> Option<PathBuf> {
    let path = config_path();
    if path.exists() {
        return Some(path);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok()?;
    }
    fs::write(
        &path,
        "# DAMX configuration\n\
         # Set these to your forks if you publish them elsewhere.\n\
         [updates]\n\
         driver_repo = \"neryaAthlan/Div-Linuwu-Sense\"\n\
         gui_repo = \"neryaAthlan/modern-damx\"\n\
         \n\
         [ui]\n\
         # \"celsius\" or \"fahrenheit\"\n\
         temperature_unit = \"celsius\"\n",
    )
    .ok()?;
    Some(path)
}
