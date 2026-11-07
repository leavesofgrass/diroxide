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
//! - A plugin list (lane L) sits over the place without replacing it: the
//!   place, its rows, its marks and its history stay as they are beneath
//!   it, every method here keeps speaking of the place, and
//!   [`Panel::shown_list`] gives the rows the panel shows. The list is
//!   never saved.
//!
//! Wave 1 (W1d) owns this module.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use diroxide_config::{ColumnKind, ColumnSpec, Settings};
use diroxide_core::filter::{ContentQuery, Filter, Lookup, NoLookup, Tally, Truth, Unknown};
use diroxide_core::format::human_size;
use diroxide_core::names::display_name;
use diroxide_core::{
    Clock, Entry, EntryKind, ListModel, ListRow, Move, Pattern, Place, RequestId, RowsReplaced,
    Side, SortSpec,
};
use diroxide_filetype::{CheckSkip, Probe, Special, TypeRegistry, TypeStage, skips_content};
use diroxide_vfs::Capabilities;

use crate::columns::{ColumnCell, ColumnItem, FieldColumn, Source};
use crate::plugins::lister::PanelList;
use crate::typing::{TypeAnswer, TypeItem};

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
    let specs: Vec<ColumnSpec> = columns.iter().copied().map(ColumnSpec::Kind).collect();
    row_with_fields(entry, &specs, &[], (false, false), clock)
}

/// The row for an entry with built-in columns and fields' columns, in
/// the columns' order (alpha 4, lane MA): a field's cell says its label
/// first, "artist Ada Example", or why it waits, "artist reading"; a key
/// with no resolved column says nothing. `here` is whether the place's
/// contents are read and whether its plugins' columns are asked.
pub fn row_with_fields(
    entry: &Entry,
    columns: &[ColumnSpec],
    fields: &[FieldColumn],
    here: (bool, bool),
    clock: &dyn Clock,
) -> ListRow {
    let mut details = Vec::new();
    if entry.needs_care || entry.name_not_unicode {
        details.push("name needs care".to_owned());
    }
    if let Some(labels) = labels_words(&entry.labels) {
        details.push(labels);
    }
    for column in columns {
        let detail = match column {
            ColumnSpec::Kind(kind) => {
                let mut detail = kind.cell(entry, clock);
                // A folder whose size was counted says it where a file's
                // size goes.
                if detail.is_empty()
                    && *kind == ColumnKind::Size
                    && entry.kind == EntryKind::Folder
                    && let Some(size) = entry.size
                {
                    detail = human_size(size);
                }
                detail
            }
            ColumnSpec::Key(key) => fields
                .iter()
                .find(|f| f.key == *key)
                .and_then(|f| f.cell_words(entry, here.0, here.1))
                .unwrap_or_default(),
        };
        if !detail.is_empty() {
            details.push(detail);
        }
    }
    ListRow::new(entry.name.clone(), entry.display_name()).with_details(details)
}

/// Types an entry by its name and kind alone, what a row shows at once:
/// its type's short name and the type's index ([`Entry::type_index`]),
/// "type pending" where the name says nothing and the first bytes are
/// still to be read, and nothing (the row says its kind, "file") where no
/// type takes it. `name_only` is a place whose contents are never read.
pub fn type_by_name(entry: &mut Entry, types: &TypeRegistry, name_only: bool, clock: &dyn Clock) {
    if entry.kind == EntryKind::Folder {
        entry.type_name = Some("folder".to_owned());
        return;
    }
    let probe = Probe::new(entry, None).skipping(name_only);
    match types.check(&probe, clock) {
        Ok(found) => {
            entry.type_index = TypeRegistry::index_of(&found);
            entry.type_name = types.by_index(entry.type_index).map(|t| t.short.clone());
        }
        Err(Special::Pending) => entry.type_name = Some(TYPE_PENDING.to_owned()),
        Err(Special::Folder | Special::Unknown) => {}
    }
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
    /// The rest of a listing that comes in batches (lane A2, R25 commit
    /// 77): the panel moved with the first batch, and the others fill it.
    Stream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Loading {
    request: RequestId,
    place: Place,
    focus: Option<String>,
    how: Load,
    /// The marks still to restore as their rows arrive, in a listing that
    /// comes in batches.
    marks: Vec<String>,
    /// The focused row after the last batch: when the owner moves the
    /// focus, the focus wanted is dropped.
    left_on: Option<String>,
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
    /// The type model's groups, the owner's own among them (lane TA);
    /// without it `group:` answers from core's extension table.
    types: Option<(&'a TypeRegistry, &'a dyn Clock)>,
}

impl Lookup for PanelLookup<'_> {
    fn group(&self, entry: &Entry) -> Result<Option<Cow<'_, str>>, Unknown> {
        match self.types {
            Some((types, clock)) => types
                .group_for(entry, clock)
                .map(|group| group.map(|g| Cow::Owned(g.id.clone()))),
            None => NoLookup
                .group(entry)
                .map(|g| g.map(|g| g.into_owned().into())),
        }
    }

    fn in_group(&self, entry: &Entry, group: &str) -> Result<bool, Unknown> {
        match self.types {
            Some((types, clock)) => types
                .groups_of(entry, clock)
                .map(|groups| groups.iter().any(|g| g.id.eq_ignore_ascii_case(group))),
            None => NoLookup.in_group(entry, group),
        }
    }

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
    columns: Vec<ColumnSpec>,
    /// The columns of fields' keys among `columns`, resolved by the app
    /// (alpha 4, lane MA): where each one's cells come from.
    fields: Vec<FieldColumn>,
    /// Shown rows whose entries took column cells the rows do not say
    /// yet: put into the rows when the focus comes near or the reading
    /// ends, since replacing every row of a big folder costs (spike M6).
    stale_rows: std::collections::BTreeSet<usize>,
    /// The shown rows' indexes by entry name, made when cells first come
    /// after a refresh.
    shown_index: Option<HashMap<String, usize>>,
    /// Folder sizes counted here, by name, until the panel leaves.
    counted: HashMap<String, u64>,
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
    /// A plugin's list shown over the place (lane L).
    plugin_list: Option<Box<PanelList>>,
    /// The file types' background check (alpha 4, lane TA): which entries
    /// of the listing had their first bytes read, one bit each, beside
    /// [`Panel::all_entries`].
    checked: Bits,
    /// The place [`Panel::all_entries`] belong to, so a refresh of the
    /// same place keeps the types already checked.
    typed_place: Option<Place>,
    /// Counts each replacement of the entries: an answer of the type
    /// check for an older set is dropped.
    entries_generation: u64,
    /// The folders whose files are typed by name alone (`check.skip`).
    check_skip: CheckSkip,
    /// "Name says PNG; contents say JPEG", by entry name: said on request,
    /// in the details and when the file is opened.
    disagreements: HashMap<String, String>,
    /// The file types the filter's `group:` asks, the owner's groups
    /// among them (lane TA).
    types: Option<std::sync::Arc<TypeRegistry>>,
}

/// One bit for each entry of a listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Bits {
    words: Vec<u64>,
}

impl Bits {
    fn with_len(len: usize) -> Self {
        Self {
            words: vec![0; len.div_ceil(64)],
        }
    }

    fn get(&self, index: usize) -> bool {
        self.words
            .get(index / 64)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }

    fn set(&mut self, index: usize) {
        if let Some(word) = self.words.get_mut(index / 64) {
            *word |= 1 << (index % 64);
        }
    }

    /// The bytes the marks take.
    fn bytes(&self) -> usize {
        self.words.len() * 8
    }
}

/// The words a row says for a file whose name says nothing until its
/// first bytes are read: "type pending" (12 cells).
pub const TYPE_PENDING: &str = Special::Pending.words();

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
            columns: settings.column_specs(),
            fields: Vec::new(),
            stale_rows: std::collections::BTreeSet::new(),
            shown_index: None,
            counted: HashMap::new(),
            filter: None,
            tally: Tally::default(),
            history: Vec::new(),
            forward: Vec::new(),
            memory: HashMap::new(),
            loading: None,
            capabilities: None,
            watching: Watching::Unknown,
            listed: false,
            plugin_list: None,
            checked: Bits::default(),
            typed_place: None,
            entries_generation: 0,
            check_skip: CheckSkip::new(&settings.check.skip),
            disagreements: HashMap::new(),
            types: None,
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

    /// The rows the panel shows: a plugin list's when one is over the
    /// place, the place's own otherwise (lane L).
    pub fn shown_list(&self) -> &ListModel {
        match &self.plugin_list {
            Some(shown) => shown.list(),
            None => &self.list,
        }
    }

    /// The plugin list shown over the place, when one is.
    pub fn plugin_list(&self) -> Option<&PanelList> {
        self.plugin_list.as_deref()
    }

    /// The plugin list shown over the place, to move in it, mark rows or
    /// change it.
    pub fn plugin_list_mut(&mut self) -> Option<&mut PanelList> {
        self.plugin_list.as_deref_mut()
    }

    /// Shows a plugin list over the place; the list shown before, if any,
    /// is returned, so its plugin can be told it closed.
    pub fn show_plugin_list(&mut self, list: PanelList) -> Option<PanelList> {
        self.plugin_list.replace(Box::new(list)).map(|old| *old)
    }

    /// Takes the plugin list away: the place shows again as it was.
    pub fn take_plugin_list(&mut self) -> Option<PanelList> {
        self.plugin_list.take().map(|old| *old)
    }

    /// Sets how many rows Page Up and Page Down move, for the place and
    /// for a plugin list over it.
    pub fn set_page_size(&mut self, rows: usize) {
        self.list.set_page_size(rows);
        if let Some(shown) = self.plugin_list.as_deref_mut() {
            shown.list_mut().set_page_size(rows);
        }
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
            marks: Vec::new(),
            left_on: None,
        });
    }

    /// The listing this panel waits for.
    pub fn loading_request(&self) -> Option<RequestId> {
        self.loading.as_ref().map(|l| l.request)
    }

    /// Whether the listing on its way comes in batches and the panel shows
    /// its first rows already.
    pub fn is_streaming(&self) -> bool {
        self.loading.as_ref().is_some_and(|l| l.how == Load::Stream)
    }

    /// Takes one batch of a listing that comes in batches (lane A2, R25
    /// commit 77). The first batch of a new place moves the panel there
    /// as a finished listing would, and the panel keeps waiting for the
    /// rest; each later batch adds its rows. The focus and the marks keep
    /// to the id rule (plan section 5.2): rows are never matched by
    /// position, the focus wanted goes to its row when it arrives unless
    /// the owner moved it meanwhile, and the marks a folder had come back
    /// to their rows as they arrive. A batch of the place the panel shows
    /// already (a reload) is kept for the end, so no row and no mark goes
    /// away in between. Returns what the first batch did.
    pub fn stream_batch(
        &mut self,
        request: RequestId,
        entries: Vec<Entry>,
        types: &TypeRegistry,
        clock: &dyn Clock,
    ) -> Option<Refreshed> {
        let loading = self.loading.as_ref().filter(|l| l.request == request)?;
        if loading.how != Load::Stream {
            let moved = loading.place != self.place || !self.listed;
            if !moved {
                return None;
            }
            let wanted = loading.focus.clone();
            let memory = matches!(loading.how, Load::Back | Load::Forward)
                .then(|| self.memory.get(&loading.place).cloned())
                .flatten();
            let refreshed = self.finish(entries, types, clock);
            let focus = wanted
                .or_else(|| memory.as_ref().and_then(|m| m.focus.clone()))
                .filter(|name| self.list.focused().is_none_or(|row| row.id != *name));
            self.loading = Some(Loading {
                request,
                place: self.place.clone(),
                focus,
                how: Load::Stream,
                marks: memory.map(|m| m.marks).unwrap_or_default(),
                left_on: self.list.focused().map(|row| row.id.clone()),
            });
            self.follow_stream();
            return Some(refreshed);
        }
        let mut all = self.all_entries.clone();
        all.extend(entries);
        self.set_entries_counted(all, types, clock);
        self.follow_stream();
        None
    }

    /// Puts the marks and the focus a streamed listing still wants on
    /// their rows, as they arrive.
    fn follow_stream(&mut self) {
        let Some(loading) = self.loading.as_mut() else {
            return;
        };
        let now_on = self.list.focused().map(|row| row.id.clone());
        if now_on != loading.left_on {
            // The owner moved the focus: it stays where they put it.
            loading.focus = None;
        }
        if let Some(name) = loading.focus.clone()
            && self.list.focus_id(&name)
        {
            loading.focus = None;
        }
        if !loading.marks.is_empty() {
            let marks: HashSet<String> = loading.marks.iter().cloned().collect();
            self.list.select_matching(|row| marks.contains(&row.id));
            let listed: HashSet<&str> = self.list.rows().iter().map(|r| r.id.as_str()).collect();
            loading.marks.retain(|id| !listed.contains(id.as_str()));
        }
        loading.left_on = self.list.focused().map(|row| row.id.clone());
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
                Load::Open | Load::Reload | Load::Refresh | Load::AfterJob | Load::Stream => {
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
            self.counted.clear();
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
        // The end of a streamed listing: the marks still wanted come back,
        // and the focus goes where it was wanted, unless the owner moved it.
        if loading.how == Load::Stream {
            let marks: HashSet<String> = loading.marks.iter().cloned().collect();
            if !marks.is_empty() {
                self.list.select_matching(|row| marks.contains(&row.id));
            }
            let now_on = self.list.focused().map(|r| r.id.clone());
            if now_on == loading.left_on
                && let Some(name) = &loading.focus
            {
                self.list.focus_id(name);
            }
            refreshed.added = 0;
            refreshed.removed = 0;
            refreshed.rows = RowsReplaced::default();
            self.listed = true;
            return refreshed;
        }
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
        // A refresh of the same place keeps the types its unchanged files
        // were checked as, so a watched folder is not read again.
        let old = std::mem::take(&mut self.all_entries);
        let old_checked = std::mem::take(&mut self.checked);
        let same_place = self.typed_place.as_ref() == Some(&self.place);
        if !same_place {
            self.disagreements.clear();
        }
        let mut checked = Bits::with_len(entries.len());
        let carried: HashMap<&str, &Entry> = if same_place {
            old.iter()
                .enumerate()
                .filter(|(i, _)| old_checked.get(*i))
                .map(|(_, e)| (e.name.as_str(), e))
                .collect()
        } else {
            HashMap::new()
        };
        let reads = self.reads_contents();
        for (i, entry) in entries.iter_mut().enumerate() {
            if let Some(before) = carried.get(entry.name.as_str())
                && before.size == entry.size
                && before.modified == entry.modified
                && before.kind == entry.kind
            {
                entry.type_index = before.type_index;
                entry.type_name = before.type_name.clone();
                checked.set(i);
            } else if entry.type_name.is_none() {
                type_by_name(entry, types, !reads, clock);
            }
            // A folder's size counted here stays with it (W3a).
            if entry.kind == EntryKind::Folder
                && let Some(size) = self.counted.get(&entry.name)
            {
                entry.size = Some(*size);
            }
            // Every provider's names are checked the same way, so a row
            // says "name needs care" whatever served it.
            if !entry.needs_care {
                entry.needs_care = diroxide_vfs::names::needs_care(&entry.name);
            }
        }
        if same_place {
            let names: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            self.disagreements
                .retain(|name, _| names.contains(name.as_str()));
        }
        drop(carried);
        // The column cells read for unchanged files stay, so a refresh of
        // a watched folder reads only what changed (alpha 4, lane MA).
        let read_keys: Vec<&str> = self
            .fields
            .iter()
            .filter(|f| matches!(f.source, Source::Meta(_) | Source::Plugin(_)))
            .map(|f| f.key.as_str())
            .collect();
        if same_place && !read_keys.is_empty() {
            let before: HashMap<&str, &Entry> = old
                .iter()
                .filter(|e| read_keys.iter().any(|k| e.extra.contains_key(*k)))
                .map(|e| (e.name.as_str(), e))
                .collect();
            for entry in &mut entries {
                if let Some(old) = before.get(entry.name.as_str())
                    && old.size == entry.size
                    && old.modified == entry.modified
                    && old.kind == entry.kind
                {
                    for key in &read_keys {
                        if let Some(value) = old.extra.get(*key) {
                            entry
                                .extra
                                .entry((*key).to_owned())
                                .or_insert_with(|| value.clone());
                        }
                    }
                }
            }
        }
        self.fill_regex_cells(&mut entries);
        self.all_entries = entries;
        self.checked = checked;
        self.typed_place = Some(self.place.clone());
        self.entries_generation += 1;
        self.refresh(clock)
    }

    /// The file types the filter's `group:` asks: the type model's
    /// groups, the owner's own among them, and the groups whose name
    /// patterns take a file (a screenshot is in images and screenshots).
    /// Without them, `group:` answers from core's extension table.
    pub fn set_types(&mut self, types: std::sync::Arc<TypeRegistry>) {
        self.types = Some(types);
    }

    /// The folders whose files are typed by name alone, from the setting
    /// `check.skip`.
    pub fn set_check_skip(&mut self, skip: CheckSkip) {
        self.check_skip = skip;
    }

    /// Types every entry again by its name, as a new listing would be,
    /// after the types or `check.skip` changed; the check then reads
    /// them again.
    pub fn retype(&mut self, types: &TypeRegistry, clock: &dyn Clock) {
        let mut entries = std::mem::take(&mut self.all_entries);
        for entry in &mut entries {
            entry.type_name = None;
            entry.type_index = 0;
        }
        self.typed_place = None;
        self.set_entries_counted(entries, types, clock);
    }

    /// Whether the files of this place may have their first bytes read
    /// for their types: a folder on this computer that `check.skip` does
    /// not name. Archives, plugins' places and remote sites are typed by
    /// name alone (R28 4.3: network places default to the name stages).
    pub fn reads_contents(&self) -> bool {
        self.place.kind() == diroxide_core::PlaceKind::Local
            && !self.check_skip.covers(&self.place.to_string())
    }

    /// Counts each time the entries were replaced; an answer of the type
    /// check carries the count it was asked under.
    pub fn entries_generation(&self) -> u64 {
        self.entries_generation
    }

    /// The files whose first bytes are still to be read for their types,
    /// nearest the focus first: the focused row, then the rows below and
    /// above it in turn, so the rows on screen come before the rest; then
    /// the entries hidden or filtered out. Empty where the place is typed
    /// by name alone.
    pub fn type_queue(&self) -> Vec<TypeItem> {
        if !self.reads_contents() {
            return Vec::new();
        }
        let wanted = |i: usize, entry: &Entry| {
            !self.checked.get(i)
                && matches!(entry.kind, EntryKind::File | EntryKind::Archive)
                && !skips_content(entry)
        };
        let index_of: HashMap<&str, usize> = self
            .all_entries
            .iter()
            .enumerate()
            .filter(|(i, e)| wanted(*i, e))
            .map(|(i, e)| (e.name.as_str(), i))
            .collect();
        if index_of.is_empty() {
            return Vec::new();
        }
        let mut queue = Vec::with_capacity(index_of.len());
        let mut taken = HashSet::new();
        let shown = self.entries.len();
        let focus = self.list.focus_index().unwrap_or(0).min(shown);
        let mut take = |row: usize| {
            if let Some(entry) = self.entries.get(row)
                && let Some(&i) = index_of.get(entry.name.as_str())
                && taken.insert(i)
            {
                queue.push(TypeItem::of(i, entry));
            }
        };
        for step in 0..shown {
            if let Some(below) = focus.checked_add(step) {
                take(below);
            }
            if step > 0
                && let Some(above) = focus.checked_sub(step)
            {
                take(above);
            }
            if focus + step >= shown && focus < step {
                break;
            }
        }
        for (i, entry) in self.all_entries.iter().enumerate() {
            if index_of.get(entry.name.as_str()) == Some(&i) && !taken.contains(&i) {
                queue.push(TypeItem::of(i, entry));
            }
        }
        queue
    }

    /// Takes what the type check found for files of this listing (the
    /// answers of an older one are left to the caller to drop): each
    /// file's type and its row's words, and the line when its name and
    /// its contents disagree. Returns whether a row's words changed, after
    /// which the rows are made again, the focus and the marks kept by id.
    pub fn apply_type_answers(
        &mut self,
        answers: Vec<TypeAnswer>,
        types: &TypeRegistry,
        clock: &dyn Clock,
    ) -> bool {
        let mut changed = false;
        for answer in answers {
            let i = answer.index as usize;
            let Some(entry) = self.all_entries.get_mut(i) else {
                continue;
            };
            if entry.name != answer.name {
                continue;
            }
            self.checked.set(i);
            let (index, name) = match answer.found {
                Ok((index, _)) => (index, types.by_index(index).map(|t| t.short.clone())),
                Err(_) => (0, None),
            };
            // The row's type so far is its name's: only a different type
            // from the contents can disagree, so the common case costs
            // nothing more.
            if let Ok((index, stage)) = answer.found
                && entry.type_index != 0
                && entry.type_index != index
                && let (Some(named), Some(checked)) =
                    (types.by_index(entry.type_index), types.by_index(index))
            {
                let found =
                    |t: &diroxide_filetype::FileType, at: u16, stage| diroxide_filetype::Found {
                        type_id: t.id.clone(),
                        stage,
                        index: usize::from(at).saturating_sub(1),
                    };
                let by_name = found(named, entry.type_index, TypeStage::Name);
                if let Some(line) = types.disagreement(&by_name, &found(checked, index, stage)) {
                    self.disagreements.insert(entry.name.clone(), line);
                }
            }
            if entry.type_index != index || entry.type_name != name {
                entry.type_index = index;
                entry.type_name = name;
                changed = true;
            }
        }
        if changed {
            self.refresh(clock);
        }
        changed
    }

    /// The line when an entry's name and its first bytes disagree: "Name
    /// says PNG; contents say JPEG". `None` when they agree or the entry
    /// was not checked.
    pub fn disagreement(&self, name: &str) -> Option<&str> {
        self.disagreements.get(name).map(String::as_str)
    }

    /// How many of the listing's entries had their first bytes read for
    /// their types, and the bytes the marks of that take (one bit an
    /// entry).
    pub fn checked_count(&self) -> (usize, usize) {
        let count = (0..self.all_entries.len())
            .filter(|i| self.checked.get(*i))
            .count();
        (count, self.checked.bytes())
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

    /// The details each row says after its name: built-in kinds and
    /// fields' keys.
    pub fn columns(&self) -> &[ColumnSpec] {
        &self.columns
    }

    /// Changes the details each row says after its name (a folder's own
    /// view, W3l); the focus and the marks stay by id. The app resolves
    /// the keys among them with [`Panel::set_field_columns`].
    pub fn set_columns(&mut self, columns: Vec<ColumnSpec>, clock: &dyn Clock) -> RowsReplaced {
        self.columns = columns;
        self.refresh(clock)
    }

    /// The columns of fields' keys, resolved (alpha 4, lane MA).
    pub fn field_columns(&self) -> &[FieldColumn] {
        &self.fields
    }

    /// Takes the resolved columns of the fields' keys among
    /// [`Panel::columns`]: each regex column's cells are made from the
    /// names, and the rows are made again.
    pub fn set_field_columns(
        &mut self,
        fields: Vec<FieldColumn>,
        clock: &dyn Clock,
    ) -> RowsReplaced {
        self.fields = fields;
        let mut entries = std::mem::take(&mut self.all_entries);
        self.fill_regex_cells(&mut entries);
        self.all_entries = entries;
        self.refresh(clock)
    }

    /// Makes the regex columns' cells from each entry's name.
    fn fill_regex_cells(&self, entries: &mut [Entry]) {
        let prefix = diroxide_config::columns::REGEX_PREFIX;
        for entry in entries.iter_mut() {
            if !entry.extra.is_empty() {
                entry.extra.retain(|key, _| !key.starts_with(prefix));
            }
        }
        for field in &self.fields {
            let Source::Regex(Some(pattern)) = &field.source else {
                continue;
            };
            for entry in entries.iter_mut() {
                if let Some(cell) = crate::columns::regex_cell(pattern, &entry.name) {
                    entry.extra.insert(field.key.clone(), cell);
                }
            }
        }
    }

    /// Whether this panel's plugins' columns are asked: a folder of this
    /// computer.
    pub fn asks_plugin_columns(&self) -> bool {
        self.place.kind() == diroxide_core::PlaceKind::Local
    }

    /// The row an entry has here: its name, its labels, then each
    /// column's words in the columns' order (see [`row_with_fields`]).
    fn row_of(&self, entry: &Entry, clock: &dyn Clock) -> ListRow {
        row_with_fields(
            entry,
            &self.columns,
            &self.fields,
            (self.reads_contents(), self.asks_plugin_columns()),
            clock,
        )
    }

    /// The files whose cells the readers still owe, in the order they
    /// take them: first the visible page and two pages either side
    /// ([`crate::columns::window_order`]), nearest the focus first, for
    /// the columns `near` takes; then every other file of the listing for
    /// the columns `rest` takes. Only columns the metadata readers fill
    /// count. The number of first items on screen comes second. Empty
    /// where contents are not read.
    pub fn column_queue(
        &self,
        near: &dyn Fn(&FieldColumn) -> bool,
        rest: &dyn Fn(&FieldColumn) -> bool,
        types: &TypeRegistry,
    ) -> (Vec<ColumnItem>, usize) {
        let read = |f: &&FieldColumn| f.cost().is_some();
        let near: Vec<&FieldColumn> = self
            .fields
            .iter()
            .filter(read)
            .filter(|f| near(f))
            .collect();
        let rest: Vec<&FieldColumn> = self
            .fields
            .iter()
            .filter(read)
            .filter(|f| rest(f))
            .collect();
        if (near.is_empty() && rest.is_empty()) || !self.reads_contents() {
            return (Vec::new(), 0);
        }
        let item = |i: usize, entry: &Entry, wanted: &[&FieldColumn]| {
            let keys: Vec<String> = wanted
                .iter()
                .filter(|f| f.waits(entry))
                .map(|f| f.key.clone())
                .collect();
            (!keys.is_empty()).then(|| {
                let file_type = types.by_index(entry.type_index);
                ColumnItem {
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    name: entry.name.clone(),
                    kind: entry.kind,
                    size: entry.size,
                    modified: entry.modified,
                    type_id: file_type.map(|t| t.id.clone()),
                    group: file_type.and_then(|t| t.group.clone()),
                    keys,
                }
            })
        };
        let index_of: HashMap<&str, usize> = self
            .all_entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.as_str(), i))
            .collect();
        let focus = self.list.focus_index().unwrap_or(0);
        let page = self.list.page_size();
        let mut taken = HashSet::new();
        let mut queue = Vec::new();
        // The window takes the columns of both kinds: what the rest
        // reads is read nearest the focus first too.
        let mut both = near.clone();
        both.extend(rest.iter().filter(|f| !near.iter().any(|n| n.key == f.key)));
        for row in crate::columns::window_order(focus, self.entries.len(), page) {
            if let Some(entry) = self.entries.get(row)
                && let Some(&i) = index_of.get(entry.name.as_str())
                && taken.insert(i)
                && let Some(item) = item(i, &self.all_entries[i], &both)
            {
                queue.push(item);
            }
        }
        let urgent = queue.len().min(page.max(1));
        if !rest.is_empty() {
            for (i, entry) in self.all_entries.iter().enumerate() {
                if !taken.contains(&i)
                    && let Some(item) = item(i, entry, &rest)
                {
                    queue.push(item);
                }
            }
        }
        (queue, urgent)
    }

    /// Takes what the column readers found for files of this listing:
    /// each file's cells go into its entry. Returns whether a cell
    /// reached the page around the focus, whose rows are made again at
    /// once; the others wait for [`Panel::put_stale_rows`], when the focus
    /// comes near or the reading ends.
    pub fn apply_column_cells(&mut self, cells: Vec<ColumnCell>, clock: &dyn Clock) -> bool {
        if self.shown_index.is_none() {
            self.shown_index = Some(
                self.entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (e.name.clone(), i))
                    .collect(),
            );
        }
        let mut touched = Vec::new();
        for cell in cells {
            let Some(entry) = self.all_entries.get_mut(cell.index as usize) else {
                continue;
            };
            if entry.name != cell.name {
                continue;
            }
            for (key, value) in &cell.values {
                entry.extra.insert(key.clone(), value.clone());
            }
            if let Some(&row) = self
                .shown_index
                .as_ref()
                .and_then(|index| index.get(&cell.name))
                && let Some(shown) = self.entries.get_mut(row)
            {
                for (key, value) in cell.values {
                    shown.extra.insert(key, value);
                }
                touched.push(row);
            }
        }
        if touched.is_empty() {
            return false;
        }
        self.stale_rows.extend(touched);
        if self.stale_near_focus() {
            self.put_stale_rows(clock);
            return true;
        }
        false
    }

    /// Whether a row waiting for its new cells is within a page of the
    /// focus.
    pub fn stale_near_focus(&self) -> bool {
        let focus = self.list.focus_index().unwrap_or(0);
        let page = self.list.page_size().max(1);
        self.stale_rows
            .range(focus.saturating_sub(page)..=focus.saturating_add(page))
            .next()
            .is_some()
    }

    /// Whether rows wait for their new cells.
    pub fn has_stale_rows(&self) -> bool {
        !self.stale_rows.is_empty()
    }

    /// Puts the cells that came into the rows that wait for them; the
    /// order stays, the focus and the marks kept by id.
    pub fn put_stale_rows(&mut self, clock: &dyn Clock) {
        if self.stale_rows.is_empty() {
            return;
        }
        let stale = std::mem::take(&mut self.stale_rows);
        let mut rows = self.list.rows().to_vec();
        for row in stale {
            if let (Some(slot), Some(entry)) = (rows.get_mut(row), self.entries.get(row)) {
                let selected = slot.selected;
                *slot = self.row_of(entry, clock);
                slot.selected = selected;
            }
        }
        self.list.set_rows(rows);
    }

    /// How many entries still wait for a column's cell: for the
    /// ask-first sort. None where the column is not read or asked.
    pub fn waiting_for(&self, key: &str) -> usize {
        let Some(field) = self.fields.iter().find(|f| f.key == key) else {
            return 0;
        };
        let asked = match field.source {
            Source::Meta(_) => self.reads_contents(),
            Source::Plugin(_) => self.asks_plugin_columns(),
            _ => false,
        };
        if !asked {
            return 0;
        }
        self.all_entries.iter().filter(|e| field.waits(e)).count()
    }

    /// The entries a plugin's column still waits for, each with its index
    /// in the listing: the visible page and two pages either side,
    /// nearest the focus first, then with `whole` every other one. Empty
    /// where plugins' columns are not asked.
    pub fn plugin_queue(&self, key: &str, whole: bool) -> Vec<(u32, String)> {
        let Some(field) = self
            .fields
            .iter()
            .find(|f| f.key == key && f.plugin().is_some())
        else {
            return Vec::new();
        };
        if !self.asks_plugin_columns() {
            return Vec::new();
        }
        let index_of: HashMap<&str, usize> = self
            .all_entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.as_str(), i))
            .collect();
        let focus = self.list.focus_index().unwrap_or(0);
        let page = self.list.page_size();
        let mut taken = HashSet::new();
        let mut queue = Vec::new();
        let index = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
        for row in crate::columns::window_order(focus, self.entries.len(), page) {
            if let Some(entry) = self.entries.get(row)
                && let Some(&i) = index_of.get(entry.name.as_str())
                && taken.insert(i)
                && field.waits(entry)
            {
                queue.push((index(i), entry.name.clone()));
            }
        }
        if whole {
            for (i, entry) in self.all_entries.iter().enumerate() {
                if !taken.contains(&i) && field.waits(entry) {
                    queue.push((index(i), entry.name.clone()));
                }
            }
        }
        queue
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

    /// Notes the sizes of folders counted here (Folder sizes, Size every
    /// folder): their rows say the size, and the size sort uses it, until
    /// the panel leaves the folder.
    pub fn set_folder_sizes(
        &mut self,
        sizes: &HashMap<String, u64>,
        clock: &dyn Clock,
    ) -> RowsReplaced {
        for (name, size) in sizes {
            self.counted.insert(name.clone(), *size);
        }
        for entry in &mut self.all_entries {
            if entry.kind == EntryKind::Folder
                && let Some(size) = self.counted.get(&entry.name)
            {
                entry.size = Some(*size);
            }
        }
        self.refresh(clock)
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
            types: self.types.as_deref().map(|t| (t, clock)),
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
                    types: self.types.as_deref().map(|t| (t, clock)),
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
        let rows = shown.iter().map(|e| self.row_of(e, clock)).collect();
        self.entries = shown;
        // Every row is made again: none waits for its cells, and the rows'
        // places by name are made again when cells next come.
        self.stale_rows.clear();
        self.shown_index = None;
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

    fn short_of(types: &TypeRegistry, id: &str) -> String {
        types.get(id).unwrap().short.clone()
    }

    fn answer(panel: &Panel, types: &TypeRegistry, name: &str, head: &[u8]) -> TypeAnswer {
        let index = panel
            .all_entries()
            .iter()
            .position(|e| e.name == name)
            .unwrap();
        let item = TypeItem::of(index, &panel.all_entries()[index]);
        crate::typing::check_item(types, &item, Some(head), &clock())
    }

    /// Lane TA, commit 29: a row says its name's type at once and "type
    /// pending" only where the name says nothing; the check reads from
    /// the focus out, the rows on screen first; its answers change the
    /// rows and keep the line where name and contents disagree; a refresh
    /// of the same folder keeps what was checked.
    #[test]
    fn the_type_check_reads_from_the_focus_out_and_keeps_its_answers() {
        let clock = clock();
        let types = TypeRegistry::builtin();
        let mut panel = panel();
        let fresh = || {
            let mut entries: Vec<Entry> = (0..10)
                .map(|i| Entry::new(format!("a{i}"), EntryKind::File).with_size(10))
                .collect();
            entries.push(Entry::new("empty", EntryKind::File).with_size(0));
            entries.push(Entry::new("photo.png", EntryKind::File).with_size(10));
            entries
        };
        panel.set_entries(fresh(), &types, &clock);
        let spoken: Vec<String> = panel.list().rows().iter().map(|r| r.spoken()).collect();
        assert_eq!(spoken[0], "a0, type pending, 10 bytes");
        assert_eq!(
            spoken[10], "empty, file, 0 bytes",
            "an empty file is never read"
        );
        assert_eq!(spoken[11], "photo.png, PNG, 10 bytes");
        assert!(within_forty(TYPE_PENDING));

        panel.list_mut().focus_id("a5");
        let order: Vec<String> = panel.type_queue().into_iter().map(|i| i.name).collect();
        assert_eq!(
            order,
            [
                "a5",
                "a6",
                "a4",
                "a7",
                "a3",
                "a8",
                "a2",
                "a9",
                "a1",
                "a0",
                "photo.png"
            ]
        );

        let script = answer(&panel, &types, "a5", b"#!/bin/sh\necho hello\n");
        let jpeg = answer(&panel, &types, "photo.png", b"\xFF\xD8\xFF\xE0\0\x10JFIF\0");
        let generation = panel.entries_generation();
        assert!(panel.apply_type_answers(vec![script, jpeg], &types, &clock));
        assert_eq!(
            panel.entries_generation(),
            generation,
            "answers keep the listing"
        );
        let row = |panel: &Panel, name: &str| {
            panel
                .list()
                .rows()
                .iter()
                .find(|r| r.id == name)
                .unwrap()
                .spoken()
        };
        let shell = short_of(&types, "shell");
        assert_eq!(row(&panel, "a5"), format!("a5, {shell}, 10 bytes"));
        assert_eq!(row(&panel, "photo.png"), "photo.png, JPEG, 10 bytes");
        assert_eq!(
            panel.disagreement("photo.png"),
            Some("Name says PNG; contents say JPEG")
        );
        assert_eq!(panel.disagreement("a5"), None, "the name gave nothing");
        assert_eq!(panel.list().focused().unwrap().id, "a5");
        let left: Vec<String> = panel.type_queue().into_iter().map(|i| i.name).collect();
        assert_eq!(left.len(), 9);
        assert!(!left.contains(&"photo.png".to_owned()));

        // A refresh of the same folder keeps the checked types of the
        // files that did not change; a changed file is checked again.
        let mut again = fresh();
        again[0] = Entry::new("a0", EntryKind::File).with_size(11);
        panel.set_entries(again, &types, &clock);
        assert_eq!(row(&panel, "photo.png"), "photo.png, JPEG, 10 bytes");
        assert_eq!(row(&panel, "a5"), format!("a5, {shell}, 10 bytes"));
        assert!(panel.disagreement("photo.png").is_some());
        assert_eq!(panel.checked_count().0, 2);
        assert_eq!(panel.type_queue().len(), 9);
        let stale = answer(&panel, &types, "a1", b"plain");
        let renamed = TypeAnswer {
            name: "gone".to_owned(),
            ..stale
        };
        assert!(!panel.apply_type_answers(vec![renamed], &types, &clock));
    }

    /// Lane TA, commit 29: a place whose contents are never read (an
    /// archive, a folder of `check.skip`) types by name alone, with no
    /// "type pending" and nothing to read.
    #[test]
    fn a_place_typed_by_name_alone_has_nothing_pending() {
        let clock = clock();
        let types = TypeRegistry::builtin();
        let mut panel = panel();
        panel.set_check_skip(CheckSkip::new(&["/home".to_owned()]));
        assert!(!panel.reads_contents());
        panel.set_entries(
            vec![Entry::new("data", EntryKind::File).with_size(5)],
            &types,
            &clock,
        );
        assert_eq!(panel.list().rows()[0].spoken(), "data, file, 5 bytes");
        assert!(panel.type_queue().is_empty());
        panel.set_check_skip(CheckSkip::default());
        panel.retype(&types, &clock);
        assert_eq!(
            panel.list().rows()[0].spoken(),
            "data, type pending, 5 bytes"
        );
        assert_eq!(panel.type_queue().len(), 1);
    }

    /// Lane TA, commit 29: at 100,000 entries a checked type costs the
    /// entry's two-byte index and one bit of the panel's marks; the
    /// answers that agree with the names change no row.
    #[test]
    fn a_hundred_thousand_checked_entries_cost_two_bytes_each() {
        let clock = clock();
        let types = TypeRegistry::builtin();
        let entries: Vec<Entry> = (0..100_000u64)
            .map(|i| Entry::new(format!("file{i}.txt"), EntryKind::File).with_size(i + 1))
            .collect();
        let mut panel = panel();
        panel.set_entries(entries, &types, &clock);
        let text = panel.all_entries()[0].type_index;
        assert_ne!(text, 0);
        assert_eq!(size_of_val(&text), 2);
        let answers: Vec<TypeAnswer> = panel
            .type_queue()
            .into_iter()
            .map(|item| TypeAnswer {
                index: item.index,
                name: item.name,
                found: Ok((text, TypeStage::Name)),
            })
            .collect();
        assert_eq!(answers.len(), 100_000);
        assert!(!panel.apply_type_answers(answers, &types, &clock));
        assert_eq!(panel.checked_count(), (100_000, 12_504));
        assert!(panel.type_queue().is_empty());
    }

    fn within_forty(line: &str) -> bool {
        line.chars().count() <= diroxide_core::format::BRAILLE_CELLS
    }
}
