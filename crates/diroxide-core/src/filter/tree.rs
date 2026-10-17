//! The tree a filter reads into: tests joined by and, or and not. Trees
//! compare by meaning: the word positions they carry are always equal, so
//! a typed filter and its read-back give equal trees.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use crate::entry::EntryKind;
use crate::sort::NameCase;

use super::dates::{DateValue, Duration};

/// Where a test was typed: its word number, counted from 1, and its bytes
/// in the text. Two spots are always equal, so trees compare by meaning.
#[derive(Clone, Debug, Default)]
pub struct Spot {
    word: usize,
    span: Range<usize>,
}

impl Spot {
    pub(crate) fn new(word: usize, span: Range<usize>) -> Self {
        Self { word, span }
    }

    /// The word number, counted from 1.
    pub fn word(&self) -> usize {
        self.word
    }

    /// The bytes of the text the test was read from.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }
}

impl PartialEq for Spot {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Spot {}

impl Hash for Spot {
    fn hash<H: Hasher>(&self, _: &mut H) {}
}

/// A filter's tree.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Expr {
    /// One test, and where it was typed.
    Test {
        /// What is tested.
        test: Test,
        /// Where it was typed.
        at: Spot,
    },
    /// The test or group after `not`.
    Not(Box<Expr>),
    /// Every part must hold.
    And(Vec<Expr>),
    /// One part must hold.
    Or(Vec<Expr>),
}

impl Expr {
    /// Every part, with nested ands spliced in and a single part kept
    /// alone, so `(a and b) and c` and `a and b and c` are one tree.
    pub(crate) fn and(parts: Vec<Expr>) -> Expr {
        Self::join(parts, true)
    }

    /// One of the parts, with nested ors spliced in.
    pub(crate) fn or(parts: Vec<Expr>) -> Expr {
        Self::join(parts, false)
    }

    fn join(parts: Vec<Expr>, and: bool) -> Expr {
        let mut flat = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                Expr::And(inner) if and => flat.extend(inner),
                Expr::Or(inner) if !and => flat.extend(inner),
                other => flat.push(other),
            }
        }
        if flat.len() == 1 {
            flat.remove(0)
        } else if and {
            Expr::And(flat)
        } else {
            Expr::Or(flat)
        }
    }

    /// Every test in the tree, depth first, in the tree's order.
    pub fn tests(&self) -> Vec<(&Test, &Spot)> {
        let mut found = Vec::new();
        self.collect(&mut found);
        found
    }

    fn collect<'a>(&'a self, found: &mut Vec<(&'a Test, &'a Spot)>) {
        match self {
            Expr::Test { test, at } => found.push((test, at)),
            Expr::Not(inner) => inner.collect(found),
            Expr::And(parts) | Expr::Or(parts) => {
                for part in parts {
                    part.collect(found);
                }
            }
        }
    }
}

/// What a test looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Field {
    /// The entry's name.
    Name,
    /// The whole path.
    Path,
    /// The extension, without the dot.
    Extension,
    /// Folder, file, link, archive, special or item.
    Kind,
    /// The detected type's short name, such as PDF.
    Type,
    /// A file-type group, such as images.
    Group,
    /// A label on the entry.
    Label,
    /// The owner's user name.
    Owner,
    /// The Unix group (`group` always means the file-type group).
    UserGroup,
    /// The size in bytes.
    Size,
    /// Folders below the start: direct children are 1.
    Depth,
    /// When the contents last changed.
    Modified,
    /// When the entry was created.
    Created,
    /// When the entry was last read.
    Accessed,
    /// When the entry's status last changed.
    Changed,
    /// The text inside a file.
    Content,
}

/// The kind of value a field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldShape {
    /// Words and patterns: name, path, extension, type, group, label,
    /// owner and user group.
    Text,
    /// One of the entry kinds.
    Kind,
    /// A size with a unit.
    Size,
    /// A plain number.
    Count,
    /// A date, a date word or a duration.
    Date,
    /// Text or a pattern searched inside files.
    Content,
}

impl Field {
    /// Every field, in the order completion offers them.
    pub const ALL: [Field; 16] = [
        Field::Name,
        Field::Path,
        Field::Extension,
        Field::Kind,
        Field::Type,
        Field::Group,
        Field::Label,
        Field::Owner,
        Field::UserGroup,
        Field::Size,
        Field::Depth,
        Field::Modified,
        Field::Created,
        Field::Accessed,
        Field::Changed,
        Field::Content,
    ];

    /// The field's word, as the read-back says it.
    pub const fn word(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Path => "path",
            Field::Extension => "extension",
            Field::Kind => "kind",
            Field::Type => "type",
            Field::Group => "group",
            Field::Label => "label",
            Field::Owner => "owner",
            Field::UserGroup => "usergroup",
            Field::Size => "size",
            Field::Depth => "depth",
            Field::Modified => "modified",
            Field::Created => "created",
            Field::Accessed => "accessed",
            Field::Changed => "changed",
            Field::Content => "content",
        }
    }

    /// The field a word names, ignoring case: its own word, or `ext`,
    /// `timemod` or `timeaccess` (Worker's long names).
    pub fn from_word(word: &str) -> Option<Field> {
        let lower = word.to_ascii_lowercase();
        match lower.as_str() {
            "ext" => Some(Field::Extension),
            "timemod" => Some(Field::Modified),
            "timeaccess" => Some(Field::Accessed),
            _ => Self::ALL.into_iter().find(|f| f.word() == lower),
        }
    }

    /// The kind of value the field takes.
    pub const fn shape(self) -> FieldShape {
        match self {
            Field::Name
            | Field::Path
            | Field::Extension
            | Field::Type
            | Field::Group
            | Field::Label
            | Field::Owner
            | Field::UserGroup => FieldShape::Text,
            Field::Kind => FieldShape::Kind,
            Field::Size => FieldShape::Size,
            Field::Depth => FieldShape::Count,
            Field::Modified | Field::Created | Field::Accessed | Field::Changed => FieldShape::Date,
            Field::Content => FieldShape::Content,
        }
    }

    /// Whether case can matter for this field: names, paths and contents.
    /// The other text fields always ignore case.
    pub const fn has_case(self) -> bool {
        matches!(self, Field::Name | Field::Path | Field::Content)
    }

    /// What the field needs after it, for the error line: "a number",
    /// "a date", "a word". The line "Filter problem: FIELD needs WHAT"
    /// fits 40 cells for every field.
    pub const fn needs(self) -> &'static str {
        match self {
            Field::Name | Field::Path | Field::Extension => "a word",
            Field::Kind => "a kind",
            Field::Type => "a type",
            Field::Group => "a group",
            Field::Label => "a label",
            Field::Owner => "a user",
            Field::UserGroup => "a name",
            Field::Size | Field::Depth => "a number",
            Field::Modified | Field::Created | Field::Accessed | Field::Changed => "a date",
            Field::Content => "text",
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// A yes-or-no word, used bare or after `not`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Flag {
    /// Hidden by its name or its attributes.
    Hidden,
    /// Read-only.
    ReadOnly,
    /// A file that can be run.
    Executable,
    /// A Windows system file.
    System,
    /// An empty file, or a folder with nothing in it.
    Empty,
    /// A link whose target is missing.
    Broken,
    /// A file whose contents are only online (a cloud placeholder).
    OnlineOnly,
    /// A name that needs care: hidden or reserved characters, trailing
    /// dots or spaces, a device name, or a name that is not Unicode, as a
    /// panel row says "name needs care".
    NeedsCare,
    /// Marked in the panel.
    Selected,
    /// A bookmarked folder.
    Bookmarked,
    /// With at least one label.
    Labeled,
}

impl Flag {
    /// Every yes-or-no word, in the order completion offers them.
    pub const ALL: [Flag; 11] = [
        Flag::Hidden,
        Flag::ReadOnly,
        Flag::Executable,
        Flag::System,
        Flag::Empty,
        Flag::Broken,
        Flag::OnlineOnly,
        Flag::NeedsCare,
        Flag::Selected,
        Flag::Bookmarked,
        Flag::Labeled,
    ];

    /// The word, as typed and said.
    pub const fn word(self) -> &'static str {
        match self {
            Flag::Hidden => "hidden",
            Flag::ReadOnly => "read-only",
            Flag::Executable => "executable",
            Flag::System => "system",
            Flag::Empty => "empty",
            Flag::Broken => "broken",
            Flag::OnlineOnly => "online-only",
            Flag::NeedsCare => "needs-care",
            Flag::Selected => "selected",
            Flag::Bookmarked => "bookmarked",
            Flag::Labeled => "labeled",
        }
    }

    /// The flag a word names, ignoring case.
    pub fn from_word(word: &str) -> Option<Flag> {
        Self::ALL
            .into_iter()
            .find(|flag| flag.word().eq_ignore_ascii_case(word))
    }
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// How a text field is compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextOp {
    /// The whole value is the text.
    Is,
    /// The whole value is not the text.
    IsNot,
    /// The value has the text in it.
    Contains,
    /// The value starts with the text.
    StartsWith,
    /// The value ends with the text.
    EndsWith,
    /// A glob matches the whole value.
    Matches,
    /// The text's characters appear in the value in order (Worker's `~`):
    /// `rprt` is like `report`.
    Like,
    /// A regular expression is found anywhere in the value.
    Regex,
}

impl TextOp {
    /// The comparison's words, as the read-back says them.
    pub const fn words(self) -> &'static str {
        match self {
            TextOp::Is => "is",
            TextOp::IsNot => "is not",
            TextOp::Contains => "contains",
            TextOp::StartsWith => "starts with",
            TextOp::EndsWith => "ends with",
            TextOp::Matches => "matches",
            TextOp::Like => "like",
            TextOp::Regex => "regex",
        }
    }
}

/// A test of a text field.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextTest {
    /// The field.
    pub field: Field,
    /// The comparison.
    pub op: TextOp,
    /// The text, glob or regular expression, as typed (an extension
    /// without its leading dot; a group by its own name).
    pub value: String,
    /// Whether case matters, decided when the filter was read.
    pub case: NameCase,
}

/// A test of the entry's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KindTest {
    /// True for `is`, false for `is not`.
    pub is: bool,
    /// The kind.
    pub kind: EntryKind,
}

/// The words of the kinds a filter names: "folder", "file", "link",
/// "archive", "special", "item".
pub const KIND_WORDS: [(&str, EntryKind); 6] = [
    ("folder", EntryKind::Folder),
    ("file", EntryKind::File),
    ("link", EntryKind::Link),
    ("archive", EntryKind::Archive),
    ("special", EntryKind::Special),
    ("item", EntryKind::Item),
];

/// The filter word of a kind.
pub fn kind_word(kind: EntryKind) -> &'static str {
    KIND_WORDS
        .iter()
        .find(|(_, k)| *k == kind)
        .map_or("special", |(word, _)| word)
}

/// The kind a word names, ignoring case and a plural `s`.
pub fn kind_from_word(word: &str) -> Option<EntryKind> {
    let lower = word.to_ascii_lowercase();
    let single = lower.strip_suffix('s').unwrap_or(&lower);
    KIND_WORDS
        .iter()
        .find(|(w, _)| *w == lower || *w == single)
        .map(|(_, kind)| *kind)
}

/// A unit of size. KB, MB, GB and TB are powers of 1,000; KiB, MiB, GiB
/// and TiB powers of 1,024 (plan question 71).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SizeUnit {
    /// Bytes.
    Bytes,
    /// 1,000 bytes.
    Kb,
    /// 1,000,000 bytes.
    Mb,
    /// 1,000,000,000 bytes.
    Gb,
    /// 1,000,000,000,000 bytes.
    Tb,
    /// 1,024 bytes.
    Kib,
    /// 1,048,576 bytes.
    Mib,
    /// 1,073,741,824 bytes.
    Gib,
    /// 1,099,511,627,776 bytes.
    Tib,
}

impl SizeUnit {
    /// Every unit, in the order the error line lists them.
    pub const ALL: [SizeUnit; 9] = [
        SizeUnit::Bytes,
        SizeUnit::Kb,
        SizeUnit::Mb,
        SizeUnit::Gb,
        SizeUnit::Tb,
        SizeUnit::Kib,
        SizeUnit::Mib,
        SizeUnit::Gib,
        SizeUnit::Tib,
    ];

    /// How many bytes one of the unit is.
    pub const fn bytes(self) -> u64 {
        match self {
            SizeUnit::Bytes => 1,
            SizeUnit::Kb => 1_000,
            SizeUnit::Mb => 1_000_000,
            SizeUnit::Gb => 1_000_000_000,
            SizeUnit::Tb => 1_000_000_000_000,
            SizeUnit::Kib => 1 << 10,
            SizeUnit::Mib => 1 << 20,
            SizeUnit::Gib => 1 << 30,
            SizeUnit::Tib => 1 << 40,
        }
    }

    /// The unit's symbol: "B", "KB", "KiB" and the rest.
    pub const fn symbol(self) -> &'static str {
        match self {
            SizeUnit::Bytes => "B",
            SizeUnit::Kb => "KB",
            SizeUnit::Mb => "MB",
            SizeUnit::Gb => "GB",
            SizeUnit::Tb => "TB",
            SizeUnit::Kib => "KiB",
            SizeUnit::Mib => "MiB",
            SizeUnit::Gib => "GiB",
            SizeUnit::Tib => "TiB",
        }
    }

    /// The unit as said after a number: "byte" or "bytes" for bytes, the
    /// symbol otherwise.
    pub fn said(self, one: bool) -> &'static str {
        match (self, one) {
            (SizeUnit::Bytes, true) => "byte",
            (SizeUnit::Bytes, false) => "bytes",
            _ => self.symbol(),
        }
    }

    /// The unit a word names, ignoring case: a symbol, or "byte" or
    /// "bytes".
    pub fn from_word(word: &str) -> Option<SizeUnit> {
        let lower = word.to_ascii_lowercase();
        if lower == "byte" || lower == "bytes" {
            return Some(SizeUnit::Bytes);
        }
        Self::ALL
            .into_iter()
            .find(|unit| unit.symbol().eq_ignore_ascii_case(&lower))
    }
}

/// A decimal number as typed: digits and at most one decimal point, kept
/// exactly so the read-back says what was meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Decimal {
    /// The digits without the point.
    mantissa: u64,
    /// How many of them come after the point.
    scale: u8,
}

/// The most digits a number may have.
const MAX_DIGITS: usize = 18;

impl Decimal {
    /// A whole number.
    pub const fn whole(n: u64) -> Self {
        Self {
            mantissa: n,
            scale: 0,
        }
    }

    /// Reads digits with at most one point ("1.5", "100", ".5"). Trailing
    /// zeros after the point are dropped, so 1.50 and 1.5 are the same.
    /// `None` when it is not a number or has more than 18 digits.
    pub fn parse(text: &str) -> Option<Self> {
        let (whole, fraction) = match text.split_once('.') {
            Some((w, f)) => (w, f),
            None => (text, ""),
        };
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        if !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let fraction = fraction.trim_end_matches('0');
        let whole = whole.trim_start_matches('0');
        if whole.len() + fraction.len() > MAX_DIGITS {
            return None;
        }
        let digits = format!("{whole}{fraction}");
        let mantissa = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        Some(Self {
            mantissa,
            scale: u8::try_from(fraction.len()).ok()?,
        })
    }

    /// Whether this is exactly one.
    pub fn is_one(self) -> bool {
        self.scale == 0 && self.mantissa == 1
    }

    /// The number times a whole multiplier, rounded down; `None` past the
    /// largest 64-bit number.
    pub fn times(self, multiplier: u64) -> Option<u64> {
        let product = u128::from(self.mantissa) * u128::from(multiplier);
        let value = product / 10u128.pow(u32::from(self.scale));
        u64::try_from(value).ok()
    }

    /// The whole part, when there is no fraction.
    pub fn as_whole(self) -> Option<u64> {
        (self.scale == 0).then_some(self.mantissa)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.mantissa);
        }
        let digits = format!(
            "{:0>width$}",
            self.mantissa,
            width = usize::from(self.scale) + 1
        );
        let (whole, fraction) = digits.split_at(digits.len() - usize::from(self.scale));
        write!(f, "{whole}.{fraction}")
    }
}

/// A number with its unit: a size, or a plain count for depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Amount {
    /// The number as typed.
    pub number: Decimal,
    /// The unit; `None` for a plain count.
    pub unit: Option<SizeUnit>,
}

impl Amount {
    /// The amount in bytes (or the plain count), rounded down to a whole
    /// byte; `None` when it is too large.
    pub fn value(&self) -> Option<u64> {
        self.number.times(self.unit.map_or(1, SizeUnit::bytes))
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unit {
            Some(unit) => write!(f, "{} {}", self.number, unit.said(self.number.is_one())),
            None => write!(f, "{}", self.number),
        }
    }
}

/// How a size or a count is compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumberOp {
    /// Exactly.
    Is(Amount),
    /// Anything else.
    IsNot(Amount),
    /// More than.
    Over(Amount),
    /// Less than.
    Under(Amount),
    /// That much or more.
    AtLeast(Amount),
    /// That much or less.
    AtMost(Amount),
    /// From the first to the second, both included.
    Between(Amount, Amount),
}

/// A test of the size or the depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NumberTest {
    /// [`Field::Size`] or [`Field::Depth`].
    pub field: Field,
    /// The comparison.
    pub op: NumberOp,
}

/// How a date field is compared. A date stands for its whole day, month,
/// year or minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateOp {
    /// During the date (a date word alone means this).
    On(DateValue),
    /// Before the date began.
    Before(DateValue),
    /// The date or later.
    Since(DateValue),
    /// After the date ended.
    After(DateValue),
    /// The date or earlier.
    OnOrBefore(DateValue),
    /// From the first date to the second, both whole.
    Between(DateValue, DateValue),
    /// No longer ago than the duration.
    Within(Duration),
    /// Longer ago than the duration.
    OlderThan(Duration),
}

/// A test of one of the times.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateTest {
    /// [`Field::Modified`], [`Field::Created`], [`Field::Accessed`] or
    /// [`Field::Changed`].
    pub field: Field,
    /// The comparison.
    pub op: DateOp,
}

/// How a content test reads a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContentOp {
    /// The text is in the file.
    Contains,
    /// A regular expression is found in the file.
    Regex,
}

/// A test of the text inside a file. It reads files, so it is evaluated
/// last, and the panel asks before it starts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentTest {
    /// Text or a regular expression.
    pub op: ContentOp,
    /// The text or the pattern.
    pub text: String,
    /// Whether case matters.
    pub case: NameCase,
    /// Read only this much of each file (`first 64 KB`).
    pub limit: Option<Amount>,
}

/// One test.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Test {
    /// A text field.
    Text(TextTest),
    /// The kind.
    Kind(KindTest),
    /// The size or the depth.
    Number(NumberTest),
    /// A time.
    Date(DateTest),
    /// The contents.
    Content(ContentTest),
    /// A yes-or-no word.
    Flag(Flag),
}

impl Test {
    /// The field tested, or `None` for a yes-or-no word.
    pub fn field(&self) -> Option<Field> {
        match self {
            Test::Text(text) => Some(text.field),
            Test::Kind(_) => Some(Field::Kind),
            Test::Number(number) => Some(number.field),
            Test::Date(date) => Some(date.field),
            Test::Content(_) => Some(Field::Content),
            Test::Flag(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_read_exactly() {
        assert_eq!(Decimal::parse("1.50"), Decimal::parse("1.5"));
        assert_eq!(Decimal::parse("1.5").unwrap().to_string(), "1.5");
        assert_eq!(Decimal::parse("0.05").unwrap().to_string(), "0.05");
        assert_eq!(Decimal::parse(".5").unwrap().to_string(), "0.5");
        assert_eq!(Decimal::parse("007").unwrap().to_string(), "7");
        assert_eq!(Decimal::parse("0").unwrap().to_string(), "0");
        assert_eq!(Decimal::parse("1.0").unwrap(), Decimal::whole(1));
        for bad in ["", ".", "1.2.3", "1,5", "x", "-1", "1e3", &"9".repeat(19)] {
            assert_eq!(Decimal::parse(bad), None, "{bad}");
        }
        assert_eq!(
            Decimal::parse("1.5").unwrap().times(1_000_000),
            Some(1_500_000)
        );
        assert_eq!(Decimal::parse("1.5").unwrap().times(1024), Some(1536));
        assert_eq!(Decimal::parse("0.0001").unwrap().times(1000), Some(0));
        assert_eq!(Decimal::whole(u64::MAX).times(2), None);
    }

    #[test]
    fn units_are_powers_of_1000_or_1024() {
        assert_eq!(SizeUnit::Mb.bytes(), 1_000_000);
        assert_eq!(SizeUnit::Mib.bytes(), 1_048_576);
        assert_eq!(SizeUnit::Tib.bytes(), 1_099_511_627_776);
        assert_eq!(SizeUnit::from_word("mib"), Some(SizeUnit::Mib));
        assert_eq!(SizeUnit::from_word("BYTES"), Some(SizeUnit::Bytes));
        assert_eq!(SizeUnit::from_word("b"), Some(SizeUnit::Bytes));
        assert_eq!(SizeUnit::from_word("m"), None);
        let one = Amount {
            number: Decimal::whole(1),
            unit: Some(SizeUnit::Bytes),
        };
        assert_eq!(one.to_string(), "1 byte");
    }

    #[test]
    fn words_name_fields_flags_and_kinds() {
        for field in Field::ALL {
            assert_eq!(Field::from_word(field.word()), Some(field));
        }
        assert_eq!(Field::from_word("EXT"), Some(Field::Extension));
        assert_eq!(Field::from_word("timemod"), Some(Field::Modified));
        for flag in Flag::ALL {
            assert_eq!(Flag::from_word(flag.word()), Some(flag));
        }
        assert_eq!(kind_from_word("Folders"), Some(EntryKind::Folder));
        assert_eq!(kind_word(EntryKind::Archive), "archive");
        assert_eq!(kind_from_word("pipe"), None);
    }
}
