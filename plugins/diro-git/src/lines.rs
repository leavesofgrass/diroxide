//! Every line the plugin says or hands the host, in one place, so each is
//! tested at 40 cells with its meaning first (the owner reads one line of
//! a 40-cell Braille display at a time). Words before symbols; the host
//! adds the plugin's name, "(Git)", to every line it says, so no line here
//! names the plugin. A cause is lowercase words for the host's "Could not
//! VERB OBJECT: CAUSE." line; a next step is a sentence of its own.

use crate::git::version::{MINIMUM, Version};

// ---- git itself ----

/// The title line, and a command's cause, when the host finds no git.
pub const GIT_NOT_FOUND: &str = "git not found";

/// The next step when git is missing or too old.
pub const GET_GIT: &str = "Install git from git-scm.com or your package manager, then try again.";

/// "git is too old: 2.20, needs 2.30" (32).
pub fn too_old(version: Version) -> String {
    format!("git is too old: {version}, needs {MINIMUM}")
}

/// A command's cause outside any repository.
pub const NOT_A_REPOSITORY: &str = "this folder is not in a git repository";

// ---- the trust scan (answer 123) ----

/// The title line of a repository whose configuration runs a program and
/// that the owner has not trusted (38).
pub const NOT_TRUSTED: &str = "Not trusted: its config runs a program";

/// The next step after a no.
pub const TRUST_HINT: &str = "Use Git: trust this repository, when you know where it came from.";

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;

    #[test]
    fn git_s_own_lines_fit_40_cells_with_their_meaning_first() {
        for line in [GIT_NOT_FOUND, NOT_A_REPOSITORY, NOT_TRUSTED] {
            assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
        }
        assert_eq!(NOT_TRUSTED.chars().count(), 38);
        let line = too_old(Version {
            major: 2,
            minor: 20,
            patch: 1,
        });
        assert_eq!(line, "git is too old: 2.20, needs 2.30");
        assert_eq!(line.chars().count(), 32);
    }
}
