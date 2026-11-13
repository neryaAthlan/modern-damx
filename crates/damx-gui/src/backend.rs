//! Small blocking helpers used from background threads by the UI.

use damx_core::config;
use damx_core::driver::{self, Driver, Status};
use damx_core::ipc::{self, Request, Response};
use damx_core::preset::BootPreset;
use damx_core::update::{self, UpdateReport};

pub struct Snapshot {
    pub status: Status,
    pub daemon: bool,
}

/// Reads the driver state through `damxd` when it is running, otherwise
/// directly from sysfs (read-only mode).
pub fn snapshot() -> Snapshot {
    match ipc::call(&Request::Status) {
        Ok(response) if response.ok => Snapshot {
            status: response.status.unwrap_or_else(direct_status),
            daemon: true,
        },
        _ => Snapshot {
            status: direct_status(),
            daemon: false,
        },
    }
}

pub fn direct_status() -> Status {
    match Driver::discover() {
        Some(driver) => driver.status(),
        None => Status {
            driver_loaded: false,
            kernel: driver::kernel_release(),
            model: std::fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
                .ok()
                .map(|s| s.trim().to_string()),
            ..Default::default()
        },
    }
}

pub fn request(request: Request) -> anyhow::Result<Response> {
    ipc::call_checked(&request)
}

pub fn boot_get() -> BootPreset {
    match ipc::call(&Request::BootGet) {
        Ok(response) => response.preset.unwrap_or_default(),
        Err(_) => BootPreset::load(),
    }
}

pub fn check_updates() -> UpdateReport {
    config::ensure_config_file();
    let config = config::load();
    let report = update::check_all(&config, env!("CARGO_PKG_VERSION"));
    let _ = update::save_report(&report);
    report
}
