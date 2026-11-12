//! The batch rename engine, the library half (alpha 5, from
//! `research/bulk-rename.md`): rules kept as data, a preview that is the
//! plan, one job with one undo record, and the names edited in the
//! owner's editor.
//!
//! The flow the app's dialog, `diro rename` and a script all follow:
//!
//! 1. A [`Recipe`] of [`Rule`]s, one or more [`Pass`]es, each with its
//!    [`Counter`]. Rules keep the stem and the extension apart ([`Part`]),
//!    find and replace as plain text or a pattern with groups, insert,
//!    remove and tidy by position in perceived characters, change case,
//!    and make a part from a template of [`PLACEHOLDERS`] (dates and size
//!    from the entry, `{meta:FIELD}` from the caller's [`FieldLookup`]).
//!    [`Presets`] is the `rename-presets.toml` shape.
//! 2. [`plan_rename`] (or [`plan_rename_with`], with the owner's
//!    [`Choices`]: rows skipped, names typed) gives a [`RenamePlan`]: one
//!    [`RenameRow`] per entry with its old name, new name and
//!    [`RowState`] in words, problems first, and its
//!    [`RenamePlan::count_line`] ("Rename 12 of 14: 2 conflicts"). The
//!    folders' facts come from [`read_context`], or by hand in tests.
//! 3. [`run_rename`] reads the folders again, refuses a change in words,
//!    and runs the pairs through the engine, whose planner orders the
//!    steps and takes temporary names for swaps, rings and case-only
//!    changes; the report's undo record is the whole batch.
//!    [`RenamePlan::request`] is the same request for an app that runs
//!    its jobs itself.
//! 4. [`editor_text`] and [`read_editor_text`]: the round trip through
//!    the owner's editor, a changed line count refused.
//!
//! Nothing is ever overwritten, deleted or moved to another folder: a
//! name that is taken is a problem row, a slash in a new name is a
//! problem row, and the job's conflict policy is Skip.

mod compile;
mod editor;
mod plan;
mod rule;
mod run;
#[cfg(test)]
mod tests;
mod text;

pub use compile::{PLACEHOLDERS, RecipeError, RuleError};
pub use editor::{EditorRefused, editor_text, read_editor_text};
pub use plan::{
    Blocker, Choices, FieldLookup, FolderFacts, Note, RenameContext, RenamePlan, RenameRefused,
    RenameRow, RenameSource, RowState, plan_rename, plan_rename_with,
};
pub use rule::{
    CaseForm, CharacterClass, Counter, End, FindMode, MAX_COUNTER_WIDTH, Part, Pass, Position,
    Preset, PresetError, Presets, Recipe, Rule, Tidy,
};
pub use run::{RenameEnv, read_context, run_rename};
