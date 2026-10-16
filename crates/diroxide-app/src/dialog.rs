//! Dialogs: modal, named, closed by Escape, returning the focus where it
//! was. The app holds a dialog's state; frontends draw it and send back an
//! [`Answer`], or keys the app handles itself.
//!
//! The rules (plan section 5.12, R23, the owner's answer 75):
//!
//! - Every confirmation starts on No, the safe choice is focused when the
//!   dialog is made, and the question says the default ("No is the
//!   default").
//! - Confirming keys (y, Enter, Space) that arrive within the settle time
//!   of the dialog opening (`dialog_settle_ms`, 250 by default) are
//!   ignored, so the key that opened it, held down, cannot answer it.
//! - A delete, move or overwrite lists every path it will touch, with
//!   hidden characters spelled out as `[U+XXXX]` and the count in words,
//!   before the question.
//! - A question about one item (a conflict, an error) is lines to read
//!   first, then verb-first choices with the focus on the one that changes
//!   nothing; Escape answers it.
//!
//! The secret dialog (ADR-0009) is its own shape: its typed text is a
//! [`Secret`], which wipes itself, never shows in `Debug`, and never
//! becomes a command argument; the answer goes straight to the job that
//! asked, through the operation's secret source.
//!
//! Wave 1 (W1d) owns this module; the secret dialog's shape is the Wave 1
//! contract's.

use diroxide_core::{Command, ListModel, Message, Secret, SecretRequest};

use crate::entry::EntryField;

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
    /// A line of text to type (the Phase 0 shape; the app's prompts use
    /// [`Dialog::Edit`]).
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
    /// A question about one item: lines to read first (the situation
    /// first), then the choices, each starting with its verb, the focus on
    /// the one that changes nothing. Escape answers that one.
    Question {
        /// The lines, the situation first: "Name taken: report.pdf".
        lines: Vec<String>,
        /// The choices.
        choices: ListModel,
    },
    /// A text field (the entry model): the label said first, the text, its
    /// cursor, and, while open, its candidate list. The palette, go to a
    /// path, rename, make folder, the filter, the patterns, the
    /// run-command prompt and the copy-or-move picker are all this shape.
    Edit {
        /// The field.
        field: EntryField,
        /// The candidates, while the list is open.
        candidates: Option<ListModel>,
        /// Whether the focus is in the candidate list rather than the
        /// field; the hardware cursor follows it.
        in_list: bool,
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
            Dialog::Question { lines, choices } => lines
                .first()
                .cloned()
                .unwrap_or_else(|| choices.title().to_owned()),
            Dialog::Edit { field, .. } => field.label().to_owned(),
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

    /// A text field with its list closed.
    pub fn edit(field: EntryField) -> Self {
        Dialog::Edit {
            field,
            candidates: None,
            in_list: false,
        }
    }

    /// What the masked field shows for what was typed: one asterisk per
    /// character, nothing when `silent`, or the text when Show is on.
    /// The frontend draws this and never the secret itself.
    pub fn masked_text(&self, silent: bool) -> Option<String> {
        match self {
            Dialog::Secret { typed, shown, .. } => Some(if *shown {
                typed.expose().to_owned()
            } else if silent {
                String::new()
            } else {
                "*".repeat(typed.char_count())
            }),
            _ => None,
        }
    }

    /// The line the hardware cursor sits on, as the dialog says it: the
    /// focused choice or candidate with its position, or the field. The
    /// frontend draws the dialog; this is what a Say Status repeats.
    pub fn focus_line(&self) -> String {
        match self {
            Dialog::Confirm { question, .. } => format!("{}, No is focused", question.text),
            Dialog::Prompt { label, text, .. } => format!("{label}: {text}"),
            Dialog::Choose { list, .. } => list.announce_focus(),
            Dialog::Read { title, .. } => title.clone(),
            Dialog::Secret { request, typed, .. } => format!(
                "{}, {}",
                request.title(),
                diroxide_core::format::count(
                    typed.char_count() as u64,
                    "character typed",
                    "characters typed"
                )
            ),
            Dialog::Question { choices, .. } => choices.announce_focus(),
            Dialog::Edit {
                field,
                candidates,
                in_list,
            } => match (in_list, candidates) {
                (true, Some(list)) => list.announce_focus(),
                _ => field.spoken(),
            },
        }
    }
}

/// The confirmation question with its default said, R23's rule: "Delete 3
/// items for good? No is the default. y or n". A question that already
/// says its keys another way (a typed count) is kept as it is.
pub fn with_default_no(question: Message) -> Message {
    match question.text.strip_suffix("? y or n") {
        Some(head) => Message {
            text: format!("{head}? No is the default. y or n"),
            ..question
        },
        None => question,
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
    use crate::entry::CompletionSource;
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
        assert_eq!(dialog.masked_text(false).as_deref(), Some("***********"));
        assert_eq!(dialog.masked_text(true).as_deref(), Some(""));
        assert!(!dialog.focus_line().contains("canary"));
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
        assert_eq!(
            with_default_no(Message::question("Move 2 items to the trash")).text,
            "Move 2 items to the trash? No is the default. y or n"
        );
        let typed = Message::info("Type 1200 to go on, or Escape");
        assert_eq!(with_default_no(typed.clone()), typed);
        let edit = Dialog::edit(EntryField::new("Rename x to", CompletionSource::None));
        assert_eq!(edit.title(), "Rename x to");
        assert_eq!(edit.focus_line(), "Rename x to, empty");
    }
}
