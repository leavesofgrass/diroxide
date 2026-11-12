//! Tabs in the app (alpha 5, lane TB; research/modern-features.md,
//! "Folder tabs"; R34's corrections in research/alpha5-plan.md, 1.1): a
//! panel holds a list of listers, one of them current. The current tab's
//! lister is the panel the app has always had (`App::panels`), so every
//! command keeps working on it unchanged; the other tabs are parked here,
//! each a whole [`Panel`] with its own place, sort, filter, columns,
//! selection, cursor and history, and the few facts the app keeps beside
//! a panel (the archive it shows, a folder view's usual look).
//!
//! The rule for parked tabs: only the shown tab of each side keeps a
//! watch and takes replies. Parking a tab stops its listing; showing it
//! again lists its place again (quietly when it was listed before), so it
//! is fresh and watched. A listing is matched by its request id, so a
//! parked tab's late reply is dropped; the type and column replies carry
//! the entries' generation, unique across every panel; a watch event and
//! the folder sizes carry a place, which must be the shown tab's; and a
//! job's ending acts on the tab it began in only (each lister has a tab
//! id), so after a switch it only speaks.
//!
//! No tab bar is drawn: the state is words. With more than one tab, the
//! title line and each arrival say the tab after the folder ("Documents,
//! tab 2 of 3"); a lock is said after it only when it holds ("Documents,
//! tab 2 of 3, locked"). The tab list is the screen reader's way to see
//! them all, a chooser whose rows say the name first.
//!
//! Locks: free; locked (closing asks first, starting on No, and a folder
//! change opens a new tab); pinned (folder changes are free, and the tab
//! remembers its base folder for Go to base folder). The last tab never
//! closes. Closed tabs go on a short stack for Undo close tab, this
//! session only. Saved tab sets are kept by name in the state folder
//! (`crate::tabs`), and the session restores every tab at the next start.

use std::collections::HashMap;
use std::sync::Arc;

use diroxide_config::{SessionState, TabLock, TabState};
use diroxide_core::format::count;
use diroxide_core::{
    Command, CommandId, Effect, JobId, ListModel, ListRow, Message, Place, Redraw, Side,
};

use super::App;
use super::organize::{Look, Wants};
use crate::archives::ArchiveFacts;
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};
use crate::panel::{Load, Panel, place_leaf};
use crate::request::Request;
use crate::tabs::{self, CLOSED_LIMIT, TabSet, TabSets};

/// A parked tab: its lister and what the app keeps beside it.
#[derive(Clone, Debug)]
struct Held {
    panel: Panel,
    facts: Option<ArchiveFacts>,
    entered: bool,
    usual: Option<Look>,
    /// The entry to focus when a tab restored from the session is listed
    /// for the first time.
    focus: Option<String>,
}

/// One tab of a panel: its lock, a pinned tab's base folder, and its
/// lister when it is parked (the current tab's lister is the panel).
#[derive(Clone, Debug)]
struct Slot {
    lock: TabLock,
    base: Option<Place>,
    held: Option<Box<Held>>,
}

impl Slot {
    fn current(lock: TabLock, base: Option<Place>) -> Self {
        Self {
            lock,
            base,
            held: None,
        }
    }

    fn parked(lock: TabLock, base: Option<Place>, held: Held) -> Self {
        Self {
            lock,
            base,
            held: Some(Box::new(held)),
        }
    }
}

/// One panel's tabs in order, and which is current.
#[derive(Clone, Debug)]
struct Strip {
    slots: Vec<Slot>,
    current: usize,
}

impl Default for Strip {
    fn default() -> Self {
        Self {
            slots: vec![Slot::current(TabLock::Free, None)],
            current: 0,
        }
    }
}

impl Strip {
    fn len(&self) -> usize {
        self.slots.len()
    }

    fn now(&self) -> &Slot {
        &self.slots[self.current]
    }
}

/// A closed tab, for Undo close tab.
#[derive(Clone, Debug)]
struct Closed {
    index: usize,
    lock: TabLock,
    base: Option<Place>,
    held: Held,
}

/// What the app holds for tabs.
#[derive(Clone, Debug, Default)]
pub(super) struct TabsState {
    strips: [Strip; 2],
    /// The closed tabs, newest last.
    closed: Vec<Closed>,
    /// The saved tab sets, from `tab-sets.toml`.
    sets: TabSets,
    /// The tab each running job began in, by its side and the lister's
    /// tab id.
    job_tabs: HashMap<JobId, (Side, u64)>,
}

impl TabsState {
    /// What the app starts with: one tab a panel and the saved sets.
    pub(super) fn new(sets: TabSets) -> Self {
        Self {
            sets,
            ..Self::default()
        }
    }
}

/// What a dialog of the tabs is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TabWants {
    /// The tab list of a panel.
    List { side: Side },
    /// The question before a locked tab is closed.
    CloseLocked { side: Side },
    /// The name typed for Save tab set.
    SaveName,
    /// The saved sets, to load one.
    Sets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    New,
    NewFocused,
    Close,
    CloseOthers,
    UndoClose,
    Next,
    Previous,
    List,
    Go(usize),
    MoveBy(bool),
    ToOther { copy: bool },
    Lock(TabLock),
    Base,
    SaveSet,
    LoadSet,
}

/// The tab commands, by id.
const ROUTES: [(CommandId, Do); 26] = [
    (ids::TAB_NEW, Do::New),
    (ids::TAB_NEW_FOCUSED, Do::NewFocused),
    (ids::TAB_CLOSE, Do::Close),
    (ids::TAB_CLOSE_OTHERS, Do::CloseOthers),
    (ids::TAB_UNDO_CLOSE, Do::UndoClose),
    (ids::TAB_NEXT, Do::Next),
    (ids::TAB_PREVIOUS, Do::Previous),
    (ids::TAB_LIST, Do::List),
    (ids::TAB_GO_1, Do::Go(1)),
    (ids::TAB_GO_2, Do::Go(2)),
    (ids::TAB_GO_3, Do::Go(3)),
    (ids::TAB_GO_4, Do::Go(4)),
    (ids::TAB_GO_5, Do::Go(5)),
    (ids::TAB_GO_6, Do::Go(6)),
    (ids::TAB_GO_7, Do::Go(7)),
    (ids::TAB_GO_8, Do::Go(8)),
    (ids::TAB_GO_9, Do::Go(9)),
    (ids::TAB_MOVE_LEFT, Do::MoveBy(false)),
    (ids::TAB_MOVE_RIGHT, Do::MoveBy(true)),
    (ids::TAB_TO_OTHER, Do::ToOther { copy: false }),
    (ids::TAB_COPY_TO_OTHER, Do::ToOther { copy: true }),
    (ids::TAB_LOCK, Do::Lock(TabLock::Locked)),
    (ids::TAB_PIN, Do::Lock(TabLock::Pinned)),
    (ids::TAB_BASE, Do::Base),
    (ids::TAB_SAVE_SET, Do::SaveSet),
    (ids::TAB_LOAD_SET, Do::LoadSet),
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES.iter().find(|(r, _)| r == id).map(|(_, d)| *d)
}

/// The name a tab is said by: its place's leaf.
fn tab_name(panel: &Panel) -> String {
    place_leaf(panel.loading_place().unwrap_or(panel.place()))
}

impl App {
    /// How many tabs a panel has.
    pub fn tab_count(&self, side: Side) -> usize {
        self.tabs.strips[side.index()].len()
    }

    /// The current tab's index in a panel, from 0.
    pub fn tab_index(&self, side: Side) -> usize {
        self.tabs.strips[side.index()].current
    }

    /// The current tab's lock in a panel.
    pub fn tab_lock(&self, side: Side) -> TabLock {
        self.tabs.strips[side.index()].now().lock
    }

    /// The current tab's base folder, when it is pinned.
    pub fn tab_base(&self, side: Side) -> Option<&Place> {
        self.tabs.strips[side.index()].now().base.as_ref()
    }

    /// Each tab of a panel in words, in order, as the tab list's rows say
    /// them: "Documents, tab 2 of 3, locked".
    pub fn tab_rows(&self, side: Side) -> Vec<String> {
        let strip = &self.tabs.strips[side.index()];
        let n = strip.len();
        strip
            .slots
            .iter()
            .enumerate()
            .map(|(i, slot)| tabs::tab_words(&tab_name(self.slot_panel(side, i)), i, n, slot.lock))
            .collect()
    }

    /// What the title line and an arrival say of the current tab after
    /// the folder: "tab 2 of 3" and its lock when it holds, only when the
    /// panel has more than one tab; `None` otherwise.
    pub fn tab_title_words(&self, side: Side) -> Option<String> {
        let strip = &self.tabs.strips[side.index()];
        if strip.len() < 2 {
            return None;
        }
        let mut words = tabs::position_words(strip.current, strip.len());
        if let Some(lock) = strip.now().lock.said() {
            words.push_str(&format!(", {lock}"));
        }
        Some(words)
    }

    /// The saved tab sets.
    pub fn tab_sets(&self) -> &TabSets {
        &self.tabs.sets
    }

    /// The lister of a panel's tab at `index`.
    fn slot_panel(&self, side: Side, index: usize) -> &Panel {
        self.tabs.strips[side.index()].slots[index]
            .held
            .as_ref()
            .map_or_else(|| self.panel(side), |h| &h.panel)
    }

    /// The current tab in words, the number only with more than one:
    /// "Documents, tab 2 of 3" or "Documents".
    fn current_tab_words(&self, side: Side) -> String {
        let name = tab_name(self.panel(side));
        match self.tab_title_words(side) {
            Some(tab) => format!("{name}, {tab}"),
            None => name,
        }
    }

    /// After each event: the jobs begun since are noted with the tab they
    /// began in, and the ended ones forgotten.
    pub(super) fn note_job_tabs(&mut self) {
        let flows: Vec<(JobId, Side)> = self.flows.iter().map(|(j, f)| (*j, f.side)).collect();
        self.tabs
            .job_tabs
            .retain(|job, _| flows.iter().any(|(j, _)| j == job));
        for (job, side) in flows {
            let tab = self.panel(side).tab_id();
            self.tabs.job_tabs.entry(job).or_insert((side, tab));
        }
    }

    /// Whether a job's panel now shows another tab than the one it began
    /// in: its ending then only speaks.
    pub(super) fn job_left_its_tab(&self, job: JobId, side: Side) -> bool {
        self.tabs
            .job_tabs
            .get(&job)
            .is_some_and(|(began, tab)| *began == side && *tab != self.panel(side).tab_id())
    }

    /// Runs a tab command; `None` when the command is another's. With a
    /// dialog or the viewer open it says what is in the way.
    pub(super) fn tabs_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        if !self.dialogs.is_empty() {
            return Some(vec![self.say(Message::status("Close this dialog first."))]);
        }
        if self.viewing.is_some() {
            return Some(vec![self.say(Message::status("Close the viewer first."))]);
        }
        let side = self.active;
        Some(match action {
            Do::New => self.new_tab(side),
            Do::NewFocused => self.new_tab_for_focused(side),
            Do::Close => self.close_tab(side),
            Do::CloseOthers => self.close_other_tabs(side),
            Do::UndoClose => self.undo_close_tab(side),
            Do::Next => self.step_tab(side, true),
            Do::Previous => self.step_tab(side, false),
            Do::List => self.tab_list(side),
            Do::Go(number) => self.go_to_tab(side, number),
            Do::MoveBy(right) => self.move_tab(side, right),
            Do::ToOther { copy } => self.tab_to_other(side, copy),
            Do::Lock(lock) => self.toggle_lock(side, lock),
            Do::Base => self.go_to_base(side),
            Do::SaveSet => self.save_set_prompt(),
            Do::LoadSet => self.load_set_list(),
        })
    }

    /// The live panel of a side and its facts, parked, with `incoming`
    /// shown in its place. Parking stops the listing on its way.
    fn swap_live(&mut self, side: Side, incoming: Held) -> Held {
        let i = side.index();
        if let Some(old) = self.panel(side).loading_request() {
            self.request(Request::StopListing(old));
        }
        self.streams[i] = None;
        if incoming.focus.is_some() {
            self.restore[i] = incoming.focus.clone();
        }
        Held {
            panel: std::mem::replace(&mut self.panels[i], incoming.panel),
            facts: std::mem::replace(&mut self.archive_facts[i], incoming.facts),
            entered: std::mem::replace(&mut self.archive_entered[i], incoming.entered),
            usual: std::mem::replace(&mut self.organize.usual[i], incoming.usual),
            focus: None,
        }
    }

    /// A copy of the live tab, to park; it keeps the live tab's id.
    fn copy_live(&self, side: Side) -> Held {
        let i = side.index();
        Held {
            panel: self.panels[i].clone(),
            facts: self.archive_facts[i].clone(),
            entered: self.archive_entered[i],
            usual: self.organize.usual[i].clone(),
            focus: None,
        }
    }

    /// A tab not listed yet, from a saved tab: its place, sort, hidden
    /// files and history.
    fn fresh_held(&self, side: Side, state: &TabState) -> Option<Held> {
        let place = state.place.clone()?;
        let clock = Arc::clone(&self.clock);
        let mut panel = Panel::new(side, place, &self.settings);
        panel.set_types(Arc::clone(&self.types));
        panel.set_history(state.history.clone());
        panel.set_sort(state.sort.clone(), clock.as_ref());
        panel.set_show_hidden(state.show_hidden, clock.as_ref());
        Some(Held {
            panel,
            facts: None,
            entered: false,
            usual: None,
            focus: state.focused.clone(),
        })
    }

    /// The tab at `to` becomes current: its lister is shown and its place
    /// listed again.
    fn switch_tab(&mut self, side: Side, to: usize) -> Vec<Effect> {
        let i = side.index();
        let from = self.tabs.strips[i].current;
        if to != from {
            let Some(incoming) = self.tabs.strips[i].slots[to].held.take() else {
                return Vec::new();
            };
            let outgoing = self.swap_live(side, *incoming);
            self.tabs.strips[i].slots[from].held = Some(Box::new(outgoing));
            self.tabs.strips[i].current = to;
        }
        self.arrive_tab(side, true)
    }

    /// Lists the current tab's place after it was shown. A tab listed
    /// before is listed again quietly, said by name ("Documents, tab 2 of
    /// 3") when `say`; a listing it was waiting for when it was parked is
    /// asked again; a tab never listed says its arrival when the listing
    /// comes. The watch follows the shown tab.
    fn arrive_tab(&mut self, side: Side, say: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.set_panel_help(side);
        let panel = self.panel(side);
        let listed = panel.is_listed();
        let waiting = panel.loading_place().cloned().zip(panel.loading_how());
        let place = panel.place().clone();
        let focus = panel.list().focused().map(|r| r.id.clone());
        if listed && say && side == self.active {
            let words = self.current_tab_words(side);
            effects.push(self.say(Message::info(words)));
        }
        match waiting {
            // The rest of a listing in batches: list the place again.
            Some((_, Load::Stream)) | None if listed => {
                effects.extend(self.open_as(side, place.clone(), focus, Load::Refresh));
                self.request(Request::Watch {
                    side,
                    place: Some(place.clone()),
                });
                self.ask_room(side, place);
            }
            Some((wanted, how)) if listed && how != Load::Stream => {
                effects.extend(self.open_as(side, wanted, None, how));
            }
            _ => {
                let wanted = waiting.map_or(place, |(wanted, _)| wanted);
                let focus = self.restored_focus(side);
                effects.extend(self.open_as(side, wanted, focus, Load::Open));
            }
        }
        effects.push(self.redraw_panel(side));
        effects.push(Effect::Redraw(Redraw::TitleLine));
        effects
    }

    /// New tab: a copy of the current one, after it, made current. The
    /// lister the owner was in goes on as the new tab, with a new id, so
    /// a listing on its way still lands; the parked copy is the old tab.
    pub(super) fn new_tab(&mut self, side: Side) -> Vec<Effect> {
        self.add_tab_copy(side);
        let words = self.current_tab_words(side);
        vec![
            self.say(Message::info(format!("New tab: {words}"))),
            Effect::Redraw(Redraw::TitleLine),
        ]
    }

    /// Parks a copy of the live tab where it is, with its lock, and makes
    /// the live lister a new free tab after it.
    fn add_tab_copy(&mut self, side: Side) {
        let i = side.index();
        let copy = self.copy_live(side);
        self.panels[i].renew_tab_id();
        let strip = &mut self.tabs.strips[i];
        let old = strip.current;
        strip.slots[old].held = Some(Box::new(copy));
        strip
            .slots
            .insert(old + 1, Slot::current(TabLock::Free, None));
        strip.current = old + 1;
    }

    /// New tab for the folder under the cursor.
    fn new_tab_for_focused(&mut self, side: Side) -> Vec<Effect> {
        let panel = self.panel(side);
        let Some(entry) = panel.focused_entry() else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if !Panel::is_folder(entry) {
            let text = format!("{} is not a folder.", entry.name);
            return vec![self.say(Message::status(text))];
        }
        let Some(place) = panel.focused_place() else {
            return Vec::new();
        };
        self.add_tab_copy(side);
        self.open(side, place, None)
    }

    /// Whether a folder change in this panel's current tab must open a
    /// new tab: the tab is locked, listed, and the place is another.
    pub(super) fn locked_tab_moves(&self, side: Side, place: &Place, how: Load) -> bool {
        matches!(how, Load::Open | Load::Back | Load::Forward)
            && self.tab_lock(side) == TabLock::Locked
            && self.panel(side).is_listed()
            && !self.panel(side).is_loading()
            && self.panel(side).place() != place
    }

    /// A locked tab keeps its place: a copy stays locked where it is and
    /// the folder change goes on in a new free tab after it.
    pub(super) fn leave_locked_tab(&mut self, side: Side) -> Vec<Effect> {
        self.add_tab_copy(side);
        let number = self.tab_index(side) + 1;
        vec![self.say(Message::info(format!(
            "Tab locked: opening in tab {number}."
        )))]
    }

    /// Close tab: the last tab stays, said so; a locked one asks first,
    /// starting on No; otherwise it closes.
    fn close_tab(&mut self, side: Side) -> Vec<Effect> {
        let name = tab_name(self.panel(side));
        if self.tab_count(side) == 1 {
            let text = format!("Last tab stays open: {name}.");
            return vec![self.say(Message::status(text))];
        }
        if self.tab_lock(side) == TabLock::Locked {
            let question = with_default_no(Message::question(&format!("Close locked tab {name}")));
            return self.open_organize(
                Dialog::Confirm {
                    question,
                    listing: Vec::new(),
                    on_yes: Command::new(ids::TAB_CLOSE),
                },
                Wants::Tabs(TabWants::CloseLocked { side }),
            );
        }
        self.close_current(side)
    }

    /// Closes the current tab of a panel of more than one: the next tab
    /// (or the one before the last) becomes current, and the closed one
    /// goes on the stack for Undo close tab.
    fn close_current(&mut self, side: Side) -> Vec<Effect> {
        let name = tab_name(self.panel(side));
        let (closed, mut effects) = self.take_current_tab(side);
        self.push_closed(closed);
        let now = if self.panel(side).is_listed() {
            format!(" {}.", self.current_tab_words(side))
        } else {
            String::new()
        };
        effects.insert(0, self.say(Message::info(format!("Closed {name}.{now}"))));
        effects
    }

    fn push_closed(&mut self, closed: Closed) {
        self.tabs.closed.push(closed);
        let excess = self.tabs.closed.len().saturating_sub(CLOSED_LIMIT);
        self.tabs.closed.drain(..excess);
    }

    /// Takes the current tab out of a panel of more than one: the tab
    /// after it becomes current, or the one before when it was the last.
    /// Returns the tab taken and the effects of listing the new current
    /// one, which says nothing until its arrival.
    fn take_current_tab(&mut self, side: Side) -> (Closed, Vec<Effect>) {
        let i = side.index();
        let from = self.tabs.strips[i].current;
        let last = self.tabs.strips[i].len() - 1;
        let to = if from < last { from + 1 } else { from - 1 };
        let incoming = self.tabs.strips[i].slots[to].held.take().map(|h| *h);
        let Some(incoming) = incoming else {
            // Never: every tab but the current one holds its lister.
            let slot = self.tabs.strips[i].now().clone();
            let held = self.copy_live(side);
            return (
                Closed {
                    index: from,
                    lock: slot.lock,
                    base: slot.base,
                    held,
                },
                Vec::new(),
            );
        };
        let outgoing = self.swap_live(side, incoming);
        let strip = &mut self.tabs.strips[i];
        let slot = strip.slots.remove(from);
        strip.current = if to > from { to - 1 } else { to };
        let closed = Closed {
            index: from,
            lock: slot.lock,
            base: slot.base,
            held: outgoing,
        };
        let effects = self.arrive_tab(side, false);
        (closed, effects)
    }

    /// Close other tabs: every other tab of this panel but the locked
    /// ones, each on the closed stack.
    fn close_other_tabs(&mut self, side: Side) -> Vec<Effect> {
        let i = side.index();
        let strip = std::mem::take(&mut self.tabs.strips[i]);
        let mut kept = Vec::new();
        let mut current = 0;
        let mut closed = 0usize;
        let mut locked = 0usize;
        for (index, slot) in strip.slots.into_iter().enumerate() {
            if index == strip.current {
                current = kept.len();
                kept.push(slot);
            } else if slot.lock == TabLock::Locked {
                locked += 1;
                kept.push(slot);
            } else if let Some(held) = slot.held {
                closed += 1;
                self.push_closed(Closed {
                    index,
                    lock: slot.lock,
                    base: slot.base,
                    held: *held,
                });
            }
        }
        self.tabs.strips[i] = Strip {
            slots: kept,
            current,
        };
        let mut text = if closed == 0 {
            "No other tabs closed.".to_owned()
        } else {
            format!("Closed {}.", count(closed as u64, "tab", "tabs"))
        };
        if locked > 0 {
            text.push_str(&format!(
                " Kept {}.",
                count(locked as u64, "locked tab", "locked tabs")
            ));
        }
        vec![
            self.say(Message::info(text)),
            Effect::Redraw(Redraw::TitleLine),
        ]
    }

    /// Undo close tab: the tab closed last comes back in the active
    /// panel, where it was, and is made current.
    fn undo_close_tab(&mut self, side: Side) -> Vec<Effect> {
        let Some(closed) = self.tabs.closed.pop() else {
            return vec![self.say(Message::status("No closed tab to bring back."))];
        };
        let mut effects = self.put_tab(side, closed.index, closed.lock, closed.base, closed.held);
        if self.panel(side).is_listed() {
            let words = self.current_tab_words(side);
            effects.insert(0, self.say(Message::info(format!("Reopened {words}"))));
        }
        effects
    }

    /// Puts a parked tab into a panel at `index` and makes it current.
    fn put_tab(
        &mut self,
        side: Side,
        index: usize,
        lock: TabLock,
        base: Option<Place>,
        mut held: Held,
    ) -> Vec<Effect> {
        let i = side.index();
        held.panel.set_side(side);
        let at = index.min(self.tabs.strips[i].len());
        let outgoing = self.swap_live(side, held);
        let strip = &mut self.tabs.strips[i];
        let old = strip.current;
        strip.slots[old].held = Some(Box::new(outgoing));
        strip.slots.insert(at, Slot::current(lock, base));
        strip.current = at;
        self.arrive_tab(side, false)
    }

    /// Next or previous tab, round the ends.
    fn step_tab(&mut self, side: Side, next: bool) -> Vec<Effect> {
        let n = self.tab_count(side);
        if n == 1 {
            let name = tab_name(self.panel(side));
            return vec![self.say(Message::status(format!("Only one tab: {name}.")))];
        }
        let current = self.tab_index(side);
        let to = if next {
            (current + 1) % n
        } else {
            (current + n - 1) % n
        };
        self.switch_tab(side, to)
    }

    /// Go to tab N, from 1.
    fn go_to_tab(&mut self, side: Side, number: usize) -> Vec<Effect> {
        let n = self.tab_count(side);
        if number == 0 || number > n {
            let text = format!(
                "No tab {number}: this panel has {}.",
                count(n as u64, "tab", "tabs")
            );
            return vec![self.say(Message::status(text))];
        }
        self.switch_tab(side, number - 1)
    }

    /// Move tab left or right: one place, said where it now is.
    fn move_tab(&mut self, side: Side, right: bool) -> Vec<Effect> {
        let strip = &mut self.tabs.strips[side.index()];
        let from = strip.current;
        let to = match (right, from) {
            (false, 0) => None,
            (false, from) => Some(from - 1),
            (true, from) if from + 1 < strip.len() => Some(from + 1),
            (true, _) => None,
        };
        let Some(to) = to else {
            let text = if right {
                "Already the last tab."
            } else {
                "Already the first tab."
            };
            return vec![self.say(Message::status(text))];
        };
        strip.slots.swap(from, to);
        strip.current = to;
        let words = self.current_tab_words(side);
        vec![
            self.say(Message::info(words)),
            Effect::Redraw(Redraw::TitleLine),
        ]
    }

    /// The tab list: a chooser of this panel's tabs, name first, the
    /// current one focused.
    fn tab_list(&mut self, side: Side) -> Vec<Effect> {
        let list = self.tab_list_model(side);
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::TAB_LIST),
            },
            Wants::Tabs(TabWants::List { side }),
        )
    }

    /// The tab list's model: "Tabs, left panel, 3 tabs", each row a tab
    /// in words, its folder's parent after.
    fn tab_list_model(&self, side: Side) -> ListModel {
        let strip = &self.tabs.strips[side.index()];
        let rows = self
            .tab_rows(side)
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                let panel = self.slot_panel(side, i);
                let place = panel.loading_place().unwrap_or(panel.place());
                let mut details = Vec::new();
                if let Some(parent) = place.parent() {
                    details.push(format!(
                        "in {}",
                        diroxide_core::names::display_name(&parent.to_string())
                    ));
                }
                ListRow::new(i.to_string(), label).with_details(details)
            })
            .collect();
        let mut list = ListModel::new(format!(
            "Tabs, {} panel, {}",
            side.word(),
            count(strip.len() as u64, "tab", "tabs")
        ))
        .with_rows(rows);
        list.focus_id(&strip.current.to_string());
        list.set_help("Enter goes to the tab, Escape closes.");
        list
    }

    /// Move or copy tab to other panel: the tab, or a copy of it, becomes
    /// the other panel's current tab, after its current one, and the
    /// other panel becomes active. A move leaves this panel's next tab
    /// current; a lone tab cannot move.
    fn tab_to_other(&mut self, side: Side, copy: bool) -> Vec<Effect> {
        if !copy && self.tab_count(side) == 1 {
            return vec![self.say(Message::status("Only one tab here: it cannot move."))];
        }
        let name = tab_name(self.panel(side));
        let (taken, mut effects) = if copy {
            let mut held = self.copy_live(side);
            held.panel.renew_tab_id();
            let slot = self.tabs.strips[side.index()].now().clone();
            let taken = Closed {
                index: 0,
                lock: slot.lock,
                base: slot.base,
                held,
            };
            (taken, Vec::new())
        } else {
            self.take_current_tab(side)
        };
        let other = side.other();
        let at = self.tab_index(other) + 1;
        effects.extend(self.put_tab(other, at, taken.lock, taken.base, taken.held));
        self.active = other;
        let place = self
            .tab_title_words(other)
            .unwrap_or_else(|| tabs::position_words(0, 1));
        let verb = if copy { "Copied" } else { "Moved" };
        let text = format!("{verb} {name} to the {} panel, {place}", other.word());
        effects.insert(0, self.say(Message::info(text)));
        effects.push(Effect::Redraw(Redraw::All));
        effects
    }

    /// Lock tab and Pin tab: each toggles its lock and free. Pinning
    /// remembers the folder as the tab's base; freeing forgets it.
    fn toggle_lock(&mut self, side: Side, lock: TabLock) -> Vec<Effect> {
        let i = side.index();
        let now = if self.tab_lock(side) == lock {
            TabLock::Free
        } else {
            lock
        };
        let place = self.panel(side).place().clone();
        let strip = &mut self.tabs.strips[i];
        let current = strip.current;
        strip.slots[current].lock = now;
        strip.slots[current].base = (now == TabLock::Pinned).then_some(place);
        let name = tab_name(self.panel(side));
        let mut text = format!("{name}, {}", now.word());
        if self.tab_count(side) > 1 {
            text.push_str(&format!(
                ", {}",
                tabs::position_words(self.tab_index(side), self.tab_count(side))
            ));
        }
        vec![
            self.say(Message::info(text)),
            Effect::Redraw(Redraw::TitleLine),
        ]
    }

    /// Go to base folder: a pinned tab goes back to the folder it was
    /// pinned in.
    fn go_to_base(&mut self, side: Side) -> Vec<Effect> {
        let Some(base) = self.tab_base(side).cloned() else {
            return vec![self.say(Message::status("Not a pinned tab: it has no base folder."))];
        };
        if self.panel(side).place() == &base {
            let text = format!("Already in the base folder, {}.", place_leaf(&base));
            return vec![self.say(Message::status(text))];
        }
        self.open(side, base, None)
    }

    /// A panel's tabs as the session keeps them, the live one as `live`
    /// (the session's own state of the panel) describes it.
    fn panel_tabs(&self, side: Side, live: &TabState) -> Vec<TabState> {
        let strip = &self.tabs.strips[side.index()];
        strip
            .slots
            .iter()
            .map(|slot| match &slot.held {
                None => TabState {
                    lock: slot.lock,
                    base: slot.base.clone(),
                    ..live.clone()
                },
                Some(held) => {
                    let panel = &held.panel;
                    let (sort, show_hidden) = match &held.usual {
                        Some(usual) => (usual.sort.clone(), usual.show_hidden),
                        None => (panel.sort().clone(), panel.shows_hidden()),
                    };
                    let focused = if panel.is_listed() {
                        panel.list().focused().map(|r| r.id.clone())
                    } else {
                        held.focus.clone()
                    };
                    TabState {
                        place: Some(panel.loading_place().unwrap_or(panel.place()).clone()),
                        focused,
                        sort,
                        show_hidden,
                        history: panel.history().to_vec(),
                        lock: slot.lock,
                        base: slot.base.clone(),
                    }
                }
            })
            .collect()
    }

    /// Writes every panel's tabs into the session beside the two panels,
    /// each panel's own state staying its current tab's.
    pub(super) fn tabs_into_session(&self, session: &mut SessionState) {
        for side in Side::BOTH {
            let live = TabState::of_panel(session.panel(side));
            let tabs = self.panel_tabs(side, &live);
            session.set_tabs(side, tabs, self.tab_index(side));
        }
    }

    /// Restores the session's tabs at start, beside the place each panel
    /// opens with. When that place is the saved current tab's, the tabs
    /// come back as they were; otherwise they come back and the place
    /// opened with is a new tab after the saved current one. A parked tab
    /// is listed when it is first shown.
    pub(super) fn restore_tabs(&mut self, session: &SessionState) {
        for side in Side::BOTH {
            let (saved, current) = session.tab_list(side);
            if saved.len() <= 1 && saved.iter().all(TabState::is_plain) {
                continue;
            }
            let live_place = self.panel(side).place().clone();
            let matches = saved[current].place.as_ref() == Some(&live_place);
            let mut slots = Vec::new();
            let mut live_at = None;
            for (i, state) in saved.iter().enumerate() {
                if matches && i == current {
                    live_at = Some(slots.len());
                    slots.push(Slot::current(state.lock, state.base.clone()));
                    continue;
                }
                if let Some(held) = self.fresh_held(side, state) {
                    slots.push(Slot::parked(state.lock, state.base.clone(), held));
                }
                if !matches && i == current {
                    live_at = Some(slots.len());
                    slots.push(Slot::current(TabLock::Free, None));
                }
            }
            let Some(live_at) = live_at else {
                continue;
            };
            self.tabs.strips[side.index()] = Strip {
                slots,
                current: live_at,
            };
        }
    }

    /// Save tab set: a field for its name, the first saved name in it.
    fn save_set_prompt(&mut self) -> Vec<Effect> {
        let mut field = EntryField::new("Tab set name", CompletionSource::None);
        if let Some(first) = self.tabs.sets.sets().first() {
            field = field.with_text(first.name.clone());
        }
        self.open_organize(Dialog::edit(field), Wants::Tabs(TabWants::SaveName))
    }

    /// The name typed: both panels' tabs saved under it, replacing a set
    /// of the same name.
    fn save_set_named(&mut self, text: &str) -> Vec<Effect> {
        let name = match tabs::check_set_name(text) {
            Ok(name) => name,
            Err(why) => return vec![self.say(Message::error(why))],
        };
        let session = self.session();
        let (left, left_at) = session.tab_list(Side::Left);
        let (right, right_at) = session.tab_list(Side::Right);
        let set = TabSet {
            name: name.clone(),
            active: self.active,
            tabs: [left, right],
            current: [left_at, right_at],
        };
        let tabs_words = count(set.tab_count() as u64, "tab", "tabs");
        let replaced = self.tabs.sets.put(set);
        self.request(Request::SaveTabSets(Box::new(self.tabs.sets.clone())));
        let mut effects = self.close_all();
        let verb = if replaced { "Replaced" } else { "Saved" };
        effects.push(self.say(Message::info(format!(
            "{verb} tab set {name}, {tabs_words}."
        ))));
        effects
    }

    /// Load tab set: the saved sets as a chooser, or why there are none.
    fn load_set_list(&mut self) -> Vec<Effect> {
        if self.tabs.sets.is_empty() {
            return vec![self.say(Message::status("No tab sets saved yet."))];
        }
        let rows = self
            .tabs
            .sets
            .sets()
            .iter()
            .map(|set| ListRow::new(set.name.clone(), set.row_words()))
            .collect();
        let mut list = ListModel::new(format!(
            "Tab sets, {}",
            count(self.tabs.sets.sets().len() as u64, "set", "sets")
        ))
        .with_rows(rows);
        list.set_help("Enter loads the set in place of the open tabs, Escape closes.");
        self.open_organize(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::TAB_LOAD_SET),
            },
            Wants::Tabs(TabWants::Sets),
        )
    }

    /// Loads a saved set in place of the open tabs. The tabs it replaces
    /// go on the closed stack, so Undo close tab brings them back one by
    /// one.
    fn load_set(&mut self, name: &str) -> Vec<Effect> {
        let Some(set) = self.tabs.sets.get(name).cloned() else {
            return vec![self.say(Message::status(format!("No tab set {name}.")))];
        };
        let mut effects = self.close_all();
        for side in Side::BOTH {
            let i = side.index();
            let Some((saved, current)) = set.tab_list(side) else {
                continue;
            };
            let mut parked: Vec<Slot> = Vec::new();
            let mut incoming = None;
            for (index, state) in saved.iter().enumerate() {
                let Some(held) = self.fresh_held(side, state) else {
                    continue;
                };
                if index == current {
                    incoming = Some((parked.len(), state.lock, state.base.clone(), held));
                } else {
                    parked.push(Slot::parked(state.lock, state.base.clone(), held));
                }
            }
            let Some((at, lock, base, incoming)) = incoming else {
                continue;
            };
            // The open tabs, kept for Undo close tab.
            let strip = std::mem::take(&mut self.tabs.strips[i]);
            for (index, slot) in strip.slots.into_iter().enumerate() {
                let held = match slot.held {
                    Some(held) => *held,
                    None => self.copy_live(side),
                };
                self.push_closed(Closed {
                    index,
                    lock: slot.lock,
                    base: slot.base,
                    held,
                });
            }
            let _ = self.swap_live(side, incoming);
            parked.insert(at, Slot::current(lock, base));
            self.tabs.strips[i] = Strip {
                slots: parked,
                current: at,
            };
        }
        self.active = set.active;
        let text = format!(
            "Tab set {} loaded, {}",
            set.name,
            count(set.tab_count() as u64, "tab", "tabs")
        );
        effects.push(self.say(Message::info(text)));
        for side in Side::BOTH {
            effects.extend(self.arrive_tab(side, false));
        }
        effects
    }

    /// Enter in one of the tabs' dialogs.
    pub(super) fn tabs_accept(
        &mut self,
        wants: TabWants,
        row: Option<String>,
        text: &str,
    ) -> Vec<Effect> {
        match wants {
            TabWants::List { side } => {
                let Some(index) = row.and_then(|r| r.parse::<usize>().ok()) else {
                    return Vec::new();
                };
                let mut effects = self.close_all();
                if index < self.tab_count(side) {
                    effects.extend(self.switch_tab(side, index));
                }
                effects
            }
            TabWants::CloseLocked { .. } => self.dialog_escape(),
            TabWants::SaveName => self.save_set_named(text),
            TabWants::Sets => match row {
                Some(name) => self.load_set(&name),
                None => Vec::new(),
            },
        }
    }

    /// Yes to one of the tabs' questions: the locked tab closes.
    pub(super) fn tabs_yes(&mut self, wants: TabWants) -> Vec<Effect> {
        match wants {
            TabWants::CloseLocked { side } if self.tab_count(side) > 1 => self.close_current(side),
            _ => Vec::new(),
        }
    }

    /// Escape or No on one of the tabs' questions.
    pub(super) fn tabs_escape(&mut self, wants: TabWants) -> Vec<Effect> {
        match wants {
            TabWants::CloseLocked { .. } => {
                vec![self.say(Message::info("Nothing closed; the tab stays."))]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
