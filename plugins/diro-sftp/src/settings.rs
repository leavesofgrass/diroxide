//! The plugin's settings, as the manifest declares them and the host
//! hands them over in `initialize` and `settings/changed` (R24, "The
//! plugin's shape"; answer 94).

use std::path::PathBuf;

use serde_json::{Map, Value};

/// The largest chunk one request may carry, in KiB, whatever the setting
/// says (R24 section 2.3).
pub const CHUNK_KIB_CAP: u32 = 128;

/// What the owner can change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The ssh program to use; `None` finds one (answer 85: the highest
    /// version wins).
    pub ssh_program: Option<PathBuf>,
    /// A configuration file passed to ssh with `-F`; `None` leaves ssh's
    /// own choice (the user's file).
    pub ssh_config_file: Option<PathBuf>,
    /// Seconds to connect, when the configuration says nothing.
    pub connect_timeout: u32,
    /// Seconds between keepalives, when the configuration says nothing.
    pub keepalive_seconds: u32,
    /// SFTP requests in flight per transfer (answer 88: 64).
    pub requests: u32,
    /// KiB per request (answer 88: 32), at most [`CHUNK_KIB_CAP`].
    pub chunk_kib: u32,
    /// Whether ssh writes a debug log to the plugin's state folder (off).
    pub ssh_debug_log: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ssh_program: None,
            ssh_config_file: None,
            connect_timeout: 20,
            keepalive_seconds: 15,
            requests: 64,
            chunk_kib: 32,
            ssh_debug_log: false,
        }
    }
}

fn path(values: &Map<String, Value>, key: &str) -> Option<PathBuf> {
    values
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "auto")
        .map(PathBuf::from)
}

fn number(values: &Map<String, Value>, key: &str, default: u32, low: u32, high: u32) -> u32 {
    values
        .get(key)
        .and_then(Value::as_u64)
        .map_or(default, |n| {
            u32::try_from(n).unwrap_or(high).clamp(low, high)
        })
}

impl Settings {
    /// The settings from the host's values; a missing or wrong value takes
    /// its default, a number out of range its nearest bound.
    pub fn from_values(values: &Map<String, Value>) -> Self {
        let default = Self::default();
        Self {
            ssh_program: path(values, "ssh_program"),
            ssh_config_file: path(values, "ssh_config_file"),
            connect_timeout: number(values, "connect_timeout", default.connect_timeout, 1, 600),
            keepalive_seconds: number(
                values,
                "keepalive_seconds",
                default.keepalive_seconds,
                1,
                3600,
            ),
            requests: number(values, "requests", default.requests, 1, 256),
            chunk_kib: number(values, "chunk_kib", default.chunk_kib, 1, CHUNK_KIB_CAP),
            ssh_debug_log: values
                .get("ssh_debug_log")
                .and_then(Value::as_bool)
                .unwrap_or(default.ssh_debug_log),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn values_are_read_with_defaults_and_bounds() {
        assert_eq!(Settings::from_values(&Map::new()), Settings::default());
        let Value::Object(values) = json!({
            "ssh_program": "auto",
            "ssh_config_file": "D:/tests/ssh_config",
            "connect_timeout": 0,
            "requests": 100000,
            "chunk_kib": 4096,
            "ssh_debug_log": true,
            "keepalive_seconds": "fifteen"
        }) else {
            unreachable!()
        };
        let settings = Settings::from_values(&values);
        assert_eq!(settings.ssh_program, None);
        assert_eq!(
            settings.ssh_config_file,
            Some(PathBuf::from("D:/tests/ssh_config"))
        );
        assert_eq!(settings.connect_timeout, 1);
        assert_eq!(settings.requests, 256);
        assert_eq!(settings.chunk_kib, CHUNK_KIB_CAP);
        assert!(settings.ssh_debug_log);
        assert_eq!(settings.keepalive_seconds, 15);
    }
}
