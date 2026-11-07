//! Custom columns (alpha 4, lane MA, R30 commit 52; R28 3.2): what a
//! panel row says after its built-in cells for each field's key in the
//! `columns` setting, where each cell comes from, and the words of the
//! ask-first sort.
//!
//! **Where a key's cells come from** ([`Source`]):
//!
//! - **A metadata field** (`audio.artist`, `doc.pages`, `archive.entries`):
//!   read by the metadata pipeline's readers in their cost class, Total
//!   Commander's three loading modes. A *fast* column is read for the
//!   whole folder, the rows near the focus first; a *background* column
//!   for the rows near the focus only, the visible page and two pages
//!   either side, following the focus; an *on demand* column says "not
//!   computed" until Fill columns or a sort by it asks. Four readers on a
//!   disk of this computer, one on a share, as the type check has; never
//!   on a remote place, in an archive, a plugin's place or a folder of
//!   `check.skip` (the type check's rule, [`crate::Panel::reads_contents`]).
//! - **The owner's own** (`custom.note`, `custom.project`): the note and
//!   named fields of `labels.toml`, at once.
//! - **A regex column** (`regex.episode`): a pattern of `config.toml`'s
//!   `regex_columns` over the name, its first group or whole match, at
//!   once, with no reading and no plugin.
//! - **A plugin's column** (`diro-git.git`): the plugin's `column/format`,
//!   a page of rows at a time, "pending" until it answers.
//!
//! **A row says each cell after its label**, as the built-in cells say
//! "modified Tuesday" and a plugin's list says its columns: "artist Ada
//! Example", "pages 12", so the words carry their meaning wherever they
//! are read; a cell the file has no value for is left out, as the
//! built-in cells are. A cell still to be read says so: "artist
//! reading", "git pending", "lines not computed".
//!
//! **Sorting by a column whose cells are not all read asks first**: "Read
//! 12,000 files to sort by Artist?" (36 cells) with "About 40 seconds."
//! after it, the answer starting on No; until the reading finishes, the
//! order does not change.
//!
//! Spike M6 (100,000 entries, three background columns): the cells say
//! "reading" from the row's own words, so the first paint is not delayed
//! (about one microsecond an entry more); the cells read live in each
//! entry's [`Entry::extra`] (about 210 bytes a filled cell), and a batch of
//! cells is put into the rows only when it reaches the page around the
//! focus, since replacing the rows of 100,000 entries costs about a fifth
//! of a second.

use std::collections::BTreeMap;
use std::sync::Arc;

use diroxide_config::ColumnSpec;
use diroxide_config::columns::REGEX_PREFIX;
use diroxide_core::format::grouped;
use diroxide_core::{Entry, EntryKind, Timestamp};
use diroxide_meta::Cost;
use diroxide_plugins::listers::PluginColumn;

/// A metadata cell still to be read (meta's word).
pub const READING: &str = "reading";

/// An on-demand cell nobody asked for yet (meta's word).
pub const NOT_COMPUTED: &str = "not computed";

/// A plugin's cell it has not answered yet (the host's word).
pub const PENDING: &str = "pending";

/// How many pages either side of the visible one a background column
/// reads.
pub const WINDOW_PAGES: usize = 2;

/// Below this many files to read, a sort reads them without asking.
pub const ASK_ABOVE: usize = 200;

/// The files a second a sort's estimate takes on a disk of this computer,
/// a cold disk's pace (spike M6 read 11,000 small files a second warm
/// with four readers).
pub const LOCAL_PACE: u64 = 300;

/// The files a second a sort's estimate takes on a share.
pub const SHARE_PACE: u64 = 30;

/// Where a column's cells come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// A metadata reader's field, read in its cost class.
    Meta(Cost),
    /// The owner's note or a named field, from `labels.toml`.
    Owner,
    /// A pattern over the name; `None` when the pattern cannot be read.
    Regex(Option<Arc<regex::Regex>>),
    /// A plugin's declared column.
    Plugin(PluginColumn),
    /// A key nothing gives: its cells stay empty.
    Unknown,
}

/// One column of a field's key, resolved.
#[derive(Clone, Debug)]
pub struct FieldColumn {
    /// Its key: `audio.artist`.
    pub key: String,
    /// Its label, said before each cell: "Artist".
    pub label: String,
    /// Where its cells come from.
    pub source: Source,
}

impl FieldColumn {
    /// The cost class when its cells are read by the metadata readers.
    pub fn cost(&self) -> Option<Cost> {
        match self.source {
            Source::Meta(cost) => Some(cost),
            _ => None,
        }
    }

    /// The words a row says for this column, label first, or `None` when
    /// the entry has nothing for it: "artist Ada Example", "artist
    /// reading". `reads_here` is whether the panel's place has its files'
    /// contents read; `plugins_here` whether its plugins' columns are
    /// asked.
    pub fn cell_words(
        &self,
        entry: &Entry,
        reads_here: bool,
        plugins_here: bool,
    ) -> Option<String> {
        match entry.extra.get(&self.key) {
            Some(value) if value.trim().is_empty() => None,
            Some(value) => Some(format!("{} {value}", self.in_row())),
            None => self
                .waiting_word(entry, reads_here, plugins_here)
                .map(|word| format!("{} {word}", self.in_row())),
        }
    }

    /// The label as a row says it, before the value: "artist", "git",
    /// "last saved by".
    pub fn in_row(&self) -> String {
        crate::labels::in_sentence(&self.label)
    }

    /// What a cell not read yet says: "reading", "not computed",
    /// "pending", or nothing.
    pub fn waiting_word(
        &self,
        entry: &Entry,
        reads_here: bool,
        plugins_here: bool,
    ) -> Option<&'static str> {
        match &self.source {
            Source::Meta(cost) if reads_here && file_like(entry) => Some(match cost {
                Cost::OnDemand => NOT_COMPUTED,
                Cost::Fast | Cost::Background => READING,
            }),
            Source::Plugin(_) if plugins_here => Some(PENDING),
            _ => None,
        }
    }

    /// Whether the entry still waits for this column's cell.
    pub fn waits(&self, entry: &Entry) -> bool {
        file_like(entry) && !entry.extra.contains_key(&self.key)
    }
}

/// Whether metadata readers read the entry: a file or an archive.
pub fn file_like(entry: &Entry) -> bool {
    matches!(entry.kind, EntryKind::File | EntryKind::Archive)
}

/// A key's label: its last part, `_` and `-` as spaces, the first letter
/// capital: `audio.artist` reads "Artist", `doc.last_saved_by` "Last saved
/// by".
pub fn key_label(key: &str) -> String {
    let last = key.rsplit('.').next().unwrap_or(key);
    crate::labels::field_label(last)
}

/// The cell of a regex column for a name: the first capture group, or
/// the whole match when the pattern has none; `None` when it does not
/// match.
pub fn regex_cell(pattern: &regex::Regex, name: &str) -> Option<String> {
    let found = pattern.captures(name)?;
    let text = found.get(1).or_else(|| found.get(0))?.as_str();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Resolves the `columns` setting's keys: each to the reader that names
/// it (`keys` is the pipeline's [`diroxide_meta::Pipeline::keys`]), the
/// owner's own, a regex column of `patterns`, or a plugin's column; a key
/// nothing gives resolves to [`Source::Unknown`], and its cells stay
/// empty.
pub fn resolve(
    specs: &[ColumnSpec],
    keys: &[(&'static str, Cost, &'static str)],
    patterns: &BTreeMap<String, String>,
    plugins: &[PluginColumn],
) -> Vec<FieldColumn> {
    specs
        .iter()
        .filter_map(ColumnSpec::key)
        .map(|key| {
            let (label, source) =
                if let Some((_, cost, _)) = keys.iter().find(|(k, _, _)| *k == key) {
                    (key_label(key), Source::Meta(*cost))
                } else if key == crate::labels::NOTE_KEY {
                    ("Note".to_owned(), Source::Owner)
                } else if let Some(name) = key.strip_prefix(crate::labels::CUSTOM_PREFIX) {
                    (crate::labels::field_label(name), Source::Owner)
                } else if let Some(name) = key.strip_prefix(REGEX_PREFIX) {
                    let pattern = patterns
                        .get(name)
                        .and_then(|p| regex::Regex::new(p).ok())
                        .map(Arc::new);
                    (crate::labels::field_label(name), Source::Regex(pattern))
                } else if let Some(column) = plugins.iter().find(|c| c.key() == key) {
                    (column.header.clone(), Source::Plugin(column.clone()))
                } else {
                    (key_label(key), Source::Unknown)
                };
            FieldColumn {
                key: key.to_owned(),
                label,
                source,
            }
        })
        .collect()
}

/// The regex columns whose patterns cannot be read, in words, for the
/// startup line: "Regex column episode: its pattern cannot be read".
pub fn pattern_problems(patterns: &BTreeMap<String, String>) -> Vec<String> {
    patterns
        .iter()
        .filter(|(_, pattern)| regex::Regex::new(pattern).is_err())
        .map(|(name, _)| format!("Regex column {name}: its pattern cannot be read"))
        .collect()
}

/// One file whose cells the readers read: its place in the panel's
/// listing and what its type check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnItem {
    /// Its index in the listing ([`crate::Panel::all_entries`]).
    pub index: u32,
    /// Its name, also the check that the answer is still for it.
    pub name: String,
    /// Its kind.
    pub kind: EntryKind,
    /// Its size.
    pub size: Option<u64>,
    /// When it changed.
    pub modified: Option<Timestamp>,
    /// Its type's id, when known: `mp3`.
    pub type_id: Option<String>,
    /// Its type's group, when known: `audio`.
    pub group: Option<String>,
    /// The keys still to read for it.
    pub keys: Vec<String>,
}

impl ColumnItem {
    /// The entry a reader looks at.
    pub fn entry(&self) -> Entry {
        let mut entry = Entry::new(self.name.clone(), self.kind);
        entry.size = self.size;
        entry.modified = self.modified;
        entry
    }
}

/// What the readers found for one file: each key's words, empty when the
/// file has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnCell {
    /// Its index in the listing it was asked for.
    pub index: u32,
    /// Its name.
    pub name: String,
    /// Each key and its words.
    pub values: Vec<(String, String)>,
}

/// The question before a sort reads files: "Read 12,000 files to sort by
/// Artist?" (36 cells), its answer starting on No.
pub fn sort_question(files: usize, label: &str) -> String {
    match files {
        1 => format!("Read 1 file to sort by {label}?"),
        n => format!("Read {} files to sort by {label}?", grouped(n as u64)),
    }
}

/// The estimate said after the question: "About 40 seconds.", "About 3
/// minutes.", "Under a second."
pub fn sort_estimate(files: usize, share: bool) -> String {
    let pace = if share { SHARE_PACE } else { LOCAL_PACE };
    let seconds = (files as u64).div_ceil(pace);
    match seconds {
        0 | 1 => "Under a second.".to_owned(),
        2..=90 => format!("About {seconds} seconds."),
        _ => format!("About {} minutes.", seconds.div_ceil(60)),
    }
}

/// The question of Fill columns: "Read 12,000 files for the columns?"
pub fn fill_question(files: usize) -> String {
    match files {
        1 => "Read 1 file for the columns?".to_owned(),
        n => format!("Read {} files for the columns?", grouped(n as u64)),
    }
}

/// The rows a background column reads, nearest the focus first: the
/// visible page and [`WINDOW_PAGES`] pages either side, as indexes into
/// the shown rows.
pub fn window_order(focus: usize, shown: usize, page: usize) -> Vec<usize> {
    if shown == 0 {
        return Vec::new();
    }
    let page = page.max(1);
    let reach = page / 2 + page * WINDOW_PAGES;
    let focus = focus.min(shown - 1);
    let mut order = Vec::with_capacity((reach * 2 + 1).min(shown));
    order.push(focus);
    for step in 1..=reach {
        if let Some(below) = focus.checked_add(step).filter(|i| *i < shown) {
            order.push(below);
        }
        if let Some(above) = focus.checked_sub(step) {
            order.push(above);
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn keys() -> Vec<(&'static str, Cost, &'static str)> {
        vec![
            ("audio.artist", Cost::Fast, "audio"),
            ("audio.length", Cost::Background, "audio-properties"),
            ("text.lines", Cost::OnDemand, "text"),
        ]
    }

    #[test]
    fn keys_resolve_to_where_their_cells_come_from() {
        let specs: Vec<ColumnSpec> = [
            "type",
            "audio.artist",
            "text.lines",
            "custom.note",
            "custom.project_code",
            "regex.episode",
            "regex.missing",
            "diro-git.git",
            "nothing.here",
        ]
        .iter()
        .filter_map(|s| ColumnSpec::parse(s))
        .collect();
        let mut patterns = BTreeMap::new();
        patterns.insert("episode".to_owned(), r"S\d+E(\d+)".to_owned());
        let git = PluginColumn {
            plugin: "diro-git".into(),
            column: "git".into(),
            header: "Git".into(),
            batch: 100,
        };
        let columns = resolve(&specs, &keys(), &patterns, &[git]);
        let labels: Vec<&str> = columns.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Artist",
                "Lines",
                "Note",
                "Project code",
                "Episode",
                "Missing",
                "Git",
                "Here"
            ]
        );
        assert!(matches!(columns[0].source, Source::Meta(Cost::Fast)));
        assert!(matches!(columns[2].source, Source::Owner));
        assert!(matches!(columns[4].source, Source::Regex(Some(_))));
        assert!(matches!(columns[5].source, Source::Regex(None)));
        assert!(matches!(columns[6].source, Source::Plugin(_)));
        assert!(matches!(columns[7].source, Source::Unknown));
    }

    #[test]
    fn a_cell_says_its_label_then_its_value_or_why_it_waits() {
        // As the built-in cells say "modified Tuesday".
        let specs = vec![
            ColumnSpec::Key("audio.artist".into()),
            ColumnSpec::Key("text.lines".into()),
            ColumnSpec::Key("diro-git.git".into()),
        ];
        let git = PluginColumn {
            plugin: "diro-git".into(),
            column: "git".into(),
            header: "Git".into(),
            batch: 100,
        };
        let columns = resolve(&specs, &keys(), &BTreeMap::new(), &[git]);
        let mut song = Entry::new("song.mp3", EntryKind::File);
        assert_eq!(
            columns[0].cell_words(&song, true, true).as_deref(),
            Some("artist reading")
        );
        assert_eq!(
            columns[1].cell_words(&song, true, true).as_deref(),
            Some("lines not computed")
        );
        assert_eq!(
            columns[2].cell_words(&song, true, true).as_deref(),
            Some("git pending")
        );
        // Nothing is read on a remote place: nothing waits there.
        assert_eq!(columns[0].cell_words(&song, false, false), None);
        song.extra
            .insert("audio.artist".into(), "Ada Example".into());
        song.extra.insert("text.lines".into(), String::new());
        assert_eq!(
            columns[0].cell_words(&song, true, true).as_deref(),
            Some("artist Ada Example")
        );
        assert_eq!(columns[1].cell_words(&song, true, true), None);
        let folder = Entry::new("Music", EntryKind::Folder);
        assert_eq!(columns[0].cell_words(&folder, true, true), None);
    }

    #[test]
    fn a_regex_column_takes_the_first_group_or_the_match() {
        let group = regex::Regex::new(r"S\d+E(\d+)").unwrap();
        assert_eq!(regex_cell(&group, "show.S02E07.mkv").as_deref(), Some("07"));
        assert_eq!(regex_cell(&group, "notes.txt"), None);
        let whole = regex::Regex::new(r"\d{4}").unwrap();
        assert_eq!(
            regex_cell(&whole, "report-2026.pdf").as_deref(),
            Some("2026")
        );
        let mut patterns = BTreeMap::new();
        patterns.insert("bad".to_owned(), "(".to_owned());
        patterns.insert("good".to_owned(), "x".to_owned());
        assert_eq!(
            pattern_problems(&patterns),
            ["Regex column bad: its pattern cannot be read"]
        );
    }

    #[test]
    fn the_ask_first_words_fit_a_braille_line() {
        let question = sort_question(12_000, "Artist");
        assert_eq!(question, "Read 12,000 files to sort by Artist?");
        assert!(within_cells(&question, &question));
        assert_eq!(sort_estimate(12_000, false), "About 40 seconds.");
        assert_eq!(sort_estimate(12_000, true), "About 7 minutes.");
        assert_eq!(sort_estimate(10, false), "Under a second.");
        assert_eq!(sort_question(1, "Pages"), "Read 1 file to sort by Pages?");
        let fill = fill_question(12_000);
        assert_eq!(fill, "Read 12,000 files for the columns?");
        assert!(within_cells(&fill, &fill));
    }

    #[test]
    fn the_window_is_the_page_and_two_either_side_nearest_first() {
        let order = window_order(50, 1000, 10);
        assert_eq!(&order[..5], [50, 51, 49, 52, 48]);
        assert_eq!(order.len(), 51);
        assert!(order.iter().all(|i| (25..=75).contains(i)));
        let start = window_order(0, 1000, 10);
        assert_eq!(&start[..3], [0, 1, 2]);
        assert_eq!(start.len(), 26);
        assert!(window_order(0, 0, 10).is_empty());
        assert_eq!(window_order(5, 3, 10), [2, 1, 0]);
    }
}
