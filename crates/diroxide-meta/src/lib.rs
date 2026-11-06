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
//!   "not computed", "unreadable: REASON" or "unavailable: plugin
//!   stopped".
//! - **A reader** ([`Reader`]) says what it takes and its cost class
//!   ([`Cost`]): fast, background or on demand, Total Commander's three
//!   loading modes. The preview crate's image and ffprobe readers
//!   implement the trait, so this crate never depends on the preview
//!   crate.
//! - **The pipeline** ([`Pipeline`]) runs the readers that take a file
//!   in one cost class, each under the guard ([`guard`]: a thread, a
//!   catch, a deadline and a byte limit), and cleans every string like a
//!   file name ([`clean`]).
//!
//! Nothing is cached on disk in this alpha (decision 6); a cache, when
//! the columns keep one, is in memory.

pub mod checksum;
pub mod clean;
pub mod exif;
pub mod field;
pub mod fuzzing;
pub mod guard;
pub mod image;
pub mod origin;
pub mod pipeline;
pub mod privacy;
pub mod reader;
pub mod readers;
pub mod strip;
pub mod words;

pub use field::{Cell, Field};
pub use guard::{Bounded, Guard};
pub use pipeline::{Outcome, Pipeline};
pub use reader::{Cost, MetaError, ReadLimits, ReadRequest, Reader};
