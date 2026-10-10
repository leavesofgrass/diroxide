//! Dialogs: modal, named, closed by Escape, returning the focus where it
//! was. The app holds a dialog's state; frontends draw it and send back an
//! [`Answer`]. Confirmations start on No.
//!
//! Wave 1 (W1d) owns this module.

use diroxide_core::{Command, ListModel, Message};

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
}

impl Dialog {
    /// The dialog's name, said when it opens.
    pub fn title(&self) -> String {
        match self {
            Dialog::Confirm { question, .. } => question.text.clone(),
            Dialog::Prompt { label, .. } => label.clone(),
            Dialog::Choose { list, .. } => list.title().to_owned(),
            Dialog::Read { title, .. } => title.clone(),
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
    /// Escape.
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::CommandId;

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
