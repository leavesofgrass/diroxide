//! Signatures, checksums and the encryption that runs a program, as jobs
//! (alpha.7, R29 part 3): each writes under a temporary name and renames
//! last, asks for a secret through the operation's masked prompt, and
//! never keeps a key. Programs (`gpg`, `minisign`, and `age` where its
//! program is installed) run through the `ToolRunner` of `diroxide-vfs`
//! that an operation's context carries.
//!
//! age in process is not here: spike ES1 (the contract step, P4) found
//! `cargo deny` not clean on age 0.12.1's tree, so by the owner's answer
//! 115 in-process age belongs to the `diro-crypt` plugin.
//!
//! Declared empty by the contract step so the lane that builds these jobs
//! fills it without editing this crate's `lib.rs`.
