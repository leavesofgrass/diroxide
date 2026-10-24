//! The trash browser (R25 3.3; lane A3, commit 89; answer 98): what is in
//! the system trash, newest first, each row its name first ("report.pdf,
//! deleted Tuesday, D:\work"), from the Go menu, the palette and the place
//! list's Trash row.
//!
//! - **Space** marks rows; **Enter** offers the actions for the marked
//!   rows, or the focused one: Restore, Delete for good, Empty the trash.
//! - **Restore** puts each item back where it was, never over a name that
//!   is taken: the plan leaves a taken name out and says so, the question
//!   starts on No ("Restore 3 items? No is the default. y or n"), and the
//!   result counts what was taken ("Restored 2 items. 1 name was taken.").
//! - **Delete for good** lists every item first, starts on No, and asks
//!   for the typed count over 1,000, as every permanent delete does;
//!   **Empty the trash** is the same over everything in it.
//! - **macOS** says "The trash cannot be listed on this system." and opens
//!   nothing.
//! - The listing runs off the input thread with a time limit; the list
//!   follows each restore or delete, the focus kept by item.

use diroxide_core::{
    Command, Effect, JobKind, JobRequest, Key, Layer, ListModel, ListRow, Message, Redraw,
};
use diroxide_vfs::TrashedItem;

use super::{App, Purpose, Then};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::keymap::Lookup;
use crate::request::Request;

/// What the app keeps for the trash browser.
#[derive(Debug, Default)]
pub(super) struct TrashState {
    /// The items the last listing gave, newest first.
    items: Vec<TrashedItem>,
    /// Whether the browser opens when the listing comes (otherwise an
    /// open browser follows it).
    opening: bool,
}

/// What a dialog of the trash browser is for.
#[derive(Clone, Debug)]
pub(super) enum Wants {
    /// The browser itself.
    List,
    /// The actions on the chosen items, by their row ids.
    Actions {
        /// The chosen rows.
        rows: Vec<String>,
    },
}

/// A row's id: the trash's own id of the item, so the focus and the marks
/// follow it from one listing to the next.
fn row_id(item: &TrashedItem) -> String {
    item.id.as_os_str().to_string_lossy().into_owned()
}

impl App {
    /// The trash browser's commands; `None` for commands that are not its.
    pub(super) fn trash_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let id = &command.id;
        if *id != ids::TRASH_BROWSER && *id != ids::TRASH_EMPTY {
            return None;
        }
        if !self.dialogs.is_empty() && !self.trash_on_top() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        if *id == ids::TRASH_EMPTY {
            return Some(self.empty_trash());
        }
        self.trash.opening = true;
        self.request(Request::ListTrash);
        Some(vec![self.say(Message::status("Reading the trash"))])
    }

    /// Whether the trash browser is the dialog on top.
    fn trash_on_top(&self) -> bool {
        matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Trash(_))
        )
    }

    /// Where the open browser is in the dialogs.
    fn trash_list_index(&self) -> Option<usize> {
        self.opens
            .iter()
            .rposition(|o| matches!(&o.purpose, Purpose::Trash(w) if matches!(**w, Wants::List)))
    }

    /// The browser's rows, newest first.
    fn trash_list(&self) -> ListModel {
        let clock = std::sync::Arc::clone(&self.clock);
        let rows = self
            .trash
            .items
            .iter()
            .map(|item| ListRow::new(row_id(item), item.words(clock.as_ref())))
            .collect();
        let title = format!(
            "Trash, {}",
            diroxide_core::format::count(self.trash.items.len() as u64, "item", "items")
        );
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_empty_text("The trash is empty.");
        list.set_help(
            "Space marks, Enter offers Restore, Delete for good and Empty the trash, Escape closes.",
        );
        list
    }

    /// The trash was listed: the browser opens, or an open one follows,
    /// the focus and the marks kept by item. A trash that cannot be
    /// listed says so whole.
    pub(super) fn trash_listed(&mut self, result: Result<Vec<TrashedItem>, String>) -> Vec<Effect> {
        let opening = std::mem::take(&mut self.trash.opening);
        let items = match result {
            Ok(items) => items,
            Err(sentence) => {
                return if opening {
                    vec![self.say(Message::info(sentence))]
                } else {
                    Vec::new()
                };
            }
        };
        self.trash.items = items;
        if let Some(index) = self.trash_list_index() {
            let fresh = self.trash_list();
            if let Some((_, Dialog::Choose { list, .. })) = self.dialogs.get_mut(index) {
                let focus = list.focused().map(|r| r.id.clone());
                let marked: Vec<String> =
                    list.selected_ids().into_iter().map(str::to_owned).collect();
                let mut fresh = fresh;
                fresh.select_matching(|row| marked.contains(&row.id));
                if let Some(focus) = focus {
                    fresh.focus_id(&focus);
                }
                *list = fresh;
            }
            return vec![Effect::Redraw(Redraw::All)];
        }
        if !opening {
            return Vec::new();
        }
        if self.trash.items.is_empty() {
            return vec![self.say(Message::info("The trash is empty."))];
        }
        let list = self.trash_list();
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Trash(Box::new(Wants::List)),
        )
    }

    /// The items the rows name, in the listing's order.
    fn trash_items(&self, rows: &[String]) -> Vec<TrashedItem> {
        self.trash
            .items
            .iter()
            .filter(|item| rows.contains(&row_id(item)))
            .cloned()
            .collect()
    }

    /// Enter in one of the browser's dialogs.
    pub(super) fn trash_accept(&mut self, wants: Wants, row: Option<String>) -> Vec<Effect> {
        match wants {
            Wants::List => {
                let rows: Vec<String> = match self.dialogs.last() {
                    Some((_, Dialog::Choose { list, .. })) => {
                        let marked: Vec<String> =
                            list.selected_ids().into_iter().map(str::to_owned).collect();
                        if marked.is_empty() {
                            row.into_iter().collect()
                        } else {
                            marked
                        }
                    }
                    _ => Vec::new(),
                };
                self.trash_actions(rows)
            }
            Wants::Actions { rows } => {
                let Some(action) = row else {
                    return Vec::new();
                };
                let mut effects = self.close_top();
                effects.extend(match action.as_str() {
                    "restore" => self.plan_trash(JobKind::Restore, &rows),
                    "purge" => self.plan_trash(JobKind::Purge, &rows),
                    "empty" => self.empty_trash(),
                    _ => Vec::new(),
                });
                effects
            }
        }
    }

    /// The actions for the chosen items: Restore and Delete for good,
    /// disabled with their reason when nothing is chosen, then Empty the
    /// trash.
    fn trash_actions(&mut self, rows: Vec<String>) -> Vec<Effect> {
        let chosen = rows.len() as u64;
        let what = diroxide_core::format::count(chosen, "item", "items");
        let mut restore = ListRow::new("restore", format!("Restore {what}"));
        let mut purge = ListRow::new("purge", format!("Delete {what} for good"));
        if chosen == 0 {
            restore = ListRow::new("restore", "Restore");
            purge = ListRow::new("purge", "Delete for good");
            for row in [&mut restore, &mut purge] {
                row.enabled = false;
                row.details.push("Space marks an item first.".to_owned());
            }
        }
        let mut empty = ListRow::new("empty", "Empty the trash");
        if self.trash.items.is_empty() {
            empty.enabled = false;
            empty.details.push("The trash is empty.".to_owned());
        }
        let title = if chosen == 1 {
            self.trash_items(&rows)
                .first()
                .map_or_else(|| "Trash".to_owned(), |item| item.name.clone())
        } else {
            format!("Trash, {what} marked")
        };
        let mut list = ListModel::new(title).with_rows(vec![restore, purge, empty]);
        list.set_help("Up and Down choose, Enter does it, Escape goes back to the trash.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::ACCEPT),
            },
            Purpose::Trash(Box::new(Wants::Actions { rows })),
        )
    }

    /// A restore or a delete for good of the chosen items: planned off the
    /// input thread, then the plan's question, starting on No, as every
    /// job's.
    fn plan_trash(&mut self, kind: JobKind, rows: &[String]) -> Vec<Effect> {
        let items = self.trash_items(rows);
        if items.is_empty() {
            return vec![self.say(Message::status("Space marks an item first."))];
        }
        let job = self.next_job_id();
        let sources = items.iter().map(|item| item.original.clone()).collect();
        let request = JobRequest::new(job, kind, sources);
        self.note_plan_flow(&request, true);
        self.request(Request::PlanTrash { job, kind, items });
        Vec::new()
    }

    /// Empty the trash: every item, listed first, behind the question
    /// that starts on No and the typed count over 1,000.
    fn empty_trash(&mut self) -> Vec<Effect> {
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Purge, Vec::new());
        self.note_plan_flow(&request, true);
        self.request(Request::PlanEmptyTrash { job });
        Vec::new()
    }

    /// A restore or a delete for good ended: an open browser lists the
    /// trash again.
    pub(super) fn trash_job_ended(&mut self, kind: JobKind) {
        if matches!(kind, JobKind::Restore | JobKind::Purge) && self.trash_list_index().is_some() {
            self.request(Request::ListTrash);
        }
    }

    /// Space in the browser marks or unmarks the focused item and moves
    /// on.
    pub(super) fn trash_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let Some(Purpose::Trash(wants)) = self.opens.last().map(|o| &o.purpose) else {
            return None;
        };
        if !matches!(**wants, Wants::List) {
            return None;
        }
        let layers = [Layer::Panel, Layer::List, Layer::Global];
        let toggles = matches!(
            self.keymap.lookup(&layers, key),
            Lookup::Command(id) if id == ids::SELECT_TOGGLE
        );
        if !toggles {
            return None;
        }
        let state = match self.dialogs.last_mut() {
            Some((_, Dialog::Choose { list, .. })) => {
                let state = list.toggle_selected();
                list.move_focus(diroxide_core::Move::Down);
                state
            }
            _ => None,
        };
        let word = match state {
            Some(true) => "Marked",
            Some(false) => "Unmarked",
            None => return Some(Vec::new()),
        };
        Some(vec![
            Effect::Redraw(Redraw::All),
            self.say(Message::status(word)),
        ])
    }

    /// Escape in the browser: it closes; nothing changed.
    pub(super) fn trash_escape(&mut self, wants: Wants) -> Vec<Effect> {
        match wants {
            Wants::List => vec![self.say(Message::info("Trash closed."))],
            Wants::Actions { .. } => Vec::new(),
        }
    }

    /// The flow of a job planned outside [`App::plan_job`]: its facts for
    /// the Jobs list, and the plan shown and confirmed when it comes.
    fn note_plan_flow(&mut self, request: &JobRequest, confirm: bool) {
        self.note_job_facts(request);
        self.flows.insert(
            request.id,
            super::Flow {
                side: self.active,
                note: None,
                then: Then::Report,
                confirm,
                kind: request.kind,
            },
        );
    }
}
