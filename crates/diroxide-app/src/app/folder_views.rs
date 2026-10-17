//! Per-folder views and the History list in the app (plan commit 25).
//!
//! Save view for this folder keeps the panel's sort, filter, hidden
//! files and columns for its folder; entering that folder again applies
//! them and says so after the folder's own line ("Backup, 12 items, in
//! D:\work. Sorted by size, largest first; saved for this folder.");
//! leaving it puts the panel's usual view back, silently. Forget this
//! folder's view takes one out. The History list reads the path jump
//! store: Enter goes to a place, the trash key forgets one, and Forget
//! this place does the same for the active folder.

use std::sync::Arc;

use diroxide_core::{Command, Effect, Message, Place, Redraw, Side};

use super::App;
use super::organize::{Look, Wants};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::panel::place_leaf;
use crate::state;
use crate::views::FolderView;

/// The first letter in lower case, for a sentence that goes on.
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl App {
    /// A panel's view as it is now.
    fn look_of(&self, side: Side) -> Look {
        let panel = self.panel(side);
        Look {
            sort: panel.sort().clone(),
            show_hidden: panel.shows_hidden(),
            columns: panel.columns().to_vec(),
        }
    }

    /// Puts a view on a panel: sort, hidden files and columns, the focus
    /// and the marks kept by id.
    fn put_look(&mut self, side: Side, look: Look) {
        let clock = Arc::clone(&self.clock);
        let panel = &mut self.panels[side.index()];
        panel.set_sort(look.sort, clock.as_ref());
        panel.set_show_hidden(look.show_hidden, clock.as_ref());
        panel.set_columns(look.columns, clock.as_ref());
    }

    /// After a panel moved to a folder: the folder's own view is applied,
    /// and its sentence returned for the folder's line; leaving a folder
    /// that had one puts the panel's usual view back, silently.
    pub(super) fn folder_view_on_entering(&mut self, side: Side) -> Option<String> {
        let place = self.panel(side).place().clone();
        let Some(view) = self.organize.views.get(&place).cloned() else {
            if let Some(usual) = self.organize.usual[side.index()].take() {
                self.put_look(side, usual);
            }
            return None;
        };
        let usual = match self.organize.usual[side.index()].take() {
            Some(usual) => usual,
            None => self.look_of(side),
        };
        let usual_hidden = usual.show_hidden;
        self.organize.usual[side.index()] = Some(usual);
        let columns = if view.columns.is_empty() {
            self.panel(side).columns().to_vec()
        } else {
            view.columns.clone()
        };
        self.put_look(
            side,
            Look {
                sort: view.sort.clone(),
                show_hidden: view.show_hidden,
                columns,
            },
        );
        // The folder's filter, in the filter language (W3a).
        if let Some(text) = &view.filter
            && let Ok(Some(filter)) = self.read_filter(text)
            && filter.panel_problem().is_none()
            && !filter.reads_content()
        {
            let clock = Arc::clone(&self.clock);
            let bookmarks = self.organize.bookmarks.clone();
            self.panels[side.index()].set_filter_and_facts(
                Some(filter),
                Some(bookmarks),
                None,
                clock.as_ref(),
            );
        }
        Some(view.sentence(usual_hidden))
    }

    /// Save view for this folder.
    pub(super) fn save_view(&mut self, side: Side) -> Vec<Effect> {
        let panel = self.panel(side);
        let view = FolderView {
            place: panel.place().clone(),
            sort: panel.sort().clone(),
            filter: panel.filter().map(|p| p.text().to_owned()),
            show_hidden: panel.shows_hidden(),
            columns: panel.columns().to_vec(),
        };
        let leaf = place_leaf(panel.place());
        let mut text = format!(
            "View saved for {leaf}: {}",
            lower_first(&view.sort.describe())
        );
        if let Some(filter) = &view.filter {
            text.push_str(&format!(
                ", filter {}",
                diroxide_core::names::display_name(filter)
            ));
        }
        if self.organize.usual[side.index()].is_none() {
            // The look before this folder's view: what leaving puts back.
            self.organize.usual[side.index()] = Some(self.look_of(side));
        }
        self.organize.views.set(view);
        self.save_views();
        vec![self.say(Message::info(text))]
    }

    /// Forget this folder's view.
    pub(super) fn forget_view(&mut self, side: Side) -> Vec<Effect> {
        let place = self.panel(side).place().clone();
        let leaf = place_leaf(&place);
        if !self.organize.views.forget(&place) {
            let text = format!("{leaf} has no saved view.");
            return vec![self.say(Message::info(text))];
        }
        self.save_views();
        let text = format!("View forgotten for {leaf}. The usual view returns on leaving.");
        vec![self.say(Message::info(text))]
    }

    /// The History list over the path jump store.
    pub(super) fn history_list(&mut self, side: Side) -> Vec<Effect> {
        let list = self.history_model();
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::HISTORY_LIST),
            },
            Wants::History { side },
        )
    }

    /// The History list with its help from the live keys.
    fn history_model(&self) -> diroxide_core::ListModel {
        let mut list = crate::views::history_list(&self.visits, self.clock.as_ref());
        let forget = self.key(&ids::TRASH);
        let help = if forget.is_empty() {
            "Enter goes there, Escape closes.".to_owned()
        } else {
            format!("Enter goes there, {forget} forgets the place, Escape closes.")
        };
        list.set_help(help);
        list
    }

    /// Forget this place: out of the History list and path jump, saved at
    /// once; an open History list loses its row.
    pub(super) fn forget_place(&mut self, place: &Place) -> Vec<Effect> {
        let leaf = place_leaf(place);
        if !self.visits.forget(place) {
            let text = format!("{leaf} is not in the history.");
            return vec![self.say(Message::info(text))];
        }
        let lines = self.visits.to_lines();
        self.histories
            .insert(state::VISITS_KEY.to_owned(), lines.clone());
        self.request(crate::request::Request::SaveHistory {
            key: state::VISITS_KEY.to_owned(),
            entries: lines,
        });
        let list = self.history_model();
        let shown = self.replace_open_list(|w| matches!(w, Wants::History { .. }), list);
        let left = diroxide_core::format::count(self.visits.len() as u64, "place", "places");
        let text = format!("Forgot {leaf}. {left} left.");
        let mut effects = Vec::new();
        if shown {
            effects.push(Effect::Redraw(Redraw::All));
        }
        effects.push(self.say(Message::info(text)));
        effects
    }
}
