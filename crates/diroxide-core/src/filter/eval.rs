//! The evaluator: a filter's tree over an entry's facts, three-valued.
//!
//! A fact the entry does not hold (a folder's size before it is counted,
//! a type not yet checked, a file not read) is unknown, and unknown
//! matches neither the test nor its negation; a [`Tally`] says how many
//! entries were unknown and why. Facts that are not on the [`Entry`]
//! (its path, its depth, whether it is selected or bookmarked, what a
//! file holds) come from a [`Lookup`] the caller supplies.
//!
//! Tests run by cost, never by position: cheap facts first, labels and
//! types next, content last. The strict and-or rule makes the reordering
//! safe, and a content test reads only files that passed the rest.

use std::fmt;

use regex::Regex;

use crate::clock::Timestamp;
use crate::entry::{Attributes, Entry, EntryKind};
use crate::pattern::{Glob, GlobScope, PatternError, build_regex, compile_regex, folder_rule};
use crate::sort::NameCase;

use super::dates::{DateContext, DateValue};
use super::tree::{
    ContentOp, ContentTest, DateOp, DateTest, Expr, Field, Flag, KindTest, NumberOp, NumberTest,
    Test, TextOp, TextTest,
};

/// Why a test could not be decided for an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Unknown {
    /// A folder has no size until it is counted.
    SizeNotCounted,
    /// The size was not given.
    SizeNotKnown,
    /// The type has not been checked yet.
    TypeNotChecked,
    /// The time was not given.
    TimeNotKnown,
    /// The owner or group is not in this listing (Windows listings never
    /// have it).
    OwnerNotRead,
    /// The attributes were not given.
    AttributesNotRead,
    /// The lister did not give the path.
    PathNotKnown,
    /// The lister did not give the depth.
    DepthNotKnown,
    /// The lister did not say what is selected.
    SelectionNotKnown,
    /// The lister did not say what is bookmarked.
    BookmarksNotKnown,
    /// A folder's contents were not listed, so emptiness is not known.
    FolderNotListed,
    /// A link's target was not checked.
    LinkNotChecked,
    /// The file was not read.
    ContentNotRead,
    /// The file could not be read.
    Unreadable,
}

impl Unknown {
    /// The reason in words, for the summary line: "folder size not
    /// counted".
    pub const fn words(self) -> &'static str {
        match self {
            Unknown::SizeNotCounted => "folder size not counted",
            Unknown::SizeNotKnown => "size not known",
            Unknown::TypeNotChecked => "type not checked",
            Unknown::TimeNotKnown => "time not known",
            Unknown::OwnerNotRead => "owner not read",
            Unknown::AttributesNotRead => "attributes not read",
            Unknown::PathNotKnown => "path not known",
            Unknown::DepthNotKnown => "depth not known",
            Unknown::SelectionNotKnown => "selection not known",
            Unknown::BookmarksNotKnown => "bookmarks not known",
            Unknown::FolderNotListed => "folder not listed",
            Unknown::LinkNotChecked => "link not checked",
            Unknown::ContentNotRead => "content not read",
            Unknown::Unreadable => "file not readable",
        }
    }
}

impl fmt::Display for Unknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.words())
    }
}

/// A three-valued answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Truth {
    /// The entry matches.
    Yes,
    /// The entry does not match.
    No,
    /// It could not be decided, and why; the entry does not match.
    Unknown(Unknown),
}

impl Truth {
    /// Whether the entry matches.
    pub fn is_yes(self) -> bool {
        self == Truth::Yes
    }

    /// The answer for a known fact.
    pub fn of(yes: bool) -> Self {
        if yes { Truth::Yes } else { Truth::No }
    }
}

impl std::ops::Not for Truth {
    type Output = Truth;

    /// The negation: unknown stays unknown, so it matches neither a test
    /// nor its negation.
    fn not(self) -> Truth {
        match self {
            Truth::Yes => Truth::No,
            Truth::No => Truth::Yes,
            unknown => unknown,
        }
    }
}

/// The facts an [`Entry`] does not hold, from whoever lists it: the panel
/// knows the place and the selection, a search knows the depth, the
/// search lane reads contents. Every method has a default that says
/// "not known here".
pub trait Lookup {
    /// The entry's whole path, as its place writes it.
    fn path(&self, entry: &Entry) -> Option<String> {
        let _ = entry;
        None
    }

    /// Folders below the start: direct children are 1.
    fn depth(&self, entry: &Entry) -> Option<u32> {
        let _ = entry;
        None
    }

    /// Whether the entry is marked in its panel.
    fn selected(&self, entry: &Entry) -> Option<bool> {
        let _ = entry;
        None
    }

    /// Whether the entry is a bookmarked place.
    fn bookmarked(&self, entry: &Entry) -> Option<bool> {
        let _ = entry;
        None
    }

    /// Whether a folder has nothing in it.
    fn folder_empty(&self, entry: &Entry) -> Option<bool> {
        let _ = entry;
        None
    }

    /// Whether a link's target is missing. The default knows only a link
    /// whose target was checked and found.
    fn link_broken(&self, entry: &Entry) -> Option<bool> {
        entry
            .link
            .as_ref()
            .and_then(|link| link.points_to)
            .map(|_| false)
    }

    /// Whether a file holds what the query looks for. Only files reach
    /// this, and only once every cheaper test has passed.
    fn content(&self, entry: &Entry, query: &ContentQuery) -> Truth {
        let _ = (entry, query);
        Truth::Unknown(Unknown::ContentNotRead)
    }
}

/// A lookup that knows nothing beyond the entry.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLookup;

impl Lookup for NoLookup {}

/// A content test, compiled, for the [`Lookup`] that reads files.
#[derive(Clone, Debug)]
pub struct ContentQuery {
    test: ContentTest,
    regex: Regex,
    limit: Option<u64>,
}

impl ContentQuery {
    /// The test as the filter wrote it.
    pub fn test(&self) -> &ContentTest {
        &self.test
    }

    /// The pattern to search for: the text escaped, or the regular
    /// expression, with the case rule set.
    pub fn regex(&self) -> &Regex {
        &self.regex
    }

    /// Whether case is ignored.
    pub fn ignores_case(&self) -> bool {
        self.test.case == NameCase::Ignore
    }

    /// Read at most this many bytes of each file (`first 64 KB`).
    pub fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Whether text holds the pattern, within the limit's bytes (cut back
    /// to a character boundary).
    pub fn is_match(&self, text: &str) -> bool {
        let end = self.limit.map_or(text.len(), |limit| {
            let mut end = usize::try_from(limit).unwrap_or(usize::MAX).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            end
        });
        self.regex.is_match(&text[..end])
    }
}

/// A text test, compiled.
#[derive(Clone, Debug)]
struct TextMatch {
    field: Field,
    negate: bool,
    regex: Option<Regex>,
    folders_only: bool,
}

impl TextMatch {
    fn hit(&self, text: &str) -> Truth {
        let found = self.regex.as_ref().is_none_or(|r| r.is_match(text));
        Truth::of(found != self.negate)
    }
}

/// A size or depth test, as inclusive bounds.
#[derive(Clone, Copy, Debug)]
struct NumberCheck {
    field: Field,
    low: u64,
    high: u64,
    negate: bool,
}

/// A date test, as a stretch of time from `start` up to `end`.
#[derive(Clone, Copy, Debug)]
struct DateCheck {
    field: Field,
    start: Option<Timestamp>,
    end: Option<Timestamp>,
}

#[derive(Clone, Debug)]
enum Compiled {
    Text(TextMatch),
    Kind(KindTest),
    Number(NumberCheck),
    Date(DateTest),
    Range(DateCheck),
    Content(Box<ContentQuery>),
    Flag(Flag),
}

#[derive(Clone, Debug)]
enum Shape {
    Test(Compiled),
    Not(Box<Node>),
    And { parts: Vec<Node>, lazy: bool },
    Or(Vec<Node>),
}

/// A compiled part of the tree, with what it costs to decide.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    shape: Shape,
    cost: u8,
}

/// What reading a file costs, next to everything else.
const CONTENT_COST: u8 = 100;

fn cost(test: &Test) -> u8 {
    match test {
        Test::Text(text) => match text.field {
            Field::Name | Field::Extension => 1,
            Field::Path => 3,
            _ => 2,
        },
        Test::Kind(_) | Test::Date(_) => 1,
        Test::Number(number) => {
            if number.field == Field::Depth {
                3
            } else {
                1
            }
        }
        Test::Content(_) => CONTENT_COST,
        Test::Flag(flag) => match flag {
            Flag::Selected | Flag::Bookmarked | Flag::Empty | Flag::Broken => 3,
            Flag::Labeled => 2,
            _ => 1,
        },
    }
}

/// A regular expression that matches the text itself.
fn literal(source: &str, case: NameCase) -> Result<Regex, PatternError> {
    build_regex(source, case)
}

fn compile_text(test: &TextTest) -> Result<TextMatch, PatternError> {
    let (value, folders_only) = if test.field == Field::Name {
        folder_rule(&test.value)
    } else {
        (test.value.as_str(), false)
    };
    let value = if test.field == Field::Path {
        value.replace('\\', "/")
    } else {
        value.to_owned()
    };
    let escaped = regex::escape(&value);
    let case = test.case;
    let regex = match test.op {
        TextOp::Contains if value.is_empty() => None,
        TextOp::Contains => Some(literal(&format!("(?s:{escaped})"), case)?),
        TextOp::Is | TextOp::IsNot => Some(literal(&format!("^(?s:{escaped})$"), case)?),
        TextOp::StartsWith => Some(literal(&format!("^(?s:{escaped})"), case)?),
        TextOp::EndsWith => Some(literal(&format!("(?s:{escaped})$"), case)?),
        TextOp::Like => {
            let parts: Vec<String> = value
                .chars()
                .map(|c| regex::escape(c.encode_utf8(&mut [0; 4])))
                .collect();
            Some(literal(&format!("(?s:{})", parts.join(".*?")), case)?)
        }
        TextOp::Matches => {
            let scope = if test.field == Field::Path {
                GlobScope::Path
            } else {
                GlobScope::Name
            };
            Some(Glob::with_scope(&value, case, scope)?.regex().clone())
        }
        TextOp::Regex => Some(compile_regex(&value, case)?),
    };
    Ok(TextMatch {
        field: test.field,
        negate: test.op == TextOp::IsNot,
        regex,
        folders_only,
    })
}

fn compile_number(test: &NumberTest) -> NumberCheck {
    let value = |a: &super::tree::Amount| a.value().unwrap_or(u64::MAX);
    let (low, high, negate) = match &test.op {
        NumberOp::Is(a) => (value(a), value(a), false),
        NumberOp::IsNot(a) => (value(a), value(a), true),
        NumberOp::Over(a) => match value(a).checked_add(1) {
            Some(low) => (low, u64::MAX, false),
            None => (1, 0, false),
        },
        NumberOp::Under(a) => match value(a).checked_sub(1) {
            Some(high) => (0, high, false),
            None => (1, 0, false),
        },
        NumberOp::AtLeast(a) => (value(a), u64::MAX, false),
        NumberOp::AtMost(a) => (0, value(a), false),
        NumberOp::Between(a, b) => {
            let (a, b) = (value(a), value(b));
            (a.min(b), a.max(b), false)
        }
    };
    NumberCheck {
        field: test.field,
        low,
        high,
        negate,
    }
}

fn compile_content(test: &ContentTest) -> Result<ContentQuery, PatternError> {
    let regex = match test.op {
        ContentOp::Contains => literal(&format!("(?s:{})", regex::escape(&test.text)), test.case)?,
        ContentOp::Regex => compile_regex(&test.text, test.case)?,
    };
    Ok(ContentQuery {
        test: test.clone(),
        regex,
        limit: test.limit.and_then(|a| a.value()),
    })
}

/// Compiles a tree: patterns built once, and each and and or with its
/// cheap parts first. `positive` is whether the part sits under an even
/// number of nots, where an and that has met an unknown may skip reading
/// files: the answer can then only be no or unknown, and neither matches.
pub(crate) fn compile(expr: &Expr, positive: bool) -> Result<Node, PatternError> {
    Ok(match expr {
        Expr::Test { test, .. } => {
            let compiled = match test {
                Test::Text(text) => Compiled::Text(compile_text(text)?),
                Test::Kind(kind) => Compiled::Kind(*kind),
                Test::Number(number) => Compiled::Number(compile_number(number)),
                Test::Date(date) => Compiled::Date(*date),
                Test::Content(content) => Compiled::Content(Box::new(compile_content(content)?)),
                Test::Flag(flag) => Compiled::Flag(*flag),
            };
            Node {
                shape: Shape::Test(compiled),
                cost: cost(test),
            }
        }
        Expr::Not(inner) => {
            let inner = compile(inner, !positive)?;
            Node {
                cost: inner.cost,
                shape: Shape::Not(Box::new(inner)),
            }
        }
        Expr::And(parts) | Expr::Or(parts) => {
            let mut nodes = parts
                .iter()
                .map(|p| compile(p, positive))
                .collect::<Result<Vec<_>, _>>()?;
            nodes.sort_by_key(|n| n.cost);
            let cost = nodes.iter().map(|n| n.cost).max().unwrap_or(0);
            let shape = if matches!(expr, Expr::And(_)) {
                Shape::And {
                    parts: nodes,
                    lazy: positive,
                }
            } else {
                Shape::Or(nodes)
            };
            Node { shape, cost }
        }
    })
}

/// A date test as a stretch of time, read against the context.
fn resolve_date(test: &DateTest, dates: &DateContext) -> DateCheck {
    let period = |v: &DateValue| dates.period(v);
    let (start, end) = match &test.op {
        DateOp::On(v) => {
            let p = period(v);
            (Some(p.start), Some(p.end))
        }
        DateOp::Before(v) => (None, Some(period(v).start)),
        DateOp::Since(v) => (Some(period(v).start), None),
        DateOp::After(v) => (Some(period(v).end), None),
        DateOp::OnOrBefore(v) => (None, Some(period(v).end)),
        DateOp::Between(a, b) => {
            let (a, b) = (period(a), period(b));
            (Some(a.start.min(b.start)), Some(a.end.max(b.end)))
        }
        DateOp::Within(d) => (Some(dates.ago(d)), None),
        DateOp::OlderThan(d) => (None, Some(dates.ago(d))),
    };
    DateCheck {
        field: test.field,
        start,
        end,
    }
}

fn resolve(node: &Node, dates: &DateContext) -> Node {
    let shape = match &node.shape {
        Shape::Test(Compiled::Date(test)) => {
            Shape::Test(Compiled::Range(resolve_date(test, dates)))
        }
        Shape::Test(other) => Shape::Test(other.clone()),
        Shape::Not(inner) => Shape::Not(Box::new(resolve(inner, dates))),
        Shape::And { parts, lazy } => Shape::And {
            parts: parts.iter().map(|p| resolve(p, dates)).collect(),
            lazy: *lazy,
        },
        Shape::Or(parts) => Shape::Or(parts.iter().map(|p| resolve(p, dates)).collect()),
    };
    Node {
        shape,
        cost: node.cost,
    }
}

/// A filter ready to test entries, its date words and durations read
/// against the clock once. It is cheap to clone and can be shared across
/// threads.
#[derive(Clone, Debug)]
pub struct Evaluator {
    root: Node,
}

const WINDOWS_READ_ONLY: u32 = 0x1;
const WINDOWS_SYSTEM: u32 = 0x4;
const WINDOWS_ONLINE_ONLY: u32 = 0x40000 | 0x400000;
const UNIX_OWNER_WRITE: u32 = 0o200;
const UNIX_ANY_RUN: u32 = 0o111;
/// The extensions Windows runs.
const WINDOWS_RUNS: [&str; 5] = ["exe", "com", "bat", "cmd", "msi"];

impl Evaluator {
    pub(crate) fn new(plan: &Node, dates: &DateContext) -> Self {
        Self {
            root: resolve(plan, dates),
        }
    }

    /// The three-valued answer for an entry.
    pub fn test(&self, entry: &Entry, lookup: &dyn Lookup) -> Truth {
        self.eval(&self.root, entry, lookup)
    }

    /// Whether an entry matches: yes, and not unknown.
    pub fn matches(&self, entry: &Entry, lookup: &dyn Lookup) -> bool {
        self.test(entry, lookup).is_yes()
    }

    fn eval(&self, node: &Node, entry: &Entry, lookup: &dyn Lookup) -> Truth {
        match &node.shape {
            Shape::Test(test) => self.test_one(test, entry, lookup),
            Shape::Not(inner) => !self.eval(inner, entry, lookup),
            Shape::And { parts, lazy } => {
                let mut unknown = None;
                for part in parts {
                    if unknown.is_some() && *lazy && part.cost >= CONTENT_COST {
                        break;
                    }
                    match self.eval(part, entry, lookup) {
                        Truth::No => return Truth::No,
                        Truth::Unknown(why) => {
                            unknown.get_or_insert(why);
                        }
                        Truth::Yes => {}
                    }
                }
                unknown.map_or(Truth::Yes, Truth::Unknown)
            }
            Shape::Or(parts) => {
                let mut unknown = None;
                for part in parts {
                    match self.eval(part, entry, lookup) {
                        Truth::Yes => return Truth::Yes,
                        Truth::Unknown(why) => {
                            unknown.get_or_insert(why);
                        }
                        Truth::No => {}
                    }
                }
                unknown.map_or(Truth::No, Truth::Unknown)
            }
        }
    }

    fn test_one(&self, test: &Compiled, entry: &Entry, lookup: &dyn Lookup) -> Truth {
        match test {
            Compiled::Text(text) => text_truth(text, entry, lookup),
            Compiled::Kind(kind) => {
                let same = entry.kind == kind.kind
                    || (kind.kind == EntryKind::File && entry.kind == EntryKind::Archive);
                Truth::of(same == kind.is)
            }
            Compiled::Number(number) => {
                let value = match number.field {
                    Field::Size => match entry.size {
                        Some(size) => size,
                        None if entry.is_folder_like() => {
                            return Truth::Unknown(Unknown::SizeNotCounted);
                        }
                        None => return Truth::Unknown(Unknown::SizeNotKnown),
                    },
                    _ => match lookup.depth(entry) {
                        Some(depth) => u64::from(depth),
                        None => return Truth::Unknown(Unknown::DepthNotKnown),
                    },
                };
                let inside = number.low <= value && value <= number.high;
                Truth::of(inside != number.negate)
            }
            // Dates are resolved when the evaluator is made.
            Compiled::Date(_) => Truth::Unknown(Unknown::TimeNotKnown),
            Compiled::Range(range) => {
                let time = match range.field {
                    Field::Modified => entry.modified,
                    Field::Created => entry.created,
                    Field::Accessed => entry.accessed,
                    _ => entry.changed,
                };
                match time {
                    Some(t) => Truth::of(
                        range.start.is_none_or(|s| t >= s) && range.end.is_none_or(|e| t < e),
                    ),
                    None => Truth::Unknown(Unknown::TimeNotKnown),
                }
            }
            Compiled::Content(query) => match entry.kind {
                EntryKind::File | EntryKind::Archive | EntryKind::Link => {
                    lookup.content(entry, query)
                }
                EntryKind::Folder | EntryKind::Item | EntryKind::Special => Truth::No,
            },
            Compiled::Flag(flag) => flag_truth(*flag, entry, lookup),
        }
    }
}

fn text_truth(text: &TextMatch, entry: &Entry, lookup: &dyn Lookup) -> Truth {
    match text.field {
        Field::Name => {
            if text.folders_only && !entry.is_folder_like() {
                return Truth::No;
            }
            text.hit(&entry.name)
        }
        Field::Path => match lookup.path(entry) {
            Some(path) => text.hit(&crate::pattern::forward_slashes(&path)),
            None => Truth::Unknown(Unknown::PathNotKnown),
        },
        Field::Extension => text.hit(entry.extension().unwrap_or("")),
        Field::Type => match &entry.type_name {
            Some(name) => text.hit(name),
            None => Truth::Unknown(Unknown::TypeNotChecked),
        },
        Field::Group => Truth::Unknown(Unknown::TypeNotChecked),
        Field::Label => {
            let any = entry
                .labels
                .iter()
                .any(|label| text.regex.as_ref().is_none_or(|r| r.is_match(label)));
            Truth::of(any != text.negate)
        }
        Field::Owner => match &entry.owner {
            Some(owner) => text.hit(owner),
            None => Truth::Unknown(Unknown::OwnerNotRead),
        },
        Field::UserGroup => match &entry.group {
            Some(group) => text.hit(group),
            None => Truth::Unknown(Unknown::OwnerNotRead),
        },
        _ => Truth::No,
    }
}

fn known(answer: Option<bool>, why: Unknown) -> Truth {
    answer.map_or(Truth::Unknown(why), Truth::of)
}

fn flag_truth(flag: Flag, entry: &Entry, lookup: &dyn Lookup) -> Truth {
    let attributes = entry.attributes;
    let file = matches!(entry.kind, EntryKind::File | EntryKind::Archive);
    match flag {
        Flag::Hidden => Truth::of(entry.hidden),
        Flag::NeedsCare => Truth::of(entry.needs_care),
        Flag::Labeled => Truth::of(!entry.labels.is_empty()),
        Flag::ReadOnly => match attributes {
            Some(Attributes::Windows(bits)) => Truth::of(bits & WINDOWS_READ_ONLY != 0),
            Some(Attributes::Unix(mode)) => Truth::of(mode & UNIX_OWNER_WRITE == 0),
            None => Truth::Unknown(Unknown::AttributesNotRead),
        },
        Flag::System => match attributes {
            Some(Attributes::Windows(bits)) => Truth::of(bits & WINDOWS_SYSTEM != 0),
            Some(Attributes::Unix(_)) => Truth::No,
            None => Truth::Unknown(Unknown::AttributesNotRead),
        },
        Flag::OnlineOnly => match attributes {
            Some(Attributes::Windows(bits)) => Truth::of(bits & WINDOWS_ONLINE_ONLY != 0),
            Some(Attributes::Unix(_)) => Truth::No,
            None => Truth::Unknown(Unknown::AttributesNotRead),
        },
        Flag::Executable if !file => Truth::No,
        Flag::Executable => match attributes {
            Some(Attributes::Unix(mode)) => Truth::of(mode & UNIX_ANY_RUN != 0),
            Some(Attributes::Windows(_)) => {
                let extension = entry.extension().unwrap_or("");
                Truth::of(
                    WINDOWS_RUNS
                        .iter()
                        .any(|run| run.eq_ignore_ascii_case(extension)),
                )
            }
            None => Truth::Unknown(Unknown::AttributesNotRead),
        },
        Flag::Empty => match entry.kind {
            EntryKind::File | EntryKind::Archive => match entry.size {
                Some(size) => Truth::of(size == 0),
                None => Truth::Unknown(Unknown::SizeNotKnown),
            },
            EntryKind::Folder => known(lookup.folder_empty(entry), Unknown::FolderNotListed),
            _ => Truth::No,
        },
        Flag::Broken => match entry.kind {
            EntryKind::Link => known(lookup.link_broken(entry), Unknown::LinkNotChecked),
            _ => Truth::No,
        },
        Flag::Selected => known(lookup.selected(entry), Unknown::SelectionNotKnown),
        Flag::Bookmarked => known(lookup.bookmarked(entry), Unknown::BookmarksNotKnown),
    }
}

/// Counts answers over a listing, for the summary line: how many matched,
/// and how many were unknown and why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    matched: usize,
    unmatched: usize,
    unknown: Vec<(Unknown, usize)>,
}

impl Tally {
    /// Counts one answer.
    pub fn add(&mut self, truth: Truth) {
        match truth {
            Truth::Yes => self.matched += 1,
            Truth::No => self.unmatched += 1,
            Truth::Unknown(why) => match self.unknown.iter_mut().find(|(w, _)| *w == why) {
                Some((_, count)) => *count += 1,
                None => self.unknown.push((why, 1)),
            },
        }
    }

    /// How many matched.
    pub fn matched(&self) -> usize {
        self.matched
    }

    /// How many could not be decided.
    pub fn unknown(&self) -> usize {
        self.unknown.iter().map(|(_, n)| n).sum()
    }

    /// The reasons, most common first, with their counts.
    pub fn reasons(&self) -> Vec<(Unknown, usize)> {
        let mut reasons = self.unknown.clone();
        reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        reasons
    }

    /// The unknown part of the summary, meaning first: "3 unknown, folder
    /// size not counted", or with several reasons "5 unknown, 3 folder
    /// size not counted, 2 type not checked". `None` when every entry was
    /// decided.
    pub fn unknown_sentence(&self) -> Option<String> {
        let reasons = self.reasons();
        match reasons.as_slice() {
            [] => None,
            [(why, count)] => Some(format!("{count} unknown, {why}")),
            many => {
                let parts: Vec<String> = many
                    .iter()
                    .map(|(why, count)| format!("{count} {why}"))
                    .collect();
                Some(format!("{} unknown, {}", self.unknown(), parts.join(", ")))
            }
        }
    }
}
