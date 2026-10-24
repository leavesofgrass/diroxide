//! Dialogs: opening and closing them, their keys, the settle time, the
//! text fields with completion and history, and what each answer does.
//!
//! Every dialog is modal and named: it opens with an [`Effect::OpenDialog`]
//! and its name said, Escape closes it (answering the safe choice), and
//! closing returns the focus where it was. Confirming keys that arrive
//! within `dialog_settle_ms` of a question opening are ignored.

use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, ConflictAction, ConflictAnswer, Effect, ErrorAction, Importance, Key,
    KeyCode, Layer, ListModel, ListRow, Message, MessageKind, Move, Place, Redraw, Side, catalog,
};

use super::{Action, App, Completing, Open, Purpose, typed};
use crate::commands::{self, ids};
use crate::dialog::{Answer, Dialog};
use crate::entry::{Candidate, Completion, CompletionSource, EntryField};
use crate::jobs::millis_between;
use crate::keymap::Lookup;
use crate::request::{Reply, Request, Spawn, SpawnKind};
use crate::state;

/// The candidates' list for a field, each row the candidate first and its
/// kind after it.
fn candidate_list(title: &str, candidates: &[Candidate]) -> ListModel {
    let rows = candidates
        .iter()
        .map(|c| ListRow::new(c.text.clone(), c.text.clone()).with_details([c.kind.clone()]))
        .collect();
    let mut list = ListModel::new(title).with_rows(rows);
    list.set_empty_text("No match");
    list.set_help("Up and Down choose, Enter takes it, Escape closes the list.");
    list
}

/// Whether a candidate path holds the typed terms in order, ignoring case
/// (path jump over the whole path).
fn terms_in_order(text: &str, candidate: &str) -> bool {
    let lower = candidate.to_lowercase();
    let mut from = 0;
    for term in text.split_whitespace() {
        let term = term.to_lowercase();
        match lower[from..].find(&term) {
            Some(at) => from += at + term.len(),
            None => return false,
        }
    }
    true
}

impl App {
    /// Opens a dialog for a purpose, says its name, and notes when it
    /// opened for the settle time.
    pub(super) fn open_dialog(&mut self, dialog: Dialog, purpose: Purpose) -> Vec<Effect> {
        let id = self.ids.dialog();
        let message = match &dialog {
            Dialog::Confirm { question, .. } => question.clone(),
            Dialog::Question { lines, .. } => Message::new(
                lines.first().cloned().unwrap_or_default(),
                Importance::High,
                MessageKind::Question,
            ),
            Dialog::Choose { list, .. } => {
                Message::info(format!("{}, {}", list.title(), list.announce_focus()))
            }
            Dialog::Edit { field, .. } => Message::info(field.spoken()),
            Dialog::Secret { request, .. } => Message::new(
                request.prompt_line().trim_end().to_owned(),
                Importance::High,
                MessageKind::Question,
            ),
            other => Message::info(other.title()),
        };
        self.dialogs.push((id, dialog));
        self.tabbed = None;
        let opened = self.now();
        self.opens.push(Open { purpose, opened });
        vec![Effect::OpenDialog(id), self.say(message)]
    }

    /// Opens a dialog for a purpose and says a line of its own instead of
    /// its name (the results lister's "Searching work for *.txt").
    pub(super) fn open_dialog_saying(
        &mut self,
        dialog: Dialog,
        purpose: Purpose,
        message: Message,
    ) -> Vec<Effect> {
        let id = self.ids.dialog();
        self.dialogs.push((id, dialog));
        self.tabbed = None;
        let opened = self.now();
        self.opens.push(Open { purpose, opened });
        vec![Effect::OpenDialog(id), self.say(message)]
    }

    /// Closes the dialog on top; the focus returns where it was.
    pub(super) fn close_top(&mut self) -> Vec<Effect> {
        // A secret dialog's typed text is wiped as it is dropped here.
        let Some((id, _)) = self.dialogs.pop() else {
            return Vec::new();
        };
        self.opens.pop();
        self.tabbed = None;
        if self.top_purpose().is_none() {
            self.completing = None;
        }
        vec![Effect::CloseDialog(id), Effect::Redraw(Redraw::All)]
    }

    /// Closes every dialog: a menu's command runs on the panel.
    pub(super) fn close_all(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        while !self.dialogs.is_empty() {
            effects.extend(self.close_top());
        }
        effects
    }

    fn top_purpose(&self) -> Option<&Purpose> {
        self.opens.last().map(|o| &o.purpose)
    }

    /// Whether the top dialog opened less than the settle time ago, so a
    /// confirming key is ignored.
    pub(super) fn settling(&self) -> bool {
        self.opens.last().is_some_and(|open| {
            millis_between(open.opened, self.now()) < self.settings.dialog_settle_ms
        })
    }

    fn top_dialog_mut(&mut self) -> Option<&mut Dialog> {
        self.dialogs.last_mut().map(|(_, dialog)| dialog)
    }

    /// The list a dialog moves in: a choice list, or the open candidates.
    fn dialog_list_mut(&mut self) -> Option<&mut ListModel> {
        match self.top_dialog_mut()? {
            Dialog::Choose { list, .. } => Some(list),
            Dialog::Question { choices, .. } => Some(choices),
            Dialog::Edit {
                candidates: Some(list),
                ..
            } => Some(list),
            _ => None,
        }
    }

    fn list_focus_said(&mut self) -> Vec<Effect> {
        let line = match self.dialogs.last() {
            Some((_, dialog)) => dialog.focus_line(),
            None => return Vec::new(),
        };
        vec![
            Effect::Redraw(Redraw::All),
            Effect::Announce(Message::status(line)),
        ]
    }

    /// A key with a dialog open.
    pub(super) fn dialog_key(&mut self, key: Key) -> Vec<Effect> {
        // A plugin's lister and choose question (W4d).
        if let Some(effects) = self.plugin_key(&key) {
            return effects;
        }
        // The Jobs list's Delete (lane A2).
        if let Some(effects) = self.jobs_key(&key) {
            return effects;
        }
        // Space in the first-run components list (lane A3).
        if let Some(effects) = self.components_key(&key) {
            return effects;
        }
        // Space in the trash browser (lane A3).
        if let Some(effects) = self.trash_key(&key) {
            return effects;
        }
        // The reload key in the Volumes list (lane A3).
        if let Some(effects) = self.volumes_key(&key) {
            return effects;
        }
        // The results lister's and the search dialog's own keys (W3a).
        if let Some(effects) = self.results_key(&key) {
            return effects;
        }
        if let Some(effects) = self.search_form_key(&key) {
            return effects;
        }
        if let Some(effects) = self.saved_key(&key) {
            return effects;
        }
        if let Some(effects) = self.space_key(&key) {
            return effects;
        }
        if let Some(effects) = self.compare_key(&key) {
            return effects;
        }
        // The labels', bookmarks' and History lists' own keys (W3l).
        if let Some(effects) = self.organize_key(&key) {
            return effects;
        }
        let Some((_, dialog)) = self.dialogs.last() else {
            return Vec::new();
        };
        match dialog {
            Dialog::Edit { .. } => return self.edit_key(key),
            Dialog::Secret { .. } => return self.secret_key(key),
            Dialog::Confirm { .. } => {}
            _ => {
                if let Some(c) = typed(&key) {
                    return self.list_letter(c);
                }
                if let Some(Purpose::Menu(id)) = self.top_purpose().cloned().as_ref()
                    && key.mods.is_empty()
                {
                    match key.code {
                        KeyCode::Left => return self.next_menu(id, false),
                        KeyCode::Right => return self.next_menu(id, true),
                        _ => {}
                    }
                }
            }
        }
        let layers = [Layer::Dialog, Layer::List, Layer::Global];
        match self.keymap.lookup(&layers, &key) {
            Lookup::Command(id) => self.dispatch(&Command::new(id)),
            Lookup::NoMatch => Vec::new(),
        }
    }

    /// A command with a dialog open: moves and answers go to the dialog;
    /// the panel's commands wait until it closes.
    pub(super) fn dialog_command(&mut self, action: Action, command: &Command) -> Vec<Effect> {
        let confirm = matches!(self.dialogs.last(), Some((_, Dialog::Confirm { .. })));
        match action {
            Action::Focus(movement) => {
                if let Some(list) = self.dialog_list_mut() {
                    if list.move_focus(movement) {
                        self.results_focus_moved();
                        return self.list_focus_said();
                    }
                    let edge = if matches!(movement, Move::Up | Move::PageUp | Move::Home) {
                        "Top of the list"
                    } else {
                        "Bottom of the list"
                    };
                    return vec![self.say(Message::status(edge))];
                }
                Vec::new()
            }
            Action::Yes if confirm => {
                if self.settling() {
                    return Vec::new();
                }
                self.confirm_yes()
            }
            Action::No if confirm => self.dialog_escape(),
            Action::Accept => {
                if confirm {
                    return self.dialog_escape();
                }
                self.accept()
            }
            Action::Cancel => self.dialog_escape(),
            Action::Quit | Action::QuitToFolder => self.ask_quit(false),
            Action::QuitKeepFolder => self.ask_quit(true),
            Action::Menu => {
                if matches!(
                    self.top_purpose(),
                    Some(Purpose::Menu(_) | Purpose::MenuBar)
                ) {
                    let mut effects = self.close_all();
                    effects.extend(self.open_menu_by_command(&command.id));
                    return effects;
                }
                vec![self.say(Message::status("Close this dialog first."))]
            }
            Action::MenuBar => {
                if matches!(
                    self.top_purpose(),
                    Some(Purpose::Menu(_) | Purpose::MenuBar)
                ) {
                    return self.close_all();
                }
                vec![self.say(Message::status("Close this dialog first."))]
            }
            Action::Yes | Action::No => Vec::new(),
            _ => vec![self.say(Message::status("Close this dialog first."))],
        }
    }

    /// A letter in a list dialog: a menu's access key runs its item;
    /// elsewhere the focus jumps to the next row starting with it.
    fn list_letter(&mut self, letter: char) -> Vec<Effect> {
        if let Some(Purpose::Menu(id)) = self.top_purpose().cloned() {
            let caps = self.active_capabilities();
            let item = self
                .menus
                .item_by_access(&id, letter, &caps)
                .map(|item| item.row_id());
            if let Some(row) = item {
                if let Some(list) = self.dialog_list_mut() {
                    list.focus_id(&row);
                }
                return self.accept();
            }
        }
        let jumped = match self.dialog_list_mut() {
            Some(list) => list.letter_jump(letter),
            None => return Vec::new(),
        };
        if jumped {
            self.list_focus_said()
        } else {
            vec![self.say(Message::status(format!("No row starts with {letter}")))]
        }
    }

    /// Escape: the safe answer, per dialog.
    pub(super) fn dialog_escape(&mut self) -> Vec<Effect> {
        let Some(purpose) = self.top_purpose().cloned() else {
            return Vec::new();
        };
        if let Some((
            _,
            Dialog::Edit {
                candidates,
                in_list,
                ..
            },
        )) = self.dialogs.last_mut()
        {
            let list_always = matches!(
                purpose,
                Purpose::Palette
                    | Purpose::Places(_)
                    | Purpose::PickDestination { .. }
                    | Purpose::TypedPlan(..)
                    | Purpose::Keys(_)
            ) || matches!(&purpose, Purpose::Organize(w) if w.list_always());
            if candidates.is_some() && !list_always {
                *candidates = None;
                *in_list = false;
                return vec![
                    Effect::Redraw(Redraw::All),
                    Effect::Announce(Message::status("List closed")),
                ];
            }
        }
        let mut effects = self.close_top();
        match purpose {
            Purpose::Plugin(wants) => effects.extend(self.plugin_escape(*wants)),
            Purpose::Component(wants) => effects.extend(self.component_escape(*wants)),
            Purpose::Jobs(wants) => effects.extend(self.jobs_escape(*wants)),
            Purpose::Trash(wants) => effects.extend(self.trash_escape(*wants)),
            Purpose::RunPlan(plan) | Purpose::TypedPlan(plan, _) => {
                let flow = self.flows.remove(&plan.job);
                self.plugin_job_ended(flow.as_ref(), Err("the owner said no".to_owned()));
                self.jobs.finish(plan.job);
                effects.push(Effect::Redraw(Redraw::JobsLine));
                effects.push(self.say(Message::info("Nothing changed.")));
            }
            Purpose::Conflict { ask, .. } | Purpose::ConflictName { ask } => {
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Conflict(ConflictAnswer::skip()),
                });
            }
            Purpose::ItemError { ask, .. } => {
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Error(ErrorAction::Skip),
                });
            }
            Purpose::Secret { ask } => {
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Secret(None),
                });
                effects.push(self.say(Message::info("No password given.")));
            }
            Purpose::Limit { ask } => self.answer_limit(ask, false),
            Purpose::TrashParts(_) => {
                effects.push(self.say(Message::info("The parts stay where they are.")));
            }
            Purpose::PreviewLink => effects.extend(self.preview_link_answered(false)),
            Purpose::WrongPassword { ask, .. } => {
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Secret(None),
                });
                effects.push(self.say(Message::info("No password given.")));
            }
            Purpose::ArchiveQuestion {
                plan,
                question,
                rest,
            } => effects.extend(self.plan_question_answered(plan, &question, rest, false)),
            Purpose::RunArchive(plan) => effects.extend(self.archive_declined(&plan)),
            Purpose::PlanQuestion {
                plan,
                question,
                rest,
                forced,
            } => effects.extend(self.plan_preflight_answered(plan, &question, rest, forced, false)),
            Purpose::PackName(form) => {
                effects.extend(self.pack_row_reopen(*form, "name"));
            }
            Purpose::PackChoice(form, _) => {
                effects.extend(self.pack_row_reopen(*form, "format"));
            }
            Purpose::NewPassword { form, .. } => {
                effects.extend(self.new_password_cancelled(*form));
            }
            Purpose::PackForm(_) => {
                effects.push(self.say(Message::info("Nothing was packed.")));
            }
            Purpose::UnpackForm(_) => {
                effects.push(self.say(Message::info("Nothing was unpacked.")));
            }
            Purpose::Filter(side, previous) => effects.extend(self.filter_escaped(side, previous)),
            Purpose::FilterContents { side, previous, .. } => {
                effects.extend(self.filter_contents_declined(side, previous.map(|f| *f)));
            }
            Purpose::SelectCount { .. } => {
                effects.push(self.say(Message::info("Nothing changed.")));
            }
            Purpose::DeleteSaved(..) => {
                effects.push(self.say(Message::info("Nothing deleted.")));
            }
            Purpose::CompareLarge(_) => {
                effects.push(self.say(Message::info("Nothing was compared.")));
            }
            Purpose::Rename {
                side, rest, done, ..
            } => match rest.split_first() {
                Some((next, more)) => {
                    effects.extend(self.rename_prompt(side, next.clone(), more.to_vec(), done));
                }
                None => effects.extend(self.rename_finish(side, done)),
            },
            Purpose::Duplicate { side, rest, .. } => {
                if let Some((next, more)) = rest.split_first() {
                    effects.extend(self.duplicate_prompt(side, next.clone(), more.to_vec()));
                }
            }
            Purpose::Keys(purpose) => effects.extend(self.keys_escape(*purpose)),
            Purpose::Organize(wants) => effects.extend(self.organize_escape(*wants)),
            Purpose::SearchText(form, field) => {
                effects.extend(self.search_text_escaped(*form, field));
            }
            Purpose::SearchForm(_) => {
                effects.push(self.say(Message::info("Nothing was searched.")));
            }
            Purpose::Results => effects.extend(self.results_closed()),
            _ => {}
        }
        effects
    }

    /// Yes on a confirmation.
    fn confirm_yes(&mut self) -> Vec<Effect> {
        let Some(purpose) = self.top_purpose().cloned() else {
            return Vec::new();
        };
        let mut effects = self.close_top();
        match purpose {
            Purpose::RunPlan(plan) => effects.extend(self.run_plan(*plan)),
            Purpose::Quit => effects.extend(self.quit_now()),
            Purpose::Limit { ask } => self.answer_limit(ask, true),
            Purpose::TrashParts(parts) => effects.extend(self.trash_parts(parts)),
            Purpose::PreviewLink => effects.extend(self.preview_link_answered(true)),
            Purpose::JunctionInstead { targets, folder } => {
                effects.extend(self.make_junctions(targets, folder));
            }
            Purpose::WrongPassword { ask, request } => {
                effects.extend(self.secret_prompt(ask, request, false));
            }
            Purpose::ArchiveQuestion {
                plan,
                question,
                rest,
            } => effects.extend(self.plan_question_answered(plan, &question, rest, true)),
            Purpose::RunArchive(plan) => effects.extend(self.run_archive(*plan)),
            Purpose::PlanQuestion {
                plan,
                question,
                rest,
                forced,
            } => effects.extend(self.plan_preflight_answered(plan, &question, rest, forced, true)),
            Purpose::RunProgram(place) => {
                let name = place
                    .name()
                    .map(diroxide_core::names::display_name)
                    .unwrap_or_default();
                effects.push(self.say(Message::info(format!("Running {name}"))));
                effects.push(Effect::OpenExternally(place));
            }
            Purpose::Keys(purpose) => effects.extend(self.keys_yes(*purpose)),
            Purpose::Plugin(wants) => effects.extend(self.plugin_yes(*wants)),
            Purpose::Component(wants) => effects.extend(self.component_yes(*wants)),
            Purpose::Jobs(wants) => effects.extend(self.jobs_yes(*wants)),
            Purpose::Organize(wants) => effects.extend(self.organize_yes(*wants)),
            Purpose::FilterContents {
                side,
                filter,
                previous,
            } => effects.extend(self.read_filter_contents(side, *filter, previous.map(|f| *f))),
            Purpose::SelectCount { side, select, ids } => {
                effects.extend(self.select_counted(side, select, ids));
            }
            Purpose::DeleteSaved(path, name) => effects.extend(self.delete_saved(path, &name)),
            Purpose::OfferSizeSort => effects.extend(self.sort_by_size_now()),
            _ => {}
        }
        effects
    }

    /// Enter: the focused choice, or the field's text, per dialog.
    pub(super) fn accept(&mut self) -> Vec<Effect> {
        let Some(purpose) = self.top_purpose().cloned() else {
            return Vec::new();
        };
        let (row, text, in_list) = match self.dialogs.last() {
            Some((_, Dialog::Choose { list, .. })) => {
                (list.focused().map(|r| r.id.clone()), String::new(), true)
            }
            Some((_, Dialog::Question { choices, .. })) => {
                (choices.focused().map(|r| r.id.clone()), String::new(), true)
            }
            Some((
                _,
                Dialog::Edit {
                    field,
                    candidates,
                    in_list,
                },
            )) => (
                candidates
                    .as_ref()
                    .and_then(|l| l.focused())
                    .map(|r| r.id.clone()),
                field.text().to_owned(),
                *in_list,
            ),
            _ => (None, String::new(), false),
        };
        match purpose {
            Purpose::Conflict {
                ask,
                conflict,
                choices,
            } => {
                if self.settling() {
                    return Vec::new();
                }
                let index: usize = row.and_then(|r| r.parse().ok()).unwrap_or(0);
                let Some(choice) = choices.get(index) else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                if let ConflictAction::Rename(_) = &choice.answer.action {
                    let field = EntryField::new("Rename the copy to", CompletionSource::None)
                        .with_text(conflict.keep_both_name.clone());
                    effects.extend(
                        self.open_dialog(Dialog::edit(field), Purpose::ConflictName { ask }),
                    );
                    return effects;
                }
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Conflict(choice.answer.clone()),
                });
                effects
            }
            Purpose::ConflictName { ask } => {
                let mut effects = self.close_top();
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Conflict(ConflictAnswer::once(ConflictAction::Rename(text))),
                });
                effects.push(Effect::Redraw(Redraw::All));
                effects
            }
            Purpose::ItemError { ask, choices } => {
                if self.settling() {
                    return Vec::new();
                }
                let index: usize = row.and_then(|r| r.parse().ok()).unwrap_or(0);
                let action = choices.get(index).copied().unwrap_or(ErrorAction::Skip);
                let effects = self.close_top();
                self.request(Request::Answer {
                    ask,
                    reply: Reply::Error(action),
                });
                effects
            }
            Purpose::TypedPlan(plan, typed) => {
                if self.settling() {
                    return Vec::new();
                }
                if text.trim() == typed {
                    let mut effects = self.close_top();
                    effects.extend(self.run_plan(*plan));
                    effects
                } else {
                    vec![self.say(Message::info(format!(
                        "That is not {typed}. Nothing was done; type it, or Cancel."
                    )))]
                }
            }
            Purpose::Palette => {
                let Some(id) = row.and_then(|r| CommandId::new(r).ok()) else {
                    return vec![self.say(Message::status("No command matches"))];
                };
                let now = self.now().unix_seconds();
                self.palette.record_at(&id, now);
                let recent: Vec<String> = self.palette.to_lines();
                self.histories
                    .insert(state::keys::PALETTE.to_owned(), recent.clone());
                self.request(Request::SaveHistory {
                    key: state::keys::PALETTE.to_owned(),
                    entries: recent,
                });
                let mut effects = self.close_all();
                effects.extend(self.dispatch(&Command::new(id)));
                effects
            }
            Purpose::MenuBar => {
                let Some(menu) = row.and_then(|r| r.strip_prefix("menu:").map(str::to_owned))
                else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.open_menu(&menu));
                effects
            }
            Purpose::Menu(_) | Purpose::ContextMenu => {
                let Some(row) = row else {
                    return Vec::new();
                };
                if let Some(sub) = row.strip_prefix("menu:") {
                    let sub = sub.to_owned();
                    return self.open_menu(&sub);
                }
                let Ok(id) = CommandId::new(row) else {
                    return Vec::new();
                };
                let mut effects = self.close_all();
                effects.extend(self.dispatch(&Command::new(id)));
                effects
            }
            Purpose::SortList => {
                let Some(id) = row.and_then(|r| CommandId::new(r).ok()) else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.sort_by(&id));
                effects
            }
            Purpose::History(side) => {
                let Some(place) = row.and_then(|r| Place::parse(&r).ok()) else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.open(side, place, None));
                effects
            }
            Purpose::Settings => match row {
                Some(key) => self.setting_chosen(&key),
                None => Vec::new(),
            },
            Purpose::SettingValue(key) => {
                let Some(value) = row else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(self.setting_value_chosen(&key, &value));
                effects
            }
            Purpose::SettingText(key) => {
                let mut effects = self.close_top();
                effects.extend(self.setting_typed(&key, &text));
                effects
            }
            Purpose::JobHistory => match row.and_then(|r| r.parse::<usize>().ok()) {
                Some(index) => self.job_items(index),
                None => Vec::new(),
            },
            Purpose::LinkForm(form) => match row {
                Some(row) => self.link_row(*form, &row),
                None => Vec::new(),
            },
            Purpose::AccessForm(form) => match row {
                Some(row) => self.access_row(*form, &row),
                None => Vec::new(),
            },
            Purpose::Relink { side, link } => {
                if in_list && let Some(row) = row {
                    return self.take_candidate(&row);
                }
                let mut effects = self.close_top();
                effects.extend(self.relink(side, link, &text));
                effects
            }
            Purpose::Read | Purpose::JobItems | Purpose::Details(_) => {
                if row
                    .as_deref()
                    .is_some_and(|r| r.starts_with(super::info::HEADING_ROW) || r == "help")
                {
                    // A heading or the help line of the keyboard list: Enter
                    // stays on it.
                    return Vec::new();
                }
                if let Some(id) = row.and_then(|r| CommandId::new(r).ok())
                    && commands::info(&id).is_some()
                {
                    let mut effects = self.close_all();
                    effects.extend(self.dispatch(&Command::new(id)));
                    return effects;
                }
                self.close_top()
            }
            Purpose::Secret { ask } => self.secret_enter(ask),
            Purpose::Plugin(wants) => self.plugin_accept(*wants, row, text),
            Purpose::Jobs(wants) => self.jobs_accept(*wants, row, text),
            Purpose::Trash(wants) => self.trash_accept(*wants, row),
            Purpose::Volumes(wants) => self.volumes_accept(*wants, row),
            Purpose::Component(wants) => self.component_accept(*wants, row, text),
            Purpose::NewPassword { form, first } => {
                let typed = match self.dialogs.last() {
                    Some((_, Dialog::Secret { typed, .. })) => typed.clone(),
                    _ => return Vec::new(),
                };
                self.new_password_entered(*form, first, typed)
            }
            Purpose::PackForm(form) => match row {
                Some(row) => self.pack_row(*form, &row),
                None => Vec::new(),
            },
            Purpose::PackChoice(form, choice) => match row {
                Some(row) => self.pack_chose(*form, choice, &row),
                None => Vec::new(),
            },
            Purpose::PackName(form) => self.pack_named(*form, &text),
            Purpose::SplitSize {
                side,
                files,
                destination,
                note,
            } => match row {
                Some(row) => self.split_size_chosen(side, files, destination, note, &row),
                None => Vec::new(),
            },
            Purpose::SplitTyped {
                side,
                files,
                destination,
                note,
                equal,
            } => self.split_typed(side, files, destination, note, equal, &text),
            Purpose::ViewerJump => {
                let mut effects = self.close_top();
                effects.extend(self.viewer_jump(&text));
                effects
            }
            Purpose::ViewerSearch { backward } => {
                let mut effects = self.close_top();
                effects.extend(self.viewer_search(&text, backward));
                effects
            }
            Purpose::ViewerOutline => match row {
                Some(row) => {
                    let mut effects = self.close_top();
                    effects.extend(self.viewer_outline_go(&row));
                    effects
                }
                None => Vec::new(),
            },
            Purpose::PreviewFind => {
                let mut effects = self.close_top();
                effects.extend(self.preview_find(&text));
                effects
            }
            Purpose::PreviewLinks => match row {
                Some(row) => {
                    let mut effects = self.close_top();
                    effects.extend(self.preview_link_chosen(&row));
                    effects
                }
                None => Vec::new(),
            },
            Purpose::UnpackForm(form) => match row {
                Some(row) => self.unpack_row(*form, &row),
                None => Vec::new(),
            },
            Purpose::GoPath(side) => {
                if in_list && let Some(row) = row {
                    return self.take_candidate(&row);
                }
                match self.typed_place(side, &text) {
                    Ok(place) => {
                        self.remember(state::keys::GO_PATH, &place.to_string());
                        let mut effects = self.close_top();
                        effects.extend(self.open(side, place, None));
                        effects
                    }
                    Err(message) => vec![self.say(message)],
                }
            }
            Purpose::Places(side) => {
                let chosen = row
                    .filter(|_| in_list || !text.is_empty())
                    .or_else(|| (!text.trim().is_empty()).then(|| text.clone()));
                let Some(chosen) = chosen else {
                    return vec![self.say(Message::status("Type a place, or choose one."))];
                };
                // The Trash row opens the trash browser (lane A3).
                if chosen == super::nav::TRASH_ROW {
                    let mut effects = self.close_top();
                    effects.extend(self.dispatch(&Command::new(ids::TRASH_BROWSER)));
                    return effects;
                }
                match self.typed_place(side, &chosen) {
                    Ok(place) => {
                        self.remember(state::keys::PLACES, &text);
                        self.bookmark_chosen(&place);
                        let mut effects = self.close_top();
                        // A site connects through its plugin, which opens
                        // its start folders (lane A3).
                        match self.connect_site(side, &place) {
                            Some(connect) => effects.extend(connect),
                            None => effects.extend(self.open(side, place, None)),
                        }
                        effects
                    }
                    Err(message) => vec![self.say(message)],
                }
            }
            Purpose::PickDestination { side, kind } => {
                let chosen = if in_list || text.trim().is_empty() {
                    row.unwrap_or_default()
                } else {
                    text.clone()
                };
                let mut effects = self.close_top();
                effects.extend(self.destination_chosen(side, kind, &chosen));
                effects
            }
            Purpose::MakeFolder(side) => {
                let mut effects = self.close_top();
                effects.extend(self.make_folder_named(side, &text));
                effects
            }
            Purpose::Rename {
                side,
                current,
                rest,
                done,
            } => {
                let mut effects = self.close_top();
                effects.extend(self.rename_answer(side, current, rest, done, &text));
                effects
            }
            Purpose::Duplicate {
                side,
                current,
                rest,
            } => {
                let mut effects = self.close_top();
                effects.extend(self.duplicate_answer(side, current, rest, &text));
                effects
            }
            Purpose::Filter(side, previous) => {
                if in_list && let Some(row) = row {
                    return self.take_candidate(&row);
                }
                self.filter_enter(side, &text, previous)
            }
            Purpose::Pattern { side, select } => {
                if in_list && let Some(row) = row {
                    return self.take_candidate(&row);
                }
                self.pattern_entered(side, select, &text)
            }
            Purpose::SearchForm(form) => match row {
                Some(row) => self.search_form_row(*form, &row),
                None => Vec::new(),
            },
            Purpose::SearchText(form, field) => {
                if in_list && let Some(row) = row {
                    return self.take_candidate(&row);
                }
                self.search_text_entered(*form, field, &text)
            }
            Purpose::Results => self.open_result(),
            Purpose::SaveSearchName => self.save_search_named(&text),
            Purpose::Compare(compared) => match row {
                Some(row) => self.change_detail(*compared, &row),
                None => Vec::new(),
            },
            Purpose::Unified(compared) => self.unified_view_line(*compared),
            Purpose::CompareLarge(pair) => match row {
                Some(row) => self.large_chosen(*pair, &row),
                None => Vec::new(),
            },
            Purpose::CompareMenu => match row {
                Some(row) => self.compare_action(&row),
                None => Vec::new(),
            },
            Purpose::SavedList(rows) => match row {
                Some(row) => self.saved_chosen(rows, &row),
                None => Vec::new(),
            },
            Purpose::Space(space) => match row {
                Some(row) => self.space_row(*space, &row),
                None => Vec::new(),
            },
            Purpose::Refine => {
                if in_list && let Some(row) = row {
                    return self.take_candidate(&row);
                }
                self.refine_entered(&text)
            }
            Purpose::ResultsMenu => match row.and_then(|r| CommandId::new(r).ok()) {
                Some(id) => {
                    let mut effects = self.close_top();
                    effects.extend(self.results_action(&id));
                    effects
                }
                None => Vec::new(),
            },
            Purpose::SelectKind(side) => match row {
                Some(row) => self.select_kind_chosen(side, &row),
                None => Vec::new(),
            },
            Purpose::NextRows(side) => self.next_rows_typed(side, &text),
            Purpose::RunCommand(side) => {
                let mut effects = self.close_top();
                effects.extend(self.run_line(side, &text));
                effects
            }
            Purpose::Keys(purpose) => self.keys_accept(*purpose, row.as_deref(), &text),
            Purpose::Organize(wants) => self.organize_accept(*wants, row, text, in_list),
            Purpose::RunPlan(_)
            | Purpose::Quit
            | Purpose::RunProgram(_)
            | Purpose::Limit { .. }
            | Purpose::WrongPassword { .. }
            | Purpose::ArchiveQuestion { .. }
            | Purpose::PlanQuestion { .. }
            | Purpose::TrashParts(_)
            | Purpose::JunctionInstead { .. }
            | Purpose::PreviewLink
            | Purpose::FilterContents { .. }
            | Purpose::SelectCount { .. }
            | Purpose::DeleteSaved(..)
            | Purpose::OfferSizeSort
            | Purpose::RunArchive(_) => self.dialog_escape(),
        }
    }

    /// A candidate chosen from a field's list: its text goes into the
    /// field and the list closes.
    pub(super) fn take_candidate(&mut self, text: &str) -> Vec<Effect> {
        let candidate = Candidate::new(text, "");
        if let Some((
            _,
            Dialog::Edit {
                field,
                candidates,
                in_list,
            },
        )) = self.dialogs.last_mut()
        {
            field.accept(&candidate);
            *candidates = None;
            *in_list = false;
            let said = field.spoken();
            return vec![Effect::Redraw(Redraw::All), self.say(Message::info(said))];
        }
        Vec::new()
    }

    fn secret_key(&mut self, key: Key) -> Vec<Effect> {
        let purpose = self.top_purpose().cloned();
        if !matches!(
            purpose,
            Some(Purpose::Secret { .. } | Purpose::NewPassword { .. })
        ) && !self.plugin_secret()
        {
            return Vec::new();
        }
        let show_toggle = key.mods.alt && !key.mods.ctrl && matches!(key.code, KeyCode::Char('s'));
        if show_toggle {
            let mut now_shown = false;
            if let Some(Dialog::Secret { shown, .. }) = self.top_dialog_mut() {
                *shown = !*shown;
                now_shown = *shown;
            }
            let line = if now_shown {
                catalog::SECRET_SHOWN
            } else {
                catalog::SECRET_HIDDEN
            };
            return vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))];
        }
        if let Some(c) = typed(&key) {
            if let Some(Dialog::Secret { typed, .. }) = self.top_dialog_mut() {
                typed.push(c);
            }
            return vec![Effect::Redraw(Redraw::All)];
        }
        match key.code {
            KeyCode::Backspace if key.mods.is_empty() => {
                if let Some(Dialog::Secret { typed, .. }) = self.top_dialog_mut() {
                    typed.pop();
                }
                vec![Effect::Redraw(Redraw::All)]
            }
            KeyCode::Enter if key.mods.is_empty() => match purpose {
                Some(Purpose::Secret { ask }) => self.secret_enter(ask),
                _ => self.accept(),
            },
            KeyCode::Escape => self.dialog_escape(),
            _ => {
                let layers = [Layer::Dialog, Layer::Global];
                match self.keymap.lookup(&layers, &key) {
                    Lookup::Command(id) if id == ids::SAY_STATUS || id == ids::HELP => {
                        self.dispatch(&Command::new(id))
                    }
                    _ => Vec::new(),
                }
            }
        }
    }

    fn secret_enter(&mut self, ask: crate::request::AskId) -> Vec<Effect> {
        let typed = match self.dialogs.last() {
            Some((_, Dialog::Secret { typed, .. })) => typed.clone(),
            _ => return Vec::new(),
        };
        if typed.is_empty() {
            return vec![self.say(Message::info(catalog::SECRET_EMPTY))];
        }
        let effects = self.close_top();
        self.request(Request::Answer {
            ask,
            reply: Reply::Secret(Some(typed)),
        });
        effects
    }

    /// A key in a text field.
    fn edit_key(&mut self, key: Key) -> Vec<Effect> {
        let purpose = self.top_purpose().cloned();
        if let Some(c) = typed(&key) {
            if let Some(Dialog::Edit { field, in_list, .. }) = self.top_dialog_mut() {
                field.insert(c);
                *in_list = false;
            }
            return self.field_changed();
        }
        let plain = key.mods.is_empty();
        let in_list = matches!(
            self.dialogs.last(),
            Some((_, Dialog::Edit { in_list: true, .. }))
        );
        let list_open = matches!(
            self.dialogs.last(),
            Some((
                _,
                Dialog::Edit {
                    candidates: Some(_),
                    ..
                }
            ))
        );
        match key.code {
            KeyCode::Backspace if plain => {
                if let Some(Dialog::Edit { field, in_list, .. }) = self.top_dialog_mut() {
                    field.backspace();
                    *in_list = false;
                }
                self.field_changed()
            }
            KeyCode::Delete if plain => {
                if let Some(Dialog::Edit { field, .. }) = self.top_dialog_mut() {
                    field.delete();
                }
                self.field_changed()
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End if plain && !in_list => {
                if let Some(Dialog::Edit { field, .. }) = self.top_dialog_mut() {
                    match key.code {
                        KeyCode::Left => {
                            field.left();
                        }
                        KeyCode::Right => {
                            field.right();
                        }
                        KeyCode::Home => field.home(),
                        _ => field.end(),
                    }
                }
                vec![Effect::Redraw(Redraw::All)]
            }
            KeyCode::Tab if plain => self.complete(),
            KeyCode::Down if plain => {
                if list_open {
                    if let Some((
                        _,
                        Dialog::Edit {
                            in_list,
                            candidates: Some(list),
                            ..
                        },
                    )) = self.dialogs.last_mut()
                    {
                        if *in_list {
                            list.move_focus(Move::Down);
                        } else {
                            *in_list = true;
                        }
                    }
                    return self.list_focus_said();
                }
                self.history_step(false)
            }
            KeyCode::Up if plain => {
                if in_list {
                    let mut back_to_field = false;
                    if let Some((
                        _,
                        Dialog::Edit {
                            in_list,
                            candidates: Some(list),
                            ..
                        },
                    )) = self.dialogs.last_mut()
                    {
                        if list.focus_index() == Some(0) {
                            *in_list = false;
                            back_to_field = true;
                        } else {
                            list.move_focus(Move::Up);
                        }
                    }
                    if back_to_field {
                        let said = match self.dialogs.last() {
                            Some((_, Dialog::Edit { field, .. })) => field.spoken(),
                            _ => String::new(),
                        };
                        return vec![
                            Effect::Redraw(Redraw::All),
                            Effect::Announce(Message::status(said)),
                        ];
                    }
                    return self.list_focus_said();
                }
                if matches!(purpose, Some(Purpose::Palette)) && list_open {
                    return Vec::new();
                }
                self.history_step(true)
            }
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                if plain && in_list =>
            {
                let movement = match key.code {
                    KeyCode::PageUp => Move::PageUp,
                    KeyCode::PageDown => Move::PageDown,
                    KeyCode::Home => Move::Home,
                    _ => Move::End,
                };
                if let Some(list) = self.dialog_list_mut() {
                    list.move_focus(movement);
                }
                self.list_focus_said()
            }
            KeyCode::Enter if plain => self.accept(),
            KeyCode::Escape => self.dialog_escape(),
            _ => {
                let layers = [Layer::Dialog, Layer::Global];
                match self.keymap.lookup(&layers, &key) {
                    Lookup::Command(id)
                        if id == ids::SAY_STATUS
                            || id == ids::HELP
                            || id == ids::QUIT
                            || id == ids::CANCEL =>
                    {
                        self.dispatch(&Command::new(id))
                    }
                    _ => Vec::new(),
                }
            }
        }
    }

    /// Up or Down with the list closed: the prompt's history.
    fn history_step(&mut self, back: bool) -> Vec<Effect> {
        let key = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => field.history_key().map(str::to_owned),
            _ => None,
        };
        let Some(key) = key else {
            return Vec::new();
        };
        let history = self.history_of(&key);
        let mut moved = false;
        let mut said = String::new();
        if let Some(Dialog::Edit { field, .. }) = self.top_dialog_mut() {
            moved = if back {
                field.history_back(&history)
            } else {
                field.history_forward(&history)
            };
            said = field.spoken();
        }
        if !moved {
            let edge = if back {
                "No earlier entry"
            } else {
                "No later entry"
            };
            return vec![self.say(Message::status(edge))];
        }
        let mut effects = vec![
            Effect::Redraw(Redraw::All),
            Effect::Announce(Message::status(said)),
        ];
        effects.extend(self.field_changed_quietly());
        effects
    }

    /// After the field's text changed: the live filter narrows the panel;
    /// the palette and the place lists narrow their candidates. Nothing is
    /// said while typing but a count on the status line.
    fn field_changed(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw(Redraw::All)];
        effects.extend(self.field_changed_quietly());
        effects
    }

    fn field_changed_quietly(&mut self) -> Vec<Effect> {
        let text = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => field.text().to_owned(),
            _ => return Vec::new(),
        };
        match self.top_purpose().cloned() {
            Some(Purpose::Filter(side, _)) => {
                if let Some((
                    _,
                    Dialog::Edit {
                        candidates,
                        in_list,
                        ..
                    },
                )) = self.dialogs.last_mut()
                {
                    *candidates = None;
                    *in_list = false;
                }
                self.filter_typed(side, &text)
            }
            Some(Purpose::Palette | Purpose::Places(_) | Purpose::PickDestination { .. }) => {
                self.refresh_candidates()
            }
            Some(Purpose::Keys(purpose)) if purpose.is_list() => self.keys_list_typed(),
            // A plugin's prompt with a plugin completer (W4d).
            Some(Purpose::Plugin(_)) => {
                if let Some((
                    _,
                    Dialog::Edit {
                        candidates,
                        in_list,
                        ..
                    },
                )) = self.dialogs.last_mut()
                {
                    *candidates = None;
                    *in_list = false;
                }
                self.plugin_field_changed(&text)
            }
            Some(Purpose::Organize(wants)) if wants.list_always() => self.refresh_candidates(),
            _ => {
                if let Some((
                    _,
                    Dialog::Edit {
                        candidates,
                        in_list,
                        ..
                    },
                )) = self.dialogs.last_mut()
                    && candidates.is_some()
                    && !matches!(
                        self.opens.last().map(|o| &o.purpose),
                        Some(Purpose::TypedPlan(..))
                    )
                {
                    *candidates = None;
                    *in_list = false;
                }
                Vec::new()
            }
        }
    }

    /// Rebuilds the always-open lists of the palette and the place lists
    /// from the field's text, and says the count on the status line.
    pub(super) fn refresh_candidates(&mut self) -> Vec<Effect> {
        let text = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => field.text().to_owned(),
            _ => return Vec::new(),
        };
        let list = match self.top_purpose().cloned() {
            Some(Purpose::Palette) => self.palette.list_with_extra(
                &text,
                &self.keymap,
                Some(&self.menus),
                &crate::plugins::registry::palette_entries(&self.plugins.catalog),
            ),
            Some(Purpose::Places(side)) => {
                let all = self.place_candidates(side, false);
                let found: Vec<Candidate> = all
                    .into_iter()
                    .filter(|c| terms_in_order(&text, &c.text) || terms_in_order(&text, &c.kind))
                    .collect();
                let mut list = candidate_list("Places", &found);
                // Bookmarks say their kind first (W3l), and sites (lane
                // A3); the other places their own name first (W3a).
                self.bookmark_rows_first(&mut list);
                self.site_rows_first(&mut list);
                self.volume_rows_first(&mut list);
                self.place_rows_first(&mut list);
                list
            }
            Some(Purpose::PickDestination { side, .. }) => {
                let all = self.place_candidates(side, true);
                let found: Vec<Candidate> = all
                    .into_iter()
                    .filter(|c| terms_in_order(&text, &c.text))
                    .collect();
                let mut list = candidate_list("Destinations", &found);
                self.bookmark_rows_first(&mut list);
                self.site_rows_first(&mut list);
                self.volume_rows_first(&mut list);
                self.place_rows_first(&mut list);
                list
            }
            Some(Purpose::Organize(wants)) => match self.organize_candidates(&wants, &text) {
                Some(list) => list,
                None => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        let shown = list.len() as u64;
        // The palette says its first match with its key, so the key is
        // heard while typing, before Down enters the list (answer 49).
        let said = match (self.top_purpose(), list.focused()) {
            (Some(Purpose::Palette), Some(first)) if !text.is_empty() => {
                format!("{}. {}", first.spoken(), count(shown, "match", "matches"))
            }
            _ => count(shown, "match", "matches"),
        };
        if let Some((_, Dialog::Edit { candidates, .. })) = self.dialogs.last_mut() {
            *candidates = Some(list);
        }
        vec![
            Effect::Redraw(Redraw::All),
            Effect::Announce(Message::status(said)),
        ]
    }

    /// Tab in a field: the single match, or the longest common start, and
    /// what it did said ("completed to Downloads", "4 matches, Down arrow
    /// lists them"). A path field asks for its folder's listing first.
    fn complete(&mut self) -> Vec<Effect> {
        // A second Tab, with the list the first one opened and the text as
        // it left it, moves into the list (plan section 5.19).
        let tabbed = self.tabbed.take();
        if let Some((
            _,
            Dialog::Edit {
                field,
                candidates: Some(list),
                in_list,
            },
        )) = self.dialogs.last_mut()
            && !*in_list
            && !list.is_empty()
            && tabbed.as_deref() == Some(field.text())
        {
            *in_list = true;
            return self.list_focus_said();
        }
        let (source, text, stem) = match self.dialogs.last() {
            Some((_, Dialog::Edit { field, .. })) => (
                field.source().clone(),
                field.text().to_owned(),
                field.stem().to_owned(),
            ),
            _ => return Vec::new(),
        };
        let candidates = match source {
            CompletionSource::Paths => {
                let side = match self.top_purpose() {
                    Some(Purpose::GoPath(side) | Purpose::Relink { side, .. }) => *side,
                    _ => self.active,
                };
                let folder_text = &text[..text.len() - stem.len()];
                let folder = if folder_text.is_empty() {
                    Ok(self.panel(side).place().clone())
                } else {
                    self.typed_place(side, folder_text)
                };
                let folder = match folder {
                    Ok(folder) => folder,
                    Err(message) => return vec![self.say(message)],
                };
                let cached = self
                    .completing
                    .as_ref()
                    .filter(|c| c.folder == folder)
                    .and_then(|c| c.entries.clone());
                match cached {
                    Some(entries) => self.path_candidates(&entries, &stem),
                    None => {
                        let request = self.ids.request();
                        self.completing = Some(Completing {
                            request,
                            folder: folder.clone(),
                            entries: None,
                        });
                        return vec![Effect::List {
                            request,
                            place: folder,
                        }];
                    }
                }
            }
            CompletionSource::Places => {
                let side = self.active;
                self.place_candidates(side, false)
            }
            CompletionSource::Commands => crate::entry::command_candidates(&self.keymap),
            CompletionSource::History => {
                let key = match self.dialogs.last() {
                    Some((_, Dialog::Edit { field, .. })) => field.history_key().map(str::to_owned),
                    _ => None,
                };
                key.map(|k| crate::entry::history_candidates(&self.history_of(&k)))
                    .unwrap_or_default()
            }
            CompletionSource::Values(values) => crate::entry::value_candidates(&values),
            CompletionSource::Filter => {
                let cursor = match self.dialogs.last() {
                    Some((_, Dialog::Edit { field, .. })) => field.cursor(),
                    _ => 0,
                };
                let (start, candidates) = self.filter_completions(&text, cursor);
                if let Some(Dialog::Edit { field, .. }) = self.top_dialog_mut() {
                    field.set_stem_start(start);
                }
                candidates
            }
            CompletionSource::Labels => crate::entry::label_candidates(&self.organize.labels),
            CompletionSource::Bookmarks => {
                crate::entry::bookmark_candidates(&self.organize.bookmarks)
            }
            CompletionSource::None => {
                return vec![self.say(Message::status("No completion here"))];
            }
        };
        self.complete_with(&candidates)
    }

    fn path_candidates(&self, entries: &[diroxide_core::Entry], stem: &str) -> Vec<Candidate> {
        let mut folders: Vec<Candidate> = Vec::new();
        let mut files: Vec<Candidate> = Vec::new();
        for entry in entries {
            if entry.hidden && !self.active_panel().shows_hidden() && !stem.starts_with('.') {
                continue;
            }
            let kind = entry
                .type_name
                .clone()
                .unwrap_or_else(|| entry.kind.word().to_owned());
            let candidate = Candidate::new(entry.name.clone(), kind);
            if entry.is_folder_like() {
                folders.push(candidate);
            } else {
                files.push(candidate);
            }
        }
        folders.sort_by(|a, b| crate::panel::natural_cmp(&a.text, &b.text));
        files.sort_by(|a, b| crate::panel::natural_cmp(&a.text, &b.text));
        folders.extend(files);
        let home = self.home.as_ref().map(ToString::to_string);
        if let Some(expansion) = crate::entry::expansion_candidate(stem, home.as_deref(), |name| {
            std::env::var(name).ok()
        }) {
            folders.insert(0, expansion);
        }
        folders
    }

    fn complete_with(&mut self, candidates: &[Candidate]) -> Vec<Effect> {
        let mut result = Completion::NoMatch;
        let mut matches = Vec::new();
        if let Some(Dialog::Edit { field, .. }) = self.top_dialog_mut() {
            matches = field.matches(candidates);
            result = field.complete(candidates);
        }
        let sentence = result.sentence();
        if let Some((
            _,
            Dialog::Edit {
                field,
                candidates,
                in_list,
            },
        )) = self.dialogs.last_mut()
        {
            match &result {
                Completion::Several(_) | Completion::Extended { .. } => {
                    *candidates = Some(candidate_list(&sentence, &matches));
                    *in_list = false;
                    self.tabbed = Some(field.text().to_owned());
                }
                _ => {
                    *candidates = None;
                    *in_list = false;
                }
            }
        }
        vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::info(sentence)),
        ]
    }

    /// The listing a path completion asked for arrived.
    pub(super) fn completion_listed(
        &mut self,
        result: Result<Vec<diroxide_core::Entry>, diroxide_vfs::VfsError>,
    ) -> Vec<Effect> {
        match result {
            Ok(mut entries) => {
                for entry in &mut entries {
                    if entry.type_name.is_none() {
                        entry.type_name = self.types.short_name(entry);
                    }
                }
                if let Some(completing) = self.completing.as_mut() {
                    completing.entries = Some(entries.clone());
                }
                let stem = match self.dialogs.last() {
                    Some((_, Dialog::Edit { field, .. })) => field.stem().to_owned(),
                    _ => return Vec::new(),
                };
                let candidates = self.path_candidates(&entries, &stem);
                self.complete_with(&candidates)
            }
            Err(error) => {
                self.completing = None;
                let message = Message::from_error("complete", "the path", &error);
                vec![self.say(message)]
            }
        }
    }

    /// An answer a frontend sends for the dialog on top (the GUI and
    /// the server answer this way; the terminal sends keys).
    pub(super) fn answer(
        &mut self,
        dialog: diroxide_core::DialogId,
        answer: Answer,
    ) -> Vec<Effect> {
        if self.dialogs.last().map(|(id, _)| *id) != Some(dialog) {
            return Vec::new();
        }
        match answer {
            Answer::Yes => self.confirm_yes(),
            Answer::No | Answer::Cancel => self.dialog_escape(),
            Answer::Text(text) => {
                if let Some(Dialog::Edit { field, in_list, .. }) = self.top_dialog_mut() {
                    field.set_text(text);
                    *in_list = false;
                }
                self.accept()
            }
            Answer::Chosen(id) => {
                if let Some(list) = self.dialog_list_mut() {
                    list.focus_id(&id);
                }
                if let Some((_, Dialog::Edit { in_list, .. })) = self.dialogs.last_mut() {
                    *in_list = true;
                }
                self.accept()
            }
            Answer::Secret(secret) => {
                if let Some(Dialog::Secret { typed, .. }) = self.top_dialog_mut() {
                    *typed = secret;
                }
                match self.top_purpose().cloned() {
                    Some(Purpose::Secret { ask }) => self.secret_enter(ask),
                    Some(Purpose::NewPassword { .. } | Purpose::Plugin(_)) => self.accept(),
                    _ => Vec::new(),
                }
            }
        }
    }

    /// Runs a command line in the user's shell, in the panel's folder.
    pub(super) fn run_line(&mut self, side: Side, line: &str) -> Vec<Effect> {
        if line.trim().is_empty() {
            return vec![self.say(Message::info("No command typed. Nothing ran."))];
        }
        self.remember(state::keys::RUN_COMMAND, line);
        let spawn = self.spawn_for(side, SpawnKind::CommandLine(line.to_owned()));
        self.request(Request::Spawn(Box::new(spawn)));
        Vec::new()
    }

    /// The program's context: the folders, the cursor and the selection.
    pub(super) fn spawn_for(&self, side: Side, kind: SpawnKind) -> Spawn {
        let panel = self.panel(side);
        Spawn {
            kind,
            folder: panel.place().clone(),
            other: self.panel(side.other()).place().clone(),
            cursor: panel.focused_place(),
            selection: panel.targets(),
        }
    }
}
