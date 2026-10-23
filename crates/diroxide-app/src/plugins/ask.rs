//! A plugin's questions and documents as the app's own dialogs (R22
//! commit 37): modal, named with the plugin's name last, closed by Escape
//! with the focus returned; a destructive confirmation starts on No; a
//! secret is never echoed and never kept.
//!
//! The five kinds:
//!
//! - **confirm**: a yes-or-no question, starting on No unless the plugin
//!   said it is not destructive; with words of its own for the buttons it
//!   is a list of the two.
//! - **choose**: the message's lines first, then the choices; with
//!   `multiple`, Space marks and Enter answers the marked ones (or the one
//!   under the cursor).
//! - **text** and **path**: a text field with the prompt's history and
//!   its completion (a fixed list, or places for a path).
//! - **secret**: the masked prompt, as an archive's password is asked.
//!
//! A text document is a list of its lines to read, one row a line, so a
//! long one is read line by line; Escape closes it.

use diroxide_core::{Command, ListModel, ListRow, Message, SecretKind, SecretRequest};
use diroxide_plugins::ask::{Answer, Completion, PathMode, Question, QuestionKind, TextDocument};

use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};

/// The title without the plugin's name at its end, and the name: "Trust
/// this host?" and "SFTP places" from "Trust this host? (SFTP places)".
fn split_title(question: &Question) -> (String, String) {
    let suffix = format!(" ({})", question.plugin);
    match question.title.strip_suffix(&suffix) {
        Some(head) => (head.to_owned(), question.plugin.clone()),
        None => (question.title.clone(), question.plugin.clone()),
    }
}

/// A confirmation's line, meaning first and the plugin's name last: "Trust
/// this host? No is the default. y or n (SFTP places)".
pub fn confirm_line(question: &Question, destructive: bool) -> String {
    let (head, plugin) = split_title(question);
    let head = head.trim().trim_end_matches(['?', '.']);
    let head = if head.is_empty() {
        format!("{plugin} asks")
    } else {
        head.to_owned()
    };
    if destructive {
        format!("{head}? No is the default. y or n ({plugin})")
    } else {
        format!("{head}? y or n ({plugin})")
    }
}

/// The history key a typed question keeps its answers under in the app.
pub fn history_key(question: &Question) -> Option<String> {
    if !question.kind.is_typed() {
        return None;
    }
    question
        .prompt_id
        .as_ref()
        .map(|prompt| format!("plugin:{}/{prompt}", question.plugin_id))
}

/// The dialog for a question, and the sentence said when it opens.
pub fn dialog_for(question: &Question) -> (Dialog, Message) {
    let opening = |dialog: &Dialog| {
        let mut words = vec![dialog.title()];
        words.extend(question.lines.iter().cloned());
        Message::new(
            words.join(" "),
            diroxide_core::Importance::High,
            diroxide_core::MessageKind::Question,
        )
    };
    match &question.kind {
        QuestionKind::Confirm {
            destructive,
            yes_label,
            no_label,
        } if yes_label.is_none() && no_label.is_none() => {
            let line = confirm_line(question, *destructive);
            let dialog = Dialog::Confirm {
                question: Message::new(
                    line,
                    diroxide_core::Importance::High,
                    diroxide_core::MessageKind::Question,
                ),
                listing: question.lines.clone(),
                on_yes: Command::new(ids::YES),
            };
            let message = opening(&dialog);
            (dialog, message)
        }
        QuestionKind::Confirm {
            destructive,
            yes_label,
            no_label,
        } => {
            let yes = yes_label.clone().unwrap_or_else(|| "Yes".to_owned());
            let no = no_label.clone().unwrap_or_else(|| "No".to_owned());
            let mut choices = ListModel::new(question.title.clone())
                .with_rows(vec![ListRow::new("yes", yes), ListRow::new("no", no)]);
            if *destructive {
                choices.focus_id("no");
            }
            choices.set_help("Up and Down choose, Enter answers, Escape answers no.");
            let mut lines = vec![question.title.clone()];
            lines.extend(question.lines.iter().cloned());
            let dialog = Dialog::Question { lines, choices };
            let message = opening(&dialog);
            (dialog, message)
        }
        QuestionKind::Choose {
            choices,
            default_id,
            multiple,
        } => {
            let rows = choices
                .iter()
                .map(|c| {
                    let row = ListRow::new(c.id.clone(), c.label.clone());
                    match &c.description {
                        Some(more) => row.with_details([more.clone()]),
                        None => row,
                    }
                })
                .collect();
            let mut list = ListModel::new(question.title.clone()).with_rows(rows);
            if let Some(id) = default_id {
                list.focus_id(id);
            }
            list.set_help(if *multiple {
                "Up and Down choose, Space marks, Enter answers the marked ones, Escape cancels."
            } else {
                "Up and Down choose, Enter answers, Escape cancels."
            });
            let mut lines = vec![question.title.clone()];
            lines.extend(question.lines.iter().cloned());
            let dialog = Dialog::Question {
                lines,
                choices: list,
            };
            let message = opening(&dialog);
            (dialog, message)
        }
        QuestionKind::Text { initial, .. } => {
            let source = match &question.completion {
                Some(Completion::List(candidates)) => {
                    CompletionSource::Values(candidates.iter().map(|c| c.text.clone()).collect())
                }
                _ => CompletionSource::None,
            };
            let mut field =
                EntryField::new(question.title.clone(), source).with_text(initial.clone());
            if let Some(key) = history_key(question) {
                field = field.with_history(key);
            }
            let dialog = Dialog::edit(field);
            let message = opening(&dialog);
            (dialog, message)
        }
        QuestionKind::Path { initial, .. } => {
            let mut field = EntryField::new(question.title.clone(), CompletionSource::Paths)
                .with_text(initial.clone());
            if let Some(key) = history_key(question) {
                field = field.with_history(key);
            }
            let dialog = Dialog::edit(field);
            let message = opening(&dialog);
            (dialog, message)
        }
        QuestionKind::Secret => {
            let dialog = Dialog::secret(SecretRequest::new(
                SecretKind::ServerPrompt,
                question.title.clone(),
            ));
            let message = Message::new(
                {
                    let mut words = vec![format!("{} (hidden)", question.title)];
                    words.extend(question.lines.iter().cloned());
                    words.join(" ")
                },
                diroxide_core::Importance::High,
                diroxide_core::MessageKind::Question,
            );
            (dialog, message)
        }
    }
}

/// What a path question's typed text is checked for, in words, when it
/// does not fit: "A folder is needed here.".
pub fn path_refusal(mode: PathMode, is_folder: Option<bool>, must_exist: bool) -> Option<String> {
    match (mode, is_folder) {
        (_, None) if must_exist => Some("Nothing is there. Type a place that exists.".to_owned()),
        (PathMode::Folder, Some(false)) => Some("A folder is needed here.".to_owned()),
        (PathMode::File, Some(true)) => Some("A file is needed here.".to_owned()),
        _ => None,
    }
}

/// The answer to a confirmation.
pub fn confirmed(yes: bool) -> Answer {
    Answer::Confirmed(yes)
}

/// The answer to a choose question from its list: the marked rows, or
/// the focused one when none is marked (or when only one may be chosen).
pub fn chosen(list: &ListModel, multiple: bool) -> Answer {
    let marked: Vec<String> = list.selected_ids().into_iter().map(str::to_owned).collect();
    if multiple && !marked.is_empty() {
        return Answer::Chosen(marked);
    }
    match list.focused() {
        Some(row) => Answer::Chosen(vec![row.id.clone()]),
        None => Answer::Cancelled,
    }
}

/// A document as a list to read, one row a line, titled with its title
/// (the plugin's name last).
pub fn document_list(document: &TextDocument) -> ListModel {
    let rows = document
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| ListRow::new(i.to_string(), line.clone()))
        .collect();
    let mut list = ListModel::new(document.title.clone()).with_rows(rows);
    list.set_empty_text("The document is empty");
    list.set_help("Up and Down read a line, Page Up and Page Down a page, Escape closes.");
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_plugins::ask::Choice;

    fn question(kind: QuestionKind) -> Question {
        Question {
            plugin_id: "sftp".into(),
            plugin: "SFTP places".into(),
            title: "Trust this host? (SFTP places)".into(),
            lines: vec!["Key: 1a2b 3c4d".into()],
            kind,
            prompt_id: Some("host".into()),
            history: Vec::new(),
            completion: None,
        }
    }

    #[test]
    fn a_confirmation_starts_on_no_and_names_the_plugin_last() {
        let q = question(QuestionKind::Confirm {
            destructive: true,
            yes_label: None,
            no_label: None,
        });
        let (dialog, message) = dialog_for(&q);
        assert_eq!(
            dialog.title(),
            "Trust this host? No is the default. y or n (SFTP places)"
        );
        assert!(within_cells(&dialog.title(), "Trust this host?"));
        assert!(message.text.ends_with("Key: 1a2b 3c4d"));
        assert!(matches!(dialog, Dialog::Confirm { .. }));
    }

    #[test]
    fn choose_and_labels_are_lists_with_the_message_first() {
        let q = question(QuestionKind::Choose {
            choices: vec![
                Choice {
                    id: "a".into(),
                    label: "Alpha".into(),
                    description: Some("the first".into()),
                },
                Choice {
                    id: "b".into(),
                    label: "Beta".into(),
                    description: None,
                },
            ],
            default_id: Some("b".into()),
            multiple: true,
        });
        let (dialog, _) = dialog_for(&q);
        let Dialog::Question { lines, choices } = &dialog else {
            panic!("a list");
        };
        assert_eq!(lines[0], "Trust this host? (SFTP places)");
        assert_eq!(choices.announce_focus(), "Beta, 2 of 2");
        let mut list = choices.clone();
        assert_eq!(chosen(&list, true), Answer::Chosen(vec!["b".into()]));
        list.focus_id("a");
        list.toggle_selected();
        list.focus_id("b");
        list.toggle_selected();
        assert_eq!(
            chosen(&list, true),
            Answer::Chosen(vec!["a".into(), "b".into()])
        );
        let labeled = question(QuestionKind::Confirm {
            destructive: true,
            yes_label: Some("Trust".into()),
            no_label: Some("Leave".into()),
        });
        let (dialog, _) = dialog_for(&labeled);
        assert_eq!(dialog.focus_line(), "Leave, 2 of 2");
    }

    #[test]
    fn text_path_and_secret_are_fields() {
        let (dialog, _) = dialog_for(&question(QuestionKind::Text {
            initial: "main".into(),
            pattern: None,
        }));
        assert_eq!(dialog.focus_line(), "Trust this host? (SFTP places): main");
        let (dialog, _) = dialog_for(&question(QuestionKind::Path {
            mode: PathMode::Folder,
            must_exist: true,
            initial: String::new(),
        }));
        assert!(matches!(dialog, Dialog::Edit { .. }));
        let (dialog, message) = dialog_for(&question(QuestionKind::Secret));
        assert_eq!(dialog.title(), "Trust this host? (SFTP places)");
        assert!(
            message
                .text
                .starts_with("Trust this host? (SFTP places) (hidden)")
        );
        assert_eq!(
            path_refusal(PathMode::Folder, Some(false), true).unwrap(),
            "A folder is needed here."
        );
        assert_eq!(path_refusal(PathMode::Any, Some(true), true), None);
    }
}
