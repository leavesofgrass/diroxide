//! `cargo xtask regen [--check]`: writes every generated page, or checks
//! that each one on disk is current. After a change to the settings, the
//! keys or the catalog, run it and commit what it writes; the check is part
//! of every agent's checks from Wave 1 on.
//!
//! Today it writes `docs/keyboard.md`, the keymap presets' sheets
//! (`docs/keyboard-NAME.md`), `docs/settings-reference.md` and the plugin
//! protocol reference, `docs/dev/plugin-protocol.md`.
//! `THIRD-PARTY-NOTICES.md` joins it when the notices task arrives.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::{keyboard, plugin_doc, settings_doc};

/// Each generated page: its path from the root, and its text.
fn pages() -> Vec<(String, String)> {
    let mut pages = keyboard::all_pages();
    pages.push((settings_doc::PAGE.to_owned(), settings_doc::generate()));
    pages.push((plugin_doc::PAGE.to_owned(), plugin_doc::generate()));
    pages
}

/// Writes every page whose text changed; says which.
pub fn write(root: &Path) -> Result<bool> {
    for (page, text) in pages() {
        let path = root.join(&page);
        if fs::read_to_string(&path).is_ok_and(|on_disk| on_disk == text) {
            println!("regen: {page} is current.");
            continue;
        }
        fs::write(&path, text).with_context(|| format!("{page} could not be written"))?;
        println!("regen: wrote {page}.");
    }
    Ok(true)
}

/// Checks every page; fails when any is out of date.
pub fn check(root: &Path) -> Result<bool> {
    let mut stale = Vec::new();
    for (page, text) in pages() {
        let on_disk = fs::read_to_string(root.join(&page)).unwrap_or_default();
        if on_disk != text {
            stale.push(page);
        }
    }
    if stale.is_empty() {
        println!("regen: Pass. Every generated page is current.");
        Ok(true)
    } else {
        println!(
            "regen: Fail. Out of date: {}. Run cargo xtask regen and commit what it writes.",
            stale.join(", ")
        );
        Ok(false)
    }
}
