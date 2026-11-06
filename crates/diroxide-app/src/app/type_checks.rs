//! The background type check in the app (alpha 4, lane TA, R30 commit
//! 29; R28 4.3). A panel's rows show the types their names give at once
//! ("type pending" only where a name says nothing); then the app asks the
//! runtime to read the first bytes of the folder's files, the rows nearest
//! the focus first, and the rows take what the bytes say as the answers
//! come, quietly: a row whose type changes is drawn again and never
//! announced (R28 4.6, risk 5).
//!
//! The queue follows the focus: when it moves more than half a page from
//! where the check began, the app asks again from there, and the readers
//! of the old order stop (the runtime counts the checks of each panel, a
//! generation, and a reader of an older one leaves). A new listing asks
//! anew; a refresh keeps the types of the files that did not change.

use std::sync::Arc;

use diroxide_core::{Effect, Side};
use diroxide_filetype::CheckSkip;

use super::App;
use crate::request::{Request, SharedClock};
use crate::typing::TypeAnswer;

/// What the app asked of each panel's check.
#[derive(Clone, Debug, Default)]
pub(super) struct TypeChecks {
    asked: [Option<Asked>; 2],
}

/// One panel's check as last asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Asked {
    /// The panel's entries it was asked for.
    listing: u64,
    /// The focused row when it was asked.
    focus: usize,
    /// The files still to be answered.
    left: usize,
}

impl App {
    /// After each event: a panel whose entries changed has its files
    /// typed by their first bytes, and a focus that moved more than half
    /// a page from where its check began asks again from there.
    pub(super) fn follow_type_checks(&mut self) {
        for side in Side::BOTH {
            let panel = self.panel(side);
            if panel.is_loading() || !panel.is_listed() {
                continue;
            }
            let listing = panel.entries_generation();
            let focus = panel.list().focus_index().unwrap_or(0);
            let half_page = (panel.list().page_size() / 2).max(1);
            if let Some(asked) = self.type_checks.asked[side.index()]
                && asked.listing == listing
                && (asked.left == 0 || focus.abs_diff(asked.focus) <= half_page)
            {
                continue;
            }
            self.ask_type_checks(side);
        }
    }

    /// Asks the runtime to type a panel's unchecked files, nearest the
    /// focus first; with none left, stops a check still running.
    fn ask_type_checks(&mut self, side: Side) {
        let panel = self.panel(side);
        let items = panel.type_queue();
        let listing = panel.entries_generation();
        let focus = panel.list().focus_index().unwrap_or(0);
        let urgent = panel.list().page_size().max(1).min(items.len());
        let folder = panel.place().clone();
        let running = self.type_checks.asked[side.index()].is_some_and(|a| a.left > 0);
        self.type_checks.asked[side.index()] = Some(Asked {
            listing,
            focus,
            left: items.len(),
        });
        if items.is_empty() {
            if running {
                self.request(Request::StopTypes(side));
            }
            return;
        }
        self.request(Request::CheckTypes {
            side,
            listing,
            folder,
            items,
            urgent,
            types: Arc::clone(&self.types),
            clock: SharedClock(Arc::clone(&self.clock)),
        });
    }

    /// A batch of answers: the rows take their types, drawn again when a
    /// word changed; answers for entries replaced since are dropped.
    pub(super) fn types_checked(
        &mut self,
        side: Side,
        listing: u64,
        answers: Vec<TypeAnswer>,
    ) -> Vec<Effect> {
        if self.panel(side).entries_generation() != listing {
            return Vec::new();
        }
        let count = answers.len();
        let types = Arc::clone(&self.types);
        let clock = Arc::clone(&self.clock);
        let changed = self.panels[side.index()].apply_type_answers(answers, &types, clock.as_ref());
        if let Some(asked) = self.type_checks.asked[side.index()].as_mut()
            && asked.listing == listing
        {
            asked.left = asked.left.saturating_sub(count);
        }
        let shown = side == self.active || !self.screen_reader;
        if changed && shown {
            vec![self.redraw_panel(side)]
        } else {
            Vec::new()
        }
    }

    /// The panels type again by name, and check again, after the types or
    /// the folders of `check.skip` changed.
    pub(super) fn retype_panels(&mut self) {
        let skip = CheckSkip::new(&self.settings.check.skip);
        let types = Arc::clone(&self.types);
        let clock = Arc::clone(&self.clock);
        for panel in &mut self.panels {
            panel.set_check_skip(skip.clone());
            panel.set_types(Arc::clone(&types));
            panel.retype(&types, clock.as_ref());
        }
    }

    /// The line when the focused file's name and contents disagree, said
    /// on request: "Name says PNG; contents say JPEG".
    pub(super) fn focused_disagreement(&self) -> Option<String> {
        let panel = self.active_panel();
        let entry = panel.focused_entry()?;
        panel.disagreement(&entry.name).map(str::to_owned)
    }
}
