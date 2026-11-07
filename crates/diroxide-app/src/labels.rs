//! Labels: the owner's own tags on files and folders (plan section 5.15
//! and `research/alpha3-plan.md`, "Labels, bookmarks, history, per-folder
//! views"). A label is a name first and a color second: the name is what
//! every row, list and sentence says ("report.pdf, label: urgent, 2.1
//! MB"); the color is an extra for sighted eyes, chosen only from blue,
//! orange, purple, yellow, gray and brown, so no two labels are ever told
//! apart by red against green, and never by color alone.
//!
//! The store keeps the label list, in the owner's order, and which places
//! carry which labels, keyed by the place as written (folded to lowercase
//! on the case-insensitive local file systems of Windows and macOS). It
//! lives in `labels.toml` in the state folder, written atomically on every
//! change. Nothing is ever written into the owner's folders.
//!
//! **Notes and named fields** (alpha 4, R30 commit 51; R28 3.2): an item
//! also keeps the owner's own note ("Note: call about the invoice", one
//! line, at most [`NOTE_LIMIT`] characters) and named fields
//! (`project = "Apollo"`), shown in the details beside the labels and as
//! the columns `custom.note` and `custom.NAME`. Format 2 of the file adds
//! them to `[[item]]`, both defaulted, so a format 1 file loads as it is.
//!
//! **They follow a move** (R28 question 3.4): when a move or a rename
//! finishes, the app hands its pairs to [`LabelStore::follow_moves`], and
//! the labels, the note and the fields of a moved file, or of anything
//! inside a moved folder, are kept under the new place in the same step.
//! A copy does not take them. A file moved by another program still loses
//! them until they are set again; the Labels list counts such missing
//! items when it opens.
//!
//! The starter set, plan question 71's: Urgent, Waiting, Review, Done,
//! Keep and "Skip in searches" (which search leaves out). The first seven
//! labels of the list are the favorites, toggled by Control with a digit
//! as the Finder does.
//!
//! A label's color reaches the screen through the theme's six label
//! roles ([`LabelColor::role`], P3b): a frontend colors the label's word
//! in a row with its role, and the row's words stay as they are, the name
//! first and the color's word after it.
//!
//! W3l owns this module. The app's commands are in `app::labeling`.

use std::collections::{BTreeMap, HashMap};

use diroxide_config::{ConfigProblem, Loaded, Paths};
use diroxide_core::format::count;
use diroxide_core::{Entry, Place, PlaceKind};
use diroxide_theme::PanelRole;
use serde::{Deserialize, Serialize};

/// The labels file in the state folder.
pub const FILE: &str = "labels.toml";

/// The label search leaves out (Worker's `exclude_search`).
pub const SKIP_IN_SEARCHES: &str = "Skip in searches";

/// How many labels have a favorite key: Control with 1 to 7.
pub const FAVORITES: usize = 7;

/// The longest label name, so a row keeps its meaning in 40 cells.
pub const NAME_LIMIT: usize = 30;

/// The version of `labels.toml` this build writes: 2 adds each item's
/// note and fields; a format 1 file loads as it is.
pub const FORMAT: u32 = 2;

/// The longest note, in characters: one line, a few hundred at most.
pub const NOTE_LIMIT: usize = 300;

/// The longest field name, so a row keeps its meaning in 40 cells.
pub const FIELD_NAME_LIMIT: usize = 24;

/// The column key of the note: `custom.note`. A named field's key is
/// `custom.NAME` ([`field_key`]).
pub const NOTE_KEY: &str = "custom.note";

/// The prefix of the owner's own columns.
pub const CUSTOM_PREFIX: &str = "custom.";

/// The column key of a named field: `custom.project`.
pub fn field_key(name: &str) -> String {
    format!("{CUSTOM_PREFIX}{name}")
}

/// Checks a note: trimmed, one line, at most [`NOTE_LIMIT`] characters.
/// An empty note is `None`, which removes it. The error is a sentence.
pub fn check_note(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > NOTE_LIMIT {
        return Err(format!("A note is at most {NOTE_LIMIT} characters."));
    }
    if text.chars().any(char::is_control) {
        return Err("A note is one line, with no control characters.".to_owned());
    }
    Ok(Some(text.to_owned()))
}

/// Checks a field's value as a note is checked: one line, at most
/// [`NOTE_LIMIT`] characters; an empty value is `None`, which removes the
/// field.
pub fn check_field_value(text: &str) -> Result<Option<String>, String> {
    check_note(text).map_err(|why| why.replacen("A note", "A field's value", 1))
}

/// Reads Set field's text, `name: value` or `name = value`: the checked
/// name, and the value or `None` to remove the field.
pub fn parse_field(text: &str) -> Result<(String, Option<String>), String> {
    let Some(at) = text.find([':', '=']) else {
        return Err("Type the field as name: value.".to_owned());
    };
    let name = check_field_name(&text[..at])?;
    let value = check_field_value(&text[at + 1..])?;
    Ok((name, value))
}

/// Checks a field's name: small letters, digits, `-` and `_`, at most
/// [`FIELD_NAME_LIMIT`] characters, never `note` (the note's own column),
/// so its column key reads `custom.project`. The error is a sentence.
pub fn check_field_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A field needs a name, as name: value.".to_owned());
    }
    if name.chars().count() > FIELD_NAME_LIMIT {
        return Err(format!(
            "A field name is at most {FIELD_NAME_LIMIT} characters."
        ));
    }
    let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
    if !name.chars().all(plain) {
        return Err("Field names use small letters, digits, - and _ only.".to_owned());
    }
    if name == "note" {
        return Err("A field cannot be called note: Set note keeps the note.".to_owned());
    }
    Ok(name.to_owned())
}

/// A field's name as a label: the first letter capital, `_` and `-` as
/// spaces: `project_code` reads "Project code".
pub fn field_label(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A label's color: a set that never pairs red with green. The color is
/// always said as a word after the name, never shown alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelColor {
    /// Blue.
    Blue,
    /// Orange.
    Orange,
    /// Purple.
    Purple,
    /// Yellow.
    Yellow,
    /// Gray.
    Gray,
    /// Brown.
    Brown,
}

impl LabelColor {
    /// Every color offered, in the order new labels take them.
    pub const ALL: [LabelColor; 6] = [
        LabelColor::Blue,
        LabelColor::Orange,
        LabelColor::Purple,
        LabelColor::Yellow,
        LabelColor::Gray,
        LabelColor::Brown,
    ];

    /// The color as a word: "orange".
    pub const fn word(self) -> &'static str {
        match self {
            LabelColor::Blue => "blue",
            LabelColor::Orange => "orange",
            LabelColor::Purple => "purple",
            LabelColor::Yellow => "yellow",
            LabelColor::Gray => "gray",
            LabelColor::Brown => "brown",
        }
    }

    /// The theme role that colors a label's word in this color: a text
    /// color checked for contrast on the page in every theme (P3b, W3l's
    /// request 1). The word is said first; the color is the extra.
    pub const fn role(self) -> PanelRole {
        match self {
            LabelColor::Blue => PanelRole::LabelBlue,
            LabelColor::Orange => PanelRole::LabelOrange,
            LabelColor::Purple => PanelRole::LabelPurple,
            LabelColor::Yellow => PanelRole::LabelYellow,
            LabelColor::Gray => PanelRole::LabelGray,
            LabelColor::Brown => PanelRole::LabelBrown,
        }
    }

    /// A color from its word, ignoring case; "grey" is gray. Red, green
    /// and every other word are not offered.
    pub fn from_word(word: &str) -> Option<Self> {
        let word = word.trim().to_lowercase();
        if word == "grey" {
            return Some(LabelColor::Gray);
        }
        Self::ALL.into_iter().find(|c| c.word() == word)
    }
}

/// One label: its name, and its color second.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Label {
    /// The name, as the owner wrote it: "Urgent".
    pub name: String,
    /// The color, a word after the name.
    pub color: LabelColor,
}

impl Label {
    /// A label.
    pub fn new(name: impl Into<String>, color: LabelColor) -> Self {
        Self {
            name: name.into(),
            color,
        }
    }

    /// The name as it reads inside a sentence: a capital first letter
    /// followed by a small one is made small ("Urgent" reads "urgent");
    /// anything else stays as written ("TODO", "Q3", "iPhone").
    pub fn in_sentence(&self) -> String {
        in_sentence(&self.name)
    }
}

/// A label's name inside a sentence (see [`Label::in_sentence`]).
pub fn in_sentence(name: &str) -> String {
    let mut chars = name.chars();
    let (Some(first), second) = (chars.next(), chars.next()) else {
        return name.to_owned();
    };
    if !first.is_uppercase() || !second.is_some_and(char::is_lowercase) {
        return name.to_owned();
    }
    first.to_lowercase().chain(name.chars().skip(1)).collect()
}

/// Checks a label name: trimmed, not empty, at most [`NAME_LIMIT`]
/// characters, no control characters. The error is a sentence.
pub fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A label needs a name.".to_owned());
    }
    if name.chars().count() > NAME_LIMIT {
        return Err(format!("A label name is at most {NAME_LIMIT} characters."));
    }
    if name.chars().any(char::is_control) {
        return Err("A label name cannot hold control characters.".to_owned());
    }
    Ok(name.to_owned())
}

/// The starter labels (plan question 71), each with its color.
pub fn starter_labels() -> Vec<Label> {
    vec![
        Label::new("Urgent", LabelColor::Orange),
        Label::new("Waiting", LabelColor::Yellow),
        Label::new("Review", LabelColor::Purple),
        Label::new("Done", LabelColor::Blue),
        Label::new("Keep", LabelColor::Brown),
        Label::new(SKIP_IN_SEARCHES, LabelColor::Gray),
    ]
}

/// Whether places of this kind compare without regard to case: the local
/// file systems of Windows and macOS do.
fn folds_case(place: &Place) -> bool {
    place.kind() == PlaceKind::Local && (cfg!(windows) || cfg!(target_os = "macos"))
}

/// The key a place is stored under: the place as written, in lower case
/// where its file system ignores case.
pub fn place_key(place: &Place) -> String {
    let text = place.to_string();
    if folds_case(place) {
        text.to_lowercase()
    } else {
        text
    }
}

/// A name as compared inside a folder: lower case where the folder's file
/// system ignores case.
fn name_key(folder: &Place, name: &str) -> String {
    if folds_case(folder) {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

/// Where `place` lands when `from` moves to `to`: `to` itself, or the
/// same names under `to` for a place inside `from`; `None` for a place
/// elsewhere, or one whose names cannot be joined under `to`.
fn moved_place(place: &Place, from: &Place, to: &Place) -> Option<Place> {
    let from_key = place_key(from);
    let mut names = Vec::new();
    let mut at = place.clone();
    while place_key(&at) != from_key {
        names.push(at.name()?.to_owned());
        at = at.parent()?;
    }
    names
        .iter()
        .rev()
        .try_fold(to.clone(), |under, name| under.join(name).ok())
}

/// One place the owner marked, and what it carries: its labels, in the
/// label list's order, its note and its named fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labeled {
    /// The place.
    pub place: Place,
    /// Its labels' names.
    pub labels: Vec<String>,
    /// The owner's note, one line.
    pub note: Option<String>,
    /// The owner's named fields, by name: `project = "Apollo"`.
    pub fields: BTreeMap<String, String>,
}

impl Labeled {
    /// Whether it carries nothing, so the store keeps no item for it.
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty() && self.note.is_none() && self.fields.is_empty()
    }
}

/// What a label command does to its items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelChange {
    /// Add the label to every item.
    Add(String),
    /// Take the label off every item.
    Remove(String),
    /// Take it off when every item has it, else add it.
    Toggle(String),
    /// Take every label off.
    Clear,
    /// Set every item's note, or remove it with `None` (alpha 4).
    Note(Option<String>),
    /// Set a named field on every item, or remove it with `None`.
    Field {
        /// The field's name, checked by [`check_field_name`].
        name: String,
        /// Its value, one line.
        value: Option<String>,
    },
}

/// How to put a label change back: each item's labels, note and fields
/// before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelUndo {
    /// What was done (never [`LabelChange::Toggle`]: the resolved change).
    pub done: LabelChange,
    /// Each changed place and its labels before the change.
    pub before: Vec<(Place, Vec<String>)>,
    /// Each changed place's note and fields before a note or field
    /// change.
    pub kept: Vec<(Place, Option<String>, BTreeMap<String, String>)>,
    /// The one item's name when one changed, for the sentence.
    pub one: Option<String>,
}

impl LabelUndo {
    /// What undoing it says: "Undid label urgent on 3 items", "Undid
    /// clear labels on report.pdf", "Undid note on report.pdf", "Undid
    /// field project on 3 items".
    pub fn sentence(&self) -> String {
        let what = subject(self.before.len() + self.kept.len(), self.one.as_deref());
        match &self.done {
            LabelChange::Clear => format!("Undid clear labels on {what}"),
            LabelChange::Add(name) | LabelChange::Remove(name) | LabelChange::Toggle(name) => {
                format!("Undid label {} on {what}", in_sentence(name))
            }
            LabelChange::Note(_) => format!("Undid note on {what}"),
            LabelChange::Field { name, .. } => format!("Undid field {name} on {what}"),
        }
    }
}

/// What a label change did, for its one sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// What was done (a toggle resolved to add or remove).
    pub done: LabelChange,
    /// How many items changed.
    pub changed: usize,
    /// How many were already as asked.
    pub unchanged: usize,
    /// The one item's name when there was one item.
    pub one: Option<String>,
    /// The undo step, when anything changed.
    pub undo: Option<LabelUndo>,
}

/// "3 items", or the item's name when there is one.
fn subject(items: usize, one: Option<&str>) -> String {
    match (items, one) {
        (1, Some(name)) => name.to_owned(),
        _ => count(items as u64, "item", "items"),
    }
}

impl Applied {
    /// The one sentence a change says, its meaning first: "Label urgent
    /// added to 3 items", "Label urgent removed from report.pdf", "Labels
    /// cleared from 2 items", "Label urgent is already on 3 items".
    pub fn sentence(&self) -> String {
        let all = self.changed + self.unchanged;
        let changed = subject(self.changed, self.one.as_deref());
        let everything = if all == 1 {
            subject(1, self.one.as_deref())
        } else {
            format!("these {}", count(all as u64, "item", "items"))
        };
        let rest = if self.changed > 0 && self.unchanged > 0 {
            match &self.done {
                LabelChange::Add(_)
                | LabelChange::Note(Some(_))
                | LabelChange::Field { value: Some(_), .. } => {
                    format!("; {} already had it", self.unchanged)
                }
                _ => format!("; {} did not have it", self.unchanged),
            }
        } else {
            String::new()
        };
        match &self.done {
            LabelChange::Note(Some(_)) if self.changed == 0 => {
                format!("The note is already on {everything}")
            }
            LabelChange::Note(Some(_)) => format!("Note set on {changed}{rest}"),
            LabelChange::Note(None) if self.changed == 0 => format!("No note on {everything}"),
            LabelChange::Note(None) => format!("Note removed from {changed}{rest}"),
            LabelChange::Field {
                name,
                value: Some(_),
            } if self.changed == 0 => format!("Field {name} is already so on {everything}"),
            LabelChange::Field {
                name,
                value: Some(_),
            } => format!("Field {name} set on {changed}{rest}"),
            LabelChange::Field { name, value: None } if self.changed == 0 => {
                format!("No field {name} on {everything}")
            }
            LabelChange::Field { name, value: None } => {
                format!("Field {name} removed from {changed}{rest}")
            }
            LabelChange::Clear if self.changed == 0 => format!("No labels on {everything}"),
            LabelChange::Clear => format!("Labels cleared from {changed}"),
            LabelChange::Add(name) | LabelChange::Toggle(name) if self.changed == 0 => {
                format!("Label {} is already on {everything}", in_sentence(name))
            }
            LabelChange::Add(name) | LabelChange::Toggle(name) => {
                format!("Label {} added to {changed}{rest}", in_sentence(name))
            }
            LabelChange::Remove(name) if self.changed == 0 => {
                format!("Label {} is not on {everything}", in_sentence(name))
            }
            LabelChange::Remove(name) => {
                format!("Label {} removed from {changed}{rest}", in_sentence(name))
            }
        }
    }
}

/// The labels and the labeled places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelStore {
    labels: Vec<Label>,
    items: BTreeMap<String, Labeled>,
}

impl Default for LabelStore {
    /// The starter set, nothing labeled.
    fn default() -> Self {
        Self::starter()
    }
}

impl LabelStore {
    /// The starter labels, nothing labeled: what a first start has.
    pub fn starter() -> Self {
        Self {
            labels: starter_labels(),
            items: BTreeMap::new(),
        }
    }

    /// No labels at all.
    pub fn empty() -> Self {
        Self {
            labels: Vec::new(),
            items: BTreeMap::new(),
        }
    }

    /// The labels, in the owner's order; the first seven are favorites.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// The labels' names, in order: what the filter language's lints and
    /// completion read (its `LintContext` and `CompletionContext`).
    pub fn names(&self) -> Vec<String> {
        self.labels.iter().map(|l| l.name.clone()).collect()
    }

    /// A label by name, ignoring case.
    pub fn label(&self, name: &str) -> Option<&Label> {
        let wanted = name.trim().to_lowercase();
        self.labels.iter().find(|l| l.name.to_lowercase() == wanted)
    }

    /// The theme role that colors a label's word, by the label's name,
    /// ignoring case: for the Labels list's rows (whose ids are the
    /// names), the Set label list and a panel row's "label: urgent". `None`
    /// for a name the store does not hold.
    pub fn role_of(&self, name: &str) -> Option<PanelRole> {
        self.label(name).map(|label| label.color.role())
    }

    /// The label a favorite key toggles: 1 is the first.
    pub fn favorite(&self, number: usize) -> Option<&Label> {
        (1..=FAVORITES)
            .contains(&number)
            .then(|| self.labels.get(number - 1))
            .flatten()
    }

    /// The favorite number of a label, 1 to 7, when it has one.
    pub fn favorite_number(&self, name: &str) -> Option<usize> {
        let wanted = name.to_lowercase();
        self.labels
            .iter()
            .take(FAVORITES)
            .position(|l| l.name.to_lowercase() == wanted)
            .map(|i| i + 1)
    }

    /// Adds a label with the next color no label has yet (or the next in
    /// turn when all six are used), or returns the one already named so.
    /// The error is a sentence.
    pub fn add_label(&mut self, name: &str) -> Result<&Label, String> {
        let name = check_name(name)?;
        if let Some(index) = self
            .labels
            .iter()
            .position(|l| l.name.to_lowercase() == name.to_lowercase())
        {
            return Ok(&self.labels[index]);
        }
        let color = LabelColor::ALL
            .into_iter()
            .find(|c| !self.labels.iter().any(|l| l.color == *c))
            .unwrap_or(LabelColor::ALL[self.labels.len() % LabelColor::ALL.len()]);
        self.labels.push(Label::new(name, color));
        let index = self.labels.len() - 1;
        Ok(&self.labels[index])
    }

    /// The labels on a place, in the label list's order.
    pub fn labels_of(&self, place: &Place) -> &[String] {
        self.items
            .get(&place_key(place))
            .map_or(&[], |item| item.labels.as_slice())
    }

    /// The owner's note on a place.
    pub fn note_of(&self, place: &Place) -> Option<&str> {
        self.items
            .get(&place_key(place))
            .and_then(|item| item.note.as_deref())
    }

    /// The owner's named fields on a place, by name.
    pub fn fields_of(&self, place: &Place) -> Option<&BTreeMap<String, String>> {
        self.items
            .get(&place_key(place))
            .map(|item| &item.fields)
            .filter(|fields| !fields.is_empty())
    }

    /// Every field name used on any item, in order: the Set field
    /// prompt's candidates and the column chooser's `custom.NAME` keys.
    pub fn field_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .items
            .values()
            .flat_map(|item| item.fields.keys().cloned())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Every place the owner marked, by key: labeled, or with a note or
    /// a field.
    pub fn items(&self) -> impl Iterator<Item = &Labeled> {
        self.items.values()
    }

    /// Keeps each moved place's labels, note and fields under its new
    /// place, and those of everything inside a moved folder under the
    /// folder's new place: what a finished move or rename hands over, as
    /// (from, to) pairs in the order done. Returns how many items moved
    /// with their places.
    pub fn follow_moves(&mut self, moves: &[(Place, Place)]) -> usize {
        let mut moved = 0;
        for (from, to) in moves {
            let landed: Vec<(String, Place)> = self
                .items
                .iter()
                .filter_map(|(key, item)| {
                    moved_place(&item.place, from, to).map(|place| (key.clone(), place))
                })
                .collect();
            for (key, place) in landed {
                let Some(mut item) = self.items.remove(&key) else {
                    continue;
                };
                item.place = place;
                moved += 1;
                self.items.insert(place_key(&item.place), item);
            }
        }
        moved
    }

    /// The places that carry a label, in key order.
    pub fn places_with(&self, name: &str) -> Vec<&Place> {
        let wanted = name.to_lowercase();
        self.items
            .values()
            .filter(|item| item.labels.iter().any(|l| l.to_lowercase() == wanted))
            .map(|item| &item.place)
            .collect()
    }

    /// Whether a place carries "Skip in searches": search leaves it out.
    pub fn skipped_in_searches(&self, place: &Place) -> bool {
        self.labels_of(place)
            .iter()
            .any(|l| l.eq_ignore_ascii_case(SKIP_IN_SEARCHES))
    }

    /// Fills in each listed entry's labels from the store, and its note
    /// and fields as the columns `custom.note` and `custom.NAME` in its
    /// [`Entry::extra`]: the entries are a listing of `folder`. A quick
    /// pass when nothing in the folder is marked.
    pub fn apply_to_entries(&self, folder: &Place, entries: &mut [Entry]) {
        let folder_key = place_key(folder);
        let mut here: HashMap<String, &Labeled> = HashMap::new();
        for item in self.items.values() {
            let in_folder = item
                .place
                .parent()
                .is_some_and(|parent| place_key(&parent) == folder_key);
            if let (true, Some(name)) = (in_folder, item.place.name()) {
                here.insert(name_key(folder, name), item);
            }
        }
        for entry in entries {
            let item = here.get(&name_key(folder, &entry.name));
            entry.labels = item.map(|item| item.labels.clone()).unwrap_or_default();
            if !entry.extra.is_empty() {
                entry.extra.retain(|key, _| !key.starts_with(CUSTOM_PREFIX));
            }
            let Some(item) = item else {
                continue;
            };
            if let Some(note) = &item.note {
                entry.extra.insert(NOTE_KEY.to_owned(), note.clone());
            }
            for (name, value) in &item.fields {
                entry.extra.insert(field_key(name), value.clone());
            }
        }
    }

    /// Sets a place's labels, in the label list's order, keeping its note
    /// and fields; a place left with nothing is taken out of the store.
    fn put(&mut self, place: &Place, labels: Vec<String>) {
        let key = place_key(place);
        let order = |name: &String| {
            self.labels
                .iter()
                .position(|l| l.name.eq_ignore_ascii_case(name))
                .unwrap_or(usize::MAX)
        };
        let mut labels = labels;
        labels.sort_by_key(order);
        labels.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let (note, fields) = self
            .items
            .remove(&key)
            .map(|item| (item.note, item.fields))
            .unwrap_or_default();
        self.keep(
            key,
            Labeled {
                place: place.clone(),
                labels,
                note,
                fields,
            },
        );
    }

    /// Sets a place's note and fields, keeping its labels; a place left
    /// with nothing is taken out of the store.
    fn put_owned(&mut self, place: &Place, note: Option<String>, fields: BTreeMap<String, String>) {
        let key = place_key(place);
        let labels = self
            .items
            .remove(&key)
            .map(|item| item.labels)
            .unwrap_or_default();
        self.keep(
            key,
            Labeled {
                place: place.clone(),
                labels,
                note,
                fields,
            },
        );
    }

    fn keep(&mut self, key: String, item: Labeled) {
        if !item.is_empty() {
            self.items.insert(key, item);
        }
    }

    /// Sets or removes a note or a field on places (alpha 4); `one` is
    /// the item's name when there is one item.
    fn apply_owned(
        &mut self,
        change: &LabelChange,
        places: &[Place],
        one: Option<&str>,
    ) -> Applied {
        let mut kept = Vec::new();
        let mut unchanged = 0;
        for place in places {
            let item = self.items.get(&place_key(place));
            let note = item.and_then(|item| item.note.clone());
            let fields = item.map(|item| item.fields.clone()).unwrap_or_default();
            let (new_note, new_fields) = match change {
                LabelChange::Note(text) => (text.clone(), fields.clone()),
                LabelChange::Field { name, value } => {
                    let mut new = fields.clone();
                    match value {
                        Some(value) => new.insert(name.clone(), value.clone()),
                        None => new.remove(name),
                    };
                    (note.clone(), new)
                }
                _ => (note.clone(), fields.clone()),
            };
            if new_note == note && new_fields == fields {
                unchanged += 1;
                continue;
            }
            kept.push((place.clone(), note, fields));
            self.put_owned(place, new_note, new_fields);
        }
        let changed = kept.len();
        let one = one.map(str::to_owned);
        let undo = (changed > 0).then(|| LabelUndo {
            done: change.clone(),
            before: Vec::new(),
            kept,
            one: one.clone(),
        });
        Applied {
            done: change.clone(),
            changed,
            unchanged,
            one,
            undo,
        }
    }

    /// Applies a change to places; `one` is the item's name when there is
    /// one item. A label that is not in the list yet is added first, so a
    /// change never refers to a label the list does not have.
    pub fn apply(&mut self, change: &LabelChange, places: &[Place], one: Option<&str>) -> Applied {
        let name = match change {
            LabelChange::Add(n) | LabelChange::Remove(n) | LabelChange::Toggle(n) => Some(
                self.label(n)
                    .map_or_else(|| n.trim().to_owned(), |l| l.name.clone()),
            ),
            LabelChange::Clear => None,
            LabelChange::Note(_) | LabelChange::Field { .. } => {
                return self.apply_owned(change, places, one);
            }
        };
        let has = |store: &Self, place: &Place, name: &str| {
            store
                .labels_of(place)
                .iter()
                .any(|l| l.eq_ignore_ascii_case(name))
        };
        let done = match (change, &name) {
            (LabelChange::Toggle(_), Some(name)) => {
                if !places.is_empty() && places.iter().all(|p| has(self, p, name)) {
                    LabelChange::Remove(name.clone())
                } else {
                    LabelChange::Add(name.clone())
                }
            }
            (LabelChange::Add(_), Some(name)) => LabelChange::Add(name.clone()),
            (LabelChange::Remove(_), Some(name)) => LabelChange::Remove(name.clone()),
            _ => LabelChange::Clear,
        };
        if let LabelChange::Add(name) = &done
            && self.label(name).is_none()
            && self.add_label(name).is_err()
        {
            // A name no label may have changes nothing.
            return Applied {
                done,
                changed: 0,
                unchanged: places.len(),
                one: one.map(str::to_owned),
                undo: None,
            };
        }
        let mut before = Vec::new();
        let mut unchanged = 0;
        for place in places {
            let now: Vec<String> = self.labels_of(place).to_vec();
            let after: Vec<String> = match &done {
                LabelChange::Add(name) if !has(self, place, name) => {
                    let mut after = now.clone();
                    after.push(name.clone());
                    after
                }
                LabelChange::Remove(name) if has(self, place, name) => now
                    .iter()
                    .filter(|l| !l.eq_ignore_ascii_case(name))
                    .cloned()
                    .collect(),
                LabelChange::Clear if !now.is_empty() => Vec::new(),
                _ => {
                    unchanged += 1;
                    continue;
                }
            };
            before.push((place.clone(), now));
            self.put(place, after);
        }
        let changed = before.len();
        let one = one.map(str::to_owned);
        let undo = (changed > 0).then(|| LabelUndo {
            done: done.clone(),
            before,
            kept: Vec::new(),
            one: one.clone(),
        });
        Applied {
            done,
            changed,
            unchanged,
            one,
            undo,
        }
    }

    /// Puts back what an undo step recorded.
    pub fn restore(&mut self, undo: &LabelUndo) {
        for (place, labels) in &undo.before {
            self.put(place, labels.clone());
        }
        for (place, note, fields) in &undo.kept {
            self.put_owned(place, note.clone(), fields.clone());
        }
    }

    /// Reads `labels.toml` text. Labels used by an item but not listed are
    /// added to the list; a color that is not offered (red, green) is
    /// replaced and said; a place that cannot be read is left out and
    /// said. The problems are sentences, meaning first.
    pub fn from_toml(text: &str) -> Result<(Self, Vec<String>), toml::de::Error> {
        let file: LabelsFile = toml::from_str(text)?;
        let mut store = Self::empty();
        let mut problems = Vec::new();
        for line in file.label {
            let name = match check_name(&line.name) {
                Ok(name) => name,
                Err(why) => {
                    problems.push(format!("Label left out: {why}"));
                    continue;
                }
            };
            if store.label(&name).is_some() {
                continue;
            }
            let color = match line.color.as_deref().map(LabelColor::from_word) {
                Some(Some(color)) => Some(color),
                Some(None) => {
                    problems.push(format!(
                        "Label {name}: {} is not offered; labels never use red or green.",
                        line.color.as_deref().unwrap_or_default()
                    ));
                    None
                }
                None => None,
            };
            match color {
                Some(color) => store.labels.push(Label::new(name, color)),
                None => {
                    let _ = store.add_label(&name);
                }
            }
        }
        for line in file.item {
            let Ok(place) = Place::parse(&line.place) else {
                problems.push(format!("A labeled place cannot be read: {}", line.place));
                continue;
            };
            let mut labels = Vec::new();
            for name in &line.labels {
                let name = match store.label(name) {
                    Some(label) => label.name.clone(),
                    None => match store.add_label(name) {
                        Ok(label) => label.name.clone(),
                        Err(_) => continue,
                    },
                };
                labels.push(name);
            }
            store.put(&place, labels);
            // Format 2's note and fields; a format 1 item has neither.
            let shown = diroxide_core::names::spell_out(&line.place).into_owned();
            let note = line.note.as_deref().and_then(|text| {
                let (kept, cut) = one_line(text);
                if cut {
                    problems.push(format!(
                        "Note on {shown} was cut to one line of {NOTE_LIMIT} characters."
                    ));
                }
                kept
            });
            let mut fields = BTreeMap::new();
            for (name, value) in &line.fields {
                match check_field_name(name) {
                    Ok(name) => {
                        let (kept, cut) = one_line(value);
                        if cut {
                            problems.push(format!(
                                "Field {name} on {shown} was cut to one line of {NOTE_LIMIT} characters."
                            ));
                        }
                        if let Some(value) = kept {
                            fields.insert(name, value);
                        }
                    }
                    Err(why) => problems.push(format!("Field left out on {shown}: {why}")),
                }
            }
            if note.is_some() || !fields.is_empty() {
                store.put_owned(&place, note, fields);
            }
        }
        Ok((store, problems))
    }

    /// The store as `labels.toml` text.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let file = LabelsFile {
            format: FORMAT,
            label: self
                .labels
                .iter()
                .map(|l| LabelLine {
                    name: l.name.clone(),
                    color: Some(l.color.word().to_owned()),
                })
                .collect(),
            item: self
                .items
                .values()
                .map(|item| ItemLine {
                    place: item.place.to_string(),
                    labels: item.labels.clone(),
                    note: item.note.clone(),
                    fields: item.fields.clone(),
                })
                .collect(),
        };
        toml::to_string(&file)
    }
}

/// A note or field value as one line of at most [`NOTE_LIMIT`]
/// characters, control characters made spaces; `None` when nothing is
/// left; and whether it had to change.
fn one_line(text: &str) -> (Option<String>, bool) {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = spaced.trim();
    let kept: String = trimmed.chars().take(NOTE_LIMIT).collect();
    let changed = kept.len() != trimmed.len() || spaced != text;
    (
        (!kept.is_empty()).then_some(kept),
        changed && !text.trim().is_empty(),
    )
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct LabelsFile {
    format: u32,
    label: Vec<LabelLine>,
    item: Vec<ItemLine>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct LabelLine {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ItemLine {
    place: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    // A table, so it is written last, after the item's plain values.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    fields: BTreeMap<String, String>,
}

/// Reads `labels.toml` from the state folder: the starter set when there
/// is none; when it cannot be read, the starter set, a copy of the file
/// kept as `labels.toml.bad`, and one sentence that says so.
pub fn load(paths: &Paths) -> Loaded<LabelStore> {
    match state_file::read(paths, FILE) {
        state_file::Read::Missing => Loaded::clean(LabelStore::starter()),
        state_file::Read::Failed(problem) => Loaded {
            value: LabelStore::starter(),
            problems: vec![problem],
        },
        state_file::Read::Text(text) => match LabelStore::from_toml(&text) {
            Ok((store, problems)) => Loaded {
                value: store,
                problems: problems
                    .into_iter()
                    .map(|p| ConfigProblem::new(FILE, p))
                    .collect(),
            },
            Err(error) => Loaded {
                value: LabelStore::starter(),
                problems: vec![state_file::unreadable(
                    paths,
                    FILE,
                    &text,
                    &error,
                    "the starter labels are in use",
                )],
            },
        },
    }
}

/// Writes `labels.toml` to the state folder atomically. The error is the
/// cause in words.
pub fn save(paths: &Paths, store: &LabelStore) -> Result<(), String> {
    let text = store.to_toml().map_err(|e| e.to_string())?;
    state_file::write(paths, FILE, &text)
}

/// Reading and writing the state files of labels, bookmarks and folder
/// views the same way: a missing file says nothing, a file that cannot be
/// read is copied to `NAME.bad` (never over a different earlier copy) and
/// said in one sentence, and every write is atomic.
pub(crate) mod state_file {
    use std::fs;
    use std::io;

    use diroxide_config::{ConfigProblem, Paths, write_atomic};

    /// What reading a state file gave.
    pub(crate) enum Read {
        /// No file yet.
        Missing,
        /// Its text.
        Text(String),
        /// It could not be read; the problem says why.
        Failed(ConfigProblem),
    }

    /// Reads a state file's text.
    pub(crate) fn read(paths: &Paths, name: &str) -> Read {
        match fs::read_to_string(paths.state_file(name)) {
            Ok(text) => Read::Text(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Read::Missing,
            Err(e) => Read::Failed(ConfigProblem::new(
                name,
                format!("{name} could not be opened: {e}."),
            )),
        }
    }

    /// Keeps a copy of text that could not be read as `NAME.bad`, or the
    /// first of `NAME.bad2` and on that is free or holds the same text;
    /// returns the copy's name, empty when none could be made.
    fn back_up(paths: &Paths, name: &str, text: &str) -> String {
        for n in 1..10 {
            let candidate = if n == 1 {
                format!("{name}.bad")
            } else {
                format!("{name}.bad{n}")
            };
            let path = paths.state_file(&candidate);
            match fs::read_to_string(&path) {
                Ok(existing) if existing == text => return candidate,
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return match write_atomic(&path, text) {
                        Ok(()) => candidate,
                        Err(_) => String::new(),
                    };
                }
                Err(_) => continue,
            }
        }
        String::new()
    }

    /// The problem for a file that is not valid: its copy is kept, and the
    /// sentence says what is in use instead.
    pub(crate) fn unreadable(
        paths: &Paths,
        name: &str,
        text: &str,
        error: &toml::de::Error,
        instead: &str,
    ) -> ConfigProblem {
        let line = error.span().map(|span| {
            text.get(..span.start.min(text.len()))
                .map_or(1, |before| before.matches('\n').count() + 1)
        });
        let place = match line {
            Some(line) => format!("line {line} is not valid"),
            None => "it is not valid".to_owned(),
        };
        let backup = back_up(paths, name, text);
        let sentence = if backup.is_empty() {
            format!("{name} could not be read: {place}. {}.", capital(instead))
        } else {
            format!("{name} could not be read: {place}. A copy is kept as {backup}; {instead}.")
        };
        ConfigProblem::new(name, sentence)
    }

    fn capital(text: &str) -> String {
        let mut chars = text.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }

    /// Writes a state file atomically; the error is the cause in words.
    pub(crate) fn write(paths: &Paths, name: &str, text: &str) -> Result<(), String> {
        write_atomic(&paths.state_file(name), text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::EntryKind;
    use diroxide_core::format::within_cells;

    fn p(text: &str) -> Place {
        if cfg!(windows) {
            Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
        } else {
            Place::parse(text).unwrap()
        }
    }

    #[test]
    fn the_starter_set_is_six_names_and_never_red_or_green() {
        let store = LabelStore::starter();
        assert_eq!(
            store.names(),
            [
                "Urgent",
                "Waiting",
                "Review",
                "Done",
                "Keep",
                "Skip in searches"
            ]
        );
        for color in LabelColor::ALL {
            assert!(!["red", "green"].contains(&color.word()));
            // Its theme role is named for it (P3b).
            assert_eq!(
                color.role().key().strip_prefix("label_"),
                Some(color.word())
            );
        }
        let roles: Vec<PanelRole> = LabelColor::ALL.iter().map(|c| c.role()).collect();
        assert_eq!(roles, PanelRole::LABELS, "one role each, in the same order");
        assert_eq!(store.role_of("urgent"), Some(PanelRole::LabelOrange));
        assert_eq!(store.role_of("Done"), Some(PanelRole::LabelBlue));
        assert_eq!(store.role_of("nothing"), None);
        assert_eq!(LabelColor::from_word("red"), None);
        assert_eq!(LabelColor::from_word("Green"), None);
        assert_eq!(LabelColor::from_word("grey"), Some(LabelColor::Gray));
        assert_eq!(store.favorite(1).unwrap().name, "Urgent");
        assert_eq!(store.favorite(6).unwrap().name, "Skip in searches");
        assert_eq!(store.favorite(7), None);
        assert_eq!(store.favorite(0), None);
        assert_eq!(store.favorite_number("review"), Some(3));
    }

    #[test]
    fn names_read_naturally_inside_a_sentence() {
        assert_eq!(in_sentence("Urgent"), "urgent");
        assert_eq!(in_sentence("Skip in searches"), "skip in searches");
        assert_eq!(in_sentence("TODO"), "TODO");
        assert_eq!(in_sentence("iPhone"), "iPhone");
        assert_eq!(in_sentence("Q3"), "Q3");
        assert_eq!(in_sentence("x"), "x");
        assert!(check_name("  ").is_err());
        assert!(check_name(&"a".repeat(31)).is_err());
        assert!(check_name("a\tb").is_err());
        assert_eq!(check_name(" Taxes ").unwrap(), "Taxes");
    }

    #[test]
    fn adding_marks_each_item_once_and_says_so() {
        let mut store = LabelStore::starter();
        let items = [p("/w/a.txt"), p("/w/b.txt"), p("/w/c.txt")];
        let applied = store.apply(&LabelChange::Add("urgent".into()), &items, None);
        assert_eq!(applied.sentence(), "Label urgent added to 3 items");
        assert_eq!(store.labels_of(&items[0]), ["Urgent"]);
        let again = store.apply(&LabelChange::Add("Urgent".into()), &items, None);
        assert_eq!(again.sentence(), "Label urgent is already on these 3 items");
        assert!(again.undo.is_none());
        let more = [p("/w/a.txt"), p("/w/d.txt")];
        let partly = store.apply(&LabelChange::Add("Urgent".into()), &more, None);
        assert_eq!(
            partly.sentence(),
            "Label urgent added to 1 item; 1 already had it"
        );
        let one = store.apply(
            &LabelChange::Add("Review".into()),
            &items[..1],
            Some("a.txt"),
        );
        assert_eq!(one.sentence(), "Label review added to a.txt");
        // The list's order, not the order applied.
        assert_eq!(store.labels_of(&items[0]), ["Urgent", "Review"]);
        for text in [applied.sentence(), one.sentence()] {
            assert!(within_cells(&text, &text[..text.len().min(30)]));
        }
    }

    #[test]
    fn a_toggle_removes_only_when_every_item_has_it_and_undo_puts_it_back() {
        let mut store = LabelStore::starter();
        let items = [p("/w/a.txt"), p("/w/b.txt"), p("/w/c.txt")];
        store.apply(&LabelChange::Add("Done".into()), &items[..2], None);
        let toggled = store.apply(&LabelChange::Toggle("done".into()), &items, None);
        assert_eq!(toggled.done, LabelChange::Add("Done".into()));
        assert_eq!(
            toggled.sentence(),
            "Label done added to 1 item; 2 already had it"
        );
        let off = store.apply(&LabelChange::Toggle("Done".into()), &items, None);
        assert_eq!(off.sentence(), "Label done removed from 3 items");
        assert!(store.labels_of(&items[0]).is_empty());
        let undo = off.undo.unwrap();
        assert_eq!(undo.sentence(), "Undid label done on 3 items");
        store.restore(&undo);
        assert_eq!(store.labels_of(&items[2]), ["Done"]);
        store.apply(&LabelChange::Add("Keep".into()), &items[..1], None);
        let cleared = store.apply(&LabelChange::Clear, &items[..1], Some("a.txt"));
        assert_eq!(cleared.sentence(), "Labels cleared from a.txt");
        assert_eq!(
            cleared.undo.as_ref().unwrap().sentence(),
            "Undid clear labels on a.txt"
        );
        let nothing = store.apply(&LabelChange::Clear, &items[..1], Some("a.txt"));
        assert_eq!(nothing.sentence(), "No labels on a.txt");
        let not_on = store.apply(&LabelChange::Remove("Keep".into()), &items, None);
        assert_eq!(not_on.sentence(), "Label keep is not on these 3 items");
    }

    #[test]
    fn a_new_label_takes_a_color_no_label_has() {
        let mut store = LabelStore::empty();
        let first = store.add_label("Taxes").unwrap().color;
        assert_eq!(first, LabelColor::Blue);
        assert_eq!(store.add_label("taxes").unwrap().name, "Taxes");
        assert_eq!(store.labels().len(), 1);
        let mut store = LabelStore::starter();
        let next = store.add_label("Taxes").unwrap().color;
        assert_eq!(next, LabelColor::Blue);
        let mut store = LabelStore::empty();
        store
            .apply(&LabelChange::Add("Brand new".into()), &[p("/x")], None)
            .undo
            .unwrap();
        assert_eq!(store.names(), ["Brand new"]);
    }

    #[test]
    fn entries_get_their_labels_from_their_folder() {
        let mut store = LabelStore::starter();
        store.apply(&LabelChange::Add("Urgent".into()), &[p("/w/a.txt")], None);
        store.apply(&LabelChange::Add("Keep".into()), &[p("/other/a.txt")], None);
        let mut entries = vec![
            Entry::new("a.txt", EntryKind::File),
            Entry::new("b.txt", EntryKind::File),
        ];
        entries[1].labels = vec!["stale".into()];
        store.apply_to_entries(&p("/w"), &mut entries);
        assert_eq!(entries[0].labels, ["Urgent"]);
        assert!(entries[1].labels.is_empty());
        if cfg!(windows) {
            let mut shouting = vec![Entry::new("A.TXT", EntryKind::File)];
            store.apply_to_entries(&p("/W"), &mut shouting);
            assert_eq!(shouting[0].labels, ["Urgent"]);
        }
    }

    #[test]
    fn the_file_round_trips_and_refuses_red_and_green() {
        let mut store = LabelStore::starter();
        store.apply(
            &LabelChange::Add("Urgent".into()),
            &[p("/w/a.txt"), p("/w/My Report.pdf")],
            None,
        );
        store.apply(&LabelChange::Add("Taxes".into()), &[p("/w/a.txt")], None);
        let text = store.to_toml().unwrap();
        let (read, problems) = LabelStore::from_toml(&text).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(read, store);
        let (read, problems) = LabelStore::from_toml(
            "[[label]]\nname = \"Hot\"\ncolor = \"red\"\n\n[[item]]\nplace = \"\"\nlabels = [\"Hot\"]\n",
        )
        .unwrap();
        assert_eq!(read.labels()[0].color, LabelColor::Blue);
        assert!(problems[0].starts_with("Label Hot: red is not offered"));
        assert!(problems[1].starts_with("A labeled place cannot be read"));
        assert!(LabelStore::from_toml("[[label]\nname=").is_err());
    }

    #[test]
    fn notes_and_fields_are_set_said_undone_and_shown_as_columns() {
        let mut store = LabelStore::starter();
        let items = [p("/w/a.txt"), p("/w/b.txt")];
        let note = check_note("  call about the invoice ").unwrap();
        let set = store.apply(&LabelChange::Note(note.clone()), &items[..1], Some("a.txt"));
        assert_eq!(set.sentence(), "Note set on a.txt");
        assert_eq!(store.note_of(&items[0]), Some("call about the invoice"));
        let again = store.apply(&LabelChange::Note(note), &items[..1], Some("a.txt"));
        assert_eq!(again.sentence(), "The note is already on a.txt");
        let field = LabelChange::Field {
            name: check_field_name("project").unwrap(),
            value: Some("Apollo".to_owned()),
        };
        let both = store.apply(&field, &items, None);
        assert_eq!(both.sentence(), "Field project set on 2 items");
        assert_eq!(store.field_names(), ["project"]);
        // Labels and notes live side by side: clearing labels keeps the
        // note, and a place keeps its item while anything is left.
        store.apply(&LabelChange::Add("Urgent".into()), &items[..1], None);
        store.apply(&LabelChange::Clear, &items[..1], None);
        assert_eq!(store.note_of(&items[0]), Some("call about the invoice"));
        let mut entries = vec![
            Entry::new("a.txt", EntryKind::File),
            Entry::new("c.txt", EntryKind::File),
        ];
        entries[1]
            .extra
            .insert("custom.stale".to_owned(), "gone".to_owned());
        store.apply_to_entries(&p("/w"), &mut entries);
        assert_eq!(entries[0].extra[NOTE_KEY], "call about the invoice");
        assert_eq!(entries[0].extra[&field_key("project")], "Apollo");
        assert!(entries[1].extra.is_empty(), "{:?}", entries[1].extra);
        // Undo puts the fields back as they were, and says so.
        let undo = both.undo.unwrap();
        assert_eq!(undo.sentence(), "Undid field project on 2 items");
        store.restore(&undo);
        assert_eq!(store.fields_of(&items[0]), None);
        assert!(store.items().all(|item| item.place == items[0]));
        let removed = store.apply(&LabelChange::Note(None), &items[..1], Some("a.txt"));
        assert_eq!(removed.sentence(), "Note removed from a.txt");
        assert_eq!(store.items().count(), 0, "nothing left, no item kept");
        for text in [
            set.sentence(),
            removed.sentence(),
            "Note: call about the invoice".to_owned(),
        ] {
            assert!(within_cells(&text, &text));
        }
    }

    #[test]
    fn a_note_and_a_field_name_are_checked_in_words() {
        assert_eq!(check_note("   "), Ok(None));
        assert!(check_note(&"x".repeat(NOTE_LIMIT + 1)).is_err());
        assert!(check_note("two\nlines").is_err());
        assert_eq!(check_field_name(" project "), Ok("project".to_owned()));
        for bad in ["", "Project", "a b", "note", "x/y", &"a".repeat(25)] {
            assert!(check_field_name(bad).is_err(), "{bad}");
        }
        assert_eq!(field_label("project_code"), "Project code");
        assert_eq!(field_key("project"), "custom.project");
    }

    #[test]
    fn labels_notes_and_fields_follow_a_move_and_a_folder_move() {
        let mut store = LabelStore::starter();
        store.apply(&LabelChange::Add("Urgent".into()), &[p("/w/a.txt")], None);
        store.apply(
            &LabelChange::Note(Some("inside".into())),
            &[p("/w/docs/deep/b.txt")],
            None,
        );
        store.apply(
            &LabelChange::Field {
                name: "project".into(),
                value: Some("Apollo".into()),
            },
            &[p("/w/docs")],
            None,
        );
        store.apply(
            &LabelChange::Add("Keep".into()),
            &[p("/w/docsx/c.txt")],
            None,
        );
        let moved = store.follow_moves(&[
            (p("/w/a.txt"), p("/w/renamed.txt")),
            (p("/w/docs"), p("/archive/docs-2026")),
        ]);
        assert_eq!(moved, 3);
        assert!(store.labels_of(&p("/w/a.txt")).is_empty());
        assert_eq!(store.labels_of(&p("/w/renamed.txt")), ["Urgent"]);
        assert_eq!(
            store.note_of(&p("/archive/docs-2026/deep/b.txt")),
            Some("inside")
        );
        assert_eq!(
            store.fields_of(&p("/archive/docs-2026")).unwrap()["project"],
            "Apollo"
        );
        // A sibling whose name only starts the same stays where it was.
        assert_eq!(store.labels_of(&p("/w/docsx/c.txt")), ["Keep"]);
        // A move of something unmarked changes nothing.
        assert_eq!(store.follow_moves(&[(p("/w/z"), p("/w/y"))]), 0);
    }

    #[test]
    fn format_1_still_loads_and_format_2_round_trips() {
        let old = "format = 1\n\n[[label]]\nname = \"Urgent\"\ncolor = \"orange\"\n\n[[item]]\nplace = \"/w/a.txt\"\nlabels = [\"Urgent\"]\n";
        let old = if cfg!(windows) {
            old.replace("/w/a.txt", "D:\\\\w\\\\a.txt")
        } else {
            old.to_owned()
        };
        let (read, problems) = LabelStore::from_toml(&old).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(read.labels_of(&p("/w/a.txt")), ["Urgent"]);
        assert_eq!(read.note_of(&p("/w/a.txt")), None);
        let mut store = read;
        store.apply(
            &LabelChange::Note(Some("a note".into())),
            &[p("/w/a.txt")],
            None,
        );
        store.apply(
            &LabelChange::Field {
                name: "project".into(),
                value: Some("Apollo".into()),
            },
            &[p("/w/b.txt")],
            None,
        );
        let text = store.to_toml().unwrap();
        assert!(text.starts_with("format = 2\n"), "{text}");
        let (back, problems) = LabelStore::from_toml(&text).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back, store);
        // A field name no column could carry is left out and said; a
        // note with a line break is made one line and said.
        let bad = "[[item]]\nplace = \"/w/c.txt\"\nnote = \"one\\ntwo\"\n[item.fields]\n\"Bad Name\" = \"x\"\n";
        let bad = if cfg!(windows) {
            bad.replace("/w/c.txt", "D:\\\\w\\\\c.txt")
        } else {
            bad.to_owned()
        };
        let (read, problems) = LabelStore::from_toml(&bad).unwrap();
        assert_eq!(read.note_of(&p("/w/c.txt")), Some("one two"));
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("Note on "), "{problems:?}");
        assert!(
            problems[1].starts_with("Field left out on "),
            "{problems:?}"
        );
    }

    #[test]
    fn the_store_round_trips_through_an_injected_state_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        let first = load(&paths);
        assert!(first.problems.is_empty());
        assert_eq!(first.value, LabelStore::starter());
        let mut store = first.value;
        store.apply(&LabelChange::Add("Urgent".into()), &[p("/w/a.txt")], None);
        save(&paths, &store).unwrap();
        assert!(paths.state_file(FILE).exists());
        assert_eq!(load(&paths).value, store);
        std::fs::write(paths.state_file(FILE), "format = [").unwrap();
        let broken = load(&paths);
        assert_eq!(broken.value, LabelStore::starter());
        let said = &broken.problems[0].text;
        assert!(
            said.starts_with("labels.toml could not be read: line 1"),
            "{said}"
        );
        assert!(said.contains("labels.toml.bad"), "{said}");
        assert!(paths.state_file("labels.toml.bad").exists());
    }
}
