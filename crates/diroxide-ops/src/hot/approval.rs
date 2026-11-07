//! The approval record (R16 section 4, rule 2; R28 6.3): a task runs only
//! after the owner has heard it in words and said yes, and again after
//! any change. The approval is keyed by a SHA-256 of the task as checked
//! ([`Task::to_shape`], so a comment or a blank line in the file changes
//! nothing) together with what its action resolves to: the recipe's
//! tool, arguments, time limit and the tool's version; the script file's
//! own hash; the plugin's id and version. A recipe edited underneath a
//! task, a script rewritten or a plugin updated turns the task off until
//! the owner approves it again ("Scans changed. Turn it on again? y or
//! n").
//!
//! The record lives in the task's state file ([`super::store`]), never
//! in the task file, which the owner writes.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::task::Task;

/// What a task's action resolves to on this computer, for the approval
/// key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// A move, a copy or a tell: the task says it all.
    Nothing,
    /// A conversion recipe.
    Recipe {
        /// The recipe's id.
        id: String,
        /// The tool's registry id: `magick`.
        tool: String,
        /// The arguments, placeholders unfilled.
        args: Vec<String>,
        /// The time limit, in seconds.
        timeout: u64,
        /// The tool's version as found, when known.
        version: Option<String>,
    },
    /// A script command.
    Script {
        /// The command's id.
        id: String,
        /// The SHA-256 of the script file, in hexadecimal.
        digest: String,
    },
    /// A plugin's command.
    Plugin {
        /// The command's id.
        id: String,
        /// The plugin's version.
        version: String,
    },
}

impl Resolved {
    /// The resolved action as lines, the same every time.
    fn canonical(&self) -> String {
        match self {
            Resolved::Nothing => "nothing\n".to_owned(),
            Resolved::Recipe {
                id,
                tool,
                args,
                timeout,
                version,
            } => {
                let mut text = format!(
                    "recipe\nid {id}\ntool {tool}\ntimeout {timeout}\nversion {}\n",
                    version.as_deref().unwrap_or("not known")
                );
                for arg in args {
                    let _ = writeln!(text, "arg {arg:?}");
                }
                text
            }
            Resolved::Script { id, digest } => format!("script\nid {id}\nsha256 {digest}\n"),
            Resolved::Plugin { id, version } => format!("plugin\nid {id}\nversion {version}\n"),
        }
    }
}

/// The approval key of a task with its action resolved: a SHA-256 in
/// hexadecimal.
pub fn approval_key(task: &Task, resolved: &Resolved) -> String {
    let mut shape = task.to_shape();
    // What it does, not how it is described.
    shape.about.clear();
    let text = toml::to_string(&shape).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"diroxide task approval 1\n");
    hasher.update(text.as_bytes());
    hasher.update(b"\n--\n");
    hasher.update(resolved.canonical().as_bytes());
    hex(&hasher.finalize())
}

/// Bytes in lowercase hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Where a task stands with the owner's approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Approval {
    /// Approved as it is now.
    Approved,
    /// Never approved.
    NotYet,
    /// Approved once, changed since.
    Changed,
}

impl Approval {
    /// Compares the key the task has now with the one approved, if any.
    pub fn of(approved: Option<&str>, key: &str) -> Self {
        match approved {
            Some(approved) if approved == key => Approval::Approved,
            Some(_) => Approval::Changed,
            None => Approval::NotYet,
        }
    }

    /// The question asked after the task's sentence, starting on No:
    /// "Turn on Scans? y or n", or "Scans changed. Turn it on again? y or
    /// n". `None` when approved.
    pub fn question(self, task: &Task) -> Option<String> {
        let title = task.title();
        match self {
            Approval::Approved => None,
            Approval::NotYet => Some(format!("Turn on {title}? y or n")),
            Approval::Changed => Some(format!("{title} changed. Turn it on again? y or n")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::refuse::Surroundings;
    use diroxide_core::place::PathStyle;

    const SCANS: &str = r#"
name = "scans"
about = "Makes a text file from each scanned PDF."
recipe = "pdf-to-text"
[trigger]
home = '/srv/scans'
match = ["*.pdf"]
"#;

    fn task(text: &str) -> Task {
        Task::from_toml(
            text,
            "tasks/scans.toml",
            &Surroundings::none(PathStyle::Unix),
        )
        .unwrap()
    }

    fn recipe(args: &[&str], version: Option<&str>) -> Resolved {
        Resolved::Recipe {
            id: "pdf-to-text".to_owned(),
            tool: "pdftotext".to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            timeout: 600,
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn the_key_follows_the_task_and_what_its_action_resolves_to() {
        let scans = task(SCANS);
        let resolved = recipe(&["{input}", "{output}"], Some("25.03.0"));
        let key = approval_key(&scans, &resolved);
        assert_eq!(key.len(), 64);
        // Comments, blank lines and the description change nothing.
        let commented = format!("# Scans, for the office.\n\n{SCANS}\n")
            .replace("Makes a text file", "Turns each scan into text");
        assert_eq!(approval_key(&task(&commented), &resolved), key);
        // A real change to the task changes the key.
        let wider = SCANS.replace("\"*.pdf\"", "\"*.pdf\", \"*.tif\"");
        assert_ne!(approval_key(&task(&wider), &resolved), key);
        // So does the recipe changing underneath it, or the tool.
        let layout = recipe(&["-layout", "{input}", "{output}"], Some("25.03.0"));
        assert_ne!(approval_key(&scans, &layout), key);
        let newer = recipe(&["{input}", "{output}"], Some("26.01.0"));
        assert_ne!(approval_key(&scans, &newer), key);
        let script = Resolved::Script {
            id: "script.ocr".into(),
            digest: "ab".into(),
        };
        let rewritten = Resolved::Script {
            id: "script.ocr".into(),
            digest: "cd".into(),
        };
        assert_ne!(
            approval_key(&scans, &script),
            approval_key(&scans, &rewritten)
        );
    }

    #[test]
    fn the_question_starts_from_what_changed() {
        let scans = task(SCANS);
        let key = approval_key(&scans, &Resolved::Nothing);
        assert_eq!(Approval::of(Some(&key), &key), Approval::Approved);
        assert_eq!(Approval::of(None, &key), Approval::NotYet);
        assert_eq!(Approval::of(Some("old"), &key), Approval::Changed);
        assert_eq!(Approval::Approved.question(&scans), None);
        // R16's lines, 21 and 39 cells.
        assert_eq!(
            Approval::NotYet.question(&scans).as_deref(),
            Some("Turn on Scans? y or n")
        );
        assert_eq!(
            Approval::Changed.question(&scans).as_deref(),
            Some("Scans changed. Turn it on again? y or n")
        );
        assert_eq!(
            Approval::Changed.question(&scans).unwrap().chars().count(),
            39
        );
    }
}
