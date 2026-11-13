//! Batch rename's types in the app (alpha 5, lane RA): the rules list,
//! the preview, apply and the editor door, over the engine half in
//! `diroxide_ops::rename_rules`.
//!
//! Reserved by the pre-wiring step (lane P5): [`RenameAsk`] travels as
//! [`Request::RenameBatch`](crate::Request::RenameBatch) to the runtime,
//! which answers in `runtime/renaming.rs`, and [`RenameNote`] comes back
//! as [`AppEvent::Renaming`](crate::AppEvent::Renaming), handled in
//! `app/renaming.rs`.
//!
//! Each ask names its batch by a number, which is the job number the
//! batch took when it opened; a note for a batch whose dialogs are
//! closed is dropped. The editor buffer is `rename-NUMBER.txt` in the
//! state folder and the presets are `rename-presets.toml` in the config
//! folder; the last rules used are `rename-last.toml` in the state
//! folder. Every file is written whole, through a temporary file.

use diroxide_core::Place;
use diroxide_ops::rename_rules::{FolderFacts, Presets, Recipe, RenameSource};

/// The presets file's name, in the config folder.
pub const PRESETS_FILE: &str = "rename-presets.toml";

/// The last rules used, in the state folder.
pub const LAST_FILE: &str = "rename-last.toml";

/// The editor buffer's name for a batch: `rename-12.txt`.
pub fn buffer_name(batch: u64) -> String {
    format!("rename-{batch}.txt")
}

/// What the app asks the runtime for a batch rename: reading the
/// folders' facts, the editor buffer, the presets.
#[derive(Clone, Debug, PartialEq)]
pub enum RenameAsk {
    /// The facts of every source's folder, the presets and the last rules
    /// used, as the dialog opens.
    Open {
        /// The batch.
        batch: u64,
        /// The entries to rename.
        sources: Vec<RenameSource>,
    },
    /// The sources and their folders read again, at the owner's yes, so
    /// the plan is made again against the folders as they are.
    Recheck {
        /// The batch.
        batch: u64,
        /// The entries as the preview had them.
        sources: Vec<RenameSource>,
    },
    /// The editor buffer written, before the editor opens on it.
    WriteBuffer {
        /// The batch, which names the file.
        batch: u64,
        /// The text.
        text: String,
    },
    /// The editor buffer read back, once the owner is done editing.
    ReadBuffer {
        /// The batch.
        batch: u64,
        /// The buffer.
        place: Place,
    },
    /// The editor buffer removed: its names are read, or the batch closed.
    DropBuffer {
        /// The buffer.
        place: Place,
    },
    /// The presets file written whole.
    SavePresets(Presets),
    /// The rules of the batch that ran, kept for the next one.
    SaveLast(Recipe),
}

/// What the runtime answers a [`RenameAsk`].
#[derive(Debug)]
pub enum RenameNote {
    /// The facts, the presets and the last rules used.
    Opened {
        /// The batch.
        batch: u64,
        /// Each folder's facts.
        facts: Vec<(Place, FolderFacts)>,
        /// The presets, or why the file could not be read.
        presets: Result<Presets, String>,
        /// The last rules used, when there are any.
        last: Option<Recipe>,
    },
    /// The sources and folders as they are now.
    Rechecked {
        /// The batch.
        batch: u64,
        /// The sources read again; one that is gone is marked so.
        sources: Vec<RenameSource>,
        /// Each folder's facts.
        facts: Vec<(Place, FolderFacts)>,
    },
    /// The editor buffer is written, or why not.
    BufferWritten {
        /// The batch.
        batch: u64,
        /// The buffer, or the cause in words.
        result: Result<Place, String>,
    },
    /// The editor buffer's text, or why it could not be read.
    BufferRead {
        /// The batch.
        batch: u64,
        /// The text, or the cause in words.
        result: Result<String, String>,
    },
    /// The presets file could not be written: the cause in words.
    PresetsNotSaved(String),
}
