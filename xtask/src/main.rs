//! The workspace's own tasks, run as `cargo xtask <task>`:
//!
//! - `deps --check`: every dependency comes from `[workspace.dependencies]`
//!   and every internal dependency points the allowed way
//!   (docs/dev/architecture.md).
//! - `regen [--check]`: writes every generated page (`docs/keyboard.md`,
//!   `docs/settings-reference.md`), or checks that each is current.
//! - `keyboard [--check]`: writes `docs/keyboard.md` from the keymap's
//!   defaults, as two lists (terminal, window), or checks that it is
//!   current.
//! - `native --check`: the pure-Rust gate: no `cc`, `cmake` or `cxx` build
//!   dependency in the main workspace but the one allow entry (ring, for
//!   the `fetch` feature of `diro`).
//!
//! Later waves add `notices`, `icons`, `dist` and `bench`.

mod deps;
mod keyboard;
mod logo;
mod native;
mod regen;
mod settings_doc;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

const USAGE: &str = "Usage: cargo xtask deps --check | native --check | regen [--check] | keyboard [--check] | logo";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Could not run the task: {error:#}.");
            ExitCode::from(2)
        }
    }
}

/// The workspace's root folder: the parent of this crate's folder.
fn root() -> Result<PathBuf> {
    match Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        Some(root) => Ok(root.to_path_buf()),
        None => bail!("the workspace root could not be found"),
    }
}

fn run() -> Result<bool> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let root = root()?;
    match args.as_slice() {
        ["deps", "--check"] | ["deps"] => deps::check(&root),
        ["native", "--check"] | ["native"] => native::check(&root),
        ["regen"] => regen::write(&root),
        ["regen", "--check"] => regen::check(&root),
        ["keyboard"] => keyboard::write(&root),
        ["keyboard", "--check"] => keyboard::check(&root),
        ["logo"] => logo::run(&root),
        _ => {
            eprintln!("{USAGE}");
            Ok(false)
        }
    }
}
