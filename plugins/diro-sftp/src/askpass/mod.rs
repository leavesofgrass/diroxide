//! The askpass helper: the plugin's own program in its second mode
//! (answer 82). ssh starts it as `SSH_ASKPASS` with the prompt as its one
//! argument whenever it needs a password, a passphrase or a yes for a new
//! host key; `SSH_ASKPASS` can hold only a path, never arguments, so the
//! mode is chosen by the environment: when [`ENDPOINT_VARIABLE`] is set
//! the program is the helper, otherwise the plugin.
//!
//! The helper reaches the plugin that started its ssh through a local
//! endpoint and proves itself with the connection's token; the plugin asks
//! the owner through the app's own dialog and hands the answer back, which
//! the helper writes for ssh alone.

use std::ffi::OsStr;

pub mod classify;
pub mod fingerprint;
pub mod helper;
pub mod listener;
pub mod proof;
pub mod wire;

pub use listener::{Handler, Listener, Prompt, Reply};

/// The environment variable that names the plugin's endpoint for this
/// connection; its presence makes the program the helper.
pub const ENDPOINT_VARIABLE: &str = "DIRO_ASKPASS_ENDPOINT";

/// The environment variable that holds the connection's token, which the
/// helper proves it knows and never sends.
pub const TOKEN_VARIABLE: &str = "DIRO_ASKPASS_TOKEN";

/// Which of its two modes the program runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The JSON-RPC plugin the host starts.
    Plugin,
    /// The askpass helper ssh starts.
    Helper,
}

/// The mode for the value of [`ENDPOINT_VARIABLE`]: the helper when it is
/// set and not empty.
pub fn mode(endpoint: Option<&OsStr>) -> Mode {
    match endpoint {
        Some(value) if !value.is_empty() => Mode::Helper,
        _ => Mode::Plugin,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn the_endpoint_variable_chooses_the_mode() {
        assert_eq!(mode(None), Mode::Plugin);
        assert_eq!(mode(Some(&OsString::new())), Mode::Plugin);
        assert_eq!(
            mode(Some(&OsString::from("tcp:127.0.0.1:50000"))),
            Mode::Helper
        );
    }
}
