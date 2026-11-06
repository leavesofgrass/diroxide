//! Metadata: what a file says about itself, read by readers in cost
//! classes, as fields with plain keys, labels and values in words (R28
//! part 3, R12's design). The details view, Say Details, the custom
//! columns, `diro info` and Check before sharing read through it; the
//! checksum engine and the strip into a copy live here beside them.
//!
//! - **A field** ([`Field`]) has a plain key (`image.width`,
//!   `audio.artist`, `file.sha256`), a label, a value in words, the group
//!   of the order of facts ([`DetailGroup`](diroxide_core::details::DetailGroup))
//!   and a private flag.
//! - **A cell is never blank** ([`Cell`]): "reading", "not available",
//!   "not computed", or "unreadable: REASON".
//! - **A reader** ([`Reader`]) says what it takes and its cost class
//!   ([`Cost`]): fast, background or on demand, Total Commander's three
//!   loading modes. The preview crate's image and ffprobe readers
//!   implement the trait, so this crate never depends on the preview
//!   crate.
//! - **The pipeline** ([`Pipeline`]) runs the readers that take a file
//!   in one cost class.
//!
//! The contract step of alpha 4 (P4) fixes these types with naive bodies:
//! the pipeline runs its readers inline, where the metadata lane (commit
//! 45) runs each under the guard (a thread, a catch, a deadline) with
//! every string cleaned like a file name. Nothing is cached on disk in
//! this alpha (decision 6); the cache is in memory.

pub mod field;
pub mod fuzzing;
pub mod pipeline;
pub mod reader;

pub use field::{Cell, Field};
pub use pipeline::Pipeline;
pub use reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
