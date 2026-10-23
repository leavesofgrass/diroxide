//! Navigation: listing a place into a panel, Enter, Right and Left, up,
//! back and forward with each folder's cursor and marks, home, the other
//! panel, swap, reload, watching, path jump and the go-to-place list, the
//! startup lines and the restored session.

use diroxide_core::format::count;
use diroxide_core::{Effect, Entry, EntryKind, Message, Move, Place, Redraw, RequestId, Side};
use diroxide_vfs::{LocalPlace, VfsError, WatchEvent};

use super::{App, Purpose, capitalized};
use crate::commands::ids;
use crate::entry::{CompletionSource, EntryField};
use crate::panel::{Load, Watching, place_leaf, place_title};
use crate::request::Request;
use crate::state;

/// Whether an entry is a program that Enter would run: by extension on
/// Windows, by the owner's execute bit elsewhere (a folder never is).
pub(super) fn is_program(entry: &Entry) -> bool {
    if entry.kind != EntryKind::File {
        return false;
    }
    if cfg!(windows) {
        let lower = entry.name.to_lowercase();
        [
            ".exe", ".com", ".bat", ".cmd", ".ps1", ".msi", ".vbs", ".js", ".wsf", ".scr", ".lnk",
        ]
        .iter()
        .any(|ext| lower.ends_with(ext))
    } else {
        matches!(entry.attributes, Some(diroxide_core::Attributes::Unix(mode)) if mode & 0o111 != 0)
    }
}

/// A streamed listing in a panel (R25 commit 77): when it began, when its
/// count was last said, and whether the owner stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StreamNote {
    request: RequestId,
    began: diroxide_core::Timestamp,
    said: Option<diroxide_core::Timestamp>,
    stopped: bool,
}

impl App {
    /// Lists a place into a panel, as a new place.
    pub(super) fn open(&mut self, side: Side, place: Place, focus: Option<String>) -> Vec<Effect> {
        self.open_as(side, place, focus, Load::Open)
    }

    /// Lists a place into a panel for a reason (back, forward, reload, a
    /// change).
    pub(super) fn open_as(
        &mut self,
        side: Side,
        place: Place,
        focus: Option<String>,
        how: Load,
    ) -> Vec<Effect> {
        if &place != self.panel(side).place() {
            self.keep_selection(side);
        }
        // A listing still on its way is not waited for any more: it stops,
        // so a slow remote folder does not keep listing for nothing.
        if let Some(old) = self.panel(side).loading_request() {
            self.request(Request::StopListing(old));
        }
        self.streams[side.index()] = None;
        let request = self.ids.request();
        self.panel_mut(side)
            .begin_load_as(request, place.clone(), focus, how);
        let mut effects = Vec::new();
        if side == self.active {
            effects.push(Effect::Redraw(Redraw::TitleLine));
        }
        effects.push(Effect::List { request, place });
        effects
    }

    /// The panel's help from the live keymap, so it never names a key
    /// that is not bound.
    pub(super) fn set_panel_help(&mut self, side: Side) {
        let key = |id| self.keymap.key_text(&id);
        let mut parts = Vec::new();
        let up = key(ids::LIST_UP);
        let down = key(ids::LIST_DOWN);
        if !up.is_empty() && !down.is_empty() {
            parts.push(format!("{up} and {down} move"));
        }
        for (id, words) in [
            (ids::ENTER, "opens"),
            (ids::UP, "goes up"),
            (ids::SELECT_TOGGLE, "selects"),
            (ids::SWITCH_PANEL, "switches panels"),
            (ids::MENU, "opens the menus"),
            (ids::PALETTE, "finds any command"),
        ] {
            let k = key(id);
            if !k.is_empty() {
                parts.push(format!("{k} {words}"));
            }
        }
        let help = format!("{}.", parts.join(", "));
        self.panel_mut(side).list_mut().set_help(help);
    }

    /// Restores the saved session where it matches the places the app
    /// opens with: each panel's sort, hidden files, history and cursor;
    /// the active panel.
    pub(super) fn apply_session(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        for side in Side::BOTH {
            let saved = session.panel(side).clone();
            let panel_place = self.panel(side).place().clone();
            if saved.place.as_ref() != Some(&panel_place) {
                continue;
            }
            let clock = std::sync::Arc::clone(&self.clock);
            let panel = self.panel_mut(side);
            panel.set_history(saved.history.clone());
            panel.set_sort(saved.sort.clone(), clock.as_ref());
            panel.set_show_hidden(saved.show_hidden, clock.as_ref());
            self.restore[side.index()] = saved.focused.clone();
        }
        let active = session.active;
        if session.panel(active).place.as_ref() == Some(self.panel(active).place()) {
            self.active = active;
        }
    }

    pub(super) fn restored_focus(&mut self, side: Side) -> Option<String> {
        self.restore[side.index()].take()
    }

    /// The lines said before anything else: each interrupted job of the
    /// last session, then the config problems, or the first-run welcome.
    pub(super) fn startup_lines(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for job in std::mem::take(&mut self.interrupted) {
            effects.push(self.say(Message::warning(job.sentence())));
            self.request(Request::Settle(job.job));
        }
        if let Some(line) = crate::messages::startup_problems(&self.problems) {
            effects.push(self.say(line));
        } else if self.first_run {
            let help = self.key(&ids::HELP);
            let palette = self.key(&ids::PALETTE);
            let quit = self.key(&ids::QUIT);
            let mut welcome = if help.is_empty() {
                format!("diroxide {}. Help is in the Help menu.", release_words())
            } else {
                format!("diroxide {}. {help} for help.", release_words())
            };
            // The owner's finding from the alpha.1 terminal: say how to
            // quit beside the help key, and name the key bar.
            if quit.is_empty() {
                welcome.push_str(" Quit is in the File menu.");
            } else {
                welcome.push_str(&format!(" {quit} quits."));
            }
            effects.push(self.say(Message::info(welcome)));
            if self.settings.key_bar && !self.settings.compact {
                let bar = Message::info("The key bar on the last line names the keys.");
                effects.push(self.say(bar));
            }
            let layout = self.settings.layout.word();
            let line = if palette.is_empty() {
                format!("Layout: {layout}. Change it in the View menu.")
            } else {
                format!("Layout: {layout}. {palette}, Layout.")
            };
            effects.push(self.say(Message::info(line)));
        }
        effects
    }

    pub(super) fn listed(
        &mut self,
        request: RequestId,
        place: &Place,
        result: Result<Vec<Entry>, VfsError>,
    ) -> Vec<Effect> {
        if self
            .completing
            .as_ref()
            .is_some_and(|c| c.request == request)
        {
            return self.completion_listed(result);
        }
        let Some(side) = Side::BOTH
            .into_iter()
            .find(|s| self.panel(*s).waits_for(request))
        else {
            return Vec::new();
        };
        if result.is_ok() {
            self.take_archive_facts(side, request, place);
        } else {
            self.pending_archive.retain(|(r, _)| *r != request);
        }
        match result {
            Ok(entries) => self.listed_ok(side, entries),
            Err(error) => self.listed_err(side, place, &error),
        }
    }

    fn listed_ok(&mut self, side: Side, mut entries: Vec<Entry>) -> Vec<Effect> {
        // The labels each entry carries (W3l).
        if let Some(place) = self.panel(side).loading_place().cloned() {
            self.organize.labels.apply_to_entries(&place, &mut entries);
        }
        let how = self.panel(side).loading_how().unwrap_or(Load::Open);
        if how == Load::Stream {
            return self.stream_listed(side, entries);
        }
        let clock = std::sync::Arc::clone(&self.clock);
        let refreshed = self.panels[side.index()].finish(entries, &self.types, clock.as_ref());
        let active = side == self.active;
        let mut effects = Vec::new();
        if active || !self.screen_reader {
            effects.push(self.redraw_panel(side));
        }
        if refreshed.moved {
            // The folder's own view, or the usual one back (W3l).
            let view_line = self.folder_view_on_entering(side);
            let place = self.panel(side).place().clone();
            if place.kind() == diroxide_core::PlaceKind::Local {
                let now = self.now().unix_seconds();
                let home = self.home.clone();
                self.visits.record(&place, now, home.as_ref());
            }
            self.request(Request::Watch {
                side,
                place: Some(place),
            });
            if active && let Some(mut line) = self.archive_landing(side) {
                // Inside an archive: "Opened course.zip: 10 items, 48 KB."
                // at its root, "week1, 3 items, in course.zip" below it.
                if let Some(view) = &view_line {
                    line = format!("{}. {view}", line.trim_end_matches('.'));
                }
                effects.push(Effect::Redraw(Redraw::TitleLine));
                effects.push(self.say(Message::info(line)));
                return effects;
            }
            if active {
                // The leaf and the count first, the folder it is in after:
                // "Backup, 12 items, in D:\work".
                let panel = self.panel(side);
                let mut text = format!(
                    "{}, {}",
                    place_leaf(panel.place()),
                    count(panel.list().len() as u64, "item", "items")
                );
                if let Some(parent) = panel.place().parent() {
                    text.push_str(&format!(
                        ", in {}",
                        diroxide_core::names::display_name(&parent.to_string())
                    ));
                }
                if let Some(view) = view_line {
                    text.push_str(&format!(". {view}"));
                }
                effects.push(Effect::Redraw(Redraw::TitleLine));
                effects.push(self.say(Message::info(text)));
            }
            return effects;
        }
        if !active {
            return effects;
        }
        let mut lines = Vec::new();
        if refreshed.rows.focus_moved {
            let now_on = self.panel(side).list().announce_focus();
            match &refreshed.focus_was {
                Some(was) => lines.push(format!("Focus moved: {was} is gone. On {now_on}")),
                None => lines.push(format!("Focus moved. On {now_on}")),
            }
        }
        if refreshed.rows.marks_lost > 0 {
            lines.push(format!(
                "{} gone",
                count(
                    refreshed.rows.marks_lost as u64,
                    "marked item is",
                    "marked items are"
                )
            ));
        }
        // After the app's own job its result already said what changed.
        let changed = refreshed.added > 0 || refreshed.removed > 0;
        let changes = (changed && how != Load::AfterJob).then(|| {
            format!(
                "Folder changed: {} added, {} removed",
                refreshed.added, refreshed.removed
            )
        });
        if !lines.is_empty() {
            let mut text = lines.join(". ");
            if let Some(changes) = &changes {
                text = format!("{changes}. {text}");
            }
            effects.push(self.say(Message::info(text)));
        } else if how == Load::Reload {
            let text = changes.unwrap_or_else(|| {
                format!(
                    "Reloaded: {}",
                    count(self.panel(side).list().len() as u64, "item", "items")
                )
            });
            effects.push(self.say(Message::info(text)));
        } else if let Some(changes) = changes {
            effects.push(self.say(Message::status(changes)));
        }
        effects
    }

    fn listed_err(&mut self, side: Side, place: &Place, error: &VfsError) -> Vec<Effect> {
        let how = self.panel(side).loading_how();
        let listed = self.panel(side).is_listed();
        self.panel_mut(side).cancel_load();
        let name = place.name().map(str::to_owned);
        if matches!(error, VfsError::NotAFolder)
            && let (Some(parent), Some(name)) = (place.parent(), name.clone())
        {
            return self.open(side, parent, Some(name));
        }
        let gone = matches!(error, VfsError::NotFound);
        let refreshing = matches!(
            how,
            Some(Load::Refresh) | Some(Load::Reload) | Some(Load::AfterJob)
        );
        let shown = place_title(place);
        let up_instead = (gone && refreshing) || !listed;
        if up_instead && let Some(parent) = place.parent() {
            let message = if listed {
                Message::warning(diroxide_core::catalog::FOLDER_GONE)
            } else {
                Message::from_error("open", &shown, error)
            };
            let mut effects = vec![self.say(message)];
            effects.extend(self.open(side, parent, name));
            return effects;
        }
        let message = match error.sentence() {
            Some(sentence) => Message::error(sentence),
            None => Message::from_error("open", &shown, error),
        };
        vec![Effect::Redraw(Redraw::TitleLine), self.say(message)]
    }

    /// A batch of a listing that comes in batches (R25 commit 77): the
    /// first moves the panel to its place with its rows, and the others
    /// add theirs, the focus and the marks keeping to the id rule. The
    /// count is said at most every progress interval in screen-reader
    /// mode ("Listing reports: 2,000 so far"), and shown at most twice a
    /// second otherwise.
    pub(super) fn list_batch(
        &mut self,
        request: RequestId,
        place: &Place,
        mut entries: Vec<Entry>,
    ) -> Vec<Effect> {
        let Some(side) = Side::BOTH
            .into_iter()
            .find(|s| self.panel(*s).waits_for(request))
        else {
            return Vec::new();
        };
        self.organize.labels.apply_to_entries(place, &mut entries);
        let clock = std::sync::Arc::clone(&self.clock);
        let first =
            self.panels[side.index()].stream_batch(request, entries, &self.types, clock.as_ref());
        if !self.panel(side).is_streaming() {
            return Vec::new();
        }
        let now = self.now();
        let active = side == self.active;
        let mut effects = Vec::new();
        if active || !self.screen_reader {
            effects.push(self.redraw_panel(side));
        }
        if first.is_some() {
            self.streams[side.index()] = Some(StreamNote {
                request,
                began: now,
                said: None,
                stopped: false,
            });
            if active {
                effects.push(Effect::Redraw(Redraw::TitleLine));
            }
            return effects;
        }
        let interval = if self.screen_reader {
            self.settings.progress_interval_seconds.saturating_mul(1000)
        } else {
            crate::jobs::SIGHTED_INTERVAL_MS
        };
        let Some(note) = self.streams[side.index()].as_mut() else {
            return effects;
        };
        let since = note.said.unwrap_or(note.began);
        if !active || crate::jobs::millis_between(since, now) < interval {
            return effects;
        }
        note.said = Some(now);
        let text = format!(
            "Listing {}: {} so far",
            place_leaf(self.panel(side).place()),
            crate::messages::grouped(self.panel(side).all_entries().len() as u64)
        );
        let message = if self.screen_reader {
            Message::info(text)
        } else {
            Message::status(text)
        };
        effects.push(self.say(message));
        effects
    }

    /// Stops the active panel's streamed listing (Cancel while it lists):
    /// what came is kept and listed.
    pub(super) fn stop_listing(&mut self) -> Option<Vec<Effect>> {
        let side = self.active;
        if !self.panel(side).is_streaming() {
            return None;
        }
        let request = self.panel(side).loading_request()?;
        if let Some(note) = self.streams[side.index()].as_mut() {
            note.stopped = true;
        }
        self.request(Request::StopListing(request));
        Some(vec![self.say(Message::info("Stopping the listing."))])
    }

    /// The end of a streamed listing: the whole list, and the count said
    /// as a finished listing says it, or that it stopped.
    fn stream_listed(&mut self, side: Side, entries: Vec<Entry>) -> Vec<Effect> {
        let clock = std::sync::Arc::clone(&self.clock);
        self.panels[side.index()].finish(entries, &self.types, clock.as_ref());
        let note = self.streams[side.index()].take();
        let place = self.panel(side).place().clone();
        let view_line = self.folder_view_on_entering(side);
        if place.kind() == diroxide_core::PlaceKind::Local {
            let now = self.now().unix_seconds();
            let home = self.home.clone();
            self.visits.record(&place, now, home.as_ref());
        }
        self.request(Request::Watch {
            side,
            place: Some(place),
        });
        let active = side == self.active;
        let mut effects = Vec::new();
        if active || !self.screen_reader {
            effects.push(self.redraw_panel(side));
        }
        if !active {
            return effects;
        }
        let panel = self.panel(side);
        let rows = panel.list().len() as u64;
        let items = if rows == 1 {
            "1 item".to_owned()
        } else {
            format!("{} items", crate::messages::grouped(rows))
        };
        let mut text = if note.is_some_and(|n| n.stopped) {
            format!("Listing stopped: {items} shown")
        } else {
            format!("{}, {items}", place_leaf(panel.place()))
        };
        if let Some(parent) = panel.place().parent() {
            text.push_str(&format!(
                ", in {}",
                diroxide_core::names::display_name(&parent.to_string())
            ));
        }
        if let Some(view) = view_line {
            text.push_str(&format!(". {view}"));
        }
        effects.push(Effect::Redraw(Redraw::TitleLine));
        effects.push(self.say(Message::info(text)));
        effects
    }

    pub(super) fn list_progress(&mut self, request: RequestId, so_far: u64) -> Vec<Effect> {
        let waiting = self.active_panel().waits_for(request);
        if waiting && so_far > 0 && so_far.is_multiple_of(10_000) {
            let text = format!(
                "Reading folder, {} so far",
                crate::messages::grouped(so_far)
            );
            return vec![self.say(Message::status(text))];
        }
        Vec::new()
    }

    pub(super) fn watched(&mut self, side: Side, place: &Place, event: &WatchEvent) -> Vec<Effect> {
        if self.panel(side).place() != place {
            return Vec::new();
        }
        self.panel_mut(side).set_watching(Watching::Yes);
        match event {
            WatchEvent::Gone => {
                let mut effects = Vec::new();
                if side == self.active {
                    effects.push(self.say(Message::warning(diroxide_core::catalog::FOLDER_GONE)));
                }
                if let Some(parent) = place.parent() {
                    let name = place.name().map(str::to_owned);
                    effects.extend(self.open(side, parent, name));
                }
                effects
            }
            _ if self.panel(side).is_loading() => Vec::new(),
            _ => {
                let place = place.clone();
                self.open_as(side, place, None, Load::Refresh)
            }
        }
    }

    pub(super) fn not_watched(&mut self, side: Side, place: &Place) -> Vec<Effect> {
        if self.panel(side).place() != place {
            return Vec::new();
        }
        self.panel_mut(side).set_watching(Watching::No);
        // A place inside an archive has nothing to watch: it changes only
        // when its archive does, and a listing reads it again from the
        // cache. Saying so would be noise.
        if side != self.active || place.kind() == diroxide_core::PlaceKind::Archive {
            return Vec::new();
        }
        let key = self.key(&ids::RELOAD);
        vec![self.say(diroxide_core::catalog::folder_not_watched(&key))]
    }

    pub(super) fn move_focus(&mut self, movement: Move) -> Vec<Effect> {
        let side = self.active;
        if self.panel_mut(side).list_mut().move_focus(movement) {
            self.focus_effects(side)
        } else if self.panel(side).list().is_empty() {
            let empty = self.panel(side).list().empty_text().to_owned();
            vec![self.say(Message::status(empty))]
        } else {
            let edge = if matches!(movement, Move::Up | Move::PageUp | Move::Home) {
                "Top of the list"
            } else {
                "Bottom of the list"
            };
            vec![self.say(Message::status(edge))]
        }
    }

    pub(super) fn focus_effects(&self, side: Side) -> Vec<Effect> {
        vec![
            Effect::Redraw(Redraw::Panel(side)),
            Effect::Announce(Message::status(self.panel(side).list().announce_focus())),
        ]
    }

    /// "Right panel: Backup, 12 items": the panel, its folder's name and
    /// its count.
    fn panel_words(&self, side: Side) -> String {
        let panel = self.panel(side);
        format!(
            "{}, {}",
            place_leaf(panel.place()),
            count(panel.list().len() as u64, "item", "items")
        )
    }

    pub(super) fn switch_panel(&mut self) -> Vec<Effect> {
        self.active = self.active.other();
        let side = self.active;
        let text = format!(
            "{} panel: {}",
            capitalized(side.word()),
            self.panel_words(side)
        );
        let mut effects = if self.screen_reader {
            vec![
                Effect::Redraw(Redraw::Panel(side)),
                Effect::Redraw(Redraw::TitleLine),
                Effect::Redraw(Redraw::StatusLine),
            ]
        } else {
            vec![Effect::Redraw(Redraw::All)]
        };
        effects.push(self.say(Message::info(text)));
        if self.panel(side).watching() == Watching::No && !self.panel(side).is_loading() {
            let place = self.panel(side).place().clone();
            let focus = self.panel(side).list().focused().map(|r| r.id.clone());
            effects.extend(self.open_as(side, place, focus, Load::Refresh));
        }
        effects
    }

    pub(super) fn swap_panels(&mut self) -> Vec<Effect> {
        self.panels.swap(0, 1);
        for side in Side::BOTH {
            self.panel_mut(side).set_side(side);
            let place = self.panel(side).place().clone();
            self.request(Request::Watch {
                side,
                place: Some(place),
            });
        }
        let text = format!(
            "Panels swapped: left {}, right {}",
            place_leaf(self.panel(Side::Left).place()),
            place_leaf(self.panel(Side::Right).place())
        );
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
    }

    pub(super) fn show_in_other(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if !entry.is_container() {
            let text = format!("Not a folder: {}", entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        let place = if entry.kind == EntryKind::Archive {
            place.into_archive()
        } else {
            place
        };
        let text = format!("Other panel: {}", place_leaf(&place));
        let mut effects = self.open(side.other(), place, None);
        effects.push(self.say(Message::info(text)));
        effects
    }

    pub(super) fn this_in_other(&mut self) -> Vec<Effect> {
        let side = self.active;
        let place = self.panel(side).place().clone();
        let focus = self.panel(side).list().focused().map(|r| r.id.clone());
        let text = format!("Other panel: {}", place_leaf(&place));
        let mut effects = self.open(side.other(), place, focus);
        effects.push(self.say(Message::info(text)));
        effects
    }

    pub(super) fn enter(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let entry = entry.clone();
        if entry.name_not_unicode {
            let message =
                Message::from_error("open", &entry.display_name(), &VfsError::NameNotUnicode);
            return vec![self.say(message)];
        }
        match entry.kind {
            EntryKind::Archive => self.open(side, place.into_archive(), None),
            _ if entry.is_container() => self.open(side, place, None),
            _ if is_program(&entry) => {
                let name = entry.display_name().into_owned();
                let question =
                    crate::dialog::with_default_no(Message::question(&format!("Run {name}")));
                let listing =
                    vec![diroxide_core::names::spell_out(&place.to_string()).into_owned()];
                self.open_dialog(
                    crate::dialog::Dialog::Confirm {
                        question,
                        listing,
                        on_yes: diroxide_core::Command::new(ids::ENTER),
                    },
                    Purpose::RunProgram(place),
                )
            }
            _ => {
                let text = format!("Opening {}", entry.display_name());
                vec![self.say(Message::info(text)), Effect::OpenExternally(place)]
            }
        }
    }

    pub(super) fn into(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let Some(entry) = panel.focused_entry() else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if entry.is_container() {
            return self.enter();
        }
        let kind = entry
            .type_name
            .clone()
            .unwrap_or_else(|| entry.kind.word().to_owned());
        let text = format!("Not a folder: {}, {kind}", entry.display_name());
        vec![self.say(Message::info(text))]
    }

    pub(super) fn up(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        match panel.place().parent() {
            Some(parent) => {
                let focus = panel.place().name().map(str::to_owned);
                self.open(side, parent, focus)
            }
            None => vec![self.say(Message::status("At the top already"))],
        }
    }

    pub(super) fn reload(&mut self, side: Side) -> Vec<Effect> {
        let panel = self.panel(side);
        let place = panel.place().clone();
        let focus = panel.list().focused().map(|r| r.id.clone());
        self.open_as(side, place, focus, Load::Reload)
    }

    pub(super) fn back(&mut self) -> Vec<Effect> {
        let side = self.active;
        match self.panel(side).history().last().cloned() {
            Some(place) => self.open_as(side, place, None, Load::Back),
            None => vec![self.say(Message::status("No folder before this one"))],
        }
    }

    pub(super) fn forward(&mut self) -> Vec<Effect> {
        let side = self.active;
        match self.panel(side).forward_history().last().cloned() {
            Some(place) => self.open_as(side, place, None, Load::Forward),
            None => vec![self.say(Message::status("No folder after this one"))],
        }
    }

    pub(super) fn home(&mut self) -> Vec<Effect> {
        match self.home.clone() {
            Some(home) => self.open(self.active, home, None),
            None => vec![self.say(Message::info("The home folder is not known here."))],
        }
    }

    /// Go to a path: a command with a `place` goes there; otherwise the
    /// path prompt opens with this folder's path, completing names.
    pub(super) fn go_path(&mut self, command: &diroxide_core::Command) -> Vec<Effect> {
        let side = self.active;
        if let Some(place) = command.place("place").cloned() {
            return self.open(side, place, None);
        }
        let text = self.panel(side).place().to_string();
        let field = EntryField::new("Go to a path", CompletionSource::Paths)
            .with_text(text)
            .with_history(state::keys::GO_PATH);
        self.open_dialog(crate::dialog::Dialog::edit(field), Purpose::GoPath(side))
    }

    /// Reads typed text as a place: quotes around it are taken off, either
    /// slash is a separator, and a relative path is read against the
    /// panel's place. Nothing is expanded.
    pub(super) fn typed_place(&self, side: Side, text: &str) -> Result<Place, Message> {
        let mut text = text.trim();
        for quote in ['"', '\''] {
            if text.len() >= 2 && text.starts_with(quote) && text.ends_with(quote) {
                text = &text[1..text.len() - 1];
            }
        }
        // On Windows a typed slash is a backslash, except in a place with
        // a scheme ("sftp://host/path"), whose slashes are its own.
        let text = if cfg!(windows) && !text.contains("://") {
            text.replace('/', "\\")
        } else {
            text.to_owned()
        };
        if text.is_empty() {
            return Err(Message::info("No path typed. Nothing changed."));
        }
        Place::resolve(&text, self.panel(side).place())
            .map_err(|e| Message::from_error("go to", &text, &e))
    }

    /// The go-to-place list: the drives, home and the places used most,
    /// typed terms narrowing it (path jump).
    pub(super) fn go_places(&mut self) -> Vec<Effect> {
        let side = self.active;
        if self.places.is_empty() {
            self.request(Request::Places);
        }
        // Tab completes a bookmark by its name or its place (W3a); the
        // list under the field narrows over every place as you type.
        let field = EntryField::new("Go to a place", CompletionSource::Bookmarks)
            .with_history(state::keys::PLACES);
        let mut effects =
            self.open_dialog(crate::dialog::Dialog::edit(field), Purpose::Places(side));
        effects.extend(self.refresh_candidates());
        effects
    }

    pub(super) fn places_arrived(&mut self, places: Vec<LocalPlace>) -> Vec<Effect> {
        self.places = places;
        self.refresh_candidates()
    }

    /// The folder history, newest first: Enter goes there.
    pub(super) fn folder_history(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let mut rows = Vec::new();
        for place in panel.forward_history().iter() {
            rows.push(history_row(place, "later"));
        }
        for place in panel.history().iter().rev() {
            rows.push(history_row(place, "earlier"));
        }
        if rows.is_empty() {
            return vec![self.say(Message::info("No folder history yet."))];
        }
        let mut list = diroxide_core::ListModel::new("Folder history").with_rows(rows);
        let first_earlier = panel.forward_history().len();
        list.move_focus(Move::To(first_earlier));
        list.set_help("Up and Down choose, Enter goes there, Escape closes.");
        self.open_dialog(
            crate::dialog::Dialog::Choose {
                list,
                on_enter: diroxide_core::Command::new(ids::HISTORY),
            },
            Purpose::History(side),
        )
    }
}

fn history_row(place: &Place, when: &str) -> diroxide_core::ListRow {
    let parent = place
        .parent()
        .map(|p| format!("in {}", diroxide_core::names::display_name(&p.to_string())))
        .unwrap_or_default();
    diroxide_core::ListRow::new(place.to_string(), place_leaf(place))
        .with_details([when.to_owned(), parent])
}

/// The release as said on the first run, from the build's version:
/// "0.1.0-alpha.2" is "alpha 2", "0.1.0-beta.1" is "beta 1", and a
/// version without a pre-release is said as it is ("1.0.0").
fn release_words() -> String {
    words_for_version(env!("CARGO_PKG_VERSION"))
}

fn words_for_version(version: &str) -> String {
    match version.split_once('-') {
        Some((_, pre)) => pre.replacen('.', " ", 1),
        None => version.to_owned(),
    }
}

#[cfg(test)]
mod release_tests {
    use super::words_for_version;

    #[test]
    fn the_release_is_said_in_words() {
        assert_eq!(words_for_version("0.1.0-alpha.1"), "alpha 1");
        assert_eq!(words_for_version("0.1.0-alpha.12"), "alpha 12");
        assert_eq!(words_for_version("0.2.0-beta.1"), "beta 1");
        assert_eq!(words_for_version("1.0.0"), "1.0.0");
    }
}
