//! The core model of diroxide.
//!
//! Every other crate speaks in these types: where things are ([`Place`]),
//! what is there ([`Entry`]), how a list reads aloud ([`ListModel`]), what
//! the user asked for ([`Command`]), what a frontend must do about it
//! ([`Effect`]), what is said ([`Message`]) and what time it is ([`Clock`]).
//!
//! This crate does no I/O. It is finished in Phase 0 and changes only through
//! a contract change request (docs/adr/0002-the-core-model.md).

pub mod catalog;
pub mod clock;
pub mod command;
pub mod details;
pub mod effect;
pub mod entry;
pub mod error;
pub mod filter;
pub mod format;
pub mod ids;
pub mod job;
pub mod keys;
pub mod list;
pub mod message;
pub mod names;
pub mod pattern;
pub mod place;
pub mod secret;
pub mod sort;

pub use clock::{CivilTime, Clock, FixedClock, SystemClock, Timestamp, UtcOffset, Weekday};
pub use command::{ArgValue, Command, CommandId};
pub use details::{DetailGroup, DetailRow};
pub use effect::{Effect, PluginCall, Redraw};
pub use entry::{Attributes, Entry, EntryKind, Link};
pub use error::{CoreError, Explain, PlaceError};
pub use ids::{DialogId, IdSource, JobId, RequestId, Side, TabId};
pub use job::{
    ConflictAction, ConflictAnswer, ConflictKind, ConflictPolicy, ErrorAction, JobKind, JobOrigin,
    JobRequest, JobState,
};
pub use keys::{Key, KeyCode, KeyColumn, Layer, Modifiers};
pub use list::{FrozenSelection, ListModel, ListRow, Move, RowsReplaced};
pub use message::{Importance, Message, MessageKind};
pub use pattern::{Pattern, PatternError};
pub use place::{Place, PlaceKind, Remote};
pub use secret::{Secret, SecretKind, SecretRequest};
pub use sort::{FolderOrder, NameCase, SortDirection, SortKey, SortSpec};
