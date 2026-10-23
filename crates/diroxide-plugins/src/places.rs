//! Places from a plugin are parsed, never trusted (R22, section 7.2):
//! every place string goes through the core's parser, which refuses a bare
//! drive letter such as `C:` as ambiguous and a device path, and resolves
//! `..`; before it, this module refuses what the core would accept but a
//! plugin must not name: a verbatim `\\?\` path, `GLOBALROOT`, and a NUL.

use diroxide_core::{Place, PlaceError};

use crate::protocol::{ErrorKind, RpcError};

/// Why a place from a plugin was refused, in words.
fn refusal(text: &str) -> Option<&'static str> {
    let unified = text.replace('/', "\\");
    if text.contains('\0') {
        return Some("a place cannot hold a NUL character");
    }
    if unified.starts_with("\\\\?\\") || unified.starts_with("\\??\\") {
        return Some("a verbatim path is not accepted from a plugin");
    }
    if unified.to_ascii_uppercase().contains("GLOBALROOT") {
        return Some("a device path is not accepted from a plugin");
    }
    None
}

/// Reads a place a plugin named; the error is -32602 with the reason.
pub fn plugin_place(text: &str) -> Result<Place, RpcError> {
    if let Some(reason) = refusal(text) {
        return Err(RpcError::new(ErrorKind::InvalidParams, reason));
    }
    Place::parse(text).map_err(|error| place_error(text, &error))
}

/// Reads a list of places a plugin named; the first refusal answers.
pub fn plugin_places(values: &[serde_json::Value]) -> Result<Vec<Place>, RpcError> {
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| RpcError::new(ErrorKind::InvalidParams, "a place must be a text"))
                .and_then(plugin_place)
        })
        .collect()
}

fn place_error(text: &str, error: &PlaceError) -> RpcError {
    let mut rpc = RpcError::new(
        ErrorKind::InvalidParams,
        format!("the place cannot be used: {error}"),
    );
    if let Some(data) = rpc.data.as_mut() {
        data.place = Some(text.chars().take(300).collect());
    }
    rpc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_places_are_refused() {
        for text in [
            "",
            "C:",
            "C:relative",
            "\\\\?\\C:\\Windows",
            "//?/C:/Windows",
            "\\\\.\\PhysicalDrive0",
            "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1",
            "relative/path",
            "/home/ada\0/x",
        ] {
            let error = plugin_place(text).unwrap_err();
            assert_eq!(error.code, -32602, "{text:?}");
        }
    }

    #[test]
    fn dot_dot_is_resolved_and_shown_in_full() {
        let text = if cfg!(windows) {
            "D:\\work\\..\\photos"
        } else {
            "/work/../photos"
        };
        let place = plugin_place(text).unwrap();
        let expected = if cfg!(windows) {
            "D:\\photos"
        } else {
            "/photos"
        };
        assert_eq!(place.to_string(), expected);
        assert!(plugin_place("sftp://example.org/home").is_ok());
    }
}
