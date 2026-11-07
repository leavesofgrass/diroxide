//! Conversions as jobs (alpha.7, R28 part 5): the planner's refusals and
//! skips, the staging name `.diro-part-` plus eight hexadecimal digits
//! that the arrival tracker already ignores by its leading dot, keep-both
//! names, and the report with its failures first. The tools run through
//! the `ToolRunner` of `diroxide-vfs` an operation's context
//! carries, so this crate never depends on the tools registry.
//!
//! - [`recipe`]: the one recipe shape, for the built-in recipes, the
//!   owner's `conversions.toml` and a plugin's `[[converter]]`, checked
//!   when it loads.
//! - [`placeholders`]: `{input}`, `{typed_input}`, `{output}`, `{stem}`
//!   and the typed actions' names, filled per argument, never a shell.
//! - [`book`]: the recipes in use, the owner's replacing the built-in
//!   ones by id, every problem a sentence.
//! - The plan, [`Engine::plan_convert`](crate::Engine::plan_convert) in
//!   [`plan`]: refusals first, then skips, then the work; overwrite
//!   refused.
//! - The run, [`Engine::run_convert`](crate::Engine::run_convert) in
//!   [`run`]: the staging name, the rename that never replaces, cancel
//!   and progress.
//! - The staging names ([`staging`]), what the engine adds to FFmpeg's
//!   and ImageMagick's runs and reads back from them ([`tools`]), and
//!   the type registry's door for `{typed_input}` ([`typing`]).
//! - The report ([`report`]): failures first, the last 20 kept in the
//!   state folder, a file only on request with names only; the undo
//!   record names every output, so Ctrl+Z trashes what the job made.
//! - carta, the built-in tool ([`carta`]): its writers, its readers with
//!   includes left out, in process or in a `diro` child the engine can
//!   kill.
//! - The spool: a source that is not on this computer is copied into the
//!   state folder's spool first, and an output for a place that is not
//!   is made there and written through the place's committing writer.

pub mod book;
pub mod carta;
pub mod placeholders;
pub mod plan;
pub mod recipe;
pub mod report;
pub mod run;
#[cfg(test)]
mod run_tests;
pub mod staging;
#[cfg(test)]
mod testing;
pub mod tools;
pub mod typing;

pub use book::RecipeBook;
pub use carta::{CHILD_ARGUMENT, CartaRunner, serve_child};
pub use plan::{ConvertError, ConvertItem, ConvertPlan, ConvertRequest};
pub use recipe::{ConverterOutput, ConverterParts, Recipe, RecipeError, RecipeShape, RecipeSource};
pub use report::{ConvertReport, KeptReport, ReportShelf};
pub use typing::{ByExtension, FileKind, Typing};
