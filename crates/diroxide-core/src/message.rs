//! Messages: what is said. Each message has an importance, which decides
//! whether it interrupts, waits its turn, or only shows, and a kind, which
//! decides how it looks. Neither is ever carried by color alone.
//!
//! The style rules: the meaning comes first, within 40 cells; errors read
//! "Could not VERB OBJECT: CAUSE. NEXT STEP."; questions end "y or n".

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::Explain;

/// How urgently a message is said.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    /// Shown in the status line and said only on Say Status, such as
    /// progress that changes often.
    Low,
    /// Said when the screen reader is idle: a polite live region.
    #[default]
    Normal,
    /// Said at once, interrupting: an assertive live region. For errors,
    /// questions and warnings.
    High,
}

/// What sort of message this is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    /// Information.
    #[default]
    Info,
    /// Something finished: "Copied 3 files to Documents."
    Done,
    /// Something to be careful of.
    Warning,
    /// Something failed.
    Error,
    /// A question that waits for an answer.
    Question,
    /// Progress of a job.
    Progress,
}

/// A message to say and show.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Message {
    /// The words, meaning first.
    pub text: String,
    /// How urgently it is said.
    pub importance: Importance,
    /// What sort of message it is.
    pub kind: MessageKind,
}

impl Message {
    /// A message with text, importance and kind.
    pub fn new(text: impl Into<String>, importance: Importance, kind: MessageKind) -> Self {
        Self {
            text: text.into(),
            importance,
            kind,
        }
    }

    /// Information, said politely.
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(text, Importance::Normal, MessageKind::Info)
    }

    /// A status line update, shown but not said.
    pub fn status(text: impl Into<String>) -> Self {
        Self::new(text, Importance::Low, MessageKind::Info)
    }

    /// Something finished, said politely.
    pub fn done(text: impl Into<String>) -> Self {
        Self::new(text, Importance::Normal, MessageKind::Done)
    }

    /// A warning, said at once.
    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(text, Importance::High, MessageKind::Warning)
    }

    /// An error with free text, said at once.
    pub fn error(text: impl Into<String>) -> Self {
        Self::new(text, Importance::High, MessageKind::Error)
    }

    /// Progress, shown but not said.
    pub fn progress(text: impl Into<String>) -> Self {
        Self::new(text, Importance::Low, MessageKind::Progress)
    }

    /// An error in the house style: "Could not VERB OBJECT: CAUSE. NEXT
    /// STEP." The cause is lowercase words with no final period; the next
    /// step may be empty.
    pub fn could_not(verb: &str, object: &str, cause: &str, next_step: &str) -> Self {
        let cause = cause.trim_end_matches('.');
        let mut text = if object.is_empty() {
            format!("Could not {verb}: {cause}.")
        } else {
            format!("Could not {verb} {object}: {cause}.")
        };
        let next_step = next_step.trim();
        if !next_step.is_empty() {
            text.push(' ');
            text.push_str(next_step);
            if !next_step.ends_with(['.', '?', '!']) {
                text.push('.');
            }
        }
        Self::error(text)
    }

    /// An error in the house style from any error that can explain itself.
    pub fn from_error(verb: &str, object: &str, error: &dyn Explain) -> Self {
        Self::could_not(verb, object, &error.to_string(), &error.next_step())
    }

    /// A yes-or-no question, said at once and ending "y or n". The answer
    /// defaults to No.
    pub fn question(text: &str) -> Self {
        let text = text.trim_end_matches(['?', '.']);
        Self::new(
            format!("{text}? y or n"),
            Importance::High,
            MessageKind::Question,
        )
    }

    /// The same message with another importance.
    pub fn with_importance(mut self, importance: Importance) -> Self {
        self.importance = importance;
        self
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PlaceError;
    use crate::format::within_cells;

    #[test]
    fn errors_follow_the_house_style() {
        let m = Message::could_not("copy", "report.pdf", "the disk is full.", "Free some space");
        assert_eq!(
            m.text,
            "Could not copy report.pdf: the disk is full. Free some space."
        );
        assert_eq!(m.importance, Importance::High);
        assert!(within_cells(&m.text, "Could not copy report.pdf"));
        let e = Message::from_error("open", "C:", &PlaceError::AmbiguousDrive("C:".into()));
        assert_eq!(
            e.text,
            "Could not open C:: \"C:\" has no slash after the drive letter. Write C:\\ for the top folder of the drive."
        );
        assert_eq!(
            Message::could_not("quit", "", "a job is running", "").text,
            "Could not quit: a job is running."
        );
    }

    #[test]
    fn questions_end_with_y_or_n() {
        let q = Message::question("Move 3 files to the trash?");
        assert_eq!(q.text, "Move 3 files to the trash? y or n");
        assert_eq!(q.kind, MessageKind::Question);
    }

    #[test]
    fn importance_orders_from_low_to_high() {
        assert!(Importance::Low < Importance::Normal && Importance::Normal < Importance::High);
        assert_eq!(Message::status("x").importance, Importance::Low);
        assert_eq!(
            Message::info("x")
                .with_importance(Importance::High)
                .importance,
            Importance::High
        );
    }
}
