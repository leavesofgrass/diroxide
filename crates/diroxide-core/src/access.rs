//! Permissions, attributes, times and links as a job request carries them
//! (research/permissions.md 9.5; W2c's request 3, P2b): what a permissions,
//! attributes or times job changes and where ([`AttributeEdit`], with its
//! [`Recursion`]), and how a link job makes its links ([`LinkEdit`], with
//! its [`LinkKind`] and [`LinkForm`]).
//!
//! They travel in a [`JobRequest`](crate::JobRequest), so a script or the
//! journal holds the whole job, and they have a text form for the command
//! text form: [`AttributeEdit::add_to`] and [`LinkEdit::add_to`] write
//! command arguments, and [`AttributeEdit::from_command`] and
//! [`LinkEdit::from_command`] read them back, so `Display` and
//! [`Command::parse_text`] round-trip them:
//!
//! `file.permissions recursion=files_and_folders mode="+44-2"`
//!
//! The job engine (`diroxide-ops`) plans from them and re-exports them
//! where it defined them first.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::clock::{Timestamp, UtcOffset};
use crate::command::{ArgValue, Command, CommandTextError};
use crate::entry::EntryKind;

/// The Windows read-only attribute.
pub const READ_ONLY: u32 = 0x1;
/// The Windows hidden attribute.
pub const HIDDEN: u32 = 0x2;
/// The Windows system attribute.
pub const SYSTEM: u32 = 0x4;
/// The Windows archive attribute, "marked for backup".
pub const ARCHIVE: u32 = 0x20;

/// The setuid bit, never given to files inside a folder in one change.
const SETUID: u32 = 0o4000;

/// Text that is not a value of the kind wanted; the error says what to
/// write instead, after "Not a value for NAME: TEXT." in
/// [`CommandTextError::BadValue`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("Write {expected}.")]
pub struct ValueTextError {
    /// What to write instead, in words with an example: "this_item,
    /// files_and_folders, files_only or folders_only".
    pub expected: &'static str,
}

impl ValueTextError {
    const fn new(expected: &'static str) -> Self {
        Self { expected }
    }
}

/// Which entries a change applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recursion {
    /// The selected entries only.
    #[default]
    ThisItem,
    /// The selected entries and everything inside them.
    FilesAndFolders,
    /// The selected entries and the files inside them.
    FilesOnly,
    /// The selected entries and the folders inside them.
    FoldersOnly,
}

impl Recursion {
    /// Every choice, in the dialog's order.
    pub const ALL: [Recursion; 4] = [
        Recursion::ThisItem,
        Recursion::FilesAndFolders,
        Recursion::FilesOnly,
        Recursion::FoldersOnly,
    ];

    const EXPECTED: &'static str = "this_item, files_and_folders, files_only or folders_only";

    /// The choice in words: "this item only", "files and folders inside".
    pub const fn words(self) -> &'static str {
        match self {
            Recursion::ThisItem => "this item only",
            Recursion::FilesAndFolders => "files and folders inside",
            Recursion::FilesOnly => "only files inside",
            Recursion::FoldersOnly => "only folders inside",
        }
    }

    /// The plan's first row: "Apply to: files and folders inside".
    pub fn row(self) -> String {
        format!("Apply to: {}", self.words())
    }

    /// Whether the change goes inside folders.
    pub const fn goes_inside(self) -> bool {
        !matches!(self, Recursion::ThisItem)
    }

    /// Whether an entry of this kind inside a selected folder is changed.
    pub fn takes(self, kind: EntryKind) -> bool {
        match self {
            Recursion::ThisItem => false,
            Recursion::FilesAndFolders => true,
            Recursion::FilesOnly => kind != EntryKind::Folder,
            Recursion::FoldersOnly => kind == EntryKind::Folder,
        }
    }

    /// The choice as written in text and TOML: `files_and_folders`.
    pub const fn name(self) -> &'static str {
        match self {
            Recursion::ThisItem => "this_item",
            Recursion::FilesAndFolders => "files_and_folders",
            Recursion::FilesOnly => "files_only",
            Recursion::FoldersOnly => "folders_only",
        }
    }
}

impl fmt::Display for Recursion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Recursion {
    type Err = ValueTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.name() == text)
            .ok_or(ValueTextError::new(Self::EXPECTED))
    }
}

/// A change of a Unix mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeChange {
    /// Turn these bits on and these off, the rest as they are: the
    /// dialog's rows.
    Bits {
        /// Bits turned on.
        on: u32,
        /// Bits turned off.
        off: u32,
    },
    /// Set exactly, files and folders each their own mode: a preset such
    /// as "Owner edits, everyone reads" (644 and 755).
    Exact {
        /// The mode of files.
        files: u32,
        /// The mode of folders.
        folders: u32,
    },
}

impl ModeChange {
    const EXPECTED: &'static str =
        "+ and - with octal bits, such as \"+44-2\", or files/folders, such as \"644/755\"";

    /// The new mode of an entry whose mode is `current`. `inside` is true
    /// for an entry inside a selected folder, where the X convention
    /// holds: a folder whose class gains read gains enter, a file gains
    /// run only where someone could already run it, and no file gains
    /// setuid.
    pub fn apply(self, current: u32, kind: EntryKind, inside: bool) -> u32 {
        let folder = kind == EntryKind::Folder;
        match self {
            ModeChange::Exact { files, folders } => (if folder { folders } else { files }) & 0o7777,
            ModeChange::Bits { on, off } => {
                let mut on = on & 0o7777;
                if inside {
                    if folder {
                        for shift in [6, 3, 0] {
                            if on & (0o4 << shift) != 0 {
                                on |= 0o1 << shift;
                            }
                        }
                    } else {
                        if current & 0o111 == 0 {
                            on &= !0o111;
                        }
                        on &= !SETUID;
                    }
                }
                ((current & !off) | on) & 0o7777
            }
        }
    }
}

/// The text form: `+44-2` turns group and others' read on and others'
/// write off (octal, the sign first, either part left out when empty, `+0`
/// for no change); `644/755` sets files and folders exactly.
impl fmt::Display for ModeChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ModeChange::Bits { on, off } => {
                if on != 0 || off == 0 {
                    write!(f, "+{on:o}")?;
                }
                if off != 0 {
                    write!(f, "-{off:o}")?;
                }
                Ok(())
            }
            ModeChange::Exact { files, folders } => write!(f, "{files:o}/{folders:o}"),
        }
    }
}

/// Octal digits of a mode, at most 0o7777.
fn octal(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    u32::from_str_radix(text, 8).ok()
}

impl FromStr for ModeChange {
    type Err = ValueTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = ValueTextError::new(Self::EXPECTED);
        if let Some((files, folders)) = text.split_once('/') {
            return match (octal(files), octal(folders)) {
                (Some(files), Some(folders)) => Ok(ModeChange::Exact { files, folders }),
                _ => Err(bad),
            };
        }
        let (mut on, mut off) = (0, 0);
        let mut rest = text;
        if rest.is_empty() {
            return Err(bad);
        }
        while let Some(sign) = rest.chars().next() {
            let body = &rest[sign.len_utf8()..];
            let end = body.find(['+', '-']).unwrap_or(body.len());
            let bits = octal(&body[..end]).ok_or_else(|| bad.clone())?;
            match sign {
                '+' => on |= bits,
                '-' => off |= bits,
                _ => return Err(bad),
            }
            rest = &body[end..];
        }
        Ok(ModeChange::Bits { on, off })
    }
}

/// Windows attributes turned on and off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowsChange {
    /// Attributes turned on.
    pub on: u32,
    /// Attributes turned off.
    pub off: u32,
}

/// The four attributes with letters in the text form, as `attrib` writes
/// them, and their words.
const WINDOWS_LETTERS: [(u32, char, &str); 4] = [
    (READ_ONLY, 'r', "read-only"),
    (HIDDEN, 'h', "hidden"),
    (SYSTEM, 's', "system"),
    (ARCHIVE, 'a', "marked for backup"),
];

impl WindowsChange {
    const EXPECTED: &'static str = "+ and - with the letters r, h, s and a, such as \"+rh-a\"";

    /// The new attributes of an entry whose attributes are `current`.
    pub fn apply(self, current: u32) -> u32 {
        (current & !self.off) | self.on
    }

    /// The change in words, one row each: "Turns on read-only", "Turns off
    /// hidden".
    pub fn rows(self) -> Vec<String> {
        let on = WINDOWS_LETTERS
            .iter()
            .filter(|(bit, ..)| self.on & bit != 0)
            .map(|(.., word)| format!("Turns on {word}"));
        let off = WINDOWS_LETTERS
            .iter()
            .filter(|(bit, ..)| self.off & bit != 0)
            .map(|(.., word)| format!("Turns off {word}"));
        on.chain(off).collect()
    }
}

/// The bits of one group in the text form: the letters, then any other
/// bits as `x` and hexadecimal digits, which end the group.
fn windows_group(bits: u32) -> String {
    let mut text: String = WINDOWS_LETTERS
        .iter()
        .filter(|(bit, ..)| bits & bit != 0)
        .map(|(_, letter, _)| *letter)
        .collect();
    let others = bits & !WINDOWS_LETTERS.iter().fold(0, |all, (bit, ..)| all | bit);
    if others != 0 {
        text.push_str(&format!("x{others:x}"));
    }
    text
}

/// The text form: `+rh-a` turns on read-only and hidden and turns off
/// marked for backup (`attrib`'s letters r, h, s, a); other bits follow
/// their group's letters as `x` and hexadecimal digits (`+rx100`); `+` alone
/// is no change.
impl fmt::Display for WindowsChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.on != 0 || self.off == 0 {
            write!(f, "+{}", windows_group(self.on))?;
        }
        if self.off != 0 {
            write!(f, "-{}", windows_group(self.off))?;
        }
        Ok(())
    }
}

impl FromStr for WindowsChange {
    type Err = ValueTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = ValueTextError::new(Self::EXPECTED);
        let mut change = WindowsChange::default();
        let mut rest = text;
        if rest.is_empty() {
            return Err(bad);
        }
        while let Some(sign) = rest.chars().next() {
            let body = &rest[sign.len_utf8()..];
            let end = body.find(['+', '-']).unwrap_or(body.len());
            let group = &body[..end];
            let (letters, hex) = match group.split_once('x') {
                Some((letters, hex)) => (letters, Some(hex)),
                None => (group, None),
            };
            let mut bits = 0;
            for letter in letters.chars() {
                let (bit, ..) = WINDOWS_LETTERS
                    .iter()
                    .find(|(_, l, _)| *l == letter)
                    .ok_or_else(|| bad.clone())?;
                bits |= bit;
            }
            if let Some(hex) = hex {
                bits |= u32::from_str_radix(hex, 16).map_err(|_| bad.clone())?;
            }
            match sign {
                '+' => change.on |= bits,
                '-' => change.off |= bits,
                _ => return Err(bad),
            }
            rest = &body[end..];
        }
        Ok(change)
    }
}

/// What a permissions, attributes or times job changes, and where.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct AttributeEdit {
    /// Which entries: listed first in the plan.
    pub recursion: Recursion,
    /// The Unix mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ModeChange>,
    /// The Windows attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows: Option<WindowsChange>,
    /// A new modified time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<Timestamp>,
    /// A new created time, where the place can set one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
    /// A new accessed time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessed: Option<Timestamp>,
    /// The offset times are said in: the app's local offset.
    pub offset: UtcOffset,
}

impl AttributeEdit {
    /// The command arguments of an edit, in the order written.
    pub const ARGUMENTS: [&'static str; 7] = [
        "recursion",
        "mode",
        "attributes",
        "modified",
        "created",
        "accessed",
        "offset",
    ];

    /// An edit that changes nothing yet, applied as `recursion` says.
    pub fn new(recursion: Recursion) -> Self {
        Self {
            recursion,
            ..Self::default()
        }
    }

    /// The same edit changing the Unix mode.
    pub fn with_mode(mut self, mode: ModeChange) -> Self {
        self.mode = Some(mode);
        self
    }

    /// The same edit changing Windows attributes.
    pub fn with_windows(mut self, windows: WindowsChange) -> Self {
        self.windows = Some(windows);
        self
    }

    /// The same edit setting the modified time.
    pub fn with_modified(mut self, time: Timestamp) -> Self {
        self.modified = Some(time);
        self
    }

    /// The same edit setting the created time.
    pub fn with_created(mut self, time: Timestamp) -> Self {
        self.created = Some(time);
        self
    }

    /// The same edit setting the accessed time.
    pub fn with_accessed(mut self, time: Timestamp) -> Self {
        self.accessed = Some(time);
        self
    }

    /// The same edit saying times at this offset.
    pub fn at_offset(mut self, offset: UtcOffset) -> Self {
        self.offset = offset;
        self
    }

    /// Whether the edit changes nothing.
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.windows.is_none()
            && self.modified.is_none()
            && self.created.is_none()
            && self.accessed.is_none()
    }

    /// Whether it changes only times.
    pub fn times_only(&self) -> bool {
        self.mode.is_none() && self.windows.is_none()
    }

    /// The command with this edit's arguments added ([`Self::ARGUMENTS`]):
    /// `recursion` always, the others when set, `offset` when not UTC.
    /// Times are written in UTC (`2026-10-17T14:05:00Z`).
    pub fn add_to(&self, mut command: Command) -> Command {
        let mut put = |name: &str, text: String| {
            command.args.insert(name.to_owned(), ArgValue::Text(text));
        };
        put("recursion", self.recursion.to_string());
        if let Some(mode) = self.mode {
            put("mode", mode.to_string());
        }
        if let Some(windows) = self.windows {
            put("attributes", windows.to_string());
        }
        for (name, time) in [
            ("modified", self.modified),
            ("created", self.created),
            ("accessed", self.accessed),
        ] {
            if let Some(time) = time {
                put(name, timestamp_text(time));
            }
        }
        if self.offset != UtcOffset::UTC {
            put("offset", offset_text(self.offset));
        }
        command
    }

    /// The edit a command's arguments hold, or `None` when it holds none of
    /// [`Self::ARGUMENTS`]; a missing `recursion` reads as this item only.
    /// A value that is not one of its kind is refused with
    /// [`CommandTextError::BadValue`], which says what to write.
    pub fn from_command(command: &Command) -> Result<Option<Self>, CommandTextError> {
        if !Self::ARGUMENTS
            .iter()
            .any(|name| command.arg(name).is_some())
        {
            return Ok(None);
        }
        let mut edit = AttributeEdit::default();
        if let Some(recursion) = value_of(command, "recursion")? {
            edit.recursion = recursion;
        }
        edit.mode = value_of(command, "mode")?;
        edit.windows = value_of(command, "attributes")?;
        edit.modified = time_of(command, "modified")?;
        edit.created = time_of(command, "created")?;
        edit.accessed = time_of(command, "accessed")?;
        if let Some(text) = text_of(command, "offset", OFFSET_EXPECTED)? {
            edit.offset =
                parse_offset(&text).ok_or_else(|| bad_value("offset", &text, OFFSET_EXPECTED))?;
        }
        Ok(Some(edit))
    }
}

/// What kind of link to make.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A symbolic link, absolute or relative as its target is written. On
    /// Windows it may need Developer Mode, which the error says in words.
    #[default]
    Symbolic,
    /// A Windows directory junction: needs no privilege, folders only, an
    /// absolute target.
    Junction,
    /// A hard link: a second name for the same file, on one volume.
    Hard,
}

impl LinkKind {
    const EXPECTED: &'static str = "symbolic, junction or hard";

    /// The kind as written in text and TOML: `symbolic`.
    pub const fn name(self) -> &'static str {
        match self {
            LinkKind::Symbolic => "symbolic",
            LinkKind::Junction => "junction",
            LinkKind::Hard => "hard",
        }
    }
}

impl fmt::Display for LinkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for LinkKind {
    type Err = ValueTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [LinkKind::Symbolic, LinkKind::Junction, LinkKind::Hard]
            .into_iter()
            .find(|kind| kind.name() == text)
            .ok_or(ValueTextError::new(Self::EXPECTED))
    }
}

/// How a new link writes its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkForm {
    /// The full path: the link keeps working wherever it is moved, as long
    /// as the target stays.
    #[default]
    Absolute,
    /// The way from the link's folder: the link keeps working when a
    /// folder holding both moves.
    Relative,
}

impl LinkForm {
    const EXPECTED: &'static str = "absolute or relative";

    /// The form in words, for the plan's rows, and as written in text:
    /// "relative", "absolute".
    pub const fn word(self) -> &'static str {
        match self {
            LinkForm::Absolute => "absolute",
            LinkForm::Relative => "relative",
        }
    }
}

impl fmt::Display for LinkForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

impl FromStr for LinkForm {
    type Err = ValueTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [LinkForm::Absolute, LinkForm::Relative]
            .into_iter()
            .find(|form| form.word() == text)
            .ok_or(ValueTextError::new(Self::EXPECTED))
    }
}

/// How a link job makes its links: their kind and how their targets are
/// written; with `retarget`, the job changes where its one source link
/// points (to the request's destination) instead of making new links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkEdit {
    /// The kind of link: symbolic by default.
    pub kind: LinkKind,
    /// How the target is written: absolute by default.
    pub form: LinkForm,
    /// Change where the source link points instead of making links.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub retarget: bool,
}

impl LinkEdit {
    /// The command arguments of a link edit, in the order written.
    pub const ARGUMENTS: [&'static str; 3] = ["link_kind", "link_form", "retarget"];

    /// Links of this kind, written in this form.
    pub const fn new(kind: LinkKind, form: LinkForm) -> Self {
        Self {
            kind,
            form,
            retarget: false,
        }
    }

    /// Changing where a link points, its new target written in this form.
    pub const fn retarget(form: LinkForm) -> Self {
        Self {
            kind: LinkKind::Symbolic,
            form,
            retarget: true,
        }
    }

    /// The command with this edit's arguments added: `link_kind` and
    /// `link_form` always, `retarget` when set.
    pub fn add_to(&self, command: Command) -> Command {
        let command = command
            .with_arg("link_kind", ArgValue::Text(self.kind.to_string()))
            .with_arg("link_form", ArgValue::Text(self.form.to_string()));
        if self.retarget {
            command.with_arg("retarget", ArgValue::Flag(true))
        } else {
            command
        }
    }

    /// The link edit a command's arguments hold, or `None` when it holds
    /// none of [`Self::ARGUMENTS`]; a missing kind or form reads as the
    /// default.
    pub fn from_command(command: &Command) -> Result<Option<Self>, CommandTextError> {
        if !Self::ARGUMENTS
            .iter()
            .any(|name| command.arg(name).is_some())
        {
            return Ok(None);
        }
        let mut edit = LinkEdit::default();
        if let Some(kind) = value_of(command, "link_kind")? {
            edit.kind = kind;
        }
        if let Some(form) = value_of(command, "link_form")? {
            edit.form = form;
        }
        edit.retarget = match command.arg("retarget") {
            None => false,
            Some(ArgValue::Flag(flag)) => *flag,
            Some(other) => {
                return Err(bad_value("retarget", &shown(other), "true or false"));
            }
        };
        Ok(Some(edit))
    }
}

const TIME_EXPECTED: &str = "a time in UTC, such as 2026-10-17T14:05:00Z";
const OFFSET_EXPECTED: &str = "an offset from UTC, such as -07:00";

fn bad_value(name: &str, value: &str, expected: &'static str) -> CommandTextError {
    CommandTextError::BadValue {
        name: name.to_owned(),
        value: value.to_owned(),
        error: ValueTextError::new(expected),
    }
}

/// An argument value as it was written, for an error.
fn shown(value: &ArgValue) -> String {
    match value {
        ArgValue::Text(text) => text.clone(),
        ArgValue::Number(n) => n.to_string(),
        ArgValue::Flag(flag) => flag.to_string(),
        ArgValue::Place(place) => place.to_string(),
        ArgValue::Places(places) => places
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// A text argument, or `None` when it is not there. A number, a flag or
/// a place where text is wanted is refused with what to write.
fn text_of(
    command: &Command,
    name: &str,
    expected: &'static str,
) -> Result<Option<String>, CommandTextError> {
    match command.arg(name) {
        None => Ok(None),
        Some(ArgValue::Text(text)) => Ok(Some(text.clone())),
        Some(other) => Err(bad_value(name, &shown(other), expected)),
    }
}

/// A value read from a text argument by its `FromStr`.
fn value_of<T>(command: &Command, name: &str) -> Result<Option<T>, CommandTextError>
where
    T: FromStr<Err = ValueTextError>,
{
    let Some(value) = command.arg(name) else {
        return Ok(None);
    };
    let text = match value {
        ArgValue::Text(text) => text.clone(),
        other => shown(other),
    };
    text.parse::<T>()
        .map(Some)
        .map_err(|error| CommandTextError::BadValue {
            name: name.to_owned(),
            value: text,
            error,
        })
}

/// A time argument.
fn time_of(command: &Command, name: &str) -> Result<Option<Timestamp>, CommandTextError> {
    match text_of(command, name, TIME_EXPECTED)? {
        None => Ok(None),
        Some(text) => parse_timestamp(&text)
            .map(Some)
            .ok_or_else(|| bad_value(name, &text, TIME_EXPECTED)),
    }
}

/// A time as the text form writes it: UTC, to the second, with the
/// nanoseconds after a dot when there are any: `2026-10-17T14:05:00Z`.
pub fn timestamp_text(time: Timestamp) -> String {
    let civil = time.to_civil(UtcOffset::UTC);
    let nanos = time.subsec_nanos();
    let fraction = if nanos == 0 {
        String::new()
    } else {
        format!(".{nanos:09}")
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{fraction}Z",
        civil.year, civil.month, civil.day, civil.hour, civil.minute, civil.second
    )
}

/// Reads a time written by [`timestamp_text`].
pub fn parse_timestamp(text: &str) -> Option<Timestamp> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let (negative, date) = match date.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, date),
    };
    let mut date = date.split('-');
    let year: i32 = date.next()?.parse().ok()?;
    let month: u8 = date.next()?.parse().ok()?;
    let day: u8 = date.next()?.parse().ok()?;
    if date.next().is_some() {
        return None;
    }
    let year = if negative { -year } else { year };
    let (clock, nanos) = match time.split_once('.') {
        Some((clock, fraction)) => {
            if fraction.len() != 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (clock, fraction.parse::<u32>().ok()?)
        }
        None => (time, 0),
    };
    let mut clock = clock.split(':');
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second: u8 = clock.next()?.parse().ok()?;
    if clock.next().is_some() {
        return None;
    }
    let whole = Timestamp::from_civil(year, month, day, hour, minute, second, UtcOffset::UTC)?;
    Some(Timestamp::from_unix(whole.unix_seconds(), nanos))
}

/// An offset as the text form writes it: `+02:00`, `-07:00`, and the
/// seconds too when there are any (`+05:30:15`).
pub fn offset_text(offset: UtcOffset) -> String {
    let seconds = offset.seconds();
    let sign = if seconds < 0 { '-' } else { '+' };
    let all = seconds.unsigned_abs();
    let (hours, minutes, rest) = (all / 3600, (all % 3600) / 60, all % 60);
    if rest == 0 {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{rest:02}")
    }
}

/// Reads an offset written by [`offset_text`].
pub fn parse_offset(text: &str) -> Option<UtcOffset> {
    let (sign, rest) = match text.chars().next()? {
        '+' => (1, &text[1..]),
        '-' => (-1, &text[1..]),
        _ => return None,
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    let numbers: Vec<i32> = parts
        .iter()
        .map(|p| p.parse::<i32>().ok())
        .collect::<Option<_>>()?;
    let (hours, minutes) = (numbers[0], numbers[1]);
    let seconds = numbers.get(2).copied().unwrap_or(0);
    if minutes > 59 || seconds > 59 {
        return None;
    }
    let total = sign * (hours * 3600 + minutes * 60 + seconds);
    let offset = UtcOffset::from_seconds(total);
    (offset.seconds() == total).then_some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandId;

    #[test]
    fn modes_read_back_from_their_text() {
        let cases = [
            (ModeChange::Bits { on: 0o44, off: 0o2 }, "+44-2"),
            (ModeChange::Bits { on: 0, off: 0o22 }, "-22"),
            (ModeChange::Bits { on: 0o4000, off: 0 }, "+4000"),
            (ModeChange::Bits { on: 0, off: 0 }, "+0"),
            (
                ModeChange::Exact {
                    files: 0o644,
                    folders: 0o755,
                },
                "644/755",
            ),
        ];
        for (mode, text) in cases {
            assert_eq!(mode.to_string(), text);
            assert_eq!(text.parse::<ModeChange>(), Ok(mode), "{text}");
        }
        assert_eq!(
            "-2+44".parse::<ModeChange>(),
            Ok(ModeChange::Bits { on: 0o44, off: 0o2 })
        );
        for bad in ["", "644", "+9", "+", "644/", "+77777", "x"] {
            assert!(bad.parse::<ModeChange>().is_err(), "{bad}");
        }
    }

    #[test]
    fn windows_changes_read_back_from_their_text() {
        let change = WindowsChange {
            on: READ_ONLY | HIDDEN | 0x100,
            off: ARCHIVE,
        };
        assert_eq!(change.to_string(), "+rhx100-a");
        assert_eq!("+rhx100-a".parse::<WindowsChange>(), Ok(change));
        assert_eq!(WindowsChange::default().to_string(), "+");
        assert_eq!("+".parse::<WindowsChange>(), Ok(WindowsChange::default()));
        assert_eq!(
            "-s".parse::<WindowsChange>(),
            Ok(WindowsChange { on: 0, off: SYSTEM })
        );
        for bad in ["", "rh", "+q", "+xzz"] {
            assert!(bad.parse::<WindowsChange>().is_err(), "{bad}");
        }
        assert_eq!(
            change.rows(),
            vec![
                "Turns on read-only",
                "Turns on hidden",
                "Turns off marked for backup"
            ]
        );
    }

    #[test]
    fn times_and_offsets_read_back_from_their_text() {
        let time = Timestamp::from_civil(2026, 10, 17, 14, 5, 0, UtcOffset::UTC).unwrap();
        assert_eq!(timestamp_text(time), "2026-10-17T14:05:00Z");
        assert_eq!(parse_timestamp("2026-10-17T14:05:00Z"), Some(time));
        let precise = Timestamp::from_unix(time.unix_seconds(), 120);
        assert_eq!(timestamp_text(precise), "2026-10-17T14:05:00.000000120Z");
        assert_eq!(parse_timestamp(&timestamp_text(precise)), Some(precise));
        let old = Timestamp::from_unix(-86_400 * 365, 0);
        assert_eq!(parse_timestamp(&timestamp_text(old)), Some(old));
        for bad in [
            "2026-10-17",
            "2026-13-01T00:00:00Z",
            "2026-10-17T14:05Z",
            "now",
        ] {
            assert_eq!(parse_timestamp(bad), None, "{bad}");
        }
        for (seconds, text) in [(0, "+00:00"), (-7 * 3600, "-07:00"), (19_800, "+05:30")] {
            let offset = UtcOffset::from_seconds(seconds);
            assert_eq!(offset_text(offset), text);
            assert_eq!(parse_offset(text), Some(offset));
        }
        assert_eq!(parse_offset("0700"), None);
    }

    #[test]
    fn an_edit_round_trips_through_the_command_text_form() {
        let time = Timestamp::from_civil(2026, 10, 17, 14, 5, 0, UtcOffset::UTC).unwrap();
        let edit = AttributeEdit::new(Recursion::FilesOnly)
            .with_mode(ModeChange::Bits { on: 0o44, off: 0o2 })
            .with_windows(WindowsChange {
                on: HIDDEN,
                off: READ_ONLY,
            })
            .with_modified(time)
            .with_accessed(time.plus_seconds(60))
            .at_offset(UtcOffset::from_hours(-7));
        let command = edit.add_to(Command::new(CommandId::from_static("file.permissions")));
        let text = command.to_string();
        assert_eq!(
            text,
            "file.permissions accessed=2026-10-17T14:06:00Z attributes=+h-r \
             mode=+44-2 modified=2026-10-17T14:05:00Z offset=-07:00 recursion=files_only"
        );
        let read = Command::parse_text(&text).unwrap();
        assert_eq!(read, command);
        assert_eq!(AttributeEdit::from_command(&read), Ok(Some(edit)));
        // A plain edit writes its recursion alone, and no edit is none.
        let plain = AttributeEdit::new(Recursion::ThisItem)
            .add_to(Command::new(CommandId::from_static("file.permissions")));
        assert_eq!(plain.to_string(), "file.permissions recursion=this_item");
        let bare = Command::new(CommandId::from_static("file.copy"));
        assert_eq!(AttributeEdit::from_command(&bare), Ok(None));
        assert_eq!(LinkEdit::from_command(&bare), Ok(None));
    }

    #[test]
    fn a_link_edit_round_trips_through_the_command_text_form() {
        for edit in [
            LinkEdit::new(LinkKind::Junction, LinkForm::Absolute),
            LinkEdit::new(LinkKind::Symbolic, LinkForm::Relative),
            LinkEdit::retarget(LinkForm::Relative),
        ] {
            let command = edit.add_to(Command::new(CommandId::from_static("file.link")));
            let read = Command::parse_text(&command.to_string()).unwrap();
            assert_eq!(LinkEdit::from_command(&read), Ok(Some(edit)), "{command}");
        }
        assert_eq!(
            LinkEdit::retarget(LinkForm::Relative)
                .add_to(Command::new(CommandId::from_static("file.link")))
                .to_string(),
            "file.link link_form=relative link_kind=symbolic retarget=true"
        );
    }

    #[test]
    fn bad_values_say_what_to_write() {
        let read = |text: &str| Command::parse_text(text).unwrap();
        let error =
            AttributeEdit::from_command(&read("file.permissions recursion=all")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Not a value for recursion: all. Write this_item, files_and_folders, files_only or folders_only."
        );
        let error = AttributeEdit::from_command(&read("file.permissions mode=644")).unwrap_err();
        assert!(
            error.to_string().starts_with("Not a value for mode: 644."),
            "{error}"
        );
        let error =
            AttributeEdit::from_command(&read("file.permissions modified=yesterday")).unwrap_err();
        assert!(
            crate::format::within_cells(&error.to_string(), "Not a value for modified"),
            "{error}"
        );
        let error = LinkEdit::from_command(&read("file.link link_kind=soft")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Not a value for link_kind: soft. Write symbolic, junction or hard."
        );
        assert!(LinkEdit::from_command(&read("file.link retarget=yes")).is_err());
    }

    #[test]
    fn edits_travel_as_json() {
        let edit = AttributeEdit::new(Recursion::FoldersOnly).with_mode(ModeChange::Exact {
            files: 0o644,
            folders: 0o755,
        });
        let json = serde_json::to_string(&edit).unwrap();
        assert_eq!(
            json,
            r#"{"recursion":"folders_only","mode":{"exact":{"files":420,"folders":493}},"offset":0}"#
        );
        assert_eq!(serde_json::from_str::<AttributeEdit>(&json).unwrap(), edit);
        let link = LinkEdit::retarget(LinkForm::Relative);
        let json = serde_json::to_string(&link).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"symbolic","form":"relative","retarget":true}"#
        );
        assert_eq!(serde_json::from_str::<LinkEdit>(&json).unwrap(), link);
        assert_eq!(
            serde_json::from_str::<LinkEdit>("{}").unwrap(),
            LinkEdit::default()
        );
    }

    #[test]
    fn recursion_says_itself_and_what_it_takes() {
        assert_eq!(
            Recursion::FilesAndFolders.row(),
            "Apply to: files and folders inside"
        );
        assert!(Recursion::FilesOnly.takes(EntryKind::File));
        assert!(!Recursion::FilesOnly.takes(EntryKind::Folder));
        assert!(!Recursion::ThisItem.goes_inside());
        for choice in Recursion::ALL {
            assert_eq!(choice.to_string().parse::<Recursion>(), Ok(choice));
        }
    }
}
