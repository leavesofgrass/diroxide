//! The core model of diroxide.
//!
//! Every other crate speaks in these types: where things are ([`Place`]),
//! what is there ([`Entry`]), how a list reads aloud ([`ListModel`]), what
//! the user asked for ([`Command`]), what a frontend must do about it
//! ([`Effect`]), what is said ([`Message`]) and what time it is ([`Clock`]).
//!
//! This crate does no I/O. It is finished in Phase 0 and changes only through
//! a contract change request (docs/adr/0002-the-core-model.md).

pub mod clock;
pub mod command;
pub mod effect;
pub mod entry;
pub mod error;
pub mod format;
pub mod ids;
pub mod job;
pub mod keys;
pub mod list;
pub mod message;
pub mod names;
pub mod place;

pub use clock::{CivilTime, Clock, FixedClock, SystemClock, Timestamp, UtcOffset, Weekday};
pub use command::{ArgValue, Command, CommandId};
pub use effect::{Effect, PluginCall, Redraw};
pub use entry::{Attributes, Entry, EntryKind, Link};
pub use error::{CoreError, Explain, PlaceError};
pub use ids::{DialogId, IdSource, JobId, RequestId, Side, TabId};
pub use job::{ConflictPolicy, JobKind, JobRequest};
pub use keys::{Key, KeyChord, KeyCode, Layer, Modifiers};
pub use list::{ListModel, ListRow, Move};
pub use message::{Importance, Message, MessageKind};
pub use place::{Place, PlaceKind, Remote};
