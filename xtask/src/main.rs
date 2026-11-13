//! The workspace's own tasks, run as `cargo xtask <task>`:
//!
//! - `deps --check`: every dependency comes from `[workspace.dependencies]`
//!   and every internal dependency points the allowed way
//!   (docs/dev/architecture.md).
//! - `regen [--check]`: writes every generated page (`docs/keyboard.md`,
//!   `docs/settings-reference.md`, `docs/dev/plugin-protocol.md`, the
//!   components list) and the icon files, or checks that each is current.
//! - `icons [--check]`: draws the application icon from the approved mark
//!   (the PNG sizes, the `.ico`, the `.icns`, the Windows `.res` and the
//!   window's own icon), or checks that each file is current.
//! - `components [--check] [--assets FOLDER] [--pin ID] [--addresses]`:
//!   builds the optional components this repository makes, writes them as
//!   a folder source under the target folder, and writes the public list
//!   the binary embeds; `--pin` pins helper files and `--addresses` checks
//!   the pinned addresses, both from the network.
//! - `keyboard [--check]`: writes `docs/keyboard.md` from the keymap's
//!   defaults, as two lists (terminal, GUI), or checks that it is
//!   current.
//! - `native --check`: the pure-Rust gate: no `cc`, `cmake` or `cxx` build
//!   dependency in the main workspace but the one allow entry (ring, for
//!   the `fetch` feature of `diro`).
//! - `bench [--quick] [--only ID] [--gui] [--cold-build] [--save-baseline]
//!   [--save-as NAME] [--gate]`: the bench, on trees it writes under the target folder
//!   (docs/dev/bench.md), with the GUI's budgets under `--gui`.
//! - `docs --check`: the documentation rules of plan section 11.
//! - `sample-folder PATH [--big-mb N]`: the owner's check folder.
//! - `search-tree PATH [--small]`: the 1 GB search tree of alpha.3.
//! - `demo PATH`: the owner's demo set for the alpha.2 check, in a new or
//!   empty folder on drive D:.
//! - `demo-tools PATH`: the owner's demo set for the alpha 4 check (the
//!   photos, the mixed types, the song, the documents, the download and
//!   its checksum lists, the git repositories, the disk images and the
//!   watched folder), with the owner's steps, in a new or empty folder on
//!   drive D:.
//! - `version --check`: before a tag, every place that carries the
//!   program's version (path dependencies, locks, the plugin workspaces,
//!   the components files, the README, the guides' sample lines, the
//!   changelog) says the version in `[workspace.package]`.
//!
//! Later waves add `notices` and `dist`.

mod bench;
mod budgets;
mod components;
mod demo;
mod demo_tools;
mod deps;
mod docs_check;
mod icons;
mod keyboard;
mod logo;
mod machine;
mod native;
mod plugin_doc;
mod regen;
mod sample;
mod settings_doc;
#[cfg(test)]
mod slow;
mod version;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

const USAGE: &str = "Usage: cargo xtask deps --check | native --check | regen [--check] | components [--check] [--assets FOLDER] [--pin ID] [--addresses] | keyboard [--check] | icons [--check] | logo | bench [--quick] [--only ID] [--gui] [--cold-build] [--save-baseline] [--save-as NAME] [--gate] | docs --check | sample-folder PATH [--big-mb N] | search-tree PATH [--small] | demo PATH | demo-tools PATH | version --check";

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
        ["icons", rest @ ..] => icons::run(&root, rest),
        ["logo"] => logo::run(&root),
        ["bench", rest @ ..] => bench::run(&root, rest),
        ["docs", "--check"] | ["docs"] => docs_check::check(&root),
        ["sample-folder", rest @ ..] => sample::sample_folder(rest),
        ["search-tree", rest @ ..] => sample::search_tree(&root, rest),
        ["demo", rest @ ..] => demo::run(&root, rest),
        ["demo-tools", rest @ ..] => demo_tools::run(&root, rest),
        ["version", "--check"] | ["version"] => version::check(&root),
        _ => {
            eprintln!("{USAGE}");
            Ok(false)
        }
    }
}
