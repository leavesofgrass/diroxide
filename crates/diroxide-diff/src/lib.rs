//! Compare two files and say the result in words (research/diffing.md,
//! R18; plan section 5.15).
//!
//! A compare is read in one fixed order, in the terminal, the window and
//! `diro diff` alike: the verdict and the counts, the direction, what was
//! ignored, the facts about the files as wholes, then one row per change,
//! each with its meaning in its first 40 cells. Words come before signs;
//! side by side is never the primary view.
//!
//! The crate is pure computation over text and bytes. Its line engine is
//! the `similar` crate (patience by default), behind the `engine` module,
//! the one file that names it, so `imara-diff`, the documented swap, can
//! replace it in one file if the bench gate asks.
//!
//! Every run has a deadline (two seconds by default) and honors a cancel
//! token. A run that reaches its deadline still returns a correct list of
//! changes, perhaps longer than the least possible, and says so; a
//! cancelled run is an error, never a partial result that looks whole.
//!
//! The two sides are "before" and "after", never left and right: the app
//! reads changes from the other panel's file (before) to this panel's file
//! (after), and Swap turns the direction around ([`TextComparison::swapped`]).

pub mod change;
pub mod content;
mod engine;
pub mod equal;
pub mod error;
pub mod explain;
pub mod facts;
mod forms;
pub mod grammar;
mod lines;
pub mod options;
pub mod text;
pub mod words;

pub use change::{Change, ChangeKind, Counts};
pub use content::{ContentComparison, compare_contents};
pub use equal::{Equality, equal};
pub use error::DiffError;
pub use explain::{Delta, Reason, explain};
pub use facts::{Endings, FactDifference, FileFacts, Sides};
pub use grammar::{Context, UnifiedKind, UnifiedRow, Verdict, verdict_line, verdict_sentence};
pub use lines::LineEnding;
pub use options::{CompareOptions, Ignore, LineAlgorithm};
pub use text::{TextComparison, compare_text};
pub use words::{ChangeWindow, WordDiff, WordSegment, change_window, word_diff};
