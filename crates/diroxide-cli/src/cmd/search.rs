//! `diro search PLACE [EXPRESSION]`: search a place by name, kind, size,
//! date and contents in the one filter language, the same text the panel
//! filter and the search dialog take.
//!
//! fd's habits are sugar that turns into the expression: `--hidden`,
//! `--follow`, `--max-depth N` (`depth at most N`), `--type f|d|l|x|e`,
//! `--extension EXT`, `--size +10M` (`size over 10 MB`; KB is 1,000 bytes
//! and KiB 1,024), `--changed-within 2d` (`modified within 2 days`),
//! `--changed-before` (a date or a duration), `--exclude GLOB` (`not name
//! matches GLOB`), `--content TEXT`, `--content-regex RE`, and `--glob`,
//! `--regex` and `--full-path`, which read the whole expression as one
//! pattern. `--explain` reads the expression back and searches nothing.
//!
//! Output: one row a hit, the name first ("notes.txt, in docs/q3, 4.0
//! KB"), between "Searching docs for *.txt" and the summary ("214 found.
//! Hidden files skipped."); `--null` writes NUL-separated paths and
//! nothing else to standard output (the summary goes to standard error);
//! `--json` writes one object a line, the summary last.
//!
//! Exit codes: 0 found something, 1 found nothing, 2 wrong use or a bad
//! expression, 3 partly done (some folders or files could not be read).

use std::io::{BufWriter, Write};

use diroxide_core::filter::{Expr, FilterOptions};
use diroxide_core::{CaseRule, Clock, Explain, Message, Place, Timestamp, UtcOffset};
use diroxide_search::{
    RowShape, Scope, SearchHit, SearchProgress, SearchQuery, SearchSink, SearchSummary, quoted,
};
use diroxide_vfs::{OpContext, Registry, VfsError};
use serde_json::json;

use crate::CliError;
use crate::open::current_place;

/// What `diro search` was asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchArgs {
    /// Where to search, as typed.
    pub place: Option<String>,
    /// The expression's words, joined by spaces.
    pub expression: Vec<String>,
    /// Search hidden entries too.
    pub hidden: bool,
    /// Read ignore files, and skip what they list.
    pub ignore_files: bool,
    /// Follow links to folders inside the place.
    pub follow: bool,
    /// Search inside archives.
    pub archives: bool,
    /// How deep to go: `depth at most N`.
    pub max_depth: Option<u32>,
    /// `--type` letters or words.
    pub types: Vec<String>,
    /// `--extension` values.
    pub extensions: Vec<String>,
    /// `--size` values, such as `+10M`.
    pub sizes: Vec<String>,
    /// `--changed-within`: a duration or a date.
    pub changed_within: Option<String>,
    /// `--changed-before`: a date or a duration.
    pub changed_before: Option<String>,
    /// `--exclude` globs.
    pub excludes: Vec<String>,
    /// Read the expression as one pattern for the whole path.
    pub full_path: bool,
    /// Read the expression as one glob.
    pub glob: bool,
    /// Read the expression as one regular expression.
    pub regex: bool,
    /// `--content`: text the files hold.
    pub content: Option<String>,
    /// `--content-regex`: a regular expression the files hold.
    pub content_regex: Option<String>,
    /// One row per matching line rather than per file.
    pub lines: bool,
    /// How case decides: `-i` ignores it, `-s` matches it.
    pub case: Option<CaseRule>,
    /// NUL-separated paths.
    pub null: bool,
    /// One JSON object a line.
    pub json: bool,
    /// Read the expression back and search nothing.
    pub explain: bool,
    /// Threads for the walk.
    pub threads: Option<usize>,
    /// The most hits written.
    pub max_results: Option<u64>,
    /// Print the help for `diro search` and nothing else.
    pub help: bool,
}

/// The help for `diro search`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro search: find entries by name, kind, size, date and contents.",
        "",
        "Usage:",
        "diro search PLACE [EXPRESSION]   EXPRESSION is the filter language",
        "diro search docs size over 1 MB and ext pdf",
        "diro search docs --content \"total due\" --extension txt",
        "",
        "Options:",
        "--hidden, -H             search hidden entries too",
        "--ignore-files           skip what .gitignore and .ignore files list",
        "--no-ignore, -I          read no ignore files (the default)",
        "--follow, -L             follow links to folders inside PLACE",
        "--archives               search inside archives",
        "--max-depth N, -d N      go at most N folders deep",
        "--type T, -t T           f file, d folder, l link, x executable, e empty",
        "--extension E, -e E      this extension",
        "--size S, -S S           +10M over 10 MB, -1ki under 1 KiB",
        "--changed-within T       a duration such as 2d, or a date such as 2026-10-01",
        "--changed-before T       a date, or a duration such as 1y",
        "--exclude GLOB, -E GLOB  leave out names that match GLOB",
        "--content TEXT           files that hold TEXT; smart case",
        "--content-regex RE       files that hold a match for RE",
        "--lines                  one row per matching line, not per file",
        "--glob, -g               read EXPRESSION as one glob",
        "--regex                  read EXPRESSION as one regular expression",
        "--full-path, -p          match the pattern against the whole path",
        "--ignore-case, -i        ignore case; --case-sensitive, -s matches it",
        "--null, -0               write NUL-separated paths only",
        "--json                   write one JSON object a line",
        "--explain                read the expression back; search nothing",
        "--threads N              walk with N threads",
        "--max-results N          write at most N hits; the count goes on",
        "",
        "Exit codes:",
        "0  found something",
        "1  found nothing",
        "2  wrong use, or a bad expression",
        "3  partly done: some folders or files could not be read",
    ]
    .join("\n")
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, CliError> {
    value
        .parse()
        .map_err(|_| CliError::Failed(format!("{flag} takes a whole number, not {value}")))
}

/// Reads the arguments after `search`.
pub fn parse(args: &[String]) -> Result<SearchArgs, CliError> {
    let mut parsed = SearchArgs::default();
    let mut rest = args.iter();
    let mut only_words = false;
    while let Some(arg) = rest.next() {
        if only_words || !arg.starts_with('-') || arg == "-" {
            if parsed.place.is_none() {
                parsed.place = Some(arg.clone());
            } else {
                parsed.expression.push(arg.clone());
            }
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |flag: &str| -> Result<String, CliError> {
            match inline.clone().or_else(|| rest.next().cloned()) {
                Some(value) if !value.is_empty() => Ok(value),
                _ => Err(CliError::Missing(flag.to_owned())),
            }
        };
        match name {
            "--" => only_words = true,
            "--help" | "-h" => parsed.help = true,
            "--hidden" | "-H" => parsed.hidden = true,
            "--ignore-files" => parsed.ignore_files = true,
            "--no-ignore" | "-I" => parsed.ignore_files = false,
            "--follow" | "-L" => parsed.follow = true,
            "--archives" => parsed.archives = true,
            "--max-depth" | "-d" => {
                parsed.max_depth = Some(number(name, &value("--max-depth")?)?);
            }
            "--type" | "-t" => parsed.types.push(value("--type")?),
            "--extension" | "-e" => parsed.extensions.push(value("--extension")?),
            "--size" | "-S" => parsed.sizes.push(value("--size")?),
            "--changed-within" => parsed.changed_within = Some(value("--changed-within")?),
            "--changed-before" => parsed.changed_before = Some(value("--changed-before")?),
            "--exclude" | "-E" => parsed.excludes.push(value("--exclude")?),
            "--full-path" | "-p" => parsed.full_path = true,
            "--glob" | "-g" => parsed.glob = true,
            "--regex" => parsed.regex = true,
            "--content" => parsed.content = Some(value("--content")?),
            "--content-regex" => parsed.content_regex = Some(value("--content-regex")?),
            "--lines" => parsed.lines = true,
            "--ignore-case" | "-i" => parsed.case = Some(CaseRule::Ignore),
            "--case-sensitive" | "-s" => parsed.case = Some(CaseRule::Match),
            "--null" | "-0" => parsed.null = true,
            "--json" => parsed.json = true,
            "--explain" => parsed.explain = true,
            "--threads" => parsed.threads = Some(number(name, &value("--threads")?)?),
            "--max-results" => {
                parsed.max_results = Some(number(name, &value("--max-results")?)?);
            }
            other => return Err(CliError::UnknownOption(other.to_owned())),
        }
    }
    if parsed.place.is_none() && !parsed.help {
        return Err(CliError::Missing("search".to_owned()));
    }
    if parsed.null && parsed.json {
        return Err(CliError::Failed(
            "--null and --json cannot be used together".to_owned(),
        ));
    }
    if parsed.glob && parsed.regex {
        return Err(CliError::Failed(
            "--glob and --regex cannot be used together".to_owned(),
        ));
    }
    Ok(parsed)
}

/// A size as fd writes it, in the filter language: `+10M` is `size over
/// 10 MB`, `-1ki` is `size under 1 KiB`, `5k` is `size is 5 KB`.
fn size_term(text: &str) -> Result<String, CliError> {
    let (op, rest) = match text.as_bytes().first() {
        Some(b'+') => ("over", &text[1..]),
        Some(b'-') => ("under", &text[1..]),
        _ => ("is", text),
    };
    let split = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    let (digits, unit) = rest.split_at(split);
    let unit = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => "B",
        "k" | "kb" => "KB",
        "m" | "mb" => "MB",
        "g" | "gb" => "GB",
        "t" | "tb" => "TB",
        "ki" | "kib" => "KiB",
        "mi" | "mib" => "MiB",
        "gi" | "gib" => "GiB",
        "ti" | "tib" => "TiB",
        _ => "",
    };
    if digits.is_empty() || unit.is_empty() {
        return Err(CliError::Failed(format!(
            "--size takes a sign, a number and a unit, such as +10M or -1ki, not {text}"
        )));
    }
    Ok(format!("size {op} {digits} {unit}"))
}

/// A duration as fd writes it (`2d`, `3weeks`, `10min`, `1y`), in the
/// filter language's words, or `None` when it is not one.
fn duration_words(text: &str) -> Option<String> {
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (digits, unit) = text.split_at(split);
    let n: u64 = digits.parse().ok()?;
    let word = match unit {
        "M" | "mo" | "month" | "months" => "month",
        _ => match unit.to_ascii_lowercase().as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => "second",
            "m" | "min" | "mins" | "minute" | "minutes" => "minute",
            "h" | "hr" | "hrs" | "hour" | "hours" => "hour",
            "d" | "day" | "days" => "day",
            "w" | "week" | "weeks" => "week",
            "y" | "yr" | "yrs" | "year" | "years" => "year",
            _ => return None,
        },
    };
    Some(if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    })
}

/// Whether text looks like a date written year first.
fn is_date(text: &str) -> bool {
    text.len() >= 4 && text.as_bytes()[..4].iter().all(u8::is_ascii_digit)
}

fn type_term(text: &str) -> Result<&'static str, CliError> {
    Ok(match text.to_ascii_lowercase().as_str() {
        "f" | "file" => "kind file",
        "d" | "dir" | "directory" | "folder" => "kind folder",
        "l" | "symlink" | "link" => "kind link",
        "x" | "executable" => "executable",
        "e" | "empty" => "empty",
        _ => {
            return Err(CliError::Failed(format!(
                "--type takes f, d, l, x or e, not {text}"
            )));
        }
    })
}

/// The expression the arguments make: the words as typed (or read as one
/// pattern), then each option's term, joined by and.
pub fn expression_text(args: &SearchArgs) -> Result<String, CliError> {
    let base = args.expression.join(" ").trim().to_owned();
    let field = if args.full_path { "path" } else { "name" };
    let base = if base.is_empty() {
        base
    } else if args.regex {
        format!("{field} regex {}", quoted(&base))
    } else if args.glob {
        format!("{field} matches {}", quoted(&base))
    } else if args.full_path {
        let op = if base.contains(['*', '?', '[']) {
            "matches"
        } else {
            "contains"
        };
        format!("path {op} {}", quoted(&base))
    } else {
        base
    };
    let mut parts = Vec::new();
    if !base.is_empty() {
        parts.push(base);
    }
    if !args.types.is_empty() {
        let terms = args
            .types
            .iter()
            .map(|t| type_term(t))
            .collect::<Result<Vec<_>, _>>()?;
        parts.push(if terms.len() == 1 {
            terms[0].to_owned()
        } else {
            format!("({})", terms.join(" or "))
        });
    }
    if !args.extensions.is_empty() {
        let list: Vec<String> = args
            .extensions
            .iter()
            .map(|e| quoted(e.trim_start_matches('.')))
            .collect();
        parts.push(format!("ext {}", list.join(", ")));
    }
    for size in &args.sizes {
        parts.push(size_term(size)?);
    }
    if let Some(depth) = args.max_depth {
        parts.push(format!("depth at most {depth}"));
    }
    if let Some(within) = &args.changed_within {
        parts.push(if is_date(within) {
            format!("modified since {within}")
        } else {
            let words = duration_words(within).ok_or_else(|| {
                CliError::Failed(format!(
                    "--changed-within takes a duration such as 2d or a date such as 2026-10-01, not {within}"
                ))
            })?;
            format!("modified within {words}")
        });
    }
    if let Some(before) = &args.changed_before {
        parts.push(if is_date(before) {
            format!("modified before {before}")
        } else {
            let words = duration_words(before).ok_or_else(|| {
                CliError::Failed(format!(
                    "--changed-before takes a date such as 2026-10-01 or a duration such as 1y, not {before}"
                ))
            })?;
            format!("modified older than {words}")
        });
    }
    for exclude in &args.excludes {
        parts.push(format!("not name matches {}", quoted(exclude)));
    }
    if let Some(text) = &args.content {
        parts.push(format!("content {}", quoted(text)));
    }
    if let Some(regex) = &args.content_regex {
        parts.push(format!("content regex {}", quoted(regex)));
    }
    Ok(match parts.len() {
        0 => String::new(),
        1 => parts.remove(0),
        _ => {
            if !args.expression.is_empty() {
                parts[0] = format!("({})", parts[0]);
            }
            parts.join(" and ")
        }
    })
}

/// The tree in words, one test a line as typed, for `--explain`.
fn outline(expr: &Expr, text: &str, depth: usize, lines: &mut Vec<String>) {
    let pad = "  ".repeat(depth);
    match expr {
        Expr::Test { at, .. } => {
            let typed = text.get(at.span()).unwrap_or("").trim();
            lines.push(format!("{pad}{typed}"));
        }
        Expr::Not(inner) => {
            lines.push(format!("{pad}not"));
            outline(inner, text, depth + 1, lines);
        }
        Expr::And(parts) => {
            lines.push(format!("{pad}all of"));
            for part in parts {
                outline(part, text, depth + 1, lines);
            }
        }
        Expr::Or(parts) => {
            lines.push(format!("{pad}one of"));
            for part in parts {
                outline(part, text, depth + 1, lines);
            }
        }
        _ => lines.push(format!("{pad}a test this version cannot show")),
    }
}

/// The scope in words, for `--explain`.
fn scope_words(scope: &Scope) -> String {
    let mut parts = vec![
        scope
            .places
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    ];
    parts.push(if scope.hidden {
        "hidden entries searched".into()
    } else {
        "hidden entries skipped".into()
    });
    parts.push(if scope.ignore_files {
        "ignore files read".into()
    } else {
        "ignore files not read".into()
    });
    parts.push(if scope.follow_links {
        "links inside followed".into()
    } else {
        "links not followed".into()
    });
    parts.push(if scope.one_file_system {
        "one drive".into()
    } else {
        "every drive".into()
    });
    parts.push(if scope.archives {
        "inside archives".into()
    } else {
        "archives not entered".into()
    });
    parts.push(match scope.max_depth {
        Some(depth) => format!("at most {depth} deep"),
        None => "any depth".into(),
    });
    format!("Scope: {}.", parts.join("; "))
}

/// A provider's error as the command line says it: a place that is not
/// there asks for the path to be checked, since no folder is on the
/// screen to reload.
struct AtTheShell<'a>(&'a VfsError);

impl std::fmt::Display for AtTheShell<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// An RFC 3339 time in UTC, for JSON.
fn rfc3339(time: Timestamp) -> String {
    let c = time.to_civil(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// Writes each hit as it comes.
struct Writer<'a> {
    out: &'a mut dyn Write,
    args: &'a SearchArgs,
    root: &'a Place,
    failed: Option<String>,
}

impl Writer<'_> {
    fn hit(&mut self, hit: &SearchHit) -> std::io::Result<()> {
        if self.args.null {
            write!(self.out, "{}\0", hit.place)
        } else if self.args.json {
            let line = hit.line.as_ref();
            writeln!(
                self.out,
                "{}",
                json!({
                    "place": hit.place.to_string(),
                    "name": hit.entry.name,
                    "kind": hit.entry.kind.word(),
                    "size": hit.entry.size,
                    "modified": hit.entry.modified.map(rfc3339),
                    "depth": hit.depth,
                    "line": line.map(|l| l.line_number),
                    "column": line.map(|l| l.column),
                    "text": line.map(|l| l.text.clone()),
                    "more": hit.more,
                })
            )
        } else {
            writeln!(self.out, "{}", hit.row(Some(self.root)).spoken())
        }
    }
}

impl SearchSink for Writer<'_> {
    fn found(&mut self, hit: SearchHit) -> std::ops::ControlFlow<()> {
        if let Err(error) = self.hit(&hit) {
            self.failed = Some(format!("the results could not be written: {error}"));
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    }

    fn progress(&mut self, _: &SearchProgress) {}
}

/// Where the summary and problems go: standard output in words and in
/// JSON, standard error beside `--null`, so a pipe gets only paths.
fn say(args: &SearchArgs, out: &mut dyn Write, key: &str, text: &str) -> Result<(), CliError> {
    let failed = |e: std::io::Error| CliError::Failed(format!("the search could not write: {e}"));
    if args.null {
        let _ = writeln!(std::io::stderr(), "{text}");
        Ok(())
    } else if args.json {
        writeln!(out, "{}", json!({ key: text })).map_err(failed)
    } else {
        writeln!(out, "{text}").map_err(failed)
    }
}

/// The exit code of a finished search.
pub fn exit_code(summary: &SearchSummary) -> i32 {
    if summary.is_partial() {
        3
    } else if summary.hits > 0 {
        0
    } else {
        1
    }
}

/// Runs `diro search`. Returns the exit code.
pub fn run(args: &SearchArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(|e| CliError::Failed(e.to_string()))?;
        return Ok(0);
    }
    let clock = diroxide_app::startup::local_clock();
    let here = current_place()?;
    let registry = crate::cmd::archive::archive_registry().0;
    run_with(args, out, &registry, &clock, &here)
}

/// [`run`] with the providers, the clock and the current folder given.
pub fn run_with(
    args: &SearchArgs,
    out: &mut dyn Write,
    registry: &Registry,
    clock: &dyn Clock,
    here: &Place,
) -> Result<i32, CliError> {
    let mut out = BufWriter::new(out);
    let typed = args.place.as_deref().unwrap_or(".");
    let root = match Place::resolve(typed, here) {
        Ok(root) => root,
        Err(error) => {
            let text = Message::from_error("search", typed, &error).text;
            say(args, &mut out, "problem", &text)?;
            return Ok(2);
        }
    };
    let text = expression_text(args)?;
    let options = FilterOptions {
        case: args.case.unwrap_or(CaseRule::Smart),
        ..FilterOptions::default()
    };
    let query = match SearchQuery::parse_with(root.clone(), &text, &options) {
        Ok(query) => query,
        Err(error) => {
            say(args, &mut out, "problem", &error.line())?;
            let next = error.next_step();
            if !next.is_empty() {
                say(args, &mut out, "next", &next)?;
            }
            return Ok(2);
        }
    };
    let scope = Scope::new(root.clone())
        .with_hidden(args.hidden)
        .with_ignore_files(args.ignore_files)
        .with_follow_links(args.follow)
        .with_archives(args.archives)
        .with_max_depth(args.max_depth)
        .with_threads(args.threads)
        .with_row_cap(args.max_results);
    let rows = if args.lines {
        RowShape::PerLine
    } else {
        RowShape::PerFile
    };
    let query = query.with_scope(scope).with_rows(rows);
    let name = root.name().map_or_else(|| root.to_string(), str::to_owned);
    if args.explain {
        let read_back = query.filter.as_ref().map(|f| f.read_back(clock));
        let mut lines = vec![format!(
            "Reads as: {}",
            read_back.as_ref().map_or("every entry", |r| r.sentence())
        )];
        if let Some(details) = read_back.as_ref().map(|r| r.details_line())
            && !details.is_empty()
        {
            lines.push(format!("Details: {details}"));
        }
        if let Some(filter) = &query.filter {
            lines.push("Tree:".to_owned());
            outline(filter.expr(), filter.text(), 1, &mut lines);
        }
        lines.push(scope_words(&query.scope));
        lines.push("Searched nothing: --explain only reads the expression back.".to_owned());
        for line in lines {
            say(args, &mut out, "explain", &line)?;
        }
        out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
        return Ok(0);
    }
    let checked = registry
        .resolve(&root)
        .and_then(|provider| provider.stat(&root, &OpContext::new()));
    if let Err(error) = checked {
        let text = Message::from_error("search", &root.to_string(), &AtTheShell(&error)).text;
        say(args, &mut out, "problem", &text)?;
        return Ok(2);
    }
    if !args.null && !args.json {
        let shown = if text.is_empty() {
            "every entry"
        } else {
            &text
        };
        say(
            args,
            &mut out,
            "message",
            &format!("Searching {name} for {shown}"),
        )?;
    }
    let control = diroxide_search::SearchControl::new();
    let mut writer = Writer {
        out: &mut out,
        args,
        root: &root,
        failed: None,
    };
    let result = diroxide_search::run_search(&query, registry, clock, &control, &mut writer);
    if let Some(failed) = writer.failed.take() {
        return Err(CliError::Failed(failed));
    }
    let summary = match result {
        Ok(summary) => summary,
        Err(error) => {
            let text = Message::from_error("search", &root.to_string(), &error).text;
            say(args, &mut out, "problem", &text)?;
            return Ok(2);
        }
    };
    let code = exit_code(&summary);
    if args.json {
        writeln!(
            out,
            "{}",
            json!({
                "summary": summary.sentence(),
                "found": summary.hits,
                "folders": summary.folders,
                "exit": code,
            })
        )
        .map_err(|e| CliError::Failed(e.to_string()))?;
    } else {
        say(args, &mut out, "summary", &summary.sentence())?;
    }
    out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use diroxide_core::FixedClock;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn tree() -> Place {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/search/tree");
        let path = std::fs::canonicalize(path).unwrap();
        let text =
            diroxide_core::place::strip_extended_prefix(&path.to_string_lossy()).into_owned();
        Place::parse(&text).unwrap()
    }

    fn search(list: &[&str]) -> (i32, String) {
        let parsed = parse(&args(list)).unwrap();
        let mut out = Vec::new();
        let clock = FixedClock::new(Timestamp::from_unix(1_791_000_000, 0), UtcOffset::UTC);
        let here = tree();
        let code = run_with(&parsed, &mut out, &Registry::with_local(), &clock, &here).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_sugar_turns_into_the_expression() {
        let parsed = parse(&args(&[
            "docs",
            "report",
            "--type",
            "f",
            "-e",
            ".pdf",
            "-e",
            "md",
            "--size",
            "+10M",
            "-S",
            "-1ki",
            "--max-depth=2",
            "--changed-within",
            "2d",
            "--changed-before",
            "2026-10-01",
            "--exclude",
            "*.tmp",
            "--content",
            "total \"due\"",
        ]))
        .unwrap();
        assert_eq!(
            expression_text(&parsed).unwrap(),
            "(report) and kind file and ext \"pdf\", \"md\" and size over 10 MB and size under 1 KiB and depth at most 2 and modified within 2 days and modified before 2026-10-01 and not name matches \"*.tmp\" and content \"total \"\"due\"\"\""
        );
        let text = expression_text(&parsed).unwrap();
        assert!(diroxide_core::Filter::parse(&text).is_ok(), "{text}");
        let types = parse(&args(&["d", "-t", "f", "-t", "d"])).unwrap();
        assert_eq!(
            expression_text(&types).unwrap(),
            "(kind file or kind folder)"
        );
        let glob = parse(&args(&["d", "a b*.txt", "--glob"])).unwrap();
        assert_eq!(expression_text(&glob).unwrap(), "name matches \"a b*.txt\"");
        let regex = parse(&args(&["d", "^IMG_\\d+", "--regex", "-p"])).unwrap();
        assert_eq!(expression_text(&regex).unwrap(), "path regex \"^IMG_\\d+\"");
        assert_eq!(duration_words("1y").unwrap(), "1 year");
        assert_eq!(duration_words("10min").unwrap(), "10 minutes");
        assert_eq!(duration_words("3M").unwrap(), "3 months");
        assert!(size_term("+10").is_ok() && size_term("+x").is_err());
    }

    #[test]
    fn wrong_use_is_said() {
        assert_eq!(parse(&[]), Err(CliError::Missing("search".into())));
        assert_eq!(
            parse(&args(&["d", "--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        assert!(parse(&args(&["d", "--null", "--json"])).is_err());
        assert!(parse(&args(&["d", "--max-depth", "deep"])).is_err());
        assert!(parse(&args(&["--help"])).unwrap().help);
    }

    #[test]
    fn finds_by_name_with_rows_and_the_summary() {
        let (code, text) = search(&[".", "ext txt"]);
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Searching tree for ext txt");
        assert!(lines.contains(&"notes.txt, in tree/q3, 9 bytes"), "{text}");
        assert!(lines.last().unwrap().starts_with("3 found."), "{text}");
    }

    #[test]
    fn content_rows_and_one_row_per_line() {
        let (code, text) = search(&[".", "--content", "total due"]);
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains("report.txt, line 2: total due now, 1 more, in tree/q3"),
            "{text}"
        );
        let (_, text) = search(&[".", "--content", "total due", "--lines"]);
        assert!(text.contains("report.txt, line 4: TOTAL DUE again, in tree/q3"));
    }

    #[test]
    fn exit_codes_say_found_nothing_and_wrong_use() {
        let (code, text) = search(&[".", "name zebracrumbs-nowhere"]);
        assert_eq!(code, 1);
        assert!(text.lines().last().unwrap().starts_with("0 found."));
        let (code, text) = search(&[".", "size over"]);
        assert_eq!(code, 2);
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "Filter problem: size needs a number",
                "Word 3. Try size over 10 MB."
            ]
        );
        let (code, text) = search(&["no-such-folder-here", "x"]);
        assert_eq!(code, 2);
        assert!(text.starts_with("Could not search "), "{text}");
        assert!(
            text.trim_end()
                .ends_with("no-such-folder-here: it does not exist. Check the path."),
            "{text}"
        );
    }

    #[test]
    fn null_and_json_write_for_programs() {
        let (_, text) = search(&[".", "ext md", "--null"]);
        let paths: Vec<&str> = text.split('\0').filter(|p| !p.is_empty()).collect();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("b.md"), "{text}");
        let (_, text) = search(&[".", "ext md", "--json"]);
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["name"], "b.md");
        assert_eq!(lines[0]["kind"], "file");
        assert_eq!(lines.last().unwrap()["found"], 1);
        assert_eq!(lines.last().unwrap()["exit"], 0);
    }

    #[test]
    fn explain_reads_back_and_searches_nothing() {
        let (code, text) = search(&[".", "size>1kb and name *.log", "--explain"]);
        assert_eq!(code, 0);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Reads as: size over 1 KB and name matches *.log");
        assert_eq!(lines[1], "Details: 1 KB is 1,000 bytes.");
        assert_eq!(&lines[2..5], ["Tree:", "  all of", "    size>1kb"]);
        assert!(lines.iter().any(|l| l.starts_with("Scope: ")));
        assert_eq!(
            *lines.last().unwrap(),
            "Searched nothing: --explain only reads the expression back."
        );
        let (_, text) = search(&[".", "modified last week", "--explain"]);
        assert!(
            text.starts_with("Reads as: modified last week, September 20 to 26"),
            "{text}"
        );
    }

    #[test]
    fn a_folder_that_cannot_be_read_exits_with_3() {
        let mut summary = SearchSummary::default();
        summary.hits = 4;
        summary.unreadable = 1;
        assert_eq!(exit_code(&summary), 3);
        assert_eq!(exit_code(&SearchSummary::default()), 1);
    }
}
