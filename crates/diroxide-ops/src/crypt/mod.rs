//! Signatures, checksums and the encryption that runs a program, as jobs
//! (alpha.7, R29 part 3, ADR-0015): each writes under a temporary name and
//! renames last, asks for a secret through the operation's masked prompt,
//! and never keeps a key. Programs (`gpg`, and `age` where its program is
//! installed) run through the `ToolRunner` of `diroxide-vfs` that the
//! engine carries.
//!
//! age in process is not here: spike ES1 (the contract step, P4) found
//! `cargo deny` not clean on age 0.12.1's tree, so by the owner's answer
//! 115 in-process age belongs to the `diro-crypt` plugin.
//!
//! - [`gpg`]: gpg's argument lists by R29 3.3's rules and the reading of
//!   its status words into an outcome or a signature's verdict.
//! - [`status`]: the bounded reader of `[GNUPG:]` lines.
//! - [`job`]: Encrypt, Decrypt and Sign over gpg as jobs:
//!   [`Engine::plan_gpg`](crate::Engine::plan_gpg) and
//!   [`Engine::run_gpg`](crate::Engine::run_gpg), with the options in
//!   [`GpgOptions`], never in `JobRequest`.
//! - [`minisign`]: minisign signatures checked in process by
//!   `minisign-verify`, the data streamed.
//! - [`verify`]: one [`verify_signature`] over gpg and minisign giving a
//!   [`SignatureVerdict`], the sidecars to look for
//!   ([`signature_sidecars`]), and the chain's sentences over a verdict
//!   and a checksum list's counts ([`chain_lines`]).
//! - [`lines`]: every word said, meaning first.

pub mod gpg;
pub mod job;
#[cfg(test)]
mod job_tests;
pub mod lines;
pub mod minisign;
pub mod status;
pub mod verify;

pub use gpg::{Outcome, Trust, Verified};
pub use job::{GpgAction, GpgError, GpgItem, GpgOptions, GpgPlan, GpgReport, GpgRequest};
pub use minisign::MinisignKey;
pub use verify::{
    Sidecar, SignatureCheck, SignatureVerdict, Signer, chain_lines, sidecars_present,
    signature_sidecars, verify_signature,
};
