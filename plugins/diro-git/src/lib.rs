//! The git plugin, the git half (alpha 4; R29 part 2): git's state in the
//! title line and the Git column, the status list, and git's everyday
//! commands, through the git program alone.
//!
//! The rules this crate keeps:
//!
//! - **The trust scan first.** Before any automatic `git status`, the
//!   repository's own configuration is read without running anything
//!   ([`git::trust`]); a setting that runs a program (`core.fsmonitor`, a
//!   filter driver, an alias with `!`) is "Not trusted: its config runs a
//!   program" until the owner says yes once, and every automatic call
//!   carries `-c core.fsmonitor=false` and `--no-optional-locks` (answer
//!   123).
//! - **git through the host.** Every call is an argument list through the
//!   host's `tools/run` ([`git::run`]), never a shell, with
//!   `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`, `core.quotepath=false` and
//!   literal pathspecs after `--`.
//! - **Nothing is lost by a command.** A command that can lose work lists
//!   what it touches and starts on No; Discard keeps what it drops in a
//!   stash; there is no `git clean`, hard reset or plain force push, and
//!   the lease push waits behind two questions that name the branch.
//! - **Words first.** Every line it says carries its meaning in the first
//!   40 cells ([`lines`]); the host adds the plugin's name last.

pub mod error;
pub mod folder;
pub mod git;
pub mod host;
pub mod lines;
pub mod plugin;
pub mod repo;
pub mod rpc;
