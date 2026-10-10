//! `diro [PATH] [OTHER]`: open the panels.
//!
//! Phase 0: prints one screen of the terminal frontend and returns. Wave 1
//! (W1d) makes it interactive.

use std::env;
use std::io::Write;

use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::{Message, Place};
use diroxide_tui::{Tui, TuiOptions};

use crate::CliError;

/// The current folder as a place.
pub fn current_place() -> Result<Place, CliError> {
    let folder = env::current_dir()
        .map_err(|e| CliError::Failed(format!("the current folder could not be read: {e}")))?;
    let text = folder
        .into_os_string()
        .into_string()
        .map_err(|raw| CliError::NotUnicode(raw.to_string_lossy().into_owned()))?;
    Place::parse(&text).map_err(|e| CliError::Failed(Message::from_error("open", &text, &e).text))
}

/// Opens the panels at up to two places, typed relative to the current
/// folder; the right panel shows the left one's place when only one is
/// given.
pub fn run(places: &[String], out: &mut dyn Write) -> Result<i32, CliError> {
    let here = current_place()?;
    let resolve = |text: &str| {
        Place::resolve(text, &here)
            .map_err(|e| CliError::Failed(Message::from_error("open", text, &e).text))
    };
    let left = match places.first() {
        Some(text) => resolve(text)?,
        None => here.clone(),
    };
    let right = match places.get(1) {
        Some(text) => resolve(text)?,
        None => left.clone(),
    };
    let mut app = App::new(AppOptions::new(left, right));
    let mut runtime = Runtime::with_local();
    Tui::new(TuiOptions::default())
        .run(&mut app, &mut runtime, out)
        .map_err(|e| CliError::Failed(e.to_string()))?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_this_crate_prints_its_folder() {
        let mut out = Vec::new();
        let code = run(&[env!("CARGO_MANIFEST_DIR").to_owned()], &mut out).unwrap();
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("diroxide-cli, in "), "{text}");
        assert!(text.contains("src, folder"));
    }
}
