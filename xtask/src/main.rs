//! The workspace's own tasks, run as `cargo xtask <task>`:
//!
//! - `deps --check`: every dependency comes from `[workspace.dependencies]`
//!   and every internal dependency points the allowed way
//!   (docs/dev/architecture.md).
//! - `regen [--check]`: writes every generated page (`docs/keyboard.md`,
//!   `docs/settings-reference.md`, `docs/dev/plugin-protocol.md`, the
//!   components list), or checks that each is current.
//! - `components [--check]`: builds the optional components this
//!   repository makes, writes them as a folder source under the target
//!   folder, and writes the public list the binary embeds.
//! - `keyboard [--check]`: writes `docs/keyboard.md` from the keymap's
//!   defaults, as two lists (terminal, GUI), or checks that it is
//!   current.
//! - `native --check`: the pure-Rust gate: no `cc`, `cmake` or `cxx` build
//!   dependency in the main workspace but the one allow entry (ring, for
//!   the `fetch` feature of `diro`).
//! - `bench [--quick] [--only ID] [--save-baseline] [--gate]`: the bench,
//!   on trees it writes under the target folder (docs/dev/bench.md).
//! - `docs --check`: the documentation rules of plan section 11.
//! - `sample-folder PATH [--big-mb N]`: the owner's check folder.
//! - `search-tree PATH [--small]`: the 1 GB search tree of alpha.3.
//! - `demo PATH`: the owner's demo set for the alpha.2 check, in a new or
//!   empty folder on drive D:.
//!
//! Later waves add `notices`, `icons` and `dist`.

mod bench;
mod components;
mod demo;
mod deps;
mod docs_check;
mod keyboard;
mod logo;
mod native;
mod plugin_doc;
mod regen;
mod sample;
mod settings_doc;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

const USAGE: &str = "Usage: cargo xtask deps --check | native --check | regen [--check] | components [--check] | keyboard [--check] | logo | bench [--quick] [--only ID] [--save-baseline] [--gate] | docs --check | sample-folder PATH [--big-mb N] | search-tree PATH [--small] | demo PATH";

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
        ["components", rest @ ..] => components::run(&root, rest),
        ["keyboard"] => keyboard::write(&root),
        ["keyboard", "--check"] => keyboard::check(&root),
        ["logo"] => logo::run(&root),
        ["bench", rest @ ..] => bench::run(&root, rest),
        ["docs", "--check"] | ["docs"] => docs_check::check(&root),
        ["sample-folder", rest @ ..] => sample::sample_folder(rest),
        ["search-tree", rest @ ..] => sample::search_tree(&root, rest),
        ["demo", rest @ ..] => demo::run(&root, rest),
        _ => {
            eprintln!("{USAGE}");
            Ok(false)
        }
    }
}
