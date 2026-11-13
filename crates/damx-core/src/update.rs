//! Checks the driver fork and this GUI's repository for newer releases.
//!
//! Only published releases and tags are compared, because a plain `main`
//! branch has no version to compare against. Both repositories are
//! configurable in `~/.config/damx/config.toml`.

use crate::config::{self, AppConfig};
use crate::driver;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub url: String,
    #[serde(default)]
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub name: String,
    pub repo: String,
    pub current: Option<String>,
    pub latest: Option<Release>,
    pub available: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateReport {
    pub checked_at: u64,
    pub driver: ComponentStatus,
    pub gui: ComponentStatus,
}

impl UpdateReport {
    pub fn any_available(&self) -> bool {
        self.driver.available || self.gui.available
    }
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn check_all(cfg: &AppConfig, gui_version: &str) -> UpdateReport {
    let driver_current = driver::dkms_version();
    UpdateReport {
        checked_at: now_epoch(),
        driver: check_component(
            "Linuwu Sense driver",
            &cfg.updates.driver_repo,
            driver_current.as_deref(),
        ),
        gui: check_component("DAMX", &cfg.updates.gui_repo, Some(gui_version)),
    }
}

fn check_component(name: &str, repo: &str, current: Option<&str>) -> ComponentStatus {
    let mut component = ComponentStatus {
        name: name.to_string(),
        repo: repo.to_string(),
        current: current.map(str::to_string),
        latest: None,
        available: false,
        note: None,
    };
    match latest_release(repo) {
        Ok(release) => {
            component.available = match current {
                Some(current) => is_newer(current, &release.tag),
                None => true,
            };
            component.latest = Some(release);
        }
        Err(err) => component.note = Some(format!("{err:#}")),
    }
    component
}

/// Latest published release, falling back to the newest tag.
fn latest_release(repo: &str) -> Result<Release> {
    let releases = get_json(&format!(
        "https://api.github.com/repos/{repo}/releases?per_page=10"
    ))?;
    if let Some(items) = releases.as_array() {
        for item in items {
            if item.get("draft").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            let Some(tag) = item.get("tag_name").and_then(|v| v.as_str()) else {
                continue;
            };
            let url = item
                .get("html_url")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("https://github.com/{repo}/releases/tag/{tag}"));
            return Ok(Release {
                tag: tag.to_string(),
                url,
                published_at: item
                    .get("published_at")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }
    }

    let tags = get_json(&format!("https://api.github.com/repos/{repo}/tags?per_page=10"))?;
    let Some(items) = tags.as_array() else {
        bail!("GitHub returned no tags for {repo}");
    };
    for item in items {
        if let Some(tag) = item.get("name").and_then(|v| v.as_str()) {
            return Ok(Release {
                tag: tag.to_string(),
                url: format!("https://github.com/{repo}/tree/{tag}"),
                published_at: None,
            });
        }
    }
    bail!("{repo} has no published releases or tags yet")
}

fn get_json(url: &str) -> Result<serde_json::Value> {
    // curl is part of a base Debian install and handles the system CA store,
    // proxies and IPv6 without pulling a full TLS stack into the binaries.
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "15",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            concat!("User-Agent: damx/", env!("CARGO_PKG_VERSION")),
            url,
        ])
        .output()
        .context("running curl (is the curl package installed?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.contains("404") {
            bail!("repository not found on GitHub (check the repository name)");
        }
        if stderr.contains("403") {
            bail!("GitHub API rate limit reached; try again later");
        }
        bail!("GitHub request failed: {stderr}");
    }

    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsing the response from {url}"))
}

/// Compares dotted versions, ignoring a leading `v` and any suffix after `-`.
/// Tags without a numeric version are treated as newer when they differ.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let current = normalise(current);
    let latest = normalise(latest);
    if current == latest {
        return false;
    }
    let parse = |v: &str| -> Option<Vec<u64>> {
        v.split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<u64>>>()
    };
    match (parse(&current), parse(&latest)) {
        (Some(a), Some(b)) => b > a,
        _ => true,
    }
}

fn normalise(version: &str) -> String {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

pub fn save_report(report: &UpdateReport) -> Result<()> {
    let path = config::update_cache_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, serde_json::to_string_pretty(report)?)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn load_report() -> Option<UpdateReport> {
    let text = fs::read_to_string(config::update_cache_path()).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn version_comparison() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("v1.2", "1.10"));
        assert!(!is_newer("1.2.0", "v1.2.0"));
        assert!(!is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("0.1.0", "release-2026"));
    }
}
