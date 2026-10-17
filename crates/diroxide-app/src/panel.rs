//! A panel (a lister): a place, its entries, sort, filter, columns,
//! selection, focus and history, shown through the list model so each row
//! reads "report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40".
//!
//! The rules (plan section 5.2, R23):
//!
//! - Rows come from the listing as it is: a panel never stats each row
//!   after a listing (W1e's bench: 5 to 7 seconds per 100,000 entries).
//! - Sorting is core's comparer over a [`SortSpec`]; the focus stays on
//!   its entry after a resort.
//! - The filter is the one filter language (`diroxide_core::filter`,
//!   W3a): the panel evaluates it over its listing, three-valued, with
//!   the facts beyond an entry from its own lookup (the path,
//!   depth 1, the selection, the bookmarks and the contents read for it),
//!   and keeps the count of entries it could not decide.
//! - Marks travel by row id through every refresh, resort and filter;
//!   when the focused row's id is gone, or marks are lost, the panel says
//!   so through [`Refreshed`], so the app can announce it.
//! - Back and forward keep each folder's cursor and marks.
//! - Names are drawn and said by their display names only.
//!
//! Wave 1 (W1d) owns this module.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use diroxide_config::{ColumnKind, Settings};
use diroxide_core::filter::{ContentQuery, Filter, Lookup, Tally, Truth, Unknown};
use diroxide_core::format::human_size;
use diroxide_core::names::display_name;
use diroxide_core::{
    Clock, Entry, EntryKind, ListModel, ListRow, Move, Pattern, Place, RequestId, RowsReplaced,
    Side, SortSpec,
};
use diroxide_filetype::TypeRegistry;
use diroxide_vfs::Capabilities;

/// A detail said after an entry's name: the Phase 0 set, kept for its
/// callers. The panel itself uses the configuration crate's
/// [`ColumnKind`], which adds created, mode, octal, group and attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    /// The file type: "PDF", "folder".
    Type,
    /// The size in words: "2.1 MB".
    Size,
    /// "modified Tuesday".
    Modified,
    /// "accessed today, 2:05 PM".
    Accessed,
    /// "changed October 3".
    Changed,
    /// Permissions or attributes.
    Permissions,
    /// The owner.
    Owner,
    /// Where a link points.
    LinkTarget,
    /// The extension.
    Extension,
}

impl Column {
    /// A column from its name in `config.toml`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "type" => Column::Type,
            "size" => Column::Size,
            "modified" => Column::Modified,
            "accessed" => Column::Accessed,
            "changed" => Column::Changed,
            "permissions" => Column::Permissions,
            "owner" => Column::Owner,
            "link" => Column::LinkTarget,
            "extension" => Column::Extension,
            _ => return None,
        })
    }

    /// The configuration crate's kind for this column.
    pub const fn kind(self) -> ColumnKind {
        match self {
            Column::Type => ColumnKind::Type,
            Column::Size => ColumnKind::Size,
            Column::Modified => ColumnKind::Modified,
            Column::Accessed => ColumnKind::Accessed,
            Column::Changed => ColumnKind::Changed,
            Column::Permissions => ColumnKind::Permissions,
            Column::Owner => ColumnKind::Owner,
            Column::LinkTarget => ColumnKind::Link,
            Column::Extension => ColumnKind::Extension,
        }
    }
}

/// The row for an entry with the Phase 0 columns: see [`row_with`].
pub fn row_for(entry: &Entry, columns: &[Column], clock: &dyn Clock) -> ListRow {
    let kinds: Vec<ColumnKind> = columns.iter().map(|c| c.kind()).collect();
    row_with(entry, &kinds, clock)
}

/// The row for an entry: its display name, then "name needs care" when it
/// does, then its labels ("label: urgent"), then each column's words;
/// empty cells are left out.
pub fn row_with(entry: &Entry, columns: &[ColumnKind], clock: &dyn Clock) -> ListRow {
    let mut details = Vec::new();
    if entry.needs_care || entry.name_not_unicode {
        details.push("name needs care".to_owned());
    }
    if let Some(labels) = labels_words(&entry.labels) {
        details.push(labels);
    }
    for column in columns {
        let detail = column.cell(entry, clock);
        if !detail.is_empty() {
            details.push(detail);
        }
    }
    ListRow::new(entry.name.clone(), entry.display_name()).with_details(details)
}

/// An entry's labels as its row says them, the names first and their
/// colors never: "label: urgent", "labels: urgent, review" (W3l).
pub fn labels_words(labels: &[String]) -> Option<String> {
    let said: Vec<String> = labels
        .iter()
        .map(|l| crate::labels::in_sentence(&display_name(l)))
        .collect();
    match said.as_slice() {
        [] => None,
        [one] => Some(format!("label: {one}")),
        many => Some(format!("labels: {}", many.join(", "))),
    }
}

/// Compares names the way people count: "file2" before "file10", ignoring
/// case; a total order (core's comparer).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    diroxide_core::sort::compare_names(a, b, diroxide_core::NameCase::Ignore, true)
}

/// A place's title as the list and the title line say it, with hidden
/// characters in names replaced (the display-name rule).
pub fn place_title(place: &Place) -> String {
    display_name(&place.title()).into_owned()
}

/// A place's own name as said, or its title at a top: "Backup", "Drive
/// D:\".
pub fn place_leaf(place: &Place) -> String {
    match place.name() {
        Some(name) => display_name(name).into_owned(),
        None => place_title(place),
    }
}

/// Why a listing was asked for, which decides what its answer does to the
/// history, the cursor and the marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Load {
    /// A new place: the old one goes on the back history.
    Open,
    /// Back in the history.
    Back,
    /// Forward again.
    Forward,
    /// The same place again, by the user (Reload).
    Reload,
    /// The same place again, because it changed (a watch).
    Refresh,
    /// The same place again after one of the app's own jobs: its changes
    /// are the job's, already said by the job's result, so they are not
    /// said again.
    AfterJob,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Loading {
    request: RequestId,
    place: Place,
    focus: Option<String>,
    how: Load,
}

/// A folder's cursor and marks, kept when the panel leaves it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Memory {
    focus: Option<String>,
    marks: Vec<String>,
}

/// Whether the panel's folder is watched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Watching {
    /// Not known yet.
    #[default]
    Unknown,
    /// Changes come in by themselves.
    Yes,
    /// Not watched: the folder is re-read when the panel gets the focus.
    No,
}

/// What a finished listing did: whether the panel moved to another place,
/// what changed, and what happened to the focus and the marks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// The panel shows another place than before.
    pub moved: bool,
    /// Names that appeared since the last listing of the same place.
    pub added: usize,
    /// Names that went away since the last listing of the same place.
    pub removed: usize,
    /// The focus and the marks.
    pub rows: RowsReplaced,
    /// The focused row's label before, when the focus moved.
    pub focus_was: Option<String>,
}

/// What a panel's content test learned when the app read the files for
/// it: the names of the files whose contents matched, and why the others
/// could not be decided (a binary file, an online-only one), by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentAnswers {
    /// The files that hold what the filter looks for.
    pub matched: HashSet<String>,
    /// The files that could not be decided, and why.
    pub undecided: HashMap<String, Unknown>,
}

/// A panel's filter and the facts the app gave it.
#[derive(Clone, Debug)]
struct PanelFilter {
    filter: Filter,
    bookmarks: Option<crate::bookmarks::Bookmarks>,
    content: Option<ContentAnswers>,
}

/// The facts a panel knows beyond each entry, for the filter language:
/// the path from its place, depth 1 (a panel lists one folder), the
/// selection from its list, the bookmarks through
/// [`crate::bookmarks::BookmarkLookup`], and the contents the app read
/// for it. A folder's emptiness is not known in a panel.
struct PanelLookup<'a> {
    place: &'a Place,
    selected: &'a HashSet<&'a str>,
    bookmarks: Option<&'a crate::bookmarks::Bookmarks>,
    content: Option<&'a ContentAnswers>,
}

impl Lookup for PanelLookup<'_> {
    fn path(&self, entry: &Entry) -> Option<String> {
        self.place.join(&entry.name).ok().map(|p| p.to_string())
    }

    fn depth(&self, _entry: &Entry) -> Option<u32> {
        Some(1)
    }

    fn selected(&self, entry: &Entry) -> Option<bool> {
        Some(self.selected.contains(entry.name.as_str()))
    }

    fn bookmarked(&self, entry: &Entry) -> Option<bool> {
        let bookmarks = self.bookmarks?;
        crate::bookmarks::BookmarkLookup::new(self.place, bookmarks).bookmarked(entry)
    }

    fn content(&self, entry: &Entry, _query: &ContentQuery) -> Truth {
        let Some(answers) = self.content else {
            return Truth::Unknown(Unknown::ContentNotRead);
        };
        if let Some(why) = answers.undecided.get(&entry.name) {
            return Truth::Unknown(*why);
        }
        Truth::of(answers.matched.contains(&entry.name))
    }
}

/// One panel.
#[derive(Clone, Debug)]
pub struct Panel {
    side: Side,
    place: Place,
    all_entries: Vec<Entry>,
    entries: Vec<Entry>,
    list: ListModel,
    sort: SortSpec,
    show_hidden: bool,
    columns: Vec<ColumnKind>,
    filter: Option<PanelFilter>,
    /// What the filter could not decide at the last refresh.
    tally: Tally,
    history: Vec<Place>,
    forward: Vec<Place>,
    memory: HashMap<Place, Memory>,
    loading: Option<Loading>,
    capabilities: Option<Capabilities>,
    watching: Watching,
    listed: bool,
}

impl Panel {
    /// A panel showing `place`, not yet listed.
    pub fn new(side: Side, place: Place, settings: &Settings) -> Self {
        let mut list = ListModel::new(place_title(&place));
        list.set_empty_text("Empty folder");
        Self {
            side,
            place,
            all_entries: Vec::new(),
            entries: Vec::new(),
            list,
            sort: settings.sort_spec(),
            show_hidden: settings.show_hidden,
            columns: settings.column_kinds(),
            filter: None,
            tally: Tally::default(),
            history: Vec::new(),
            forward: Vec::new(),
            memory: HashMap::new(),
            loading: None,
            capabilities: None,
            watching: Watching::Unknown,
            listed: false,
        }
    }

    /// Which side it is on.
    pub fn side(&self) -> Side {
        self.side
    }

    /// Moves the panel to the other side, for a swap.
    pub fn set_side(&mut self, side: Side) {
        self.side = side;
    }

    /// The place shown.
    pub fn place(&self) -> &Place {
        &self.place
    }

    /// The list of rows.
    pub fn list(&self) -> &ListModel {
        &self.list
    }

    /// The list of rows, to move the focus or select.
    pub fn list_mut(&mut self) -> &mut ListModel {
        &mut self.list
    }

    /// The entries shown, in row order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Every entry of the last listing, hidden and filtered ones too.
    pub fn all_entries(&self) -> &[Entry] {
        &self.all_entries
    }

    /// The places visited before this one, oldest first.
    pub fn history(&self) -> &[Place] {
        &self.history
    }

    /// The places Forward goes to, the next one last.
    pub fn forward_history(&self) -> &[Place] {
        &self.forward
    }

    /// Replaces the history, for a restored session.
    pub fn set_history(&mut self, history: Vec<Place>) {
        self.history = history;
        self.forward.clear();
    }

    /// The sort.
    pub fn sort(&self) -> &SortSpec {
        &self.sort
    }

    /// The filter, when one is set: the one filter language (W3a), whose
    /// [`Filter::text`] is the text as typed.
    pub fn filter(&self) -> Option<&Filter> {
        self.filter.as_ref().map(|f| &f.filter)
    }

    /// What the filter could not decide at the last refresh, for the count
    /// line: "3 unknown, folder size not counted".
    pub fn filter_tally(&self) -> &Tally {
        &self.tally
    }

    /// Whether the files' contents were read for the filter, so its
    /// content test decides.
    pub fn filter_contents_read(&self) -> bool {
        self.filter.as_ref().is_some_and(|f| f.content.is_some())
    }

    /// What the place can do, once a listing said it.
    pub fn capabilities(&self) -> Option<Capabilities> {
        self.capabilities
    }

    /// Notes what the place can do.
    pub fn set_capabilities(&mut self, capabilities: Capabilities) {
        self.capabilities = Some(capabilities);
    }

    /// Whether the folder is watched.
    pub fn watching(&self) -> Watching {
        self.watching
    }

    /// Notes whether the folder is watched.
    pub fn set_watching(&mut self, watching: Watching) {
        self.watching = watching;
    }

    /// Whether a listing has arrived since the panel was made.
    pub fn is_listed(&self) -> bool {
        self.listed
    }

    /// The entry with a row id (its real name).
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == id)
    }

    /// The focused entry.
    pub fn focused_entry(&self) -> Option<&Entry> {
        let index = self.list.focus_index()?;
        self.entries.get(index)
    }

    /// The focused entry's place.
    pub fn focused_place(&self) -> Option<Place> {
        self.place.join(&self.focused_entry()?.name).ok()
    }

    /// The selected entries' places, or the focused entry's when nothing is
    /// selected: what a command acts on.
    pub fn targets(&self) -> Vec<Place> {
        let selected: Vec<Place> = self
            .list
            .selected_ids()
            .into_iter()
            .filter_map(|id| self.place.join(id).ok())
            .collect();
        if selected.is_empty() {
            self.focused_place().into_iter().collect()
        } else {
            selected
        }
    }

    /// The selected entries, in row order.
    pub fn selected_entries(&self) -> Vec<&Entry> {
        self.list
            .rows()
            .iter()
            .zip(&self.entries)
            .filter(|(row, _)| row.selected)
            .map(|(_, entry)| entry)
            .collect()
    }

    /// Whether a listing is on its way.
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The place a listing on its way is for.
    pub fn loading_place(&self) -> Option<&Place> {
        self.loading.as_ref().map(|l| &l.place)
    }

    /// Notes that `place` is being listed under `request`; the focus will
    /// go to the entry named `focus` when it arrives.
    pub fn begin_load(&mut self, request: RequestId, place: Place, focus: Option<String>) {
        let how = if place == self.place && self.listed {
            Load::Reload
        } else {
            Load::Open
        };
        self.begin_load_as(request, place, focus, how);
    }

    /// [`Panel::begin_load`] with the reason, which decides what the
    /// answer does to the history, the cursor and the marks.
    pub fn begin_load_as(
        &mut self,
        request: RequestId,
        place: Place,
        focus: Option<String>,
        how: Load,
    ) {
        self.loading = Some(Loading {
            request,
            place,
            focus,
            how,
        });
    }

    /// Whether `request` is the listing this panel waits for.
    pub fn waits_for(&self, request: RequestId) -> bool {
        matches!(&self.loading, Some(l) if l.request == request)
    }

    /// The reason of the listing on its way.
    pub fn loading_how(&self) -> Option<Load> {
        self.loading.as_ref().map(|l| l.how)
    }

    /// Takes a finished listing: the panel moves to the place it asked for
    /// (remembering the old one in history) and shows the entries.
    pub fn finish_load(&mut self, entries: Vec<Entry>, types: &TypeRegistry, clock: &dyn Clock) {
        self.finish(entries, types, clock);
    }

    /// Takes a finished listing and says what it did.
    pub fn finish(
        &mut self,
        entries: Vec<Entry>,
        types: &TypeRegistry,
        clock: &dyn Clock,
    ) -> Refreshed {
        let Some(loading) = self.loading.take() else {
            return Refreshed::default();
        };
        let moved = loading.place != self.place || !self.listed;
        let mut refreshed = Refreshed {
            moved,
            ..Refreshed::default()
        };
        if moved {
            if self.listed {
                self.remember_here();
            }
            match loading.how {
                Load::Back => {
                    self.history.pop();
                    if self.listed {
                        self.forward.push(self.place.clone());
                    }
                }
                Load::Forward => {
                    self.forward.pop();
                    if self.listed {
                        self.history.push(self.place.clone());
                    }
                }
                Load::Open | Load::Reload | Load::Refresh | Load::AfterJob => {
                    if self.listed && loading.place != self.place {
                        self.history.push(self.place.clone());
                        self.forward.clear();
                    }
                }
            }
            self.place = loading.place.clone();
            self.list.select_none();
            self.list.move_focus(Move::Home);
            self.list.set_title(place_title(&self.place));
            self.filter = None;
            self.tally = Tally::default();
            self.capabilities = None;
            self.watching = Watching::Unknown;
            self.list.set_rows(Vec::new());
        } else {
            let before: HashSet<&str> = self.all_entries.iter().map(|e| e.name.as_str()).collect();
            let after: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            refreshed.added = after.difference(&before).count();
            refreshed.removed = before.difference(&after).count();
        }
        refreshed.focus_was = self.list.focused().map(|r| r.label.clone());
        refreshed.rows = self.set_entries_counted(entries, types, clock);
        if moved {
            refreshed.rows = RowsReplaced::default();
            let memory = matches!(loading.how, Load::Back | Load::Forward)
                .then(|| self.memory.get(&self.place).cloned())
                .flatten();
            if let Some(memory) = memory {
                let marks: HashSet<String> = memory.marks.into_iter().collect();
                self.list.select_matching(|row| marks.contains(&row.id));
                if loading.focus.is_none()
                    && let Some(focus) = memory.focus
                {
                    self.list.focus_id(&focus);
                }
            }
        }
        if let Some(name) = loading.focus
            && self.list.focus_id(&name)
        {
            refreshed.rows.focus_moved = false;
        }
        self.listed = true;
        refreshed
    }

    fn remember_here(&mut self) {
        let memory = Memory {
            focus: self.list.focused().map(|r| r.id.clone()),
            marks: self
                .list
                .selected_ids()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        self.memory.insert(self.place.clone(), memory);
    }

    /// Drops a failed listing; the panel stays where it was.
    pub fn cancel_load(&mut self) {
        self.loading = None;
    }

    /// Replaces the entries: fills in types, hides what is hidden, filters,
    /// sorts, and rebuilds the rows, keeping the focus and the selection by
    /// name.
    pub fn set_entries(&mut self, entries: Vec<Entry>, types: &TypeRegistry, clock: &dyn Clock) {
        self.set_entries_counted(entries, types, clock);
    }

    fn set_entries_counted(
        &mut self,
        mut entries: Vec<Entry>,
        types: &TypeRegistry,
        clock: &dyn Clock,
    ) -> RowsReplaced {
        for entry in &mut entries {
            if entry.type_name.is_none() {
                entry.type_name = types.short_name(entry);
            }
            // Every provider's names are checked the same way, so a row
            // says "name needs care" whatever served it.
            if !entry.needs_care {
                entry.needs_care = diroxide_vfs::names::needs_care(&entry.name);
            }
        }
        self.all_entries = entries;
        self.refresh(clock)
    }

    /// Shows or hides hidden entries.
    pub fn set_show_hidden(&mut self, show: bool, clock: &dyn Clock) -> RowsReplaced {
        self.show_hidden = show;
        self.refresh(clock)
    }

    /// Whether hidden entries are shown.
    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Sorts by a spec; the focus stays on its entry.
    pub fn set_sort(&mut self, sort: SortSpec, clock: &dyn Clock) -> RowsReplaced {
        self.sort = sort;
        self.refresh(clock)
    }

    /// Sets or clears the filter from alpha.1's pattern: its text is read
    /// in the filter language, where it means the same (a name that
    /// starts with a bracket is read as one quoted name).
    pub fn set_filter(&mut self, filter: Option<Pattern>, clock: &dyn Clock) -> RowsReplaced {
        let filter = filter.and_then(|pattern| {
            let text = pattern.text();
            Filter::parse(text)
                .or_else(|_| Filter::parse(&format!("\"{}\"", text.replace('"', "\"\""))))
                .ok()
        });
        self.set_language_filter(filter, clock)
    }

    /// Sets or clears the filter in the one filter language. Nothing
    /// beyond the entry is known yet but the place and the selection; see
    /// [`Panel::set_filter_facts`].
    pub fn set_language_filter(
        &mut self,
        filter: Option<Filter>,
        clock: &dyn Clock,
    ) -> RowsReplaced {
        self.set_filter_and_facts(filter, None, None, clock)
    }

    /// Sets or clears the filter with the facts the app holds, in one
    /// refresh: see [`Panel::set_filter_facts`].
    pub fn set_filter_and_facts(
        &mut self,
        filter: Option<Filter>,
        bookmarks: Option<crate::bookmarks::Bookmarks>,
        content: Option<ContentAnswers>,
        clock: &dyn Clock,
    ) -> RowsReplaced {
        self.filter = filter.map(|filter| PanelFilter {
            filter,
            bookmarks,
            content,
        });
        self.refresh(clock)
    }

    /// Gives the filter the facts the app holds: the bookmarks, for
    /// `bookmarked`, and what the files' contents held, once they were
    /// read ([`ContentAnswers`]). Without a filter it does nothing.
    pub fn set_filter_facts(
        &mut self,
        bookmarks: Option<crate::bookmarks::Bookmarks>,
        content: Option<ContentAnswers>,
        clock: &dyn Clock,
    ) -> RowsReplaced {
        match self.filter.as_mut() {
            Some(filter) => {
                filter.bookmarks = bookmarks;
                filter.content = content;
                self.refresh(clock)
            }
            None => RowsReplaced::default(),
        }
    }

    /// The details each row says after its name.
    pub fn columns(&self) -> &[ColumnKind] {
        &self.columns
    }

    /// Changes the details each row says after its name (a folder's own
    /// view, W3l); the focus and the marks stay by id.
    pub fn set_columns(&mut self, columns: Vec<ColumnKind>, clock: &dyn Clock) -> RowsReplaced {
        self.columns = columns;
        self.refresh(clock)
    }

    /// How many entries the filter lets through of those hidden rules
    /// allow, and of how many: (shown, all). A filter with a `hidden`
    /// test lets hidden entries through, so they count.
    pub fn filter_counts(&self) -> (usize, usize) {
        let hidden = self.hidden_shown();
        let visible = self
            .all_entries
            .iter()
            .filter(|e| hidden || !e.hidden)
            .count();
        (self.entries.len(), visible)
    }

    /// The ids of the rows shown whose entries a filter matches, in row
    /// order, with the panel's own facts and the bookmarks: what select by
    /// pattern marks.
    pub fn matching_ids(
        &self,
        filter: &Filter,
        bookmarks: Option<&crate::bookmarks::Bookmarks>,
        clock: &dyn Clock,
    ) -> Vec<String> {
        let selected: HashSet<&str> = self.list.selected_ids().into_iter().collect();
        let lookup = PanelLookup {
            place: &self.place,
            selected: &selected,
            bookmarks,
            content: None,
        };
        let evaluator = filter.evaluator(clock);
        self.entries
            .iter()
            .filter(|e| evaluator.matches(e, &lookup))
            .map(|e| e.name.clone())
            .collect()
    }

    /// Whether hidden entries are listed: the panel's setting, or a filter
    /// that asks for them.
    fn hidden_shown(&self) -> bool {
        self.show_hidden
            || self
                .filter
                .as_ref()
                .is_some_and(|f| f.filter.shows_hidden())
    }

    /// The rows, in order, whose labels start with `prefix`, ignoring
    /// case: the type-ahead's matches.
    pub fn prefix_matches(&self, prefix: &str) -> Vec<usize> {
        let wanted = prefix.to_lowercase();
        self.list
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.label.to_lowercase().starts_with(&wanted))
            .map(|(i, _)| i)
            .collect()
    }

    /// The total size of the selected files, and whether a folder is among
    /// them (its size is not counted until measured).
    pub fn selection_size(&self) -> (u64, bool) {
        let mut bytes = 0;
        let mut folders = false;
        for entry in self.selected_entries() {
            if entry.is_folder_like() {
                folders = true;
            } else {
                bytes += entry.size.unwrap_or(0);
            }
        }
        (bytes, folders)
    }

    fn refresh(&mut self, clock: &dyn Clock) -> RowsReplaced {
        let hidden = self.hidden_shown();
        let mut tally = Tally::default();
        let mut shown: Vec<Entry> = match &self.filter {
            None => self
                .all_entries
                .iter()
                .filter(|e| hidden || !e.hidden)
                .cloned()
                .collect(),
            Some(filter) => {
                let selected: HashSet<&str> = self.list.selected_ids().into_iter().collect();
                let lookup = PanelLookup {
                    place: &self.place,
                    selected: &selected,
                    bookmarks: filter.bookmarks.as_ref(),
                    content: filter.content.as_ref(),
                };
                let evaluator = filter.filter.evaluator(clock);
                self.all_entries
                    .iter()
                    .filter(|e| hidden || !e.hidden)
                    .filter(|e| {
                        let truth = evaluator.test(e, &lookup);
                        tally.add(truth);
                        truth.is_yes()
                    })
                    .cloned()
                    .collect()
            }
        };
        self.tally = tally;
        diroxide_core::sort::sort_entries(&mut shown, &self.sort);
        let rows = shown
            .iter()
            .map(|e| row_with(e, &self.columns, clock))
            .collect();
        self.entries = shown;
        self.list.set_rows(rows)
    }

    /// Says the selection's size in words: "4.2 MB", "4.2 MB and folders
    /// not counted".
    pub fn selection_size_words(&self) -> String {
        let (bytes, folders) = self.selection_size();
        match (bytes, folders) {
            (0, true) => "folder sizes not counted".to_owned(),
            (bytes, true) => format!("{}, folder sizes not counted", human_size(bytes)),
            (bytes, false) => human_size(bytes),
        }
    }

    /// Whether the entry kind of the focused row is a folder or archive.
    pub fn focused_is_container(&self) -> bool {
        self.focused_entry().is_some_and(Entry::is_container)
    }

    /// The names in the folder, for name checks (case twins) and the
    /// keep-both name.
    pub fn names(&self) -> Vec<&str> {
        self.all_entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Whether the entry is a folder.
    pub fn is_folder(entry: &Entry) -> bool {
        entry.kind == EntryKind::Folder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;
    use diroxide_core::{FixedClock, SortKey, Timestamp, UtcOffset};

    fn clock() -> FixedClock {
        FixedClock::new(
            Timestamp::from_civil(2026, 10, 10, 15, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        )
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut names = vec!["file10", "File2", "file1", "a", "file02x"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["a", "file1", "File2", "file02x", "file10"]);
    }

    #[test]
    fn rows_read_meaning_first() {
        let clock = clock();
        let tuesday = Timestamp::from_civil(2026, 10, 6, 9, 0, 0, UtcOffset::UTC).unwrap();
        let mut entry = Entry::new("report.pdf", EntryKind::File)
            .with_size(2_100_000)
            .with_modified(tuesday);
        entry.type_name = Some("PDF".into());
        let row = row_for(
            &entry,
            &[Column::Type, Column::Size, Column::Modified],
            &clock,
        );
        assert_eq!(row.spoken(), "report.pdf, PDF, 2.1 MB, modified Tuesday");
        let mut odd = Entry::new("notes.", EntryKind::File);
        odd.needs_care = true;
        assert_eq!(
            row_for(&odd, &[], &clock).spoken(),
            "notes., name needs care"
        );
    }

    fn panel() -> Panel {
        let place = Place::parse_with("/home/ada", PathStyle::Unix).unwrap();
        Panel::new(Side::Left, place, &Settings::default())
    }

    fn sample() -> Vec<Entry> {
        let mut hidden = Entry::new(".profile", EntryKind::File);
        hidden.hidden = true;
        vec![
            Entry::new("b.txt", EntryKind::File).with_size(10),
            Entry::new("Music", EntryKind::Folder),
            hidden,
            Entry::new("a.pdf", EntryKind::File).with_size(5),
        ]
    }

    #[test]
    fn a_panel_sorts_folders_first_and_hides_hidden() {
        let clock = clock();
        let mut panel = panel();
        panel.set_entries(sample(), &TypeRegistry::builtin(), &clock);
        let labels: Vec<&str> = panel
            .list()
            .rows()
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Music", "a.pdf", "b.txt"]);
        assert_eq!(panel.list().announce_focus(), "Music, folder, 1 of 3");
        panel.list_mut().move_focus(Move::Down);
        panel.list_mut().toggle_selected();
        panel.set_show_hidden(true, &clock);
        assert_eq!(panel.list().len(), 4);
        assert_eq!(panel.list().focused().unwrap().id, "a.pdf");
        assert_eq!(panel.list().selected_ids(), vec!["a.pdf"]);
        assert_eq!(panel.targets().len(), 1);
        assert_eq!(panel.selection_size_words(), "5 bytes");
    }

    #[test]
    fn a_resort_keeps_the_cursor_on_its_entry() {
        let clock = clock();
        let mut panel = panel();
        panel.set_entries(sample(), &TypeRegistry::builtin(), &clock);
        panel.list_mut().focus_id("a.pdf");
        let replaced = panel.set_sort(SortSpec::by(SortKey::Size), &clock);
        assert!(!replaced.focus_moved);
        assert_eq!(panel.list().focused().unwrap().id, "a.pdf");
        let labels: Vec<&str> = panel.list().rows().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(labels, ["Music", "b.txt", "a.pdf"]);
    }

    #[test]
    fn the_filter_narrows_and_counts() {
        let clock = clock();
        let mut panel = panel();
        panel.set_entries(sample(), &TypeRegistry::builtin(), &clock);
        panel.set_filter(Some(Pattern::parse("*.pdf, */").unwrap()), &clock);
        assert_eq!(panel.filter_counts(), (2, 3));
        panel.set_filter(None, &clock);
        assert_eq!(panel.filter_counts(), (3, 3));
        assert_eq!(panel.prefix_matches("b"), vec![2]);
    }

    #[test]
    fn back_and_forward_keep_each_folders_cursor_and_marks() {
        let clock = clock();
        let types = TypeRegistry::builtin();
        let mut panel = panel();
        let home = panel.place().clone();
        panel.begin_load(RequestId::new(1), home.clone(), None);
        panel.finish(sample(), &types, &clock);
        panel.list_mut().focus_id("b.txt");
        panel.list_mut().toggle_selected();
        let music = home.join("Music").unwrap();
        panel.begin_load(RequestId::new(2), music.clone(), None);
        let went = panel.finish(
            vec![Entry::new("song.mp3", EntryKind::File)],
            &types,
            &clock,
        );
        assert!(went.moved);
        assert_eq!(panel.history(), std::slice::from_ref(&home));
        assert_eq!(panel.list().selected_count(), 0);
        panel.begin_load_as(RequestId::new(3), home.clone(), None, Load::Back);
        panel.finish(sample(), &types, &clock);
        assert_eq!(panel.list().focused().unwrap().id, "b.txt");
        assert_eq!(panel.list().selected_ids(), ["b.txt"]);
        assert!(panel.history().is_empty());
        assert_eq!(panel.forward_history(), std::slice::from_ref(&music));
        panel.begin_load_as(RequestId::new(4), music.clone(), None, Load::Forward);
        panel.finish(
            vec![Entry::new("song.mp3", EntryKind::File)],
            &types,
            &clock,
        );
        assert_eq!(panel.place(), &music);
        assert!(panel.forward_history().is_empty());
    }

    #[test]
    fn a_refresh_counts_changes_and_says_when_the_focus_moved() {
        let clock = clock();
        let types = TypeRegistry::builtin();
        let mut panel = panel();
        let home = panel.place().clone();
        panel.begin_load(RequestId::new(1), home.clone(), None);
        panel.finish(sample(), &types, &clock);
        panel.list_mut().focus_id("b.txt");
        panel.list_mut().toggle_selected();
        panel.begin_load_as(RequestId::new(2), home, None, Load::Refresh);
        let refreshed = panel.finish(
            vec![
                Entry::new("Music", EntryKind::Folder),
                Entry::new("c.txt", EntryKind::File),
            ],
            &types,
            &clock,
        );
        assert!(!refreshed.moved);
        assert_eq!((refreshed.added, refreshed.removed), (1, 3));
        assert!(refreshed.rows.focus_moved);
        assert_eq!(refreshed.rows.marks_lost, 1);
        assert_eq!(refreshed.focus_was.as_deref(), Some("b.txt"));
    }

    #[test]
    fn a_hostile_name_shows_by_its_display_name() {
        let clock = clock();
        let mut panel = panel();
        let mut odd = Entry::new("evil\u{1b}[2Jname.txt", EntryKind::File);
        odd.needs_care = true;
        panel.set_entries(vec![odd], &TypeRegistry::builtin(), &clock);
        let row = &panel.list().rows()[0];
        assert!(!row.label.contains('\u{1b}'), "{}", row.label);
        assert!(
            row.spoken()
                .starts_with("evil?[2Jname.txt, name needs care")
        );
        assert_eq!(row.id, "evil\u{1b}[2Jname.txt");
    }

    /// Spike five's panel half: 100,000 in-memory entries through the
    /// panel. The budgets (sort within 200 ms, a type-ahead key within 30
    /// ms) are for release builds; this run prints its numbers and holds
    /// a debug build to ten times them.
    #[test]
    fn a_folder_of_100_000_entries_sorts_and_searches_in_time() {
        use std::time::Instant;
        let clock = clock();
        let types = TypeRegistry::builtin();
        let entries: Vec<Entry> = (0..100_000u64)
            .map(|i| {
                Entry::new(format!("file{i}.txt"), EntryKind::File)
                    .with_size(i)
                    .with_modified(Timestamp::from_unix(1_760_000_000 + i as i64, 0))
            })
            .collect();
        let mut panel = panel();
        let started = Instant::now();
        panel.set_entries(entries, &types, &clock);
        let listed = started.elapsed();
        let started = Instant::now();
        panel.set_sort(SortSpec::by(SortKey::Size), &clock);
        let sorted = started.elapsed();
        let started = Instant::now();
        let matches = panel.prefix_matches("file9999");
        let searched = started.elapsed();
        let started = Instant::now();
        panel.list_mut().select_all();
        let selected = started.elapsed();
        assert_eq!(matches.len(), 11);
        assert_eq!(panel.list().selected_count(), 100_000);
        println!(
            "100,000 entries: rows {listed:?}, sort {sorted:?}, type-ahead {searched:?}, select all {selected:?}"
        );
        let slack = if cfg!(debug_assertions) { 10 } else { 1 };
        assert!(sorted.as_millis() < 200 * slack, "sort took {sorted:?}");
        assert!(
            searched.as_millis() < 30 * slack,
            "type-ahead took {searched:?}"
        );
    }
}
