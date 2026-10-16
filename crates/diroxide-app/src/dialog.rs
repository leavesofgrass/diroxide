//! Dialogs: modal, named, closed by Escape, returning the focus where it
//! was. The app holds a dialog's state; frontends draw it and send back an
//! [`Answer`]. Confirmations start on No.
//!
//! The secret dialog (ADR-0009) is its own shape: its typed text is a
//! [`Secret`], which wipes itself, never shows in `Debug`, and never
//! becomes a command argument; the answer goes straight to the job that
//! asked, through the operation's secret source.
//!
//! Wave 1 (W1d) owns this module; the secret dialog's shape is the Wave 1
//! contract's.

use diroxide_core::{Command, ListModel, Message, Secret, SecretRequest};

/// An open dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// A yes-or-no question with what it will touch listed first. The
    /// answer starts on No.
    Confirm {
        /// The question, ending "y or n".
        question: Message,
        /// Every path it will touch, hidden characters spelled out.
        listing: Vec<String>,
        /// What runs on Yes.
        on_yes: Command,
    },
    /// A line of text to type.
    Prompt {
        /// The label, said first.
        label: String,
        /// The text so far.
        text: String,
        /// What runs on Enter, with the text as its `text` argument.
        on_enter: Command,
    },
    /// A list to choose from.
    Choose {
        /// The list.
        list: ListModel,
        /// What runs on Enter, with the chosen row's id as its `id`
        /// argument.
        on_enter: Command,
    },
    /// Text to read, such as help.
    Read {
        /// The title.
        title: String,
        /// The lines.
        lines: Vec<String>,
    },
    /// A password or passphrase, typed into a masked field: the terminal
    /// prompt "Password for course.zip (hidden): ", one asterisk per
    /// character (or silence, a setting), and a Show toggle that is off by
    /// default and says what it does when turned on.
    Secret {
        /// What is asked, whose and which try; its title is the dialog's
        /// name.
        request: SecretRequest,
        /// What has been typed, wiped when the dialog closes.
        typed: Secret,
        /// Whether the typed text is shown (the Show toggle).
        shown: bool,
    },
}

impl Dialog {
    /// The dialog's name, said when it opens.
    pub fn title(&self) -> String {
        match self {
            Dialog::Confirm { question, .. } => question.text.clone(),
            Dialog::Prompt { label, .. } => label.clone(),
            Dialog::Choose { list, .. } => list.title().to_owned(),
            Dialog::Read { title, .. } => title.clone(),
            Dialog::Secret { request, .. } => request.title(),
        }
    }

    /// A masked prompt for a request, nothing typed, the Show toggle off.
    pub fn secret(request: SecretRequest) -> Self {
        Dialog::Secret {
            request,
            typed: Secret::default(),
            shown: false,
        }
    }
}

/// What the user answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Yes.
    Yes,
    /// No, or Escape on a confirmation.
    No,
    /// Typed text, on Enter.
    Text(String),
    /// A chosen row's id.
    Chosen(String),
    /// A typed secret, on Enter. It goes to the job that asked, never into
    /// a command.
    Secret(Secret),
    /// Escape.
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::CommandId;

    #[test]
    fn a_secret_dialog_never_shows_what_was_typed() {
        use diroxide_core::SecretKind;
        let mut dialog = Dialog::secret(SecretRequest::new(
            SecretKind::ArchivePassword,
            "course.zip",
        ));
        if let Dialog::Secret { typed, .. } = &mut dialog {
            for c in "canary-7f3a".chars() {
                typed.push(c);
            }
        }
        assert_eq!(dialog.title(), "Password for course.zip");
        let shown = format!("{dialog:?}");
        assert!(!shown.contains("canary"), "{shown}");
        let answer = Answer::Secret(Secret::new("canary-7f3a".to_owned()));
        assert!(!format!("{answer:?}").contains("canary"));
    }

    #[test]
    fn a_dialog_is_named_by_its_question() {
        let dialog = Dialog::Confirm {
            question: Message::question("Move 2 items to the trash"),
            listing: vec!["/a/x".into(), "/a/y".into()],
            on_yes: Command::new(CommandId::from_static("file.trash")),
        };
        assert_eq!(dialog.title(), "Move 2 items to the trash? y or n");
    }
}
