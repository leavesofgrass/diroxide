//! The git plugin, the git half (alpha 4; R29 part 2): git's state in the
//! title line and the Git column, the status list, and git's everyday
//! commands, through the git program alone.
//!
//! The rules this crate keeps, as the git lane builds it:
//!
//! - **The trust scan first.** Before any automatic `git status`, the
//!   repository's own configuration is read without running anything; a
//!   setting that runs a program (`core.fsmonitor`, a filter driver) is
//!   "Not trusted: its config runs a program" until the owner says yes,
//!   and every automatic call carries `-c core.fsmonitor=false` (answer
//!   123).
//! - Every command that changes something lists what it will do and
//!   starts on No; there is no `git clean`, hard reset or plain force
//!   push.
//! - git runs through the host's `tools/run`, an argument list, never a
//!   shell.
//! - Every line it says carries its meaning in the first 40 cells; the
//!   host adds the plugin's name last.
//!
//! The contract step of alpha 4 (P4) made this skeleton: it describes no
//! folder and leaves every cell pending, in the protocol's shapes, and
//! passes the conformance suite.

pub mod plugin;
pub mod rpc;
