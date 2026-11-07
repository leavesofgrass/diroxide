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

pub mod book;
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
pub use plan::{ConvertError, ConvertItem, ConvertPlan, ConvertRequest};
pub use recipe::{ConverterOutput, ConverterParts, Recipe, RecipeError, RecipeShape, RecipeSource};
pub use report::ConvertReport;
pub use typing::{ByExtension, FileKind, Typing};
