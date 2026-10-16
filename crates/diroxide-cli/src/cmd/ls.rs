//! `diro ls`: list a place in plain lines, one entry a line, the name
//! first, then the kind, the size and the date in words.
//!
//! - `diro ls [PLACE]` lists a folder through the provider registry, so a
//!   place inside an archive lists the same way once the archive provider
//!   is registered (alpha.2). A file lists as itself.
//! - `diro ls -` lists the paths given on standard input, NUL-separated by
//!   default (`find -print0`, `fd -0`), newline-separated with `--lines`.
//!   An empty list does nothing and says so. It is the last-resort mode
//!   for a terminal where the full screen fails, and the seed of the
//!   collection mode of a later alpha.
//!
//! Everything `diro ls` has to say goes to standard output in order, its
//! problems included, so a screen reader hears the listing and its
//! problems as one story; the exit code says how it went: 0 listed, 1
//! nothing could be listed, 2 wrong use, 3 partly listed.
//!
//! Times are said in local time, from the clock at the local offset that
//! `jiff` reads (W1e's contract change request, resolved for W1d).

use std::borrow::Cow;
use std::fmt;
use std::io::{BufWriter, Read, Write};

use diroxide_core::clock::describe_time;
use diroxide_core::format::{count, human_size};
use diroxide_core::names::display_name;
use diroxide_core::sort::{SortSpec, sort_entries};
use diroxide_core::{
    Attributes, Clock, Entry, EntryKind, Explain, Message, Place, Timestamp, UtcOffset,
};
use diroxide_vfs::{OpContext, Registry, VfsError};
use serde_json::{Value, json};

use crate::CliError;
use crate::open::current_place;

/// The most `diro ls -` reads from standard input: 64 MiB, the limit nnn
/// keeps for the same kind of list.
pub const MAX_LIST_BYTES: u64 = 64 * 1024 * 1024;

/// The Windows attribute bits that mean a cloud placeholder whose data is
/// not on this computer: recall on open, and recall on data access.
const ONLINE_ONLY_BITS: u32 = 0x40000 | 0x400000;

/// Where `diro ls` takes its entries from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A place as typed, or `None` for the current folder.
    Place(Option<String>),
    /// The paths on standard input (`diro ls -`).
    Stdin,
}

/// How the paths on standard input are separated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Separator {
    /// NUL, the one byte no name can hold: the default.
    #[default]
    Nul,
    /// Line breaks (`--lines`), for names known to hold none.
    Lines,
}

/// What `diro ls` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LsArgs {
    /// Where the entries come from.
    pub source: Source,
    /// More facts on each line: exact size, times, attributes, owner,
    /// link target.
    pub long: bool,
    /// One JSON object a line instead of words.
    pub json: bool,
    /// List hidden entries too.
    pub hidden: bool,
    /// How `diro ls -` splits standard input.
    pub separator: Separator,
    /// Print the help for `diro ls` and nothing else.
    pub help: bool,
}

impl Default for LsArgs {
    fn default() -> Self {
        Self {
            source: Source::Place(None),
            long: false,
            json: false,
            hidden: false,
            separator: Separator::Nul,
            help: false,
        }
    }
}

/// The help for `diro ls`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro ls: list a place, one entry a line, the name first.",
        "",
        "Usage:",
        "diro ls [PLACE]      list a folder; the current folder by default",
        "diro ls -            list the paths on standard input, NUL-separated",
        "",
        "Options:",
        "--long               add the exact size, times, attributes and owner",
        "--json               write one JSON object a line, real names kept",
        "--hidden             list hidden entries too",
        "--lines              with -, read one path a line instead of NUL",
        "--null, -0           with -, read NUL-separated paths (the default)",
        "",
        "Exit codes:",
        "0  listed",
        "1  nothing could be listed",
        "2  wrong use: a bad option, or a list that is not NUL-separated",
        "3  partly listed: some paths could not be listed",
    ]
    .join("\n")
}

/// Reads the arguments after `ls`.
pub fn parse(args: &[String]) -> Result<LsArgs, CliError> {
    let mut parsed = LsArgs::default();
    let mut place: Option<Source> = None;
    let mut lines = false;
    let mut null = false;
    let mut only_places = false;
    for arg in args {
        let source = match arg.as_str() {
            "--" if !only_places => {
                only_places = true;
                continue;
            }
            "--long" if !only_places => {
                parsed.long = true;
                continue;
            }
            "--json" if !only_places => {
                parsed.json = true;
                continue;
            }
            "--hidden" if !only_places => {
                parsed.hidden = true;
                continue;
            }
            "--lines" if !only_places => {
                lines = true;
                continue;
            }
            "--null" | "-0" if !only_places => {
                null = true;
                continue;
            }
            "--help" | "-h" if !only_places => {
                parsed.help = true;
                continue;
            }
            "-" if !only_places => Source::Stdin,
            option if option.starts_with('-') && !only_places => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            text => Source::Place(Some(text.to_owned())),
        };
        if place.is_some() {
            return Err(CliError::TooMany("ls".to_owned()));
        }
        place = Some(source);
    }
    if let Some(source) = place {
        parsed.source = source;
    }
    if lines && null {
        return Err(CliError::Failed(
            "--lines and --null cannot be used together".to_owned(),
        ));
    }
    if (lines || null) && parsed.source != Source::Stdin {
        let flag = if lines { "--lines" } else { "--null" };
        return Err(CliError::Failed(format!(
            "{flag} only works with diro ls -, which reads paths from standard input"
        )));
    }
    if lines {
        parsed.separator = Separator::Lines;
    }
    Ok(parsed)
}

/// Runs `diro ls`, reading standard input for `diro ls -`. Returns the exit
/// code: 0 listed, 1 nothing could be listed, 2 wrong use, 3 partly listed.
pub fn run(args: &LsArgs, input: &mut dyn Read, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_failed)?;
        return Ok(0);
    }
    let clock = diroxide_app::startup::local_clock();
    let here = current_place()?;
    let registry = Registry::with_local();
    run_with(args, input, out, &Lister::new(&registry, &clock, &here))
}

/// What a listing needs from outside: the providers, the clock and the
/// current folder. Tests give their own.
pub struct Lister<'a> {
    registry: &'a Registry,
    clock: &'a dyn Clock,
    here: &'a Place,
}

impl<'a> Lister<'a> {
    /// A lister over these providers, reading time from `clock` and
    /// resolving relative paths against `here`.
    pub fn new(registry: &'a Registry, clock: &'a dyn Clock, here: &'a Place) -> Self {
        Self {
            registry,
            clock,
            here,
        }
    }
}

/// [`run`] with the providers, the clock and the current folder given.
pub fn run_with(
    args: &LsArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    lister: &Lister<'_>,
) -> Result<i32, CliError> {
    let mut out = BufWriter::new(out);
    let code = match &args.source {
        Source::Place(text) => list_place(args, text.as_deref(), &mut out, lister)?,
        Source::Stdin => list_stdin(args, input, &mut out, lister)?,
    };
    out.flush().map_err(write_failed)?;
    Ok(code)
}

fn write_failed(error: std::io::Error) -> CliError {
    CliError::Failed(format!("the listing could not be written: {error}"))
}

/// "Could not list X: CAUSE. NEXT STEP." with hidden characters in X
/// spelled as question marks.
fn could_not_list(what: &str, error: &dyn Explain) -> String {
    Message::from_error("list", &display_name(what), error).text
}

/// A provider's error as the command line says it: a place that is not
/// there asks for the path to be checked, since there is no folder on the
/// screen to reload.
struct AtTheShell<'a>(&'a VfsError);

impl fmt::Display for AtTheShell<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Explain for AtTheShell<'_> {
    fn next_step(&self) -> String {
        match self.0 {
            VfsError::NotFound => "Check the path.".to_owned(),
            other => other.next_step(),
        }
    }
}

fn list_place(
    args: &LsArgs,
    text: Option<&str>,
    out: &mut dyn Write,
    lister: &Lister<'_>,
) -> Result<i32, CliError> {
    let place = match text {
        Some(text) => Place::resolve(text, lister.here)
            .map_err(|e| CliError::Failed(could_not_list(text, &e)))?,
        None => lister.here.clone(),
    };
    let shown = place.to_string();
    let provider = lister
        .registry
        .resolve(&place)
        .map_err(|e| CliError::Failed(could_not_list(&shown, &AtTheShell(&e))))?;
    let ctx = OpContext::new();
    let mut entries = match provider.list(&place, &ctx) {
        Ok(entries) => entries,
        Err(list_error) => {
            // A file lists as itself; anything else is the listing's error.
            return match provider.stat(&place, &ctx) {
                Ok(entry) if entry.kind != EntryKind::Folder => {
                    write_entry(out, args, &entry, &place, place.parent().as_ref(), lister)?;
                    if !args.json {
                        writeln!(out, "1 item: {}", display_name(&shown)).map_err(write_failed)?;
                    }
                    Ok(0)
                }
                _ => Err(CliError::Failed(could_not_list(
                    &shown,
                    &AtTheShell(&list_error),
                ))),
            };
        }
    };
    let before = entries.len();
    if !args.hidden {
        entries.retain(|entry| !entry.hidden);
    }
    let hidden = (before - entries.len()) as u64;
    sort_entries(&mut entries, &SortSpec::default());
    for entry in &entries {
        let entry_place = place.join(&entry.name).unwrap_or_else(|_| place.clone());
        write_entry(out, args, entry, &entry_place, None, lister)?;
    }
    if !args.json {
        let name = place
            .name()
            .map_or(Cow::Borrowed(shown.as_str()), display_name);
        let mut summary = capitalized(&format!(
            "{} in {name}",
            count(entries.len() as u64, "item", "items")
        ));
        if hidden > 0 {
            summary.push_str(&format!(", {hidden} hidden not shown"));
        }
        summary.push('.');
        writeln!(out, "{summary}").map_err(write_failed)?;
    }
    Ok(0)
}

fn list_stdin(
    args: &LsArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    lister: &Lister<'_>,
) -> Result<i32, CliError> {
    let mut bytes = Vec::new();
    input
        .take(MAX_LIST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| CliError::Failed(format!("standard input could not be read: {e}")))?;
    if bytes.len() as u64 > MAX_LIST_BYTES {
        say(
            out,
            args,
            "List is too long: over 64 MB. Nothing was listed.",
        )?;
        return Ok(1);
    }
    let has_nul = bytes.contains(&0);
    let has_line_break = bytes.contains(&b'\n');
    let items: Vec<&[u8]> = match args.separator {
        Separator::Nul => {
            if !has_nul && has_line_break {
                say(out, args, "No NUL separators found. Use --lines.")?;
                return Ok(2);
            }
            bytes.split(|b| *b == 0).filter(|i| !i.is_empty()).collect()
        }
        Separator::Lines => {
            if has_nul {
                say(out, args, "The list has NUL separators. Leave out --lines.")?;
                return Ok(2);
            }
            bytes
                .split(|b| *b == b'\n')
                .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
                .filter(|line| !line.is_empty())
                .collect()
        }
    };
    if items.is_empty() {
        say(out, args, "List is empty. Nothing to list.")?;
        return Ok(0);
    }
    let mut listed = 0u64;
    let mut failed = 0u64;
    for (index, item) in items.iter().enumerate() {
        let text = match std::str::from_utf8(item) {
            Ok(text) => text,
            Err(_) => {
                failed += 1;
                let what = format!("item {}", index + 1);
                problem(
                    out,
                    args,
                    &what,
                    &could_not_list(&what, &VfsError::NameNotUnicode),
                )?;
                continue;
            }
        };
        match stat_path(text, lister) {
            Ok((place, entry)) => {
                listed += 1;
                write_entry(out, args, &entry, &place, place.parent().as_ref(), lister)?;
            }
            Err(message) => {
                failed += 1;
                problem(out, args, text, &message)?;
            }
        }
    }
    if !args.json {
        let summary = match (listed, failed) {
            (_, 0) => format!("{listed} listed."),
            (0, _) => format!("None listed: {failed} could not be listed."),
            _ => format!("{listed} listed, {failed} could not be listed."),
        };
        writeln!(out, "{summary}").map_err(write_failed)?;
    }
    Ok(match (listed, failed) {
        (_, 0) => 0,
        (0, _) => 1,
        _ => 3,
    })
}

/// One path from standard input, resolved against the current folder and
/// read through its provider.
fn stat_path(text: &str, lister: &Lister<'_>) -> Result<(Place, Entry), String> {
    let place = Place::resolve(text, lister.here).map_err(|e| could_not_list(text, &e))?;
    let provider = lister
        .registry
        .resolve(&place)
        .map_err(|e| could_not_list(text, &AtTheShell(&e)))?;
    let entry = provider
        .stat(&place, &OpContext::new())
        .map_err(|e| could_not_list(text, &AtTheShell(&e)))?;
    Ok((place, entry))
}

/// A sentence about the whole list: words in plain mode, a `message`
/// object in JSON mode.
fn say(out: &mut dyn Write, args: &LsArgs, sentence: &str) -> Result<(), CliError> {
    if args.json {
        writeln!(out, "{}", json!({ "message": sentence }))
    } else {
        writeln!(out, "{sentence}")
    }
    .map_err(write_failed)
}

/// A path that could not be listed: the sentence in plain mode, an
/// `error` object in JSON mode.
fn problem(
    out: &mut dyn Write,
    args: &LsArgs,
    input: &str,
    sentence: &str,
) -> Result<(), CliError> {
    if args.json {
        writeln!(out, "{}", json!({ "input": input, "error": sentence }))
    } else {
        writeln!(out, "{sentence}")
    }
    .map_err(write_failed)
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Whether the entry is a cloud placeholder whose data is not here.
fn online_only(entry: &Entry) -> bool {
    matches!(entry.attributes, Some(Attributes::Windows(bits)) if bits & ONLINE_ONLY_BITS != 0)
}

/// A time in words, in the clock's local time.
fn time_words(time: Timestamp, clock: &dyn Clock) -> String {
    describe_time(time, clock)
}

/// A byte count with thousands separators: "2,104,331 bytes".
fn exact_bytes(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if bytes == 1 {
        "1 byte".to_owned()
    } else {
        format!("{grouped} bytes")
    }
}

/// An RFC 3339 time in UTC, for JSON: "2026-10-16T15:41:07Z".
fn rfc3339(time: Timestamp) -> String {
    let c = time.to_civil(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// The words of one entry: name, kind, size, date, then the long facts,
/// then where it is when `parent` is given (a path from standard input).
pub fn entry_line(entry: &Entry, long: bool, parent: Option<&Place>, clock: &dyn Clock) -> String {
    let mut parts: Vec<String> = vec![entry.display_name().into_owned()];
    parts.push(match &entry.type_name {
        Some(type_name) if entry.kind == EntryKind::File => type_name.clone(),
        _ => entry.kind.word().to_owned(),
    });
    if let Some(size) = entry.size
        && entry.kind != EntryKind::Folder
    {
        parts.push(human_size(size));
    }
    if let Some(modified) = entry.modified {
        parts.push(format!("modified {}", time_words(modified, clock)));
    }
    if online_only(entry) {
        parts.push("online only".to_owned());
    }
    if entry.name_not_unicode {
        parts.push("name is not valid Unicode".to_owned());
    } else if entry.needs_care {
        parts.push("name needs care".to_owned());
    }
    if long {
        if let Some(size) = entry.size {
            parts.push(exact_bytes(size));
        }
        if let Some(created) = entry.created {
            parts.push(format!("created {}", time_words(created, clock)));
        }
        if let Some(accessed) = entry.accessed {
            parts.push(format!("read {}", time_words(accessed, clock)));
        }
        if let Some(attributes) = entry.attributes {
            let label = match attributes {
                Attributes::Unix(_) => "permissions",
                Attributes::Windows(_) => "attributes",
            };
            parts.push(format!("{label}: {}", attributes.short_words(entry.kind)));
        }
        if entry.hidden {
            parts.push("hidden".to_owned());
        }
        if let Some(owner) = &entry.owner {
            parts.push(format!("owner {}", display_name(owner)));
        }
        if let Some(group) = &entry.group {
            parts.push(format!("group {}", display_name(group)));
        }
        if let Some(link) = &entry.link {
            parts.push(format!("points to {}", display_name(&link.target)));
        }
        if let Some(links) = entry.hard_links
            && links > 1
        {
            parts.push(format!("{links} hard links"));
        }
    }
    if let Some(parent) = parent {
        parts.push(format!("in {}", display_name(&parent.to_string())));
    }
    parts.join(", ")
}

/// The JSON object of one entry: real names, times in RFC 3339 UTC,
/// `null` where a fact is not known.
pub fn entry_json(entry: &Entry, place: &Place) -> Value {
    json!({
        "name": entry.name,
        "display_name": entry.display_name(),
        "place": place.to_string(),
        "kind": entry.kind.word(),
        "size": entry.size,
        "modified": entry.modified.map(rfc3339),
        "created": entry.created.map(rfc3339),
        "accessed": entry.accessed.map(rfc3339),
        "hidden": entry.hidden,
        "online_only": online_only(entry),
        "needs_care": entry.needs_care,
        "name_not_unicode": entry.name_not_unicode,
        "attributes": entry.attributes.map(|a| a.short_words(entry.kind)),
        "owner": entry.owner,
        "group": entry.group,
        "link_target": entry.link.as_ref().map(|link| link.target.clone()),
    })
}

fn write_entry(
    out: &mut dyn Write,
    args: &LsArgs,
    entry: &Entry,
    place: &Place,
    parent: Option<&Place>,
    lister: &Lister<'_>,
) -> Result<(), CliError> {
    if args.json {
        writeln!(out, "{}", entry_json(entry, place))
    } else {
        writeln!(
            out,
            "{}",
            entry_line(entry, args.long, parent, lister.clock)
        )
    }
    .map_err(write_failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{EntryKind, FixedClock, Link};
    use diroxide_vfs::{AttributeChange, Capabilities, FileWriter, Provider, WriteMode};
    use std::fs::{self, File};
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Friday, October 16, 2026, 3:00 PM UTC.
    fn now() -> Timestamp {
        Timestamp::from_civil(2026, 10, 16, 15, 0, 0, UtcOffset::UTC).unwrap()
    }

    fn at(day: u8, hour: u8) -> Timestamp {
        Timestamp::from_civil(2026, 10, day, hour, 5, 0, UtcOffset::UTC).unwrap()
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(&args(&[])), Ok(LsArgs::default()));
        let parsed = parse(&args(&["--long", "--json", "--hidden", "D:\\x"])).unwrap();
        assert!(parsed.long && parsed.json && parsed.hidden);
        assert_eq!(parsed.source, Source::Place(Some("D:\\x".into())));
        let stdin = parse(&args(&["-", "--lines"])).unwrap();
        assert_eq!(stdin.source, Source::Stdin);
        assert_eq!(stdin.separator, Separator::Lines);
        assert_eq!(
            parse(&args(&["-0", "-"])).unwrap().separator,
            Separator::Nul
        );
        assert_eq!(
            parse(&args(&["--", "-"])).unwrap().source,
            Source::Place(Some("-".into()))
        );
        assert!(parse(&args(&["--help"])).unwrap().help);
        assert_eq!(
            parse(&args(&["--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert_eq!(
            parse(&args(&["a", "b"])),
            Err(CliError::TooMany("ls".into()))
        );
        assert!(matches!(
            parse(&args(&["--lines", "a"])),
            Err(CliError::Failed(text)) if text.starts_with("--lines only works with diro ls -")
        ));
        assert!(matches!(
            parse(&args(&["--lines", "--null", "-"])),
            Err(CliError::Failed(_))
        ));
    }

    #[test]
    fn lines_say_the_name_first_then_kind_size_and_date() {
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let file = Entry::new("report.pdf", EntryKind::File)
            .with_size(2_104_331)
            .with_modified(at(16, 14));
        assert_eq!(
            entry_line(&file, false, None, &clock),
            "report.pdf, file, 2.1 MB, modified today, 2:05 PM"
        );
        let folder = Entry::new("Photos", EntryKind::Folder)
            .with_size(4096)
            .with_modified(at(13, 9));
        assert_eq!(
            entry_line(&folder, false, None, &clock),
            "Photos, folder, modified Tuesday"
        );
        let pacific = FixedClock::new(now(), UtcOffset::from_hours(-7));
        assert_eq!(
            entry_line(&file, false, None, &pacific),
            "report.pdf, file, 2.1 MB, modified today, 7:05 AM",
            "local time, and no UTC"
        );
        let mut cloud = Entry::new("notes.txt", EntryKind::File).with_size(10);
        cloud.attributes = Some(Attributes::Windows(0x20 | 0x400000));
        assert_eq!(
            entry_line(&cloud, false, None, &clock),
            "notes.txt, file, 10 bytes, online only"
        );
        let odd = Entry::new("C\u{F03A}", EntryKind::Folder);
        assert_eq!(
            entry_line(&odd, false, None, &clock),
            "C?, folder, name needs care"
        );
        assert!(diroxide_core::format::within_cells(
            &entry_line(&file, true, None, &clock),
            "report.pdf, file, 2.1 MB"
        ));
    }

    #[test]
    fn long_lines_add_the_exact_facts_after_the_short_ones() {
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let mut entry = Entry::new("run.sh", EntryKind::Link)
            .with_size(1_234_567)
            .with_modified(at(15, 8));
        entry.created = Some(at(1, 10));
        entry.attributes = Some(Attributes::Unix(0o755));
        entry.owner = Some("ada".into());
        entry.link = Some(Link {
            target: "../bin/run.sh".into(),
            points_to: None,
        });
        assert_eq!(
            entry_line(&entry, true, None, &clock),
            "run.sh, link, 1.2 MB, modified yesterday, 8:05 AM, 1,234,567 bytes, \
             created October 1, permissions: read and run: everyone; write: owner, \
             owner ada, points to ../bin/run.sh"
        );
        assert_eq!(exact_bytes(1), "1 byte");
        assert_eq!(exact_bytes(999), "999 bytes");
        assert_eq!(exact_bytes(1000), "1,000 bytes");
    }

    #[test]
    fn json_keeps_real_names_and_says_times_in_utc() {
        let mut entry = Entry::new("C\u{F03A}", EntryKind::File)
            .with_size(5)
            .with_modified(at(16, 14));
        entry.hidden = true;
        let place = Place::parse(if cfg!(windows) { "D:\\x" } else { "/x" }).unwrap();
        let value = entry_json(&entry, &place);
        assert_eq!(value["name"], "C\u{F03A}");
        assert_eq!(value["display_name"], "C?");
        assert_eq!(value["modified"], "2026-10-16T14:05:00Z");
        assert_eq!(value["created"], Value::Null);
        assert_eq!(value["kind"], "file");
        assert_eq!(value["hidden"], true);
    }

    /// A fixture folder of plain names on disk, with fixed modified times.
    /// No hidden entry: whether a dot name is hidden is the local
    /// provider's rule per platform, so hidden entries are tested on the
    /// in-memory provider below.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: usize, secs: u64| {
            let path = dir.path().join(name);
            fs::write(&path, vec![b'x'; bytes]).unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_secs(secs))
                .unwrap();
        };
        let today = u64::try_from(at(16, 14).unix_seconds()).unwrap();
        write("report.pdf", 2_100_000, today);
        write("file10.txt", 10, today);
        write("file2.txt", 2, today);
        write("R\u{e9}sum\u{e9}.txt", 1, today);
        write("\u{65e5}\u{672c}\u{8a9e}.txt", 3, today);
        fs::create_dir(dir.path().join("Photos")).unwrap();
        dir
    }

    /// An in-memory folder: one folder place and its entries, for what the
    /// disk cannot hold under the owner's naming rules (names with spaces)
    /// or holds differently per platform (hidden entries).
    #[derive(Debug)]
    struct Fixed {
        root: Place,
        entries: Vec<Entry>,
    }

    impl Provider for Fixed {
        fn scheme(&self) -> &str {
            "file"
        }
        fn capabilities(&self, _: &Place) -> Capabilities {
            Capabilities::read_only()
        }
        fn list(&self, place: &Place, _: &OpContext) -> Result<Vec<Entry>, VfsError> {
            if *place == self.root {
                Ok(self.entries.clone())
            } else {
                Err(VfsError::NotFound)
            }
        }
        fn stat(&self, place: &Place, _: &OpContext) -> Result<Entry, VfsError> {
            if *place == self.root {
                return Ok(Entry::new("fix", EntryKind::Folder));
            }
            if place.parent().as_ref() != Some(&self.root) {
                return Err(VfsError::NotFound);
            }
            let name = place.name().ok_or(VfsError::NotFound)?;
            self.entries
                .iter()
                .find(|entry| entry.name == name)
                .cloned()
                .ok_or(VfsError::NotFound)
        }
        fn read(&self, _: &Place, _: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
            Err(VfsError::NotFound)
        }
        fn write(
            &self,
            _: &Place,
            _: WriteMode,
            _: &OpContext,
        ) -> Result<Box<dyn FileWriter>, VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn create_dir(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn rename(&self, _: &Place, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn remove(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
        fn set_attributes(
            &self,
            _: &Place,
            _: &AttributeChange,
            _: &OpContext,
        ) -> Result<AttributeChange, VfsError> {
            Err(VfsError::ReadOnlyFilesystem)
        }
    }

    fn fixed_root() -> Place {
        Place::parse(if cfg!(windows) { "D:\\fix" } else { "/fix" }).unwrap()
    }

    fn fixed_registry() -> Registry {
        let mut hidden = Entry::new(".hidden-notes.txt", EntryKind::File).with_size(4);
        hidden.hidden = true;
        let entries = vec![
            Entry::new("two words.txt", EntryKind::File)
                .with_size(2)
                .with_modified(at(16, 14)),
            hidden,
            Entry::new("notes", EntryKind::Folder).with_modified(at(15, 9)),
        ];
        let mut registry = Registry::new();
        registry.set_local(Arc::new(Fixed {
            root: fixed_root(),
            entries,
        }));
        registry
    }

    fn run_in(registry: &Registry, here: &Place, list: &[&str], input: &[u8]) -> (i32, String) {
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let lister = Lister::new(registry, &clock, here);
        let mut out = Vec::new();
        let code = run_with(
            &parse(&args(list)).unwrap(),
            &mut &input[..],
            &mut out,
            &lister,
        );
        let text = String::from_utf8(out).unwrap();
        match code {
            Ok(code) => (code, text),
            Err(error) => (-1, format!("{text}{error}")),
        }
    }

    fn list(dir: &tempfile::TempDir, list: &[&str], input: &[u8]) -> (i32, String) {
        let here = Place::parse(dir.path().to_str().unwrap()).unwrap();
        run_in(&Registry::with_local(), &here, list, input)
    }

    fn fixed(list: &[&str], input: &[u8]) -> (i32, String) {
        run_in(&fixed_registry(), &fixed_root(), list, input)
    }

    #[test]
    fn a_folder_lists_folders_first_in_natural_order() {
        let dir = fixture();
        let (code, text) = list(&dir, &[], b"");
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("Photos, folder, modified "), "{text}");
        assert_eq!(
            &lines[1..],
            [
                "file2.txt, file, 2 bytes, modified today, 2:05 PM",
                "file10.txt, file, 10 bytes, modified today, 2:05 PM",
                "report.pdf, file, 2.1 MB, modified today, 2:05 PM",
                "R\u{e9}sum\u{e9}.txt, file, 1 byte, modified today, 2:05 PM",
                "\u{65e5}\u{672c}\u{8a9e}.txt, file, 3 bytes, modified today, 2:05 PM",
                &format!(
                    "6 items in {}.",
                    dir.path().file_name().unwrap().to_str().unwrap()
                ),
            ]
        );
    }

    #[test]
    fn hidden_entries_are_counted_and_listed_on_request() {
        let (code, text) = fixed(&[], b"");
        assert_eq!(code, 0, "{text}");
        assert_eq!(
            text,
            "notes, folder, modified yesterday, 9:05 AM\n\
             two words.txt, file, 2 bytes, modified today, 2:05 PM\n\
             2 items in fix, 1 hidden not shown.\n"
        );
        let (_, all) = fixed(&["--hidden"], b"");
        assert!(all.lines().any(|l| l == ".hidden-notes.txt, file, 4 bytes"));
        assert_eq!(all.lines().last().unwrap(), "3 items in fix.");
    }

    #[test]
    fn json_lists_one_object_a_line_and_no_summary() {
        let dir = fixture();
        let (code, text) = list(&dir, &["--json"], b"");
        assert_eq!(code, 0);
        assert_eq!(text.lines().count(), 6);
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            assert!(
                value["place"]
                    .as_str()
                    .unwrap()
                    .ends_with(value["name"].as_str().unwrap())
            );
        }
    }

    #[test]
    fn a_file_lists_as_itself_and_a_missing_place_fails() {
        let dir = fixture();
        let (code, text) = list(&dir, &["file2.txt"], b"");
        assert_eq!(code, 0);
        assert!(text.starts_with("file2.txt, file, 2 bytes, modified today, 2:05 PM, in "));
        assert!(text.lines().last().unwrap().starts_with("1 item: "));
        let (code, text) = list(&dir, &["no-such-folder"], b"");
        assert_eq!(code, -1);
        assert!(text.starts_with("Could not list "), "{text}");
        assert!(text.contains("no-such-folder: it does not exist"), "{text}");
    }

    #[test]
    fn standard_input_lists_nul_separated_paths_with_spaces() {
        let (code, text) = fixed(&["-"], b"two words.txt\0notes\0");
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            format!(
                "two words.txt, file, 2 bytes, modified today, 2:05 PM, in {}",
                fixed_root()
            )
        );
        assert!(lines[1].starts_with("notes, folder, "));
        assert_eq!(lines[2], "2 listed.");
        let dir = fixture();
        let (code, text) = list(&dir, &["-"], b"R\xc3\xa9sum\xc3\xa9.txt\0file2.txt\0");
        assert_eq!(code, 0, "{text}");
        assert_eq!(text.lines().last().unwrap(), "2 listed.");
    }

    #[test]
    fn standard_input_says_what_it_could_not_list_and_exits_three() {
        let dir = fixture();
        let (code, text) = list(&dir, &["-"], b"file2.txt\0missing.txt\0");
        assert_eq!(code, 3);
        assert!(text.contains("Could not list missing.txt: it does not exist."));
        assert_eq!(
            text.lines().last().unwrap(),
            "1 listed, 1 could not be listed."
        );
        let (code, text) = list(&dir, &["-"], b"missing.txt\0\xff\xfe\0");
        assert_eq!(code, 1);
        assert!(text.contains("Could not list item 2: the name is not valid Unicode."));
        assert_eq!(
            text.lines().last().unwrap(),
            "None listed: 2 could not be listed."
        );
    }

    #[test]
    fn an_empty_list_does_nothing_and_says_so() {
        let dir = fixture();
        assert_eq!(
            list(&dir, &["-"], b""),
            (0, "List is empty. Nothing to list.\n".to_owned())
        );
        assert_eq!(
            list(&dir, &["-"], b"\0\0"),
            (0, "List is empty. Nothing to list.\n".to_owned())
        );
        assert_eq!(
            list(&dir, &["-", "--lines"], b"\n\r\n"),
            (0, "List is empty. Nothing to list.\n".to_owned())
        );
    }

    #[test]
    fn lines_must_be_asked_for() {
        let dir = fixture();
        assert_eq!(
            list(&dir, &["-"], b"file2.txt\nfile10.txt\n"),
            (2, "No NUL separators found. Use --lines.\n".to_owned())
        );
        let (code, text) = list(&dir, &["-", "--lines"], b"file2.txt\r\nfile10.txt\n");
        assert_eq!(code, 0, "{text}");
        assert_eq!(text.lines().last().unwrap(), "2 listed.");
        assert_eq!(
            list(&dir, &["-", "--lines"], b"a\0b"),
            (
                2,
                "The list has NUL separators. Leave out --lines.\n".to_owned()
            )
        );
        // One path with no separator at all is a list of one.
        let (code, _) = list(&dir, &["-"], b"file2.txt");
        assert_eq!(code, 0);
    }

    #[test]
    fn every_fixed_sentence_says_its_meaning_in_40_cells() {
        for (line, meaning) in [
            (
                "List is too long: over 64 MB. Nothing was listed.",
                "List is too long: over 64 MB",
            ),
            (
                "No NUL separators found. Use --lines.",
                "No NUL separators found. Use --lines.",
            ),
            (
                "The list has NUL separators. Leave out --lines.",
                "The list has NUL separators",
            ),
            (
                "List is empty. Nothing to list.",
                "List is empty. Nothing to list.",
            ),
        ] {
            assert!(diroxide_core::format::within_cells(line, meaning), "{line}");
        }
        for line in help_text().lines() {
            assert!(line.chars().count() <= 80, "{line}");
        }
    }
}
