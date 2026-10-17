//! The message grammar (R18, "The wording grammar, at 40 cells"): every
//! line a compare says, meaning first, words before signs.
//!
//! The reading order of a report is fixed: the verdict and the counts,
//! the direction, what was ignored, the facts about the files as wholes,
//! then the change list. The verdict line is at most 40 cells, the
//! owner's Braille line: "7 changes between report.txt and old.txt". The
//! names come last because they are the part that can be long, and when
//! they do not fit they are the part cut, in the middle, so a name's
//! start and its extension both stay.
//!
//! Line text in rows is shown in the viewer's safe form
//! (`diroxide_preview::doc::text::neutralize`): tabs to spaces, every
//! control, escape, format and direction character as a question mark,
//! so a hostile file can neither repaint the screen nor reorder what a
//! reader hears.

use std::ops::Range;
use std::time::Duration;

use diroxide_core::format::{BRAILLE_CELLS, grouped};
use diroxide_core::names::display_name;
use diroxide_preview::doc::text::neutralize;
use diroxide_vfs::CancelToken;

use crate::change::{Change, ChangeKind, Counts};
use crate::content::ContentComparison;
use crate::facts::Sides;
use crate::options::{CompareOptions, Ignore};
use crate::text::{Segment, SegmentKind};
use crate::words::WordDiff;

/// Lines this long or shorter are said whole in a row; a longer changed
/// line is said through its change window.
pub const WHOLE_LINE_CHARS: usize = 40;

/// The most characters of an added or removed line a row says; the
/// detail says it all.
pub const ROW_TEXT_CHARS: usize = 80;

/// The lines of context around each change in the unified view: two (R18;
/// a setting from 0 to 9).
pub const DEFAULT_CONTEXT: usize = 2;

/// What a compare found, as its first line says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// The same bytes: "Same: a and b".
    Same,
    /// The same text, the bytes differing only in what was ignored or in
    /// the files' facts (an encoding, a byte-order mark): "Same text: a
    /// and b".
    SameText,
    /// One file named twice, the same place or two links to one file:
    /// "Same file: a".
    SameFile,
    /// Text that differs: "7 changes between a and b".
    Changes(usize),
    /// Binary files of one size whose bytes differ: "Binary, 1,204 bytes
    /// differ: a and b".
    BinaryBytes(u64),
    /// Binary files of different sizes: "Binary, sizes differ: a and b".
    BinarySizes,
    /// Too large to compare as text: "Too large to compare as text: a and
    /// b", and the choices follow.
    TooLarge,
    /// Not compared, after a cancel: "Not compared: a and b".
    NotCompared,
    /// A file changed while it was being compared, so no result stands:
    /// "Changed while comparing: a".
    ChangedWhileComparing,
}

impl Verdict {
    /// Whether the files differ, for exit codes and the counts sentence.
    pub const fn differs(self) -> bool {
        matches!(
            self,
            Verdict::Changes(_) | Verdict::BinaryBytes(_) | Verdict::BinarySizes
        )
    }

    /// Whether the files are the same, ignoring what was ignored.
    pub const fn same(self) -> bool {
        matches!(self, Verdict::Same | Verdict::SameText | Verdict::SameFile)
    }

    /// Whether the verdict names one file only.
    const fn one_name(self) -> bool {
        matches!(self, Verdict::SameFile | Verdict::ChangedWhileComparing)
    }

    fn prefix(self) -> String {
        match self {
            Verdict::Same => "Same: ".to_owned(),
            Verdict::SameText => "Same text: ".to_owned(),
            Verdict::SameFile => "Same file: ".to_owned(),
            Verdict::Changes(1) => "1 change between ".to_owned(),
            Verdict::Changes(n) => format!("{} changes between ", grouped(n as u64)),
            Verdict::BinaryBytes(1) => "Binary, 1 byte differs: ".to_owned(),
            Verdict::BinaryBytes(n) => format!("Binary, {} bytes differ: ", grouped(n)),
            Verdict::BinarySizes => "Binary, sizes differ: ".to_owned(),
            Verdict::TooLarge => "Too large to compare as text: ".to_owned(),
            Verdict::NotCompared => "Not compared: ".to_owned(),
            Verdict::ChangedWhileComparing => "Changed while comparing: ".to_owned(),
        }
    }
}

fn cells(text: &str) -> usize {
    text.chars().count()
}

/// A name cut to `cells` characters in the middle, "report-2...final.txt",
/// so its start and its end both stay; an odd cell goes to the end, where
/// extensions and version numbers are, as long as one character of the
/// start stays.
fn shorten(name: &str, cells_left: usize) -> String {
    let count = cells(name);
    if count <= cells_left {
        return name.to_owned();
    }
    if cells_left <= 3 {
        return name.chars().take(cells_left).collect();
    }
    let head = ((cells_left - 3) / 2).max(1);
    let tail = cells_left - 3 - head;
    let mut out: String = name.chars().take(head).collect();
    out.push_str("...");
    out.extend(name.chars().skip(count - tail));
    out
}

/// The fewest cells a cut name keeps: one character and "...". Below
/// that a name says nothing, so the line leaves the names out.
const MIN_NAME_CELLS: usize = 4;

/// The verdict line, at most 40 cells: the verdict and the counts first,
/// then the names, cut in the middle when they do not fit, so the line
/// is then exactly 40 cells. When the verdict leaves too little room for
/// each name to keep four cells, the line is the verdict alone ("Binary,
/// 1,204 bytes differ"), and [`verdict_sentence`] keeps the names.
/// `first` and `second` are said in that order; a verdict about one file
/// ([`Verdict::SameFile`], [`Verdict::ChangedWhileComparing`]) says only
/// `first`. Hidden characters in the names show as question marks.
pub fn verdict_line(verdict: Verdict, first: &str, second: &str) -> String {
    let prefix = verdict.prefix();
    let first = display_name(first);
    let second = display_name(second);
    let fixed = cells(&prefix) + if verdict.one_name() { 0 } else { 5 };
    let budget = BRAILLE_CELLS.saturating_sub(fixed);
    let needed = |names: usize| names.min(MIN_NAME_CELLS);
    let room = if verdict.one_name() {
        budget >= needed(cells(&first))
    } else {
        budget >= needed(cells(&first)) + needed(cells(&second))
    };
    if !room {
        return prefix
            .trim_end_matches([':', ' '])
            .trim_end_matches(" between")
            .to_owned();
    }
    if verdict.one_name() {
        return format!("{prefix}{}", shorten(&first, budget));
    }
    let (a, b) = (cells(&first), cells(&second));
    let (fa, fb) = if a + b <= budget {
        (a, b)
    } else {
        let half = budget / 2;
        if a <= half {
            (a, budget - a)
        } else if b <= budget - half {
            (budget - b, b)
        } else {
            (budget - half, half)
        }
    };
    format!(
        "{prefix}{} and {}",
        shorten(&first, fa),
        shorten(&second, fb)
    )
}

/// The verdict as a sentence with the names whole, for speech where the
/// 40-cell line had to cut them.
pub fn verdict_sentence(verdict: Verdict, first: &str, second: &str) -> String {
    let prefix = verdict.prefix();
    if verdict.one_name() {
        format!("{prefix}{}", display_name(first))
    } else {
        format!(
            "{prefix}{} and {}",
            display_name(first),
            display_name(second)
        )
    }
}

/// The counts after the verdict: "Changed 4, added 2, removed 1.", the
/// kinds that are not zero, in that order. Empty when nothing changed.
pub fn counts_sentence(counts: &Counts) -> String {
    let parts: Vec<String> = [
        ("changed", counts.changed),
        ("added", counts.added),
        ("removed", counts.removed),
    ]
    .into_iter()
    .filter(|&(_, n)| n > 0)
    .map(|(word, n)| format!("{word} {}", grouped(n as u64)))
    .collect();
    if parts.is_empty() {
        return String::new();
    }
    let mut text = parts.join(", ");
    text.push('.');
    capitalized(&text)
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What was left out, as every report says it when anything beyond line
/// endings was: "Ignoring changes in case and blank lines." Empty when
/// only line endings were (they are said as a fact when they differ).
pub fn ignore_note(ignore: &Ignore) -> String {
    let mut parts = Vec::new();
    if ignore.all_space {
        parts.push("spaces");
    } else if ignore.space_amount {
        parts.push("the amount of space");
    } else if ignore.trailing_space {
        parts.push("trailing spaces");
    }
    if ignore.case {
        parts.push("case");
    }
    if ignore.blank_lines {
        parts.push("blank lines");
    }
    match parts.as_slice() {
        [] => String::new(),
        [one] => format!("Ignoring changes in {one}."),
        [rest @ .., last] => format!("Ignoring changes in {} and {last}.", rest.join(", ")),
    }
}

/// How long a deadline is, in words: "2 seconds", "1 second", "0.5
/// seconds".
fn seconds(deadline: Duration) -> String {
    let millis = deadline.as_millis();
    if millis == 1000 {
        "1 second".to_owned()
    } else if millis.is_multiple_of(1000) {
        format!("{} seconds", millis / 1000)
    } else {
        format!("{:.1} seconds", deadline.as_secs_f64())
    }
}

/// Said when a run reached its deadline.
pub fn stopped_early_sentence(deadline: Duration) -> String {
    format!(
        "Stopped after {}; the changes may be more than the least possible.",
        seconds(deadline)
    )
}

/// One line range as words: "line 40", "lines 40 to 44", "none".
fn range_words(range: &Range<usize>) -> String {
    match range.len() {
        0 => "none".to_owned(),
        1 => format!("line {}", grouped(range.start as u64 + 1)),
        _ => format!(
            "lines {} to {}",
            grouped(range.start as u64 + 1),
            grouped(range.end as u64)
        ),
    }
}

/// A line's text for a row: in the viewer's safe form, "blank" when it
/// holds nothing to say, and cut to `most` characters with "...".
fn row_text(line: &str, most: usize) -> String {
    let shown = neutralize(line);
    let shown = shown.trim();
    if shown.is_empty() {
        return "blank".to_owned();
    }
    if cells(shown) <= most {
        return shown.to_owned();
    }
    let mut out: String = shown.chars().take(most).collect();
    out.push_str("...");
    out
}

/// A window's text for a row, in the safe form.
fn window_text(text: &str) -> String {
    let shown = neutralize(text);
    if shown.trim().is_empty() {
        "blank".to_owned()
    } else {
        shown
    }
}

/// The word pass's differences in words: "due became owed", "today
/// added", up to three, then how many more.
fn differences_words(before: &str, after: &str, words: &WordDiff) -> String {
    let mut parts: Vec<String> = Vec::new();
    let differing: Vec<_> = words.segments().iter().filter(|s| !s.same).collect();
    for segment in differing.iter().take(3) {
        let b = neutralize(&before[segment.before.clone()]);
        let a = neutralize(&after[segment.after.clone()]);
        let (b, a) = (b.trim(), a.trim());
        parts.push(match (b.is_empty(), a.is_empty()) {
            (true, true) => "space changed".to_owned(),
            (true, false) => format!("{a} added"),
            (false, true) => format!("{b} removed"),
            (false, false) => format!("{b} became {a}"),
        });
    }
    if differing.len() > 3 {
        parts.push(format!("and {} more", differing.len() - 3));
    }
    parts.join("; ")
}

impl ContentComparison {
    /// The verdict of this comparison of contents.
    pub fn verdict(&self) -> Verdict {
        if self.identical() {
            Verdict::Same
        } else if self.text().is_same() {
            Verdict::SameText
        } else {
            Verdict::Changes(self.text().counts().changes)
        }
    }

    /// The report before the change list, one sentence a line, in the
    /// reading order: the verdict line (40 cells), the counts and the
    /// direction (when the files differ), what was ignored, the facts
    /// about the files that differ, and a note when the run reached its
    /// deadline. The names are the before and after files'; `sides` says
    /// which is named first and how the sides are called.
    pub fn summary(
        &self,
        before_name: &str,
        after_name: &str,
        sides: &Sides,
        options: &CompareOptions,
    ) -> Vec<String> {
        let verdict = self.verdict();
        let (first, second) = if sides.before_first() {
            (before_name, after_name)
        } else {
            (after_name, before_name)
        };
        let mut lines = vec![verdict_line(verdict, first, second)];
        if verdict.differs() {
            lines.push(counts_sentence(&self.text().counts()));
            lines.push(sides.direction());
        }
        let note = ignore_note(&self.text().ignore());
        if !note.is_empty() {
            lines.push(note);
        }
        match self.text().ignored_lines() {
            0 => {}
            1 => lines.push("1 line differs only in what was ignored.".to_owned()),
            n => lines.push(format!(
                "{} lines differ only in what was ignored.",
                grouped(n as u64)
            )),
        }
        lines.extend(self.fact_differences().iter().map(|f| f.sentence(sides)));
        if self.text().stopped_early() {
            lines.push(stopped_early_sentence(options.deadline));
        }
        lines
    }

    /// One change as its row in the change list, meaning first; the list
    /// model adds the position last ("2 of 7"):
    ///
    /// - "Changed line 40, was: total due, now: total owed"
    /// - "Changed line 40, trailing spaces added"
    /// - "Changed lines 40 to 41, 3 lines became 2"
    /// - "Added lines 52 to 54, 3 lines"
    /// - "Removed line 12, Copyright 2025"
    ///
    /// Empty for an index past the list.
    pub fn row(&self, index: usize, options: &CompareOptions) -> String {
        let Some(change) = self.text().changes().get(index) else {
            return String::new();
        };
        let kind = change.kind.word();
        let lines = change.lines();
        let place = if lines.len() == 1 {
            format!("line {}", grouped(lines.start as u64))
        } else {
            format!(
                "lines {} to {}",
                grouped(lines.start as u64),
                grouped(lines.end as u64 - 1)
            )
        };
        match change.kind {
            ChangeKind::Added | ChangeKind::Removed => {
                let range = if change.kind == ChangeKind::Added {
                    &change.after
                } else {
                    &change.before
                };
                if range.len() == 1 {
                    let line = if change.kind == ChangeKind::Added {
                        self.text().after_line(range.start)
                    } else {
                        self.text().before_line(range.start)
                    };
                    format!(
                        "{kind} {place}, {}",
                        row_text(line.unwrap_or_default(), ROW_TEXT_CHARS)
                    )
                } else {
                    format!("{kind} {place}, {} lines", grouped(range.len() as u64))
                }
            }
            ChangeKind::Changed => {
                format!("{kind} {place}, {}", self.changed_words(change, options))
            }
        }
    }

    /// Every row of the change list, in order.
    pub fn rows(&self, options: &CompareOptions) -> Vec<String> {
        (0..self.text().changes().len())
            .map(|index| self.row(index, options))
            .collect()
    }

    /// What a Changed row says after its line numbers.
    fn changed_words(&self, change: &Change, options: &CompareOptions) -> String {
        let (b, a) = (change.before.len(), change.after.len());
        if b == 1 && a == 1 {
            return self.one_line_words(change.before.start, change.after.start, options);
        }
        if b == a {
            // The same reason on every pair says it once.
            let first = self.pair_reason(change.before.start, change.after.start);
            if let Some(reason) = first
                && change
                    .before
                    .clone()
                    .zip(change.after.clone())
                    .all(|(x, y)| self.pair_reason(x, y).as_ref() == Some(&reason))
            {
                return reason.to_string();
            }
            return format!("{} lines", grouped(b as u64));
        }
        format!("{} lines became {}", grouped(b as u64), grouped(a as u64))
    }

    /// What a changed pair of single lines says: its reason, both lines
    /// whole when short, or the change window.
    fn one_line_words(&self, before: usize, after: usize, options: &CompareOptions) -> String {
        if let Some(reason) = self.pair_reason(before, after) {
            return reason.to_string();
        }
        let b = self.text().before_line(before).unwrap_or_default();
        let a = self.text().after_line(after).unwrap_or_default();
        if cells(b) <= WHOLE_LINE_CHARS && cells(a) <= WHOLE_LINE_CHARS {
            return format!(
                "was: {}, now: {}",
                row_text(b, WHOLE_LINE_CHARS),
                row_text(a, WHOLE_LINE_CHARS)
            );
        }
        let cancel = CancelToken::new();
        match self.pair_words(before, after, options, &cancel) {
            Ok(Some(words)) if words.too_long() => "line too long to compare by word".to_owned(),
            _ => match self.pair_window(before, after, options, &cancel) {
                Ok(Some(window)) => {
                    let mut text = format!(
                        "was: {}, now: {}",
                        window_text(&window.before),
                        window_text(&window.after)
                    );
                    match window.more {
                        0 => {}
                        1 => text.push_str(", and 1 more change in this line"),
                        n => text.push_str(&format!(", and {n} more changes in this line")),
                    }
                    text
                }
                _ => format!(
                    "was: {}, now: {}",
                    row_text(b, ROW_TEXT_CHARS),
                    row_text(a, ROW_TEXT_CHARS)
                ),
            },
        }
    }

    /// The header of a change, in the unified view and the detail:
    /// "Change 2 of 7, lines 40 to 44 before, lines 41 to 46 after".
    pub fn header(&self, index: usize) -> String {
        let changes = self.text().changes();
        let Some(change) = changes.get(index) else {
            return String::new();
        };
        format!(
            "Change {} of {}, {} before, {} after",
            grouped(index as u64 + 1),
            grouped(changes.len() as u64),
            range_words(&change.before),
            range_words(&change.after)
        )
    }

    /// One change in full, as Enter shows it: the header, then each line
    /// before and after ("Before, line 40: total due"), and what differs
    /// in each changed pair ("Differs: due became owed").
    pub fn detail(&self, index: usize, options: &CompareOptions) -> Vec<String> {
        let Some(change) = self.text().changes().get(index) else {
            return Vec::new();
        };
        let text = self.text();
        let before = |i: usize| {
            format!(
                "Before, line {}: {}",
                grouped(i as u64 + 1),
                neutralize(text.before_line(i).unwrap_or_default())
            )
        };
        let after = |i: usize| {
            format!(
                "After, line {}: {}",
                grouped(i as u64 + 1),
                neutralize(text.after_line(i).unwrap_or_default())
            )
        };
        let mut lines = vec![self.header(index)];
        if change.before.len() == change.after.len() {
            for (b, a) in change.before.clone().zip(change.after.clone()) {
                lines.push(before(b));
                lines.push(after(a));
                lines.push(format!("Differs: {}", self.pair_differs(b, a, options)));
            }
        } else {
            lines.extend(change.before.clone().map(before));
            lines.extend(change.after.clone().map(after));
        }
        lines
    }

    /// What differs in one pair of lines, for the detail.
    fn pair_differs(&self, before: usize, after: usize, options: &CompareOptions) -> String {
        if let Some(reason) = self.pair_reason(before, after) {
            return reason.to_string();
        }
        let cancel = CancelToken::new();
        match self.pair_words(before, after, options, &cancel) {
            Ok(Some(words)) if words.too_long() => "line too long to compare by word".to_owned(),
            Ok(Some(words)) => differences_words(
                self.text().before_line(before).unwrap_or_default(),
                self.text().after_line(after).unwrap_or_default(),
                &words,
            ),
            _ => "the line".to_owned(),
        }
    }

    /// The unified view as rows: each change under its header, removed
    /// lines then added lines, with `context` same lines around it, and
    /// a row saying how many same lines were left out between. Only the
    /// rows shown are built, so a large file with few changes stays cheap.
    pub fn unified(&self, context: Context) -> Vec<UnifiedRow> {
        let slots = self.slots();
        // How far each same line is from the nearest change, in same lines.
        let mut near = vec![usize::MAX; slots.len()];
        let mut since = usize::MAX;
        for (at, slot) in slots.iter().enumerate() {
            since = match slot {
                Slot::Same { .. } => since.saturating_add(1),
                Slot::Change(_) => 0,
            };
            near[at] = since;
        }
        since = usize::MAX;
        for (at, slot) in slots.iter().enumerate().rev() {
            since = match slot {
                Slot::Same { .. } => since.saturating_add(1),
                Slot::Change(_) => 0,
            };
            near[at] = near[at].min(since);
        }
        let shown = |distance: usize| match context {
            Context::WholeFile => true,
            Context::Lines(lines) => distance <= lines,
        };
        let text = self.text();
        let mut out = Vec::new();
        let mut left_out = 0usize;
        for (slot, distance) in slots.into_iter().zip(near) {
            if let Slot::Same { .. } = slot
                && !shown(distance)
            {
                left_out += 1;
                continue;
            }
            if left_out > 0 {
                out.push(UnifiedRow::gap(left_out));
                left_out = 0;
            }
            match slot {
                Slot::Same { before, after } => out.push(UnifiedRow {
                    kind: UnifiedKind::Same,
                    before_line: before.map(|b| b + 1),
                    after_line: Some(after + 1),
                    text: neutralize(text.after_line(after).unwrap_or_default()),
                    change: None,
                }),
                Slot::Change(index) => self.push_change_rows(index, &mut out),
            }
        }
        if left_out > 0 {
            out.push(UnifiedRow::gap(left_out));
        }
        out
    }

    /// The file as a sequence of same lines and changes, without text.
    fn slots(&self) -> Vec<Slot> {
        let mut slots = Vec::new();
        for Segment {
            kind,
            before,
            after,
        } in self.text().segments()
        {
            match kind {
                SegmentKind::Same => {
                    slots.extend(before.clone().zip(after.clone()).map(|(b, a)| Slot::Same {
                        before: Some(b),
                        after: a,
                    }));
                }
                // Blank lines left out of the compare read as the after
                // file has them.
                SegmentKind::Blank => {
                    slots.extend(after.clone().map(|a| Slot::Same {
                        before: None,
                        after: a,
                    }));
                }
                SegmentKind::Change(index) => slots.push(Slot::Change(*index)),
            }
        }
        slots
    }

    /// A change's rows in the unified view: its header, its removed lines,
    /// then its added lines.
    fn push_change_rows(&self, index: usize, out: &mut Vec<UnifiedRow>) {
        let text = self.text();
        let Some(change) = text.changes().get(index) else {
            return;
        };
        out.push(UnifiedRow {
            kind: UnifiedKind::Header,
            before_line: None,
            after_line: None,
            text: self.header(index),
            change: Some(index),
        });
        for b in change.before.clone() {
            out.push(UnifiedRow {
                kind: UnifiedKind::Removed,
                before_line: Some(b + 1),
                after_line: None,
                text: neutralize(text.before_line(b).unwrap_or_default()),
                change: Some(index),
            });
        }
        for a in change.after.clone() {
            out.push(UnifiedRow {
                kind: UnifiedKind::Added,
                before_line: None,
                after_line: Some(a + 1),
                text: neutralize(text.after_line(a).unwrap_or_default()),
                change: Some(index),
            });
        }
    }
}

/// One step of the unified view before its text is built: a same line,
/// by its numbers counted from 0, or a change by its index.
enum Slot {
    Same { before: Option<usize>, after: usize },
    Change(usize),
}

/// How much of the file the unified view shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    /// Each change with this many same lines around it ("show only
    /// differences", the default with [`DEFAULT_CONTEXT`]).
    Lines(usize),
    /// Every line ("show whole file").
    WholeFile,
}

impl Default for Context {
    fn default() -> Self {
        Context::Lines(DEFAULT_CONTEXT)
    }
}

/// What a row of the unified view is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnifiedKind {
    /// A change's header: "Change 2 of 7, line 40 before, line 40 after".
    Header,
    /// A line the same in both files.
    Same,
    /// A line only in the after file.
    Added,
    /// A line only in the before file.
    Removed,
    /// Same lines left out: "12 same lines left out".
    Gap,
}

/// One row of the unified view, the viewer's line-row shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnifiedRow {
    /// What the row is.
    pub kind: UnifiedKind,
    /// The before file's line number, counted from 1, where it has one.
    pub before_line: Option<usize>,
    /// The after file's line number, counted from 1, where it has one.
    pub after_line: Option<usize>,
    /// The line in the safe form, or the header's or the gap's words.
    pub text: String,
    /// The change the row belongs to, for a header, an added or a removed
    /// row.
    pub change: Option<usize>,
}

impl UnifiedRow {
    fn gap(lines: usize) -> UnifiedRow {
        UnifiedRow {
            kind: UnifiedKind::Gap,
            before_line: None,
            after_line: None,
            text: if lines == 1 {
                "1 same line left out".to_owned()
            } else {
                format!("{} same lines left out", grouped(lines as u64))
            },
            change: None,
        }
    }

    /// The row as said: "Added 52: text", "Removed 9: text", "Same 10:
    /// text" (the after file's number for a same line), or the header's
    /// or the gap's words.
    pub fn spoken(&self) -> String {
        let number = |n: Option<usize>| n.map_or_else(String::new, |n| grouped(n as u64));
        match self.kind {
            UnifiedKind::Header | UnifiedKind::Gap => self.text.clone(),
            UnifiedKind::Same => {
                format!("Same {}: {}", number(self.after_line), self.text)
            }
            UnifiedKind::Added => format!("Added {}: {}", number(self.after_line), self.text),
            UnifiedKind::Removed => {
                format!("Removed {}: {}", number(self.before_line), self.text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cut_in_the_middle() {
        assert_eq!(shorten("report.txt", 20), "report.txt");
        assert_eq!(shorten("quarterly-report-final.txt", 13), "quart...l.txt");
        assert_eq!(shorten("abcdef", 4), "a...");
        assert_eq!(shorten("abcdef", 3), "abc");
        assert_eq!(shorten("abcdef", 0), "");
    }

    #[test]
    fn the_verdict_examples_read_as_r18_wrote_them() {
        assert_eq!(
            verdict_line(Verdict::Changes(7), "report.txt", "old.txt"),
            "7 changes between report.txt and old.txt"
        );
        assert_eq!(
            verdict_line(Verdict::Changes(7), "report.txt", "old.txt")
                .chars()
                .count(),
            40
        );
        assert_eq!(
            verdict_line(Verdict::Same, "report.txt", "old.txt"),
            "Same: report.txt and old.txt"
        );
        assert_eq!(
            verdict_line(Verdict::Changes(1), "a", "b"),
            "1 change between a and b"
        );
        // R18's binary example is 43 cells: the verdict keeps its words
        // and leaves the names to the full sentence.
        assert_eq!(
            verdict_line(Verdict::BinaryBytes(1204), "a.bin", "b.bin"),
            "Binary, 1,204 bytes differ"
        );
        assert_eq!(
            verdict_sentence(Verdict::BinaryBytes(1204), "a.bin", "b.bin"),
            "Binary, 1,204 bytes differ: a.bin and b.bin"
        );
        assert_eq!(
            verdict_line(Verdict::BinaryBytes(4), "a.bin", "b.bin"),
            "Binary, 4 bytes differ: a.bin and b.bin"
        );
        assert_eq!(
            verdict_line(Verdict::SameFile, "a.txt", "a.txt"),
            "Same file: a.txt"
        );
        assert_eq!(
            verdict_line(
                Verdict::Changes(3),
                "quarterly-report-2026-final.txt",
                "quarterly-report-2026-draft.txt"
            ),
            "3 changes between qua...txt and qu...txt"
        );
    }

    #[test]
    fn counts_say_only_what_is_not_zero() {
        let counts = Counts {
            changes: 7,
            changed: 4,
            added: 2,
            removed: 1,
            ..Counts::default()
        };
        assert_eq!(counts_sentence(&counts), "Changed 4, added 2, removed 1.");
        let added = Counts {
            changes: 3,
            added: 3,
            ..Counts::default()
        };
        assert_eq!(counts_sentence(&added), "Added 3.");
        assert_eq!(counts_sentence(&Counts::default()), "");
    }

    #[test]
    fn the_ignore_note_names_what_is_on() {
        assert_eq!(ignore_note(&Ignore::default()), "");
        let spaces = Ignore {
            all_space: true,
            trailing_space: true,
            ..Ignore::default()
        };
        assert_eq!(ignore_note(&spaces), "Ignoring changes in spaces.");
        let three = Ignore {
            trailing_space: true,
            case: true,
            blank_lines: true,
            ..Ignore::default()
        };
        assert_eq!(
            ignore_note(&three),
            "Ignoring changes in trailing spaces, case and blank lines."
        );
    }

    #[test]
    fn deadlines_are_said_in_seconds() {
        assert_eq!(seconds(Duration::from_secs(2)), "2 seconds");
        assert_eq!(seconds(Duration::from_secs(1)), "1 second");
        assert_eq!(seconds(Duration::from_millis(500)), "0.5 seconds");
    }
}
