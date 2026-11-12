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
//!    [`Choices`]: rows skipped, names typed), or [`plan_pairs`] for a
//!    caller's own old and new names, gives a [`RenamePlan`]: one
//!    [`RenameRow`] per entry in the panel's order, with its old name,
//!    new name and [`RowState`] in words at two levels, Problem and Care;
//!    [`RenamePlan::problems_first`] is the other view; and its
//!    [`RenamePlan::count_line`] ("2 problems. 46 of 50 files will
//!    change."). [`RenamePlan::make_unique`] offers numbered names for
//!    the clashes. The folders' facts come from [`read_context`], or by
//!    hand in tests.
//! 3. [`run_rename`] reads the folders again, refuses a change in words,
//!    and hands the engine only clean pairs, deepest folder first. The
//!    engine's own planner orders the steps, takes temporary names for
//!    swaps and rings, renames a case-only change directly, rolls a
//!    failed ring back, and keeps one undo record for the batch, in its
//!    journal when it has one. [`RenamePlan::request`] is the same
//!    request for an app that runs its jobs itself.
//! 4. [`editor_text`] and [`read_editor_text`]: the round trip through
//!    the owner's editor, keyed lines (`new name (was old name)`) by
//!    default and plain names as the second [`EditorFormat`], a lost or
//!    changed line refused in words.
//!
//! Nothing is ever overwritten, deleted or moved to another folder: a
//! name that is taken is a problem row, a slash in a new name is a
//! problem row, a source inside an archive is a problem row, and the
//! job's conflict policy is Skip.

mod compile;
mod editor;
mod plan;
mod rule;
mod run;
#[cfg(test)]
mod tests;
mod text;

pub use compile::{PLACEHOLDERS, RecipeError, RuleError};
pub use editor::{EditorFormat, EditorRefused, editor_text, read_editor_text};
pub use plan::{
    Blocker, Choices, FieldLookup, FolderFacts, Note, RenameContext, RenamePlan, RenameRefused,
    RenameRow, RenameSource, RowState, plan_pairs, plan_rename, plan_rename_with,
};
pub use rule::{
    CaseForm, CharacterClass, Counter, End, FindMode, MAX_COUNTER_WIDTH, Part, Pass, Position,
    Preset, PresetError, Presets, Recipe, Rule, Tidy,
};
pub use run::{RenameEnv, read_context, run_rename};
