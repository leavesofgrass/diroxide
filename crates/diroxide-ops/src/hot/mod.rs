//! The hot-folder engine's pure parts (alpha.7, R28 part 6): the task's
//! validation and refused folders, the ledger and the log, the runner
//! loop around the arrival tracker, catch-up, the dry run and the
//! summaries. Its actions (a conversion, a script command, a plugin
//! command) come in through a trait the app and the command line
//! implement, so this crate depends on nothing but core and vfs.
//!
//! - [`task`]: a task file's keys checked into a [`Task`], every problem
//!   a sentence that names the task first; [`refuse`]: the literal path
//!   rules and the folders a task may never name; [`check`]: the tasks
//!   together, a shared `in` and loops refused.
//! - [`words`]: the task as one sentence ("Say rule"), the same in the
//!   list, the approval question and `diro watch --list`.
//! - [`approval`]: the approval key, a hash of the task and what its
//!   action resolves to; [`store`]: the task's state, beside its ledger,
//!   log and lock, under the state folder's `hot`.
//! - [`ledger`]: one line per handled file, written after the output is
//!   committed and before the input moves, so nothing is done twice;
//!   [`log`]: the outcome first and the time last, rotated at 1 MiB with
//!   five kept.

pub mod approval;
pub mod check;
pub mod ledger;
pub mod log;
pub mod refuse;
pub mod store;
pub mod task;
#[cfg(test)]
mod task_tests;
#[cfg(test)]
mod testing;
pub mod words;

pub use approval::{Approval, Resolved, approval_key};
pub use check::{Checked, MOST_TASKS, check_set, check_tasks};
pub use ledger::{Identity, Ledger, Line, Outcome};
pub use log::{TaskLog, log_line, log_time};
pub use refuse::Surroundings;
pub use store::{StoreError, TaskState, TaskStore};
pub use task::{
    After, Announce, CatchUpChoice, OnlineOnly, Task, TaskAction, TaskFolders, TaskProblem,
    TaskShape,
};
pub use words::{duration_words, sentence};
