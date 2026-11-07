//! The encryption plugin (alpha 4; R29 part 3, R30 commit 71): age's
//! Encrypt with a passphrase, Encrypt for a recipient and Decrypt, in
//! process, as the owner's answer 115 places them after spike ES1
//! (docs/adr/0015-encryption.md).
//!
//! The rules this crate keeps:
//!
//! - **Every secret asked, none kept.** A passphrase is asked in the
//!   host's hidden fields (twice to encrypt, compared here), taken at
//!   once into age's wiping string, and dropped with the command; an
//!   identity file is asked by its path for each Decrypt and its keys
//!   live in memory for that command only. Nothing is written to a
//!   setting, the keyring, the log or standard error.
//! - **Nothing replaced, nothing removed.** Each output is written beside
//!   its file under an unfinished `.diro-part-` name and renamed last
//!   with a rename that never replaces; a taken name is numbered as
//!   diroxide's copies are; the original always stays (the trash offer is
//!   the app's).
//! - **A hostile file is refused before anything is typed:** a header
//!   whose passphrase asks for more work than [`header::WORK_CAP`] (spike
//!   ES2), and only age's own X25519 keys and passphrases are used, so
//!   the code under age's two advisories is never reached.
//! - **Words first.** Every line carries its meaning in the first 40
//!   cells ([`lines`]); the host adds the plugin's name last.

pub mod commands;
pub mod crypt;
pub mod error;
pub mod fuzzing;
pub mod header;
pub mod host;
pub mod lines;
pub mod names;
pub mod plugin;
pub mod recipients;
pub mod rpc;
