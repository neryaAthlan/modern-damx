//! Newline-delimited JSON over a Unix socket, used by the GUI and the CLI to
//! talk to the privileged `damxd` daemon.

use crate::driver::Status;
use crate::preset::BootPreset;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_SOCKET: &str = "/run/damxd/damx.sock";

pub fn socket_path() -> PathBuf {
    env::var_os("DAMX_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Status,
    BootGet,
    SetProfile {
        value: String,
    },
    SetFan {
        cpu: u8,
        gpu: u8,
    },
    SetLighting {
        zones: [String; 4],
        brightness: u8,
    },
    SetBacklightTimeout {
        value: bool,
    },
    SetBatteryLimiter {
        value: bool,
    },
    BootSave {
        preset: BootPreset,
    },
    BootApply,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<BootPreset>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Default::default()
        }
    }

    pub fn with_status(status: Status) -> Self {
        Self {
            ok: true,
            status: Some(status),
            ..Default::default()
        }
    }

    pub fn with_preset(preset: BootPreset) -> Self {
        Self {
            ok: true,
            preset: Some(preset),
            ..Default::default()
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Default::default()
        }
    }
}

/// Sends one request and returns the parsed response. A missing daemon shows
/// up as a connection error, which callers treat as "read-only mode".
pub fn call(request: &Request) -> Result<Response> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .with_context(|| format!("connecting to {} (is damxd running?)", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut writer = stream.try_clone()?;
    let line = serde_json::to_string(request)?;
    writeln!(writer, "{line}")?;
    writer.flush()?;

    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf)?;
    if buf.trim().is_empty() {
        bail!("damxd closed the connection without replying");
    }
    let response: Response = serde_json::from_str(buf.trim())?;
    Ok(response)
}

/// Like [`call`], but turns an `ok = false` reply into an error.
pub fn call_checked(request: &Request) -> Result<Response> {
    let response = call(request)?;
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "damxd reported an unknown error".to_string())
        );
    }
    Ok(response)
}

pub fn is_available() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}
