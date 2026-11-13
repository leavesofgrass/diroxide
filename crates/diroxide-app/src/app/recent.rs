//! Recent files in the app (alpha 5, lane LA): recorded on Enter and F4
//! in a panel (never F3, question 138) while the `recent_files` setting
//! is on, kept in the state folder only, and listed newest first in the
//! general results lister (`App::open_list` with `ListKind::Recent`),
//! over `diroxide_config`'s recent files. Each row says how and when the
//! file was used after its name: "report.md, edited today, 10:00 AM, in
//! D:\docs".

use diroxide_config::RecentFiles;
use diroxide_core::{Command, Message, Place, Side};
use diroxide_search::SearchHit;

use super::searching::{ListKind, ListOpen};
use super::{App, Effect, Purpose};
use crate::lists::ListsAsk;

impl App {
    /// Recent files: the list, newest first, or why there is none.
    pub(super) fn recent_files_list(&mut self, _command: &Command) -> Vec<Effect> {
        if !self.settings.recent_files {
            let line = RecentFiles::default().summary_line(false, self.clock.as_ref());
            return vec![self.say(Message::status(format!("{line} in Settings.")))];
        }
        let side = self.active;
        self.lists_ask(ListsAsk::Recent { side });
        Vec::new()
    }

    /// The recent files were read: they open in the lister, the summary
    /// first ("Recent: 2 files, newest report.md, today").
    pub(super) fn recent_read(
        &mut self,
        side: Side,
        recent: RecentFiles,
        hits: Vec<SearchHit>,
        gone: Vec<Place>,
    ) -> Vec<Effect> {
        let clock = std::sync::Arc::clone(&self.clock);
        let start = recent.summary_line(true, clock.as_ref());
        if recent.items.is_empty() {
            return vec![self.say(Message::status(format!("{start}.")))];
        }
        // How and when, after each name: "edited today, 10:00 AM".
        let notes = recent
            .items
            .iter()
            .filter_map(|item| {
                let row = item.row(clock.as_ref());
                row.details
                    .first()
                    .map(|when| (item.place.clone(), when.clone()))
            })
            .collect();
        let open = ListOpen {
            kind: ListKind::Recent,
            side,
            title: "Recent files".to_owned(),
            root: None,
            hits,
            gone,
            notes,
            start,
            streaming: false,
        };
        self.open_list(open).0
    }

    /// Clear recent files: every one forgotten; no file is touched.
    pub(super) fn clear_recent_files(&mut self, _command: &Command) -> Vec<Effect> {
        self.lists_ask(ListsAsk::ClearRecent);
        Vec::new()
    }

    /// The recent files were forgotten: "Cleared 2 recent files." The
    /// recent files' list closes when it is on the screen.
    pub(super) fn recent_cleared(&mut self, cleared: usize) -> Vec<Effect> {
        let mut effects = Vec::new();
        let showing = self.list_kind() == Some(&ListKind::Recent)
            && self
                .opens
                .last()
                .is_some_and(|open| matches!(open.purpose, Purpose::Results));
        if showing {
            effects.extend(self.close_all());
        }
        effects.push(self.say(Message::done(RecentFiles::cleared_words(cleared))));
        effects
    }
}
