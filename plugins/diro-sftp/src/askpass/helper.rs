//! The helper's side: what the program does when ssh starts it.
//!
//! Its output is for ssh alone: a secret and a line break on standard
//! output, or a yes or no for a host key question; on a refusal, a
//! cancel or any failure it writes nothing and exits non-zero, which ssh
//! takes as "no answer". It writes nothing to standard error, which ssh
//! may show.

/// The exit code for "no answer": ssh stops asking.
pub const NO_ANSWER: i32 = 1;

/// Runs the helper with the prompt ssh gave as its one argument and
/// returns the exit code. This build cannot reach the plugin yet (the
/// endpoint and its proof are the next commit's), so it answers nothing.
pub fn run(prompt: Option<&str>) -> i32 {
    let _ = prompt;
    NO_ANSWER
}
