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

pub mod book;
pub mod placeholders;
pub mod recipe;

pub use book::RecipeBook;
pub use recipe::{ConverterOutput, ConverterParts, Recipe, RecipeError, RecipeShape, RecipeSource};
