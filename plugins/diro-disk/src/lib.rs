//! The disk-image places plugin, reading half (alpha 4; R29 part 1):
//! places such as `fedora.iso!EFI/BOOT` and `two-parts.img!partition-1/`,
//! which the host sends here by the claim route because the manifest's
//! `[[provider]] extensions` claim them. Read only: it lists, details and
//! reads, and never writes, trashes or watches.
//!
//! The rules this crate keeps:
//!
//! - **The host's ranged `fs/read` and the spool are the only ways to an
//!   image's bytes** ([`host::HostFile`], [`image::source`]), a 64 KiB
//!   block at a time through a cache of sixteen, so a listing reads
//!   kilobytes, not the image.
//! - **Every name** goes through the container rules of
//!   `diroxide_vfs::archive::names`, and every listing through
//!   `diroxide_vfs::archive::limits`, per folder ([`image::tree`]); a
//!   sparse image never makes anything allocate its claimed size.
//! - **A link in an image is a row with its target as text,** never
//!   followed; a folder that leads back to one it is inside is cut, and
//!   no path goes more than 64 folders deep.
//! - **Every line it says** carries its meaning in the first 40 cells
//!   ([`lines`]); the host adds the plugin's name last.

pub mod error;
pub mod fuzzing;
pub mod host;
pub mod image;
pub mod lines;
pub mod plugin;
pub mod provider;
pub mod rpc;
pub mod spool;
pub mod tool;
