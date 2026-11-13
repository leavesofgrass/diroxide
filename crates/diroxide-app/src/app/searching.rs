//! Search in the app (W3a, plan commit 37): the search dialog as a list
//! of settings, the results lister, and what the results do.
//!
//! - **The dialog** (Find files): a list named Search whose first row
//!   says what will happen and starts it ("Search work for *.txt"), then
//!   one labeled setting a row, meaning first: "Name or filter: *.txt",
//!   "Contains text: none", "Look in: work", "Hidden files: left out",
//!   "Ignore files: not read", "Links to folders: not followed", "Inside
//!   archives: not searched", "Depth: any", "Rows: one per file",
//!   "Online-only files: left out". Enter changes a setting; typing on a
//!   text row starts its field with what was typed, and Enter in a text
//!   field starts the search.
//! - **The results lister**: a list that streams the hits in arrival
//!   order and never moves the focus under the owner. "Searching work for
//!   *.txt" is said once; the count on the status line at the progress
//!   interval; "Done: 4 found in 3 folders. Hidden files skipped." at the
//!   end, or "Stopped at 214 found." after Escape. The panel's keys work
//!   on the real files: View at the match, Edit at the line, Copy, Move,
//!   Trash, Delete for good, mark, and the filter refines the results
//!   without a new walk; the key that goes up shows a result in its
//!   folder. A row is checked when it takes the focus, and one gone since
//!   the list was made says so and is refused.
//!
//! The walk runs on the runtime's worker ([`Request::Search`]); the app
//! keeps the hits and draws them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use diroxide_core::filter::{Filter, Lookup};
use diroxide_core::format::{count, grouped};
use diroxide_core::{
    Command, CommandId, DialogId, Effect, Entry, JobKind, JobRequest, Key, KeyCode, Layer,
    ListModel, ListRow, Message, Move, Place, Redraw, Side, Timestamp,
};
use diroxide_search::{
    ContentPattern, Freshness, OnlineOnly, RowShape, Scope, SearchHit, SearchProgress, SearchQuery,
    SearchSummary,
};

use super::{App, Open, Purpose, Then};
use crate::commands::ids;
use crate::dialog::Dialog;
use crate::entry::{CompletionSource, EntryField};
use crate::jobs::{SIGHTED_INTERVAL_MS, millis_between};
use crate::keymap::Lookup as KeyLookup;
use crate::panel::place_leaf;
use crate::request::{Request, SharedClock, SpawnKind};
use crate::state;

/// The history key of the search dialog's expression field.
const EXPRESSION_HISTORY: &str = "search";
/// The history key of its contents field.
const CONTENT_HISTORY: &str = "search-content";
/// The history key of its place field.
const PLACE_HISTORY: &str = "search-place";
/// How long the focus rests on a result before its file is checked.
const STALE_PAUSE_MS: u64 = 300;

/// The settings of a search, as its dialog shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchForm {
    pub side: Side,
    pub places: Vec<Place>,
    pub expression: String,
    pub content: String,
    pub hidden: bool,
    pub ignore_files: bool,
    pub follow_links: bool,
    pub archives: bool,
    pub depth: Option<u32>,
    pub per_line: bool,
    pub online_read: bool,
}

/// Which text a search setting's field changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchField {
    Expression,
    Content,
    Place,
    Depth,
}

/// One search: what was asked, the hits so far, and how it ended.
#[derive(Clone, Debug)]
pub(super) struct Search {
    id: u64,
    /// The start, when there is one, for the rows' "in Documents/q3".
    root: Option<Place>,
    title: String,
    hits: Vec<SearchHit>,
    running: bool,
    paused: bool,
    found: u64,
    summary: Option<SearchSummary>,
    /// When the progress line was last said.
    said_at: Option<Timestamp>,
    /// A refining filter on the results, with no new walk.
    refine: Option<Filter>,
    /// The rows checked since they took the focus.
    stale: HashMap<String, Freshness>,
    /// The rows taken out of the list by Remove from results.
    removed: HashSet<String>,
    /// The label whose items these are, when the Labels list opened them:
    /// the trash key then takes the label off.
    label: Option<String>,
    /// The day newest files listed, for Earlier day.
    day: Option<diroxide_search::Day>,
    /// What the list is (alpha 5, lane P5): a search's results, a
    /// collection, the flat view, ...
    kind: ListKind,
    /// The panel and tab that asked for it: a list belongs to its tab.
    owner: Option<(Side, u64)>,
    /// Words said after a row's name, by the row's id: a duplicate's
    /// "oldest", a compare's status.
    notes: HashMap<String, String>,
}

/// The results lister taken off the dialog stack while the viewer shows
/// one of its files, put back when the viewer closes.
#[derive(Clone, Debug)]
pub(super) struct Parked {
    id: DialogId,
    dialog: Dialog,
    open: Open,
}

/// What the app keeps for search.
#[derive(Clone, Debug, Default)]
pub(super) struct SearchState {
    next: u64,
    /// The last search, live or ended.
    current: Option<Box<Search>>,
    /// The last dialog's settings, for the next Find files.
    form: Option<SearchForm>,
    /// The dialogs (the results lister, a compare's lists) taken aside
    /// while the viewer shows one of their files, the top last.
    parked: Vec<Parked>,
    /// The viewer opening now came from the results: park them.
    park_on_open: bool,
    /// When the focus last moved on a result, for its check.
    stale_at: Option<Timestamp>,
    /// The line the viewer goes to when the result opening now has one.
    view_line: Option<u64>,
    /// The last search that can be saved, with the name offered for it.
    last: Option<(String, SearchQuery)>,
    /// "Replaces the search results." was said once this session.
    replaced_said: bool,
}

/// A search command of this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Find,
    Results,
    Pause,
    ShowInFolder,
    Remove,
}

const ROUTES: [(CommandId, Do); 5] = [
    (ids::FIND, Do::Find),
    (ids::SEARCH_RESULTS, Do::Results),
    (ids::SEARCH_PAUSE, Do::Pause),
    (ids::SEARCH_SHOW_IN_FOLDER, Do::ShowInFolder),
    (ids::SEARCH_REMOVE_ROW, Do::Remove),
];

/// The commands that act on the list on the screen when the palette or a
/// menu runs them over the results lister (alpha 5, lane AS).
const LIST_COMMANDS: [CommandId; 3] = [
    ids::SAVE_AS_COLLECTION,
    ids::COLLECT,
    ids::COLLECTION_REMOVE_MISSING,
];

fn route(id: &CommandId) -> Option<Do> {
    ROUTES
        .iter()
        .find(|(route, _)| route == id)
        .map(|(_, action)| *action)
}

/// The facts of a hit beyond its entry, for a refining filter: its path
/// and its depth below the start.
struct HitLookup<'a>(&'a SearchHit);

impl Lookup for HitLookup<'_> {
    fn path(&self, _entry: &Entry) -> Option<String> {
        Some(self.0.place.to_string())
    }

    fn depth(&self, _entry: &Entry) -> Option<u32> {
        Some(self.0.depth)
    }
}

/// A place said in a row: its own name, or its title at a top.
fn place_words(places: &[Place]) -> String {
    match places {
        [one] => place_leaf(one),
        many => count(many.len() as u64, "place", "places"),
    }
}

impl SearchForm {
    /// The line the first row says, and what starts: "Search work for
    /// *.txt, containing total due".
    fn summary(&self) -> String {
        let what = if self.expression.trim().is_empty() {
            "every entry".to_owned()
        } else {
            self.expression.trim().to_owned()
        };
        let mut line = format!("Search {} for {what}", place_words(&self.places));
        if !self.content.is_empty() {
            line.push_str(&format!(", containing {}", self.content));
        }
        line
    }

    /// The dialog: the summary row, then each setting.
    fn list(&self) -> ListModel {
        let yes_no =
            |on: bool, yes: &str, no: &str| if on { yes.to_owned() } else { no.to_owned() };
        let expression = if self.expression.trim().is_empty() {
            "every entry".to_owned()
        } else {
            self.expression.clone()
        };
        let content = if self.content.is_empty() {
            "none".to_owned()
        } else {
            self.content.clone()
        };
        let place = match self.places.as_slice() {
            [one] => place_leaf(one),
            many => format!("{} marked items", many.len()),
        };
        let depth = match self.depth {
            None => "any".to_owned(),
            Some(n) => count(u64::from(n), "level down", "levels down"),
        };
        let rows = vec![
            ListRow::new("go", self.summary()),
            ListRow::new("expression", format!("Name or filter: {expression}")),
            ListRow::new("content", format!("Contains text: {content}")),
            ListRow::new("place", format!("Look in: {place}")),
            ListRow::new(
                "hidden",
                format!(
                    "Hidden files: {}",
                    yes_no(self.hidden, "searched", "left out")
                ),
            ),
            ListRow::new(
                "ignore",
                format!(
                    "Ignore files: {}",
                    yes_no(
                        self.ignore_files,
                        "read, ignored files left out",
                        "not read"
                    )
                ),
            ),
            ListRow::new(
                "links",
                format!(
                    "Links to folders: {}",
                    yes_no(self.follow_links, "followed", "not followed")
                ),
            ),
            ListRow::new(
                "archives",
                format!(
                    "Inside archives: {}",
                    yes_no(self.archives, "searched", "not searched")
                ),
            ),
            ListRow::new("depth", format!("Depth: {depth}")),
            ListRow::new(
                "rows",
                format!(
                    "Rows: {}",
                    yes_no(self.per_line, "one per matching line", "one per file")
                ),
            ),
            ListRow::new(
                "online",
                format!(
                    "Online-only files: {}",
                    yes_no(self.online_read, "read after asking", "left out")
                ),
            ),
        ];
        let mut list = ListModel::new("Search").with_rows(rows);
        list.set_help(
            "Up and Down choose, Enter changes a setting or starts the search, typing on a text row fills it, Escape closes.",
        );
        list
    }
}

impl App {
    /// Runs a search command; `None` when the command is another's.
    pub(super) fn search_dispatch(&mut self, command: &Command) -> Option<Vec<Effect>> {
        let action = route(&command.id)?;
        let in_results = self.results_on_top();
        Some(match action {
            Do::Find if in_results => {
                let mut effects = self.close_top();
                effects.extend(self.find_files());
                effects
            }
            Do::Pause => self.pause_search(),
            Do::ShowInFolder if in_results => self.show_result_in_folder(),
            Do::Remove if in_results => self.remove_result_row(),
            _ if !self.dialogs.is_empty() => {
                vec![self.say(Message::status("Close this dialog first."))]
            }
            Do::Find => self.find_files(),
            Do::Results => self.reopen_results(),
            Do::ShowInFolder | Do::Remove => {
                vec![self.say(Message::status("No search results are open."))]
            }
        })
    }

    /// Whether the results lister is the dialog on top.
    fn results_on_top(&self) -> bool {
        matches!(
            self.opens.last().map(|o| &o.purpose),
            Some(Purpose::Results)
        )
    }

    /// Whether the search results are on the screen, so the key bar shows
    /// their keys.
    pub fn results_shown(&self) -> bool {
        self.results_on_top()
    }

    /// Find files: the search dialog, on the active panel's folder or its
    /// marked entries, with the last search's settings.
    pub(super) fn find_files(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let marked: Vec<Place> = panel
            .list()
            .selected_ids()
            .into_iter()
            .filter_map(|id| panel.place().join(id).ok())
            .collect();
        let places = if marked.is_empty() {
            vec![panel.place().clone()]
        } else {
            marked
        };
        let hidden = panel.shows_hidden();
        let form = match self.searching.form.clone() {
            Some(last) => SearchForm {
                side,
                places,
                hidden,
                ..last
            },
            None => SearchForm {
                side,
                places,
                expression: String::new(),
                content: String::new(),
                hidden,
                ignore_files: self.settings.search_respect_ignore,
                follow_links: false,
                archives: false,
                depth: None,
                per_line: false,
                online_read: false,
            },
        };
        self.show_search_form(form, "expression")
    }

    fn show_search_form(&mut self, form: SearchForm, focus: &str) -> Vec<Effect> {
        let mut list = form.list();
        list.focus_id(focus);
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::FIND),
            },
            Purpose::SearchForm(Box::new(form)),
        )
    }

    /// A letter on a text row of the search dialog: its field opens with
    /// what was typed. `None` for any other key or row.
    pub(super) fn search_form_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        let Some(Purpose::SearchForm(form)) = self.opens.last().map(|o| o.purpose.clone()) else {
            return None;
        };
        let c = super::typed(key)?;
        let row = self.top_row_id()?;
        let field = match row.as_str() {
            "expression" => SearchField::Expression,
            "content" => SearchField::Content,
            "place" => SearchField::Place,
            "depth" => SearchField::Depth,
            _ => return None,
        };
        let mut effects = self.close_top();
        effects.extend(self.search_field(*form, field, Some(c.to_string())));
        Some(effects)
    }

    /// Enter on a row of the search dialog.
    pub(super) fn search_form_row(&mut self, mut form: SearchForm, row: &str) -> Vec<Effect> {
        let mut effects = self.close_top();
        let toggled = |form: &mut SearchForm| match row {
            "hidden" => form.hidden = !form.hidden,
            "ignore" => form.ignore_files = !form.ignore_files,
            "links" => form.follow_links = !form.follow_links,
            "archives" => form.archives = !form.archives,
            "rows" => form.per_line = !form.per_line,
            "online" => form.online_read = !form.online_read,
            _ => {}
        };
        match row {
            "go" => effects.extend(self.start_search(form)),
            "expression" => effects.extend(self.search_field(form, SearchField::Expression, None)),
            "content" => effects.extend(self.search_field(form, SearchField::Content, None)),
            "place" => effects.extend(self.search_field(form, SearchField::Place, None)),
            "depth" => effects.extend(self.search_field(form, SearchField::Depth, None)),
            other => {
                toggled(&mut form);
                effects.extend(self.show_search_form(form, other));
            }
        }
        effects
    }

    /// A text setting's field, with its text, or with what was typed.
    fn search_field(
        &mut self,
        form: SearchForm,
        field: SearchField,
        typed: Option<String>,
    ) -> Vec<Effect> {
        let (label, source, history, text) = match field {
            SearchField::Expression => (
                "Name or filter",
                CompletionSource::Filter,
                EXPRESSION_HISTORY,
                form.expression.clone(),
            ),
            SearchField::Content => (
                "Contains text",
                CompletionSource::History,
                CONTENT_HISTORY,
                form.content.clone(),
            ),
            SearchField::Place => (
                "Look in",
                CompletionSource::Paths,
                PLACE_HISTORY,
                match form.places.as_slice() {
                    [one] => one.to_string(),
                    _ => String::new(),
                },
            ),
            SearchField::Depth => (
                "Depth, levels down, empty for any",
                CompletionSource::None,
                EXPRESSION_HISTORY,
                form.depth.map(|d| d.to_string()).unwrap_or_default(),
            ),
        };
        let mut entry = EntryField::new(label, source).with_text(typed.unwrap_or(text));
        if field != SearchField::Depth {
            entry = entry.with_history(history);
        }
        self.open_dialog(
            Dialog::edit(entry),
            Purpose::SearchText(Box::new(form), field),
        )
    }

    /// Enter in a field of the search dialog: the text and depth start the
    /// search; the place and the depth go back to the dialog.
    pub(super) fn search_text_entered(
        &mut self,
        mut form: SearchForm,
        field: SearchField,
        text: &str,
    ) -> Vec<Effect> {
        match field {
            SearchField::Expression => {
                if let Err(error) = self.read_filter(text) {
                    return vec![self.say(Message::error(error.line()))];
                }
                form.expression = text.trim().to_owned();
                let mut effects = self.close_top();
                effects.extend(self.start_search(form));
                effects
            }
            SearchField::Content => {
                form.content = text.to_owned();
                let mut effects = self.close_top();
                effects.extend(self.start_search(form));
                effects
            }
            SearchField::Place => match self.typed_place(form.side, text) {
                Ok(place) => {
                    self.remember(PLACE_HISTORY, &place.to_string());
                    form.places = vec![place];
                    let mut effects = self.close_top();
                    effects.extend(self.show_search_form(form, "place"));
                    effects
                }
                Err(message) => vec![self.say(message)],
            },
            SearchField::Depth => {
                let text = text.trim();
                if text.is_empty() {
                    form.depth = None;
                } else {
                    match text.parse::<u32>() {
                        Ok(n) if n > 0 => form.depth = Some(n),
                        _ => {
                            let line = "Type a number of levels, such as 2, or leave it empty";
                            return vec![self.say(Message::error(line))];
                        }
                    }
                }
                let mut effects = self.close_top();
                effects.extend(self.show_search_form(form, "depth"));
                effects
            }
        }
    }

    /// Escape in a field of the search dialog: back to the dialog.
    pub(super) fn search_text_escaped(
        &mut self,
        form: SearchForm,
        field: SearchField,
    ) -> Vec<Effect> {
        let row = match field {
            SearchField::Expression => "expression",
            SearchField::Content => "content",
            SearchField::Place => "place",
            SearchField::Depth => "depth",
        };
        self.show_search_form(form, row)
    }

    /// The line said when a search starts and the results' title, the
    /// expression as typed in the first and read back in the second:
    /// "Searching work for *.txt", "Results: name matches *.txt"; a text
    /// to find inside files follows as ", containing total due".
    fn search_words(&self, form: &SearchForm) -> (String, String) {
        let typed = form.expression.trim();
        let (typed, sentence) = if typed.is_empty() {
            ("every entry".to_owned(), "every entry".to_owned())
        } else {
            let sentence = self
                .read_filter(typed)
                .ok()
                .flatten()
                .map_or_else(|| typed.to_owned(), |f| f.sentence());
            (typed.to_owned(), sentence)
        };
        let containing = if form.content.is_empty() {
            String::new()
        } else {
            format!(", containing {}", form.content)
        };
        let place = diroxide_core::names::display_name(&place_words(&form.places)).into_owned();
        (
            format!("Searching {place} for {typed}{containing}"),
            format!("Results: {sentence}{containing}"),
        )
    }

    /// The query for a dialog's settings, or the problem in words.
    fn query_for(&self, form: &SearchForm) -> Result<SearchQuery, Message> {
        let options = self.filter_options();
        let root = form
            .places
            .first()
            .cloned()
            .ok_or_else(|| Message::error("Choose a folder to search in."))?;
        let mut query = SearchQuery::parse_with(root, &form.expression, &options)
            .map_err(|error| Message::error(error.sentences()))?;
        if !form.content.is_empty() {
            let case_sensitive = match options.case {
                diroxide_core::pattern::CaseRule::Smart => {
                    form.content.chars().any(char::is_uppercase)
                }
                diroxide_core::pattern::CaseRule::Match => true,
                _ => false,
            };
            query = query
                .with_content(&ContentPattern::literal(&form.content, case_sensitive))
                .map_err(|error| Message::error(error.sentences()))?;
        }
        let scope = Scope::of(form.places.clone())
            .with_hidden(form.hidden)
            .with_ignore_files(form.ignore_files)
            .with_follow_links(form.follow_links)
            .with_archives(form.archives)
            .with_max_depth(form.depth)
            .with_online_only(if form.online_read {
                OnlineOnly::Read
            } else {
                OnlineOnly::Skip
            })
            .with_row_cap(Some(self.settings.search_row_cap))
            .with_skip_places(
                self.organize
                    .labels
                    .places_with(crate::labels::SKIP_IN_SEARCHES)
                    .into_iter()
                    .cloned()
                    .collect(),
            );
        let rows = if form.per_line {
            RowShape::PerLine
        } else {
            RowShape::PerFile
        };
        Ok(query.with_scope(scope).with_rows(rows))
    }

    /// Starts a search and opens its results lister: "Searching work for
    /// *.txt" is said once.
    pub(super) fn start_search(&mut self, form: SearchForm) -> Vec<Effect> {
        let query = match self.query_for(&form) {
            Ok(query) => query,
            Err(message) => {
                let mut effects = vec![self.say(message)];
                effects.extend(self.show_search_form(form, "expression"));
                return effects;
            }
        };
        if !form.expression.is_empty() {
            self.remember(EXPRESSION_HISTORY, &form.expression);
        }
        if !form.content.is_empty() {
            self.remember(CONTENT_HISTORY, &form.content);
        }
        self.searching.form = Some(form.clone());
        self.searching.next += 1;
        let id = self.searching.next;
        let root = match form.places.as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        };
        let (start, title) = self.search_words(&form);
        self.keep_last_search(&title, &query);
        self.request(Request::Search {
            search: id,
            query: Box::new(query.clone()),
            clock: SharedClock(Arc::clone(&self.clock)),
        });
        self.searching.current = Some(Box::new(Search {
            id,
            root,
            title,
            hits: Vec::new(),
            running: true,
            paused: false,
            found: 0,
            summary: None,
            said_at: Some(self.now()),
            refine: None,
            stale: HashMap::new(),
            removed: HashSet::new(),
            label: None,
            day: None,
            kind: ListKind::Search,
            owner: Some((self.active, self.panel(self.active).tab_id())),
            notes: HashMap::new(),
        }));
        self.searching.parked.clear();
        let list = self.results_model();
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SEARCH_RESULTS),
            },
            Purpose::Results,
            Message::info(start),
        )
    }

    /// Runs a search from its query (a saved search), titled by its name
    /// when it has one.
    pub(super) fn start_query_search(
        &mut self,
        query: SearchQuery,
        name: Option<String>,
    ) -> Vec<Effect> {
        let root = match query.scope.places.as_slice() {
            [one] => Some(one.clone()),
            _ => None,
        };
        let (title, start) = match &name {
            Some(name) => (
                format!("Results: {name}"),
                format!("Running {name}: {}", query.start_line()),
            ),
            None => (format!("Results: {}", query.sentence()), query.start_line()),
        };
        self.keep_last_search(&title, &query);
        let id = self.begin_results(root, title);
        self.request(Request::Search {
            search: id,
            query: Box::new(query),
            clock: SharedClock(Arc::clone(&self.clock)),
        });
        self.open_results_saying(start)
    }

    /// A new search's state, running, its results empty: returns its
    /// number.
    pub(super) fn begin_results(&mut self, root: Option<Place>, title: String) -> u64 {
        self.searching.next += 1;
        let id = self.searching.next;
        self.searching.current = Some(Box::new(Search {
            id,
            root,
            title,
            hits: Vec::new(),
            running: true,
            paused: false,
            found: 0,
            summary: None,
            said_at: Some(self.now()),
            refine: None,
            stale: HashMap::new(),
            removed: HashSet::new(),
            label: None,
            day: None,
            kind: ListKind::Search,
            owner: Some((self.active, self.panel(self.active).tab_id())),
            notes: HashMap::new(),
        }));
        self.searching.parked.clear();
        id
    }

    /// Opens the results lister of the current search, saying `start`.
    pub(super) fn open_results_saying(&mut self, start: String) -> Vec<Effect> {
        let list = self.results_model();
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SEARCH_RESULTS),
            },
            Purpose::Results,
            Message::info(start),
        )
    }

    /// Hits found all at once (the largest items, newest files): they fill
    /// the results, the end said; newest files retitle them with the day.
    pub(super) fn finish_results(
        &mut self,
        id: u64,
        hits: Vec<SearchHit>,
        line: String,
        newest: Option<(String, Option<diroxide_search::Day>)>,
    ) -> Vec<Effect> {
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        search.running = false;
        search.found = hits.len() as u64;
        search.hits = hits;
        if let Some((title, day)) = newest {
            search.title = title;
            search.day = day;
            search.kind = ListKind::Newest;
        }
        let title = search.title.clone();
        let rows = match self.searching.current.as_ref() {
            Some(search) => self.result_rows(search),
            None => Vec::new(),
        };
        if let Some(list) = self.results_list_mut() {
            list.set_rows(rows);
            list.set_title(title);
            list.set_empty_text("Nothing found");
        }
        self.searching.stale_at = Some(self.now());
        vec![Effect::Redraw(Redraw::All), self.say(Message::done(line))]
    }

    /// A walk for the results failed: it is said, and the list stays.
    /// A list that streamed ended with its own line (alpha 5, lane AS):
    /// "Flat view on, 3 files in 3 folders." The list stops running and
    /// the line is said; an empty list says "Nothing found". A line for
    /// a list that is no longer on the screen is dropped.
    pub(super) fn finish_list(&mut self, id: u64, line: Message) -> Vec<Effect> {
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        search.running = false;
        search.paused = false;
        let empty = search.hits.is_empty();
        if empty && let Some(list) = self.results_list_mut() {
            list.set_empty_text("Nothing found");
        }
        vec![Effect::Redraw(Redraw::All), self.say(line)]
    }

    pub(super) fn finish_results_failed(&mut self, id: u64, message: Message) -> Vec<Effect> {
        if let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) {
            search.running = false;
        }
        if let Some(list) = self.results_list_mut() {
            list.set_empty_text("Nothing found");
        }
        vec![Effect::Redraw(Redraw::All), self.say(message)]
    }

    /// The day the newest files on the screen were changed.
    pub(super) fn newest_day(&self) -> Option<diroxide_search::Day> {
        self.searching.current.as_ref()?.day
    }

    /// The last search's query and a name to save it under: its sentence.
    pub(super) fn last_search_query(&self) -> Option<(String, SearchQuery)> {
        self.searching.last.clone()
    }

    /// Notes a search that can be saved: Find files' and a saved one's,
    /// not a label's items, the largest items or newest files.
    fn keep_last_search(&mut self, title: &str, query: &SearchQuery) {
        let name = title.strip_prefix("Results: ").unwrap_or(title).to_owned();
        self.searching.last = Some((name, query.clone()));
    }

    /// Enter on a row of the Labels list: the label's items as results,
    /// each looked up where it was labeled; a missing one says so, and
    /// the trash key takes the label off.
    pub(super) fn labeled_results(&mut self, label: &str) -> Vec<Effect> {
        let places: Vec<Place> = self
            .organize
            .labels
            .places_with(label)
            .into_iter()
            .cloned()
            .collect();
        let name = crate::labels::in_sentence(label);
        if places.is_empty() {
            return vec![self.say(Message::info(format!("No item has the label {name}.")))];
        }
        self.searching.next += 1;
        let id = self.searching.next;
        self.request(Request::StatPlaces {
            search: id,
            places: places.clone(),
        });
        self.searching.current = Some(Box::new(Search {
            id,
            root: None,
            title: format!("Results: label {name}"),
            hits: Vec::new(),
            running: true,
            paused: false,
            found: 0,
            summary: None,
            said_at: Some(self.now()),
            refine: None,
            stale: HashMap::new(),
            removed: HashSet::new(),
            label: Some(label.to_owned()),
            day: None,
            kind: ListKind::Labels,
            owner: Some((self.active, self.panel(self.active).tab_id())),
            notes: HashMap::new(),
        }));
        self.searching.parked.clear();
        let list = self.results_model();
        let start = format!(
            "Looking for {} labeled {name}",
            count(places.len() as u64, "item", "items")
        );
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SEARCH_RESULTS),
            },
            Purpose::Results,
            Message::info(start),
        )
    }

    /// A label's items were looked up: the ones there join the results,
    /// the missing ones too, said so.
    pub(super) fn labeled_found(
        &mut self,
        id: u64,
        found: Vec<SearchHit>,
        missing: Vec<Place>,
        unreadable: usize,
    ) -> Vec<Effect> {
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        let label = search.label.clone().unwrap_or_default();
        let mut hits = found;
        for hit in &mut hits {
            hit.entry.labels = self.organize.labels.labels_of(&hit.place).to_vec();
        }
        for place in &missing {
            let name = place.name().unwrap_or_default().to_owned();
            let entry = Entry::new(name, diroxide_core::EntryKind::File);
            let hit = SearchHit::new(place.clone(), entry);
            search.stale.insert(hit.id(), Freshness::Gone);
            hits.push(hit);
        }
        let there = hits.len() - missing.len();
        search.running = false;
        search.found = hits.len() as u64;
        search.hits = hits;
        let rows = match self.searching.current.as_ref() {
            Some(search) => self.result_rows(search),
            None => Vec::new(),
        };
        if let Some(list) = self.results_list_mut() {
            list.set_rows(rows);
            list.set_empty_text("Nothing found");
        }
        self.searching.stale_at = None;
        let name = crate::labels::in_sentence(&label);
        let mut line = format!(
            "Done: {} labeled {name}",
            count(there as u64, "item", "items")
        );
        if !missing.is_empty() {
            line.push_str(&format!(", {} missing", missing.len()));
        }
        if unreadable > 0 {
            line.push_str(&format!(", {unreadable} not checked"));
        }
        line.push('.');
        vec![Effect::Redraw(Redraw::All), self.say(Message::done(line))]
    }

    /// The results as a list, the refining filter and removed rows left
    /// out, in arrival order.
    fn results_model(&self) -> ListModel {
        let Some(search) = self.searching.current.as_ref() else {
            return ListModel::new("Results");
        };
        let mut list = ListModel::new(search.title.clone()).with_rows(self.result_rows(search));
        list.set_empty_text(if search.running {
            "Searching, nothing found yet"
        } else {
            "Nothing found"
        });
        list.set_help(
            "Enter opens, the view key views at the match, the edit key edits at the line, the copy, move and trash keys act on the files, Escape stops or closes.",
        );
        list
    }

    fn shows_hit(search: &Search, hit: &SearchHit, clock: &dyn diroxide_core::Clock) -> bool {
        if search.removed.contains(&hit.id()) {
            return false;
        }
        let Some(filter) = &search.refine else {
            return true;
        };
        let evaluator = filter.evaluator(clock);
        if evaluator.matches(&hit.entry, &HitLookup(hit)) {
            return true;
        }
        // In a flat view the filter also matches the folder words said
        // after the name, "work/b/deep" (alpha 5, lane AS): the folder
        // text stands in for the name.
        if search.kind == ListKind::Flat {
            let words = hit.folder_words(search.root.as_ref());
            let folder = words.strip_prefix("in ").unwrap_or(&words);
            if !folder.is_empty() {
                let mut entry = hit.entry.clone();
                entry.name = folder.to_owned();
                return evaluator.matches(&entry, &HitLookup(hit));
            }
        }
        false
    }

    fn result_rows(&self, search: &Search) -> Vec<ListRow> {
        let clock = self.clock.as_ref();
        search
            .hits
            .iter()
            .filter(|hit| Self::shows_hit(search, hit, clock))
            .map(|hit| Self::result_row(search, hit))
            .collect()
    }

    /// A result's row: the hit's words, with "missing" after the name of a
    /// labeled item that is gone.
    fn result_row(search: &Search, hit: &SearchHit) -> ListRow {
        let row = hit.row(search.root.as_ref());
        let gone = search.stale.get(&hit.id()) == Some(&Freshness::Gone);
        if (search.label.is_some() || search.kind.trash_removes_item()) && gone {
            let mut details = vec!["missing".to_owned()];
            details.push(hit.folder_words(None));
            return ListRow::new(row.id, row.label).with_details(details);
        }
        // The list's own words after the name (alpha 5): a duplicate's
        // "oldest", a compare's status.
        if let Some(note) = search.notes.get(&row.id) {
            let mut details = vec![note.clone()];
            details.extend(row.details.iter().cloned());
            return ListRow::new(row.id, row.label).with_details(details);
        }
        row
    }

    /// The results lister's list, open or parked.
    fn results_list_mut(&mut self) -> Option<&mut ListModel> {
        if let Some(parked) = self
            .searching
            .parked
            .iter_mut()
            .rfind(|p| matches!(p.open.purpose, Purpose::Results))
        {
            return match &mut parked.dialog {
                Dialog::Choose { list, .. } => Some(list),
                _ => None,
            };
        }
        let index = self
            .opens
            .iter()
            .rposition(|o| matches!(o.purpose, Purpose::Results))?;
        match self.dialogs.get_mut(index) {
            Some((_, Dialog::Choose { list, .. })) => Some(list),
            _ => None,
        }
    }

    /// Hits arrived: they join the list at its end, silently; the focus
    /// stays where it is.
    pub(super) fn search_found(&mut self, id: u64, hits: Vec<SearchHit>) -> Vec<Effect> {
        let clock = Arc::clone(&self.clock);
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        search.found += hits.len() as u64;
        let root = search.root.clone();
        let new_rows: Vec<ListRow> = hits
            .iter()
            .filter(|hit| Self::shows_hit(search, hit, clock.as_ref()))
            .map(|hit| hit.row(root.as_ref()))
            .collect();
        search.hits.extend(hits);
        if new_rows.is_empty() {
            return Vec::new();
        }
        let first = self.results_list_mut().is_some_and(|l| l.is_empty());
        if let Some(list) = self.results_list_mut() {
            let mut rows = list.rows().to_vec();
            rows.extend(new_rows);
            list.set_rows(rows);
        }
        if first {
            self.searching.stale_at = Some(self.now());
        }
        if self.results_on_top() {
            vec![Effect::Redraw(Redraw::All)]
        } else {
            Vec::new()
        }
    }

    /// The pace of the progress line: the progress interval in
    /// screen-reader mode, half a second otherwise.
    fn progress_pace_ms(&self) -> u64 {
        if self.screen_reader {
            self.settings
                .progress_interval_seconds
                .max(1)
                .saturating_mul(1000)
        } else {
            SIGHTED_INTERVAL_MS
        }
    }

    /// How far the search got: its line is said at the progress interval,
    /// never on every report.
    pub(super) fn search_progress(&mut self, id: u64, progress: &SearchProgress) -> Vec<Effect> {
        let now = self.now();
        let pace = self.progress_pace_ms();
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        if !search.running || search.paused {
            return Vec::new();
        }
        let due = search
            .said_at
            .is_none_or(|at| millis_between(at, now) >= pace);
        if !due {
            return Vec::new();
        }
        search.said_at = Some(now);
        let line = progress.line();
        vec![self.say(Message::progress(line))]
    }

    /// The search ended: the count and everything skipped, in one line.
    pub(super) fn search_done(
        &mut self,
        id: u64,
        result: Result<SearchSummary, Message>,
    ) -> Vec<Effect> {
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        search.running = false;
        search.paused = false;
        let summary = match result {
            Ok(summary) => summary,
            Err(message) => return vec![self.say(message)],
        };
        let mut line = if summary.cancelled {
            summary.status_line()
        } else {
            format!("{}.", summary.status_line())
        };
        for sentence in summary.skip_sentences() {
            line.push_str(&format!(" {sentence}"));
        }
        search.summary = Some(summary);
        let empty = search.hits.is_empty();
        if empty && let Some(list) = self.results_list_mut() {
            list.set_empty_text("Nothing found");
        }
        vec![Effect::Redraw(Redraw::All), self.say(Message::done(line))]
    }

    /// Pause search: the walk holds, and resumes on the same command.
    fn pause_search(&mut self) -> Vec<Effect> {
        let Some(search) = self.searching.current.as_mut().filter(|s| s.running) else {
            return vec![self.say(Message::status("No search is running."))];
        };
        search.paused = !search.paused;
        let paused = search.paused;
        let found = grouped(search.found);
        self.request(Request::PauseSearch(paused));
        let line = if paused {
            format!("Paused at {found} found.")
        } else {
            format!("Searching again, {found} found so far.")
        };
        vec![self.say(Message::info(line))]
    }

    /// Search results: the last results again.
    fn reopen_results(&mut self) -> Vec<Effect> {
        if self.searching.current.is_none() {
            return vec![self.say(Message::status("No search yet. Find files starts one."))];
        }
        self.searching.parked.clear();
        let list = self.results_model();
        let line = match self.searching.current.as_ref() {
            Some(search) if search.running => {
                format!(
                    "{}, searching, {} found so far",
                    search.title,
                    grouped(search.found)
                )
            }
            Some(search) => format!("{}, {}", search.title, list.announce_focus()),
            None => String::new(),
        };
        self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SEARCH_RESULTS),
            },
            Purpose::Results,
            Message::info(line),
        )
    }

    /// The hit a results row stands for.
    fn hit_of(&self, row: &str) -> Option<SearchHit> {
        self.searching
            .current
            .as_ref()?
            .hits
            .iter()
            .find(|hit| hit.id() == row)
            .cloned()
    }

    /// The focused result, refused in words when it is known to be gone.
    fn focused_hit(&mut self) -> Result<SearchHit, Vec<Effect>> {
        let Some(row) = self.top_row_id() else {
            let text = match self.searching.current.as_ref() {
                Some(search) if search.running => "Searching, nothing found yet",
                _ => "Nothing found",
            };
            return Err(vec![self.say(Message::status(text))]);
        };
        let gone = self
            .searching
            .current
            .as_ref()
            .and_then(|s| s.stale.get(&row))
            .is_some_and(|f| !f.allows_action());
        let Some(hit) = self.hit_of(&row) else {
            return Err(Vec::new());
        };
        if gone {
            let words = Freshness::Gone.words(&hit.entry.name).unwrap_or_default();
            return Err(vec![
                self.say(Message::error(format!("{words}. Nothing was done."))),
            ]);
        }
        Ok(hit)
    }

    /// The marked results, or the focused one: what an action acts on.
    fn result_targets(&self) -> Vec<Place> {
        let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last() else {
            return Vec::new();
        };
        let frozen = list.freeze();
        let gone: HashSet<&String> = self
            .searching
            .current
            .as_ref()
            .map(|s| {
                s.stale
                    .iter()
                    .filter(|(_, f)| !f.allows_action())
                    .map(|(id, _)| id)
                    .collect()
            })
            .unwrap_or_default();
        let mut places: Vec<Place> = Vec::new();
        for id in &frozen.ids {
            if gone.contains(id) {
                continue;
            }
            if let Some(hit) = self.hit_of(id)
                && !places.contains(&hit.place)
            {
                places.push(hit.place);
            }
        }
        places
    }

    /// A key in the results lister, before the dialog's own: the panel's
    /// file keys act on the results; Escape stops a running search.
    pub(super) fn results_key(&mut self, key: &Key) -> Option<Vec<Effect>> {
        if !self.results_on_top() {
            return None;
        }
        if key.code == KeyCode::Escape && key.mods.is_empty() {
            let running = self.searching.current.as_ref().is_some_and(|s| s.running);
            if running {
                self.request(Request::StopSearch);
                return Some(Vec::new());
            }
            return None;
        }
        let command = match self.keymap.lookup(&[Layer::Panel], key) {
            KeyLookup::Command(id) => id,
            KeyLookup::NoMatch => return None,
        };
        let effects = match command {
            id if id == ids::VIEW => self.view_result(),
            id if id == ids::EDIT => self.edit_result(),
            id if id == ids::COPY => self.result_job(JobKind::Copy),
            id if id == ids::MOVE => self.result_job(JobKind::Move),
            id if id == ids::TRASH && self.labeled_label().is_some() => self.unlabel_result(),
            id if id == ids::TRASH && self.list_collection().is_some() => self.uncollect_result(),
            id if id == ids::TRASH => self.result_job(JobKind::Trash),
            id if id == ids::DELETE_PERMANENTLY => self.result_job(JobKind::DeletePermanently),
            id if id == ids::SELECT_TOGGLE => self.toggle_result_mark(),
            id if id == ids::SELECT_ALL || id == ids::SELECT_NONE || id == ids::SELECT_INVERT => {
                self.mark_all_results(&id)
            }
            id if id == ids::FILTER => self.refine_prompt(),
            id if id == ids::UP => self.show_result_in_folder(),
            id if id == ids::CONTEXT_MENU => self.results_menu(),
            _ => return None,
        };
        Some(effects)
    }

    /// The actions on the results, as a list (the context menu's key):
    /// each runs on the marked results, or the focused one.
    fn results_menu(&mut self) -> Vec<Effect> {
        let mut actions: Vec<(CommandId, &str)> = vec![
            (ids::VIEW, "View at the match"),
            (ids::EDIT, "Edit at the line"),
            (ids::SEARCH_SHOW_IN_FOLDER, "Show in its folder"),
            (ids::COPY, "Copy to other panel"),
            (ids::MOVE, "Move to other panel"),
            (ids::TRASH, "Move to the trash"),
            (ids::FILTER, "Refine results"),
            (ids::SEARCH_REMOVE_ROW, "Remove from results"),
            (ids::SEARCH_PAUSE, "Pause or resume search"),
            (ids::SAVE_AS_COLLECTION, "Save as collection"),
            (ids::COLLECT, "Collect"),
        ];
        if self.list_collection().is_some() {
            actions.push((ids::COLLECTION_REMOVE_MISSING, "Remove missing items"));
        }
        actions.extend([
            (ids::FIND, "Search again"),
            (ids::SEARCH_SAVE, "Save this search"),
            (ids::EARLIER_DAY, "Earlier day"),
        ]);
        let rows = actions
            .iter()
            .map(|(id, name)| {
                let key = self.key(id);
                let row = ListRow::new(id.as_str(), *name);
                if key.is_empty() {
                    row
                } else {
                    row.with_details([key])
                }
            })
            .collect();
        let mut list = ListModel::new("Actions on the results").with_rows(rows);
        list.set_help("Up and Down choose, Enter runs it on the marked results or the focused one, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::CONTEXT_MENU),
            },
            Purpose::ResultsMenu,
        )
    }

    /// An action chosen from the results' list, with the results on top
    /// again.
    pub(super) fn results_action(&mut self, id: &CommandId) -> Vec<Effect> {
        if !self.results_on_top() {
            return Vec::new();
        }
        match id {
            id if *id == ids::VIEW => self.view_result(),
            id if *id == ids::EDIT => self.edit_result(),
            id if *id == ids::COPY => self.result_job(JobKind::Copy),
            id if *id == ids::MOVE => self.result_job(JobKind::Move),
            id if *id == ids::TRASH => self.result_job(JobKind::Trash),
            id if *id == ids::FILTER => self.refine_prompt(),
            // Save as collection, Collect and Remove missing items act on
            // the list on the screen, which is on top here.
            id if LIST_COMMANDS.contains(id) => self
                .lists_dispatch(&Command::new(id.clone()))
                .unwrap_or_default(),
            id => self.dispatch(&Command::new(id.clone())),
        }
    }

    /// The label of the results when the Labels list opened them.
    fn labeled_label(&self) -> Option<String> {
        self.searching.current.as_ref()?.label.clone()
    }

    /// The trash key in a label's results: the label comes off the
    /// focused item, which leaves the list.
    fn unlabel_result(&mut self) -> Vec<Effect> {
        let Some(label) = self.labeled_label() else {
            return Vec::new();
        };
        let Some(row) = self.top_row_id() else {
            return Vec::new();
        };
        let Some(hit) = self.hit_of(&row) else {
            return Vec::new();
        };
        let one = Some(hit.entry.display_name().into_owned());
        let side = self.active;
        let mut effects = self.change_labels(
            side,
            crate::labels::LabelChange::Remove(label),
            vec![hit.place],
            one,
        );
        if let Some(search) = self.searching.current.as_mut() {
            search.removed.insert(row);
        }
        let rows = match self.searching.current.as_ref() {
            Some(search) => self.result_rows(search),
            None => Vec::new(),
        };
        if let Some(list) = self.results_list_mut() {
            list.set_rows(rows);
        }
        // The Labels list under the results counts one item fewer.
        let labels = self.labels_list_model();
        self.replace_open_list(|w| matches!(w, super::organize::Wants::Labels), labels);
        effects.push(Effect::Redraw(Redraw::All));
        effects
    }

    /// Enter on a result: a file opens with its program, a folder opens in
    /// the panel; a label's item goes to its folder, focused, as the
    /// Labels list always did.
    pub(super) fn open_result(&mut self) -> Vec<Effect> {
        if self.labeled_label().is_some() {
            return self.show_result_in_folder();
        }
        let hit = match self.focused_hit() {
            Ok(hit) => hit,
            Err(effects) => return effects,
        };
        if hit.entry.is_container() {
            let side = self.active;
            let place = if hit.entry.kind == diroxide_core::EntryKind::Archive {
                hit.place.into_archive()
            } else {
                hit.place
            };
            let mut effects = self.close_top();
            effects.extend(self.open(side, place, None));
            return effects;
        }
        let text = format!("Opening {}", hit.entry.display_name());
        vec![
            self.say(Message::info(text)),
            Effect::OpenExternally(hit.place),
        ]
    }

    /// The view key on a result: the viewer at the matching line, the
    /// results put back when it closes; Next and Previous file step
    /// through the results.
    fn view_result(&mut self) -> Vec<Effect> {
        let hit = match self.focused_hit() {
            Ok(hit) => hit,
            Err(effects) => return effects,
        };
        if hit.entry.is_folder_like() && hit.entry.kind != diroxide_core::EntryKind::Archive {
            let text = format!("Not a file: {}, folder", hit.entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        let siblings: Vec<crate::viewer::Sibling> = self
            .searching
            .current
            .as_ref()
            .map(|s| {
                let mut seen: Vec<&Place> = Vec::new();
                s.hits
                    .iter()
                    .filter(|h| !h.entry.is_folder_like())
                    .filter(|h| {
                        let fresh = !seen.contains(&&h.place);
                        if fresh {
                            seen.push(&h.place);
                        }
                        fresh
                    })
                    .map(|h| crate::viewer::Sibling::new(h.place.clone(), h.entry.size))
                    .collect()
            })
            .unwrap_or_default();
        self.searching.park_on_open = true;
        self.searching.view_line = hit.line.as_ref().map(|l| l.line_number);
        self.view_place_at(hit.place, hit.entry.size, siblings)
    }

    /// The viewer on a file from a dialog (a compared file), at a line,
    /// the dialogs aside until it closes.
    pub(super) fn searching_view_at(
        &mut self,
        place: Place,
        size: Option<u64>,
        siblings: Vec<crate::viewer::Sibling>,
        line: u64,
    ) -> Vec<Effect> {
        self.searching.park_on_open = true;
        self.searching.view_line = Some(line);
        self.view_place_at(place, size, siblings)
    }

    /// The edit key on a result: the editor at the matching line.
    fn edit_result(&mut self) -> Vec<Effect> {
        let hit = match self.focused_hit() {
            Ok(hit) => hit,
            Err(effects) => return effects,
        };
        if hit.entry.is_folder_like() {
            let text = format!("Not a file: {}", hit.entry.display_name());
            return vec![self.say(Message::info(text))];
        }
        let kind = match hit.line.as_ref() {
            Some(line) => SpawnKind::EditorAt {
                file: hit.place.clone(),
                line: line.line_number,
            },
            None => SpawnKind::Editor(hit.place.clone()),
        };
        let side = self.active;
        let mut spawn = self.spawn_for(side, kind);
        if let Some(folder) = hit.place.parent() {
            spawn.folder = folder;
        }
        spawn.cursor = Some(hit.place.clone());
        spawn.selection = vec![hit.place];
        self.request(Request::Spawn(Box::new(spawn)));
        Vec::new()
    }

    /// Show in its folder: the results close (Search results opens them
    /// again) and the panel shows the result's folder with it focused.
    fn show_result_in_folder(&mut self) -> Vec<Effect> {
        let hit = match self.focused_hit() {
            Ok(hit) => hit,
            Err(effects) => return effects,
        };
        let side = self.active;
        let (Some(folder), Some(name)) = (hit.place.parent(), hit.place.name().map(str::to_owned))
        else {
            return Vec::new();
        };
        let mut effects = self.close_all();
        if &folder == self.panel(side).place() {
            self.panel_mut(side).list_mut().focus_id(&name);
            effects.extend(self.focus_effects(side));
        } else {
            effects.extend(self.open(side, folder, Some(name)));
        }
        effects
    }

    /// Remove from results: the row leaves the list; the file stays.
    fn remove_result_row(&mut self) -> Vec<Effect> {
        let Some(row) = self.top_row_id() else {
            return Vec::new();
        };
        let label = self
            .hit_of(&row)
            .map(|h| h.entry.display_name().into_owned())
            .unwrap_or_default();
        if let Some(search) = self.searching.current.as_mut() {
            search.removed.insert(row);
        }
        let rows = match self.searching.current.as_ref() {
            Some(search) => self.result_rows(search),
            None => Vec::new(),
        };
        if let Some(list) = self.results_list_mut() {
            list.set_rows(rows);
        }
        let line = format!("Removed {label} from the results. The file stays.");
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(line))]
    }

    /// The copy, move and trash keys on the results: a job on the real
    /// files, planned and confirmed as from a panel. Copy and move go to
    /// the other panel's folder.
    fn result_job(&mut self, kind: JobKind) -> Vec<Effect> {
        let sources = self.result_targets();
        if sources.is_empty() {
            return match self.focused_hit() {
                Ok(_) => Vec::new(),
                Err(effects) => effects,
            };
        }
        let side = self.active;
        let job = self.next_job_id();
        let mut request = JobRequest::new(job, kind, sources);
        let confirm = match kind {
            JobKind::Copy | JobKind::Move => {
                let destination = self.panel(side.other()).place().clone();
                if destination.kind() == diroxide_core::PlaceKind::Archive {
                    return vec![self.say(Message::error(super::archive::READ_ONLY))];
                }
                let confirm = if kind == JobKind::Move {
                    self.settings.confirm.move_items
                } else {
                    self.settings.confirm.copy
                };
                // Out of a flat view: keep the folders, or all in one
                // folder? (lane AS)
                if let Some(search) = self.searching.current.as_ref()
                    && search.kind == ListKind::Flat
                    && let Some(root) = search.root.clone()
                {
                    return self.flat_copy_question(super::flat::FlatCopy {
                        side,
                        kind,
                        root,
                        destination,
                        sources: request.sources,
                        confirm,
                    });
                }
                request = request.to(destination);
                confirm
            }
            JobKind::Trash => self.settings.confirm.trash,
            _ => true,
        };
        self.plan_job(side, request, None, Then::Report, confirm)
    }

    /// Space on a result: marked or not, and the focus moves on.
    fn toggle_result_mark(&mut self) -> Vec<Effect> {
        let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut() else {
            return Vec::new();
        };
        let Some(selected) = list.toggle_selected() else {
            return Vec::new();
        };
        let label = list.focused().map(|r| r.label.clone()).unwrap_or_default();
        let total = list.selected_count();
        list.move_focus(Move::Down);
        let text = if selected {
            format!("Selected {label}. {total} selected.")
        } else {
            format!("Not selected {label}. {total} selected.")
        };
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
    }

    fn mark_all_results(&mut self, id: &CommandId) -> Vec<Effect> {
        let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last_mut() else {
            return Vec::new();
        };
        if *id == ids::SELECT_ALL {
            list.select_all();
        } else if *id == ids::SELECT_NONE {
            list.select_none();
        } else {
            list.invert_selection();
        }
        let total = list.selected_count() as u64;
        let text = if total == 0 {
            "Nothing selected".to_owned()
        } else {
            format!("{} selected", count(total, "item", "items"))
        };
        vec![Effect::Redraw(Redraw::All), self.say(Message::info(text))]
    }

    /// The filter key on the results: refine them with the filter
    /// language, no new walk.
    fn refine_prompt(&mut self) -> Vec<Effect> {
        let text = self
            .searching
            .current
            .as_ref()
            .and_then(|s| s.refine.as_ref())
            .map(|f| f.text().to_owned())
            .unwrap_or_default();
        let field = EntryField::new("Refine results", CompletionSource::Filter)
            .with_text(text)
            .with_history(state::keys::FILTER);
        self.open_dialog(Dialog::edit(field), Purpose::Refine)
    }

    /// Enter in the refine field: the results narrow, "Refined: 2 of 4
    /// shown", or come back whole when it is empty.
    pub(super) fn refine_entered(&mut self, text: &str) -> Vec<Effect> {
        let filter = match self.read_filter(text) {
            Ok(filter) => filter,
            Err(error) => return vec![self.say(Message::error(error.line()))],
        };
        if filter.as_ref().is_some_and(Filter::reads_content) {
            let line = "Refining by contents is not built yet. Search again with Contains text.";
            return vec![self.say(Message::error(line))];
        }
        let mut effects = self.close_top();
        let Some(search) = self.searching.current.as_mut() else {
            return effects;
        };
        search.refine = filter;
        let total = search.hits.len();
        let refined = search.refine.is_some();
        let rows = match self.searching.current.as_ref() {
            Some(search) => self.result_rows(search),
            None => Vec::new(),
        };
        let shown = rows.len();
        if let Some(list) = self.results_list_mut() {
            list.set_rows(rows);
            list.move_focus(Move::Home);
        }
        let line = if refined {
            format!("Refined: {shown} of {total} shown")
        } else {
            format!("Refine off, {total} shown")
        };
        effects.push(Effect::Redraw(Redraw::All));
        effects.push(self.say(Message::info(line)));
        effects
    }

    /// The focus moved in the results: its file is checked after a short
    /// rest.
    pub(super) fn results_focus_moved(&mut self) {
        if self.results_on_top() {
            self.searching.stale_at = Some(self.now());
        }
    }

    /// The milliseconds until the focused result's check, when one waits.
    pub(super) fn stale_wants_tick(&self) -> Option<u64> {
        let at = self.searching.stale_at?;
        Some(STALE_PAUSE_MS.saturating_sub(millis_between(at, self.now())))
    }

    /// The rest is over: the focused result's file is checked.
    pub(super) fn stale_tick(&mut self) {
        let Some(at) = self.searching.stale_at else {
            return;
        };
        if millis_between(at, self.now()) < STALE_PAUSE_MS {
            return;
        }
        self.searching.stale_at = None;
        if !self.results_on_top() {
            return;
        }
        let Some(row) = self.top_row_id() else {
            return;
        };
        let Some(hit) = self.hit_of(&row) else {
            return;
        };
        let Some(search) = self.searching.current.as_ref() else {
            return;
        };
        if search.stale.contains_key(&row) {
            return;
        }
        let id = search.id;
        self.request(Request::CheckStale {
            search: id,
            row,
            hit: Box::new(hit),
        });
    }

    /// A result's file was checked: one that changed or is gone says so.
    pub(super) fn stale_checked(
        &mut self,
        id: u64,
        row: String,
        freshness: Freshness,
    ) -> Vec<Effect> {
        let name = self.hit_of(&row).map(|h| h.entry.name).unwrap_or_default();
        let Some(search) = self.searching.current.as_mut().filter(|s| s.id == id) else {
            return Vec::new();
        };
        search.stale.insert(row.clone(), freshness);
        let focused = self.results_on_top() && self.top_row_id().as_deref() == Some(row.as_str());
        match freshness.words(&name) {
            Some(words) if focused => vec![self.say(Message::info(words))],
            _ => Vec::new(),
        }
    }

    /// Takes every open dialog off the stack while the viewer shows a file
    /// one of them named (a search result, a compared file).
    pub(super) fn park_results(&mut self) {
        while let (Some((id, dialog)), Some(open)) = (self.dialogs.pop(), self.opens.pop()) {
            self.searching.parked.push(Parked { id, dialog, open });
        }
        self.searching.parked.reverse();
    }

    /// Whether dialogs wait aside while the viewer shows one of their
    /// files.
    pub(super) fn results_parked(&self) -> bool {
        !self.searching.parked.is_empty()
    }

    /// Whether a viewer opening now came from the results, which then go
    /// aside until it closes.
    pub(super) fn take_park_on_open(&mut self) -> bool {
        std::mem::take(&mut self.searching.park_on_open)
    }

    /// The line a result opening in the viewer goes to.
    pub(super) fn take_view_line(&mut self) -> Option<u64> {
        self.searching.view_line.take()
    }

    /// Puts the dialogs back after the viewer closed, each focus as it
    /// was: "Back in the results, notes.txt, ...".
    pub(super) fn unpark_results(&mut self) -> Vec<Effect> {
        if self.searching.parked.is_empty() {
            return Vec::new();
        }
        let mut effects = Vec::new();
        let opened = self.now();
        for Parked { id, dialog, open } in std::mem::take(&mut self.searching.parked) {
            self.dialogs.push((id, dialog));
            self.opens.push(Open { opened, ..open });
            effects.push(Effect::OpenDialog(id));
        }
        let line = self
            .dialogs
            .last()
            .map(|(_, d)| d.focus_line())
            .unwrap_or_default();
        let back = if self.results_on_top() {
            format!("Back in the results, {line}")
        } else {
            format!("Back in {line}")
        };
        effects.push(Effect::Redraw(Redraw::All));
        effects.push(self.say(Message::info(back)));
        effects
    }

    /// Escape on the results lister, after it closed.
    pub(super) fn results_closed(&mut self) -> Vec<Effect> {
        let key = self.key(&ids::SEARCH_RESULTS);
        let again = if key.is_empty() {
            "Search results opens them again".to_owned()
        } else {
            format!("{key} opens them again")
        };
        vec![self.say(Message::info(format!("Results closed. {again}.")))]
    }

    /// Say Status in the results lister: how the search stands, then the
    /// focused row.
    pub(super) fn results_status(&self) -> Option<String> {
        if !self.results_on_top() {
            return None;
        }
        let search = self.searching.current.as_ref()?;
        let state = if search.paused {
            format!("Paused at {} found", grouped(search.found))
        } else if search.running {
            format!("Searching, {} found so far", grouped(search.found))
        } else {
            match &search.summary {
                Some(summary) => summary.status_line().trim_end_matches('.').to_owned(),
                None => format!("{} found", grouped(search.found)),
            }
        };
        let focus = match self.dialogs.last() {
            Some((_, dialog)) => dialog.focus_line(),
            None => String::new(),
        };
        Some(format!("{state}. {focus}"))
    }
}

/// What a list in the results lister is (alpha 5, the pre-wiring step,
/// lane P5): the results of a search, and the lists that are not folders
/// (question 130). It decides the words of the list's rows and what the
/// trash key does in it; the title and the start line are the opener's.
/// One list at a time: opening one replaces the last.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
)]
pub(crate) enum ListKind {
    /// A search's results: Find files, a saved search, the largest items.
    Search,
    /// A label's items, from the Labels list: the trash key takes the
    /// label off.
    Labels,
    /// The newest files of a day.
    Newest,
    /// The flat view of a folder (lane LA).
    Flat,
    /// A collection, by its name: the trash key takes the item out of the
    /// collection and touches no file (lane LA).
    Collection(String),
    /// The recent files (lane LA).
    Recent,
    /// A folder compare's files (lane SA).
    Compared,
    /// One duplicate group's members (lane DA).
    DuplicateGroup,
}

#[allow(
    dead_code,
    reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
)]
impl ListKind {
    /// What the list is, in words, for a sentence: "search results", "the
    /// collection Trip photos".
    pub(crate) fn words(&self) -> String {
        match self {
            ListKind::Search => "search results".to_owned(),
            ListKind::Labels => "a label's items".to_owned(),
            ListKind::Newest => "newest files".to_owned(),
            ListKind::Flat => "the flat view".to_owned(),
            ListKind::Collection(name) => format!("the collection {name}"),
            ListKind::Recent => "recent files".to_owned(),
            ListKind::Compared => "the compared files".to_owned(),
            ListKind::DuplicateGroup => "a duplicate group".to_owned(),
        }
    }

    /// Whether the trash key takes the focused item out of the list's
    /// store instead of sending the file to the trash.
    pub(crate) fn trash_removes_item(&self) -> bool {
        matches!(self, ListKind::Labels | ListKind::Collection(_))
    }
}

/// A list to open in the results lister ([`App::open_list`]).
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
)]
pub(crate) struct ListOpen {
    /// What it is.
    pub kind: ListKind,
    /// The panel that asked; the list belongs to its shown tab.
    pub side: Side,
    /// The list's name, said when it opens: "Trip photos, collection".
    pub title: String,
    /// The folder the rows' "in b/deep" words start from, when there is
    /// one (the flat view's root).
    pub root: Option<Place>,
    /// The rows' files, in the list's order.
    pub hits: Vec<SearchHit>,
    /// Places in the list that are gone: their rows say "missing".
    pub gone: Vec<Place>,
    /// Words after a row's name, by place: "oldest", "Different".
    pub notes: Vec<(Place, String)>,
    /// The line said as it opens, its count first: "Trip photos,
    /// collection, 3 items".
    pub start: String,
    /// Whether more rows come later ([`App::search_found`] with the
    /// returned number); else the list is complete as given.
    pub streaming: bool,
}

/// Said once a session when another list replaces a search's results: 28
/// cells.
#[allow(
    dead_code,
    reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
)]
pub(crate) const REPLACES_RESULTS: &str = "Replaces the search results.";

impl App {
    /// Opens a list in the results lister, in place of the last search's
    /// results or list (one list at a time, question 130): its rows, its
    /// title, its start line; "Replaces the search results." is said once
    /// a session when a search's results were there. Returns the effects
    /// and the list's number, which [`App::search_found`] and
    /// [`App::finish_results`] take for a list that streams. Every key of
    /// the lister works on it: Enter, the view and edit keys, Copy, Move,
    /// the trash key (a collection's item leaves the collection; a label's
    /// item loses the label), Show in its folder, Remove from results and
    /// the refining filter.
    #[allow(
        dead_code,
        reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
    )]
    pub(crate) fn open_list(&mut self, open: ListOpen) -> (Vec<Effect>, u64) {
        let replaces = !self.searching.replaced_said
            && open.kind != ListKind::Search
            && self
                .searching
                .current
                .as_ref()
                .is_some_and(|s| s.kind == ListKind::Search && !s.hits.is_empty());
        self.searching.next += 1;
        let id = self.searching.next;
        let mut stale = HashMap::new();
        let mut hits = open.hits;
        for place in &open.gone {
            let name = place.name().unwrap_or_default().to_owned();
            let entry = Entry::new(name, diroxide_core::EntryKind::File);
            let hit = SearchHit::new(place.clone(), entry);
            stale.insert(hit.id(), Freshness::Gone);
            hits.push(hit);
        }
        // A row's id is its place for a hit without a line.
        let notes = open
            .notes
            .into_iter()
            .map(|(place, note)| (place.to_string(), note))
            .collect();
        let found = hits.len() as u64;
        self.searching.current = Some(Box::new(Search {
            id,
            root: open.root,
            title: open.title,
            hits,
            running: open.streaming,
            paused: false,
            found,
            summary: None,
            said_at: Some(self.now()),
            refine: None,
            stale,
            removed: HashSet::new(),
            label: None,
            day: None,
            kind: open.kind,
            owner: Some((open.side, self.panel(open.side).tab_id())),
            notes,
        }));
        self.searching.parked.clear();
        if !open.streaming {
            self.searching.stale_at = Some(self.now());
        }
        let list = self.results_model();
        let mut effects = self.open_dialog_saying(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SEARCH_RESULTS),
            },
            Purpose::Results,
            Message::info(open.start),
        );
        if replaces {
            self.searching.replaced_said = true;
            effects.push(self.say(Message::info(REPLACES_RESULTS)));
        }
        (effects, id)
    }

    /// What the list on the screen is, when one is.
    #[allow(
        dead_code,
        reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
    )]
    pub(crate) fn list_kind(&self) -> Option<&ListKind> {
        self.searching.current.as_ref().map(|s| &s.kind)
    }

    /// The panel and tab the list belongs to.
    #[allow(
        dead_code,
        reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
    )]
    pub(crate) fn list_owner(&self) -> Option<(Side, u64)> {
        self.searching.current.as_ref()?.owner
    }

    /// The places of the list on the screen (alpha 5, lane AS): its marked
    /// rows, or every row it shows, each place once, in the list's order;
    /// empty when the results lister is not on top. Save as collection
    /// keeps these.
    pub(crate) fn list_places(&self) -> Vec<Place> {
        if !self.results_on_top() {
            return Vec::new();
        }
        let Some((_, Dialog::Choose { list, .. })) = self.dialogs.last() else {
            return Vec::new();
        };
        let marked: Vec<String> = list.selected_ids().into_iter().map(str::to_owned).collect();
        let ids: Vec<String> = if marked.is_empty() {
            list.rows().iter().map(|r| r.id.clone()).collect()
        } else {
            marked
        };
        let mut places: Vec<Place> = Vec::new();
        for id in &ids {
            if let Some(hit) = self.hit_of(id)
                && !places.contains(&hit.place)
            {
                places.push(hit.place);
            }
        }
        places
    }

    /// A command chosen in the palette or a menu over the results lister
    /// that acts on the list (Save as collection, Collect, Remove missing
    /// items): the dialogs above the lister close, the lister stays, and
    /// the command runs on it through [`App::results_action`]. `None` for
    /// any other command, or when no lister is open under the dialog.
    pub(super) fn list_command_over(&mut self, id: &CommandId) -> Option<Vec<Effect>> {
        if !LIST_COMMANDS.contains(id) {
            return None;
        }
        let index = self
            .opens
            .iter()
            .rposition(|o| matches!(o.purpose, Purpose::Results))?;
        let mut effects = Vec::new();
        while self.opens.len() > index + 1 && !self.dialogs.is_empty() {
            effects.extend(self.close_top());
        }
        effects.extend(self.results_action(id));
        Some(effects)
    }

    /// The collection whose items the list shows, when it is one.
    fn list_collection(&self) -> Option<String> {
        match &self.searching.current.as_ref()?.kind {
            ListKind::Collection(name) => Some(name.clone()),
            _ => None,
        }
    }

    /// The trash key in a collection's list: the focused item leaves the
    /// collection through lane LA's [`App::collection_remove_items`], and
    /// its row leaves the list once that says so. No file is touched.
    fn uncollect_result(&mut self) -> Vec<Effect> {
        let Some(name) = self.list_collection() else {
            return Vec::new();
        };
        let Some(row) = self.top_row_id() else {
            return Vec::new();
        };
        let Some(hit) = self.hit_of(&row) else {
            return Vec::new();
        };
        self.collection_remove_items(&name, vec![hit.place])
    }

    /// Takes rows out of the list on the screen by their places, as Remove
    /// from results does, for a list whose store dropped them.
    #[allow(
        dead_code,
        reason = "reserved for an alpha 5 app lane by the pre-wiring step (P5)"
    )]
    pub(crate) fn list_remove_places(&mut self, places: &[Place]) -> Vec<Effect> {
        let Some(search) = self.searching.current.as_mut() else {
            return Vec::new();
        };
        for hit in &search.hits {
            if places.contains(&hit.place) {
                search.removed.insert(hit.id());
            }
        }
        let rows = match self.searching.current.as_ref() {
            Some(search) => self.result_rows(search),
            None => Vec::new(),
        };
        if let Some(list) = self.results_list_mut() {
            list.set_rows(rows);
        }
        vec![Effect::Redraw(Redraw::All)]
    }
}
