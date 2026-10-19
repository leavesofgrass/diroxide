//! The entry model (plan section 5.19): one text field for every prompt in
//! both frontends, as the list model is one list. A field has a label said
//! first, its text, a cursor, a completion source and a history key; the
//! frontend draws it (the minibuffer in a terminal, an editable combo box
//! in the GUI) and the app feeds it candidates from its source.
//!
//! Completion reads the same everywhere: candidates that start with what
//! was typed come first, then those that contain it, without regard to
//! case, and the real name is inserted. Tab completes the single match, or
//! the longest common start, and says what it did: "completed to
//! Downloads", "4 matches, Down arrow lists them". Up and Down with the
//! list closed recall the prompt's history, kept per prompt in the state
//! folder by its history key, and never for a password (the masked prompt
//! is its own dialog, ADR-0009). A `~` or a variable typed in a path field
//! is offered as a candidate that completes to the literal path; nothing
//! is expanded when the text is used.

use diroxide_core::Layer;

use crate::commands::CATALOG;
use crate::keymap::Keymap;

/// Where a field's candidates come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionSource {
    /// No completion.
    None,
    /// Folders and files through the field's provider; the part after the
    /// last separator is completed.
    Paths,
    /// Places: bookmarks, history, drives and the path jump store.
    Places,
    /// Commands by name (the palette, the run-command prompt's names).
    Commands,
    /// The prompt's own history only.
    History,
    /// A fixed list, such as a setting's choices.
    Values(Vec<String>),
    /// The labels' names (W3l): the Set label prompt.
    Labels,
    /// The bookmarks (W3l): each completes to its place, its name said
    /// after it.
    Bookmarks,
    /// The filter language's words (W3a): fields, comparisons, units,
    /// date words, labels, groups and types, from the parser's
    /// expected-next list; the word before the cursor is completed.
    Filter,
}

/// One candidate: the text inserted, and its kind said after it
/// ("Downloads, folder").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The text inserted when chosen.
    pub text: String,
    /// What it is, in a word or two: "folder", "file", "command, F5".
    pub kind: String,
}

impl Candidate {
    /// A candidate.
    pub fn new(text: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: kind.into(),
        }
    }

    /// The candidate as said, the text first: "Downloads, folder".
    pub fn spoken(&self) -> String {
        if self.kind.is_empty() {
            self.text.clone()
        } else {
            format!("{}, {}", self.text, self.kind)
        }
    }
}

/// What Tab did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// Nothing matches.
    NoMatch,
    /// The one match was inserted.
    Completed(String),
    /// Several match; their common start was inserted.
    Extended {
        /// The text now in the field's completed part.
        prefix: String,
        /// How many match.
        count: usize,
    },
    /// Several match, and nothing more could be inserted.
    Several(usize),
}

impl Completion {
    /// What is said, meaning first: "completed to Downloads", "4 matches,
    /// Down arrow lists them", "No match".
    pub fn sentence(&self) -> String {
        match self {
            Completion::NoMatch => "No match".to_owned(),
            Completion::Completed(text) => format!("completed to {text}"),
            Completion::Extended { prefix, count } => {
                format!("{count} matches start {prefix}, Down arrow lists them")
            }
            Completion::Several(count) => format!("{count} matches, Down arrow lists them"),
        }
    }
}

/// A text field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryField {
    label: String,
    text: String,
    cursor: usize,
    source: CompletionSource,
    history_key: Option<String>,
    history_at: Option<usize>,
    /// Where the word being completed starts, in characters, when the
    /// source says (the filter language's completion).
    stem_start: Option<usize>,
}

fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

fn is_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

impl EntryField {
    /// An empty field with a label and a completion source.
    pub fn new(label: impl Into<String>, source: CompletionSource) -> Self {
        Self {
            label: label.into(),
            text: String::new(),
            cursor: 0,
            source,
            history_key: None,
            history_at: None,
            stem_start: None,
        }
    }

    /// The same field with text in it and the cursor at its end.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.set_text(text);
        self
    }

    /// The same field with a history, kept under this key in the state
    /// folder: "go-path", "rename", "run-command".
    pub fn with_history(mut self, key: impl Into<String>) -> Self {
        self.history_key = Some(key.into());
        self
    }

    /// The label, said first.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor, in characters from the start.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Where candidates come from.
    pub fn source(&self) -> &CompletionSource {
        &self.source
    }

    /// The history key, or `None` when the field keeps no history.
    pub fn history_key(&self) -> Option<&str> {
        self.history_key.as_deref()
    }

    /// The field as said when it opens: "Go to a path: D:\Docs".
    pub fn spoken(&self) -> String {
        if self.text.is_empty() {
            format!("{}, empty", self.label)
        } else {
            format!("{}: {}", self.label, self.text)
        }
    }

    /// Replaces the text; the cursor goes to its end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
        self.history_at = None;
        self.stem_start = None;
    }

    /// Notes where the word being completed starts, in characters from
    /// the start of the text, for a source that knows (the filter
    /// language's completion says it); the part from there to the cursor
    /// is the stem.
    pub fn set_stem_start(&mut self, start: usize) {
        self.stem_start = Some(start.min(self.cursor));
    }

    /// Where the stem starts in a filter field: as the completion said, or
    /// after the last space or bracket before the cursor.
    fn filter_stem_start(&self) -> usize {
        if let Some(start) = self.stem_start {
            return start.min(self.cursor);
        }
        let before: Vec<char> = self.text.chars().take(self.cursor).collect();
        before
            .iter()
            .rposition(|c| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ':' | '"'))
            .map_or(0, |i| i + 1)
    }

    /// Types a character at the cursor.
    pub fn insert(&mut self, c: char) {
        let at = byte_index(&self.text, self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
        self.history_at = None;
        self.stem_start = None;
    }

    /// Types text at the cursor, such as a paste.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            self.insert(c);
        }
    }

    /// Removes the character before the cursor; returns it.
    pub fn backspace(&mut self) -> Option<char> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        let at = byte_index(&self.text, self.cursor);
        self.history_at = None;
        self.stem_start = None;
        Some(self.text.remove(at))
    }

    /// Removes the character at the cursor; returns it.
    pub fn delete(&mut self) -> Option<char> {
        if self.cursor >= self.text.chars().count() {
            return None;
        }
        let at = byte_index(&self.text, self.cursor);
        self.history_at = None;
        Some(self.text.remove(at))
    }

    /// Moves the cursor one character left; false at the start.
    pub fn left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Moves the cursor one character right; false at the end.
    pub fn right(&mut self) -> bool {
        if self.cursor >= self.text.chars().count() {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// Moves the cursor to the start.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the cursor to the end.
    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// The part completion works on: after the last separator in a path
    /// field, else the whole text.
    pub fn stem(&self) -> &str {
        match self.source {
            CompletionSource::Paths => self
                .text
                .rfind(is_separator)
                .map_or(self.text.as_str(), |i| &self.text[i + 1..]),
            CompletionSource::Filter => {
                let start = byte_index(&self.text, self.filter_stem_start());
                let end = byte_index(&self.text, self.cursor);
                &self.text[start..end]
            }
            _ => &self.text,
        }
    }

    fn replace_stem(&mut self, with: &str) {
        if self.source == CompletionSource::Filter {
            let start_chars = self.filter_stem_start();
            let start = byte_index(&self.text, start_chars);
            let end = byte_index(&self.text, self.cursor);
            self.text.replace_range(start..end, with);
            self.cursor = start_chars + with.chars().count();
            self.history_at = None;
            self.stem_start = None;
            return;
        }
        let keep = match self.source {
            CompletionSource::Paths => self.text.rfind(is_separator).map_or(0, |i| i + 1),
            _ => 0,
        };
        self.text.truncate(keep);
        self.text.push_str(with);
        self.cursor = self.text.chars().count();
        self.history_at = None;
    }

    /// The candidates that match the stem: those that start with it first,
    /// then those that contain it, each group in the order given, without
    /// regard to case.
    pub fn matches(&self, candidates: &[Candidate]) -> Vec<Candidate> {
        let stem = self.stem().to_lowercase();
        let (mut starting, mut containing): (Vec<Candidate>, Vec<Candidate>) =
            (Vec::new(), Vec::new());
        for candidate in candidates {
            let text = candidate.text.to_lowercase();
            // A bookmark is found by its name too (W3a).
            let named = self.source == CompletionSource::Bookmarks
                && !stem.is_empty()
                && candidate.kind.to_lowercase().contains(&stem);
            if text.starts_with(&stem) {
                starting.push(candidate.clone());
            } else if !stem.is_empty() && text.contains(&stem) || named {
                containing.push(candidate.clone());
            }
        }
        starting.extend(containing);
        starting
    }

    /// Tab: inserts the single match, or the longest common start of the
    /// matches that start with the stem, and says what it did.
    pub fn complete(&mut self, candidates: &[Candidate]) -> Completion {
        let found = self.matches(candidates);
        match found.as_slice() {
            [] => Completion::NoMatch,
            [one] => {
                self.replace_stem(&one.text);
                Completion::Completed(one.text.clone())
            }
            many => {
                let stem = self.stem().to_lowercase();
                let starting: Vec<&Candidate> = many
                    .iter()
                    .filter(|c| c.text.to_lowercase().starts_with(&stem))
                    .collect();
                let common = common_start(&starting);
                if common.chars().count() > stem.chars().count() {
                    self.replace_stem(&common);
                    Completion::Extended {
                        prefix: common,
                        count: many.len(),
                    }
                } else {
                    Completion::Several(many.len())
                }
            }
        }
    }

    /// Takes a candidate chosen from the list: its text replaces the stem.
    pub fn accept(&mut self, candidate: &Candidate) {
        self.replace_stem(&candidate.text);
    }

    /// Up with the list closed: the previous entry of the prompt's history
    /// (newest first), or `false` at the oldest.
    pub fn history_back(&mut self, history: &[String]) -> bool {
        let next = self.history_at.map_or(0, |i| i + 1);
        match history.get(next) {
            Some(entry) => {
                self.text = entry.clone();
                self.cursor = self.text.chars().count();
                self.history_at = Some(next);
                true
            }
            None => false,
        }
    }

    /// Down with the list closed: the next newer entry of the history, or
    /// an empty field after the newest.
    pub fn history_forward(&mut self, history: &[String]) -> bool {
        match self.history_at {
            None => false,
            Some(0) => {
                self.text.clear();
                self.cursor = 0;
                self.history_at = None;
                true
            }
            Some(i) => {
                self.text = history.get(i - 1).cloned().unwrap_or_default();
                self.cursor = self.text.chars().count();
                self.history_at = Some(i - 1);
                true
            }
        }
    }
}

/// The longest start every candidate shares, without regard to case, in
/// the first candidate's own letters.
fn common_start(candidates: &[&Candidate]) -> String {
    let Some(first) = candidates.first() else {
        return String::new();
    };
    let mut length = first.text.chars().count();
    for other in &candidates[1..] {
        let shared = first
            .text
            .chars()
            .zip(other.text.chars())
            .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            .count();
        length = length.min(shared);
    }
    first.text.chars().take(length).collect()
}

/// The command-name source: every command the palette reaches, by name,
/// its key after it ("Copy to other panel", kind "command, F5").
pub fn command_candidates(keymap: &Keymap) -> Vec<Candidate> {
    CATALOG
        .iter()
        .filter(|info| info.layer != Layer::Dialog)
        .map(|info| {
            let key = keymap.key_text(&info.id);
            let kind = if key.is_empty() {
                "command".to_owned()
            } else {
                format!("command, {key}")
            };
            Candidate::new(info.name, kind)
        })
        .collect()
}

/// A history source: the prompt's entries, newest first.
pub fn history_candidates(history: &[String]) -> Vec<Candidate> {
    history
        .iter()
        .map(|entry| Candidate::new(entry.clone(), "earlier"))
        .collect()
}

/// A fixed list's candidates, such as a setting's choices.
pub fn value_candidates(values: &[String]) -> Vec<Candidate> {
    values
        .iter()
        .map(|value| Candidate::new(value.clone(), "choice"))
        .collect()
}

/// The label source: every label by name, its color said after it as a
/// word ("Urgent, label, orange"), in the Labels list's order.
pub fn label_candidates(store: &crate::labels::LabelStore) -> Vec<Candidate> {
    store
        .labels()
        .iter()
        .map(|label| Candidate::new(label.name.clone(), format!("label, {}", label.color.word())))
        .collect()
}

/// The bookmark source: each bookmark's place, its name after it
/// ("D:\work\Reports, bookmark Reports"), sticky ones first.
pub fn bookmark_candidates(bookmarks: &crate::bookmarks::Bookmarks) -> Vec<Candidate> {
    bookmarks
        .ordered()
        .into_iter()
        .map(|b| Candidate::new(b.place.to_string(), format!("bookmark {}", b.name)))
        .collect()
}

/// The candidate offered for a `~` or a `$NAME` typed in a path field: the
/// literal path it stands for, so the field never holds text that is
/// expanded later. `home` and `variable` are the caller's lookups.
pub fn expansion_candidate(
    stem: &str,
    home: Option<&str>,
    variable: impl Fn(&str) -> Option<String>,
) -> Option<Candidate> {
    if stem == "~" {
        return home.map(|h| Candidate::new(h, "home folder"));
    }
    let name = stem
        .strip_prefix('$')
        .or_else(|| stem.strip_prefix('%').and_then(|s| s.strip_suffix('%')))?;
    if name.is_empty() {
        return None;
    }
    variable(name).map(|value| Candidate::new(value, format!("the value of {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(found: &[Candidate]) -> Vec<&str> {
        found.iter().map(|c| c.text.as_str()).collect()
    }

    fn folder(name: &str) -> Candidate {
        Candidate::new(name, "folder")
    }

    #[test]
    fn typing_and_moving_work_by_characters() {
        let mut field = EntryField::new("Rename", CompletionSource::None).with_text("rapport.pdf");
        assert_eq!(field.cursor(), 11);
        field.home();
        assert!(!field.left());
        field.delete();
        field.delete();
        field.insert_str("ré");
        assert_eq!(field.text(), "répport.pdf");
        assert!(field.right());
        assert_eq!(field.backspace(), Some('p'));
        assert_eq!(field.text(), "réport.pdf");
        field.end();
        assert!(!field.right());
        assert_eq!(field.spoken(), "Rename: réport.pdf");
        field.set_text("");
        assert_eq!(field.spoken(), "Rename, empty");
    }

    #[test]
    fn tab_completes_one_match_or_the_common_start_and_says_so() {
        let here = [folder("Downloads"), folder("Documents"), folder("Desktop")];
        let mut field = EntryField::new("Go to a path", CompletionSource::Paths)
            .with_text("D:\\Users\\ada\\dow");
        let done = field.complete(&here);
        assert_eq!(done, Completion::Completed("Downloads".into()));
        assert_eq!(done.sentence(), "completed to Downloads");
        assert_eq!(field.text(), "D:\\Users\\ada\\Downloads");
        let mut field =
            EntryField::new("Go to a path", CompletionSource::Paths).with_text("/home/ada/d");
        assert_eq!(field.complete(&here), Completion::Several(3));
        assert_eq!(
            Completion::Several(3).sentence(),
            "3 matches, Down arrow lists them"
        );
        let more = [
            folder("Downloads"),
            folder("Download-old"),
            folder("Documents"),
        ];
        field.insert_str("ow");
        let done = field.complete(&more);
        assert_eq!(
            done,
            Completion::Extended {
                prefix: "Download".into(),
                count: 2
            }
        );
        assert_eq!(field.text(), "/home/ada/Download");
        assert_eq!(
            done.sentence(),
            "2 matches start Download, Down arrow lists them"
        );
        field.accept(&more[1]);
        assert_eq!(field.text(), "/home/ada/Download-old");
        let mut none = EntryField::new("Go to a path", CompletionSource::Paths).with_text("zz");
        assert_eq!(none.complete(&here).sentence(), "No match");
    }

    #[test]
    fn a_filter_field_completes_the_word_before_the_cursor() {
        let mut field = EntryField::new("Filter", CompletionSource::Filter).with_text("size ov");
        assert_eq!(field.stem(), "ov");
        field.set_stem_start(5);
        let done = field.complete(&[Candidate::new("over", "comparison")]);
        assert_eq!(done, Completion::Completed("over".into()));
        assert_eq!(field.text(), "size over");
        assert_eq!(field.cursor(), 9);
        let mut field =
            EntryField::new("Filter", CompletionSource::Filter).with_text("ext pdf and mo");
        field.home();
        for _ in 0..3 {
            field.right();
        }
        assert_eq!(field.stem(), "ext");
        field.end();
        assert_eq!(field.stem(), "mo");
        field.accept(&Candidate::new("modified", "field"));
        assert_eq!(field.text(), "ext pdf and modified");
    }

    #[test]
    fn prefix_matches_come_before_substring_matches() {
        let field = EntryField::new("Run", CompletionSource::Commands).with_text("sort");
        let keymap = Keymap::defaults();
        let found = field.matches(&command_candidates(&keymap));
        assert_eq!(found[0].text, "Sort by name");
        assert_eq!(found[0].spoken(), "Sort by name, command, Ctrl+F3");
        let field = EntryField::new("Run", CompletionSource::Commands).with_text("list");
        let found = field.matches(&command_candidates(&keymap));
        assert_eq!(names(&found[..1]), ["Sort list"]);
    }

    #[test]
    fn history_goes_back_and_forward() {
        let history = vec!["D:\\new".to_owned(), "D:\\old".to_owned()];
        let mut field =
            EntryField::new("Go to a path", CompletionSource::Paths).with_history("go-path");
        assert_eq!(field.history_key(), Some("go-path"));
        assert!(field.history_back(&history));
        assert_eq!(field.text(), "D:\\new");
        assert!(field.history_back(&history));
        assert_eq!(field.text(), "D:\\old");
        assert!(!field.history_back(&history));
        assert!(field.history_forward(&history));
        assert_eq!(field.text(), "D:\\new");
        assert!(field.history_forward(&history));
        assert_eq!(field.text(), "");
        assert!(!field.history_forward(&history));
        assert_eq!(history_candidates(&history)[0].spoken(), "D:\\new, earlier");
    }

    #[test]
    fn a_tilde_or_a_variable_completes_to_the_literal_path() {
        let lookup = |name: &str| (name == "TEMP").then(|| "D:\\diroxide\\tmp".to_owned());
        assert_eq!(
            expansion_candidate("~", Some("/home/ada"), lookup),
            Some(Candidate::new("/home/ada", "home folder"))
        );
        assert_eq!(
            expansion_candidate("$TEMP", None, lookup).unwrap().spoken(),
            "D:\\diroxide\\tmp, the value of TEMP"
        );
        assert_eq!(
            expansion_candidate("%TEMP%", None, lookup).unwrap().text,
            "D:\\diroxide\\tmp"
        );
        assert_eq!(expansion_candidate("$NOPE", None, lookup), None);
        assert_eq!(expansion_candidate("docs", None, lookup), None);
        assert_eq!(value_candidates(&["stacked".to_owned()])[0].kind, "choice");
    }
}
