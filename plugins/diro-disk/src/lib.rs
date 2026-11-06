//! The disk-image places plugin, reading half (alpha 4; R29 part 1):
//! places such as `fedora.iso!EFI/BOOT` and `two-parts.img!Partition 1/`,
//! which the host sends here by the claim route because the manifest's
//! `[[provider]] extensions` claim them. Read only: it lists, details and
//! reads, and never writes, trashes or watches.
//!
//! The rules this crate keeps, as the disk lane builds it:
//!
//! - Every name goes through the container rules of
//!   `diroxide_vfs::archive::names`, and every listing, member and total
//!   through `diroxide_vfs::archive::limits`; a sparse image never makes
//!   the host allocate its claimed size.
//! - A link in an image is a row with its target as text, never followed.
//! - An image inside an archive or on SFTP is read through the host's
//!   ranged `fs/read`, a block at a time.
//! - Every line it says carries its meaning in the first 40 cells; the
//!   host adds the plugin's name last.
//!
//! The contract step of alpha 4 (P4) made this skeleton: the provider
//! answers with nothing listed yet, in the protocol's shapes, and passes
//! the conformance suite.

pub mod plugin;
pub mod rpc;
