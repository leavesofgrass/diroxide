//! A script's header: the `diro:` lines in the comments at the top of a
//! one-file script (R3 section 5.1, completed by R22 section 2.6).
//!
//! ```text
//! #!/usr/bin/env bash
//! # diro: name = "Total size of selection"
//! # diro: selection_file = "nul"
//! # diro: timeout = "5m"
//! ```
//!
//! - **Where.** The first 40 lines. A shebang on line 1 is skipped, and so
//!   is `@echo off` in a batch file; blank lines and ordinary comments are
//!   passed over; reading stops at the first line that is neither.
//! - **Comment marks.** `#`, `//`, `--`, `::` and `REM` (any case), then
//!   `diro:`, then `key = value`.
//! - **Values are TOML values,** read by the same parser as the
//!   configuration: text in quotes, whole numbers, `true` and `false`,
//!   lists, and inline tables. An unquoted word is refused with "put the
//!   value in quotes"; an unknown key is kept out with a warning, so a
//!   newer script still runs on an older diroxide.
//!
//! [`parse_header`] returns the header and every problem, each naming its
//! key and line and giving the fix; `diro plugin check` prints them.

use std::collections::BTreeMap;

use diroxide_core::Key;
use diroxide_core::names::has_hidden;
use toml::{Table, Value};

use crate::manifest::{
    MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, Os, Places, Problem, Selection, Severity,
};
use crate::protocol::PROTOCOL;
use crate::script::read::Reader;
use crate::script::recipe::{RECIPE_KEYS, Recipe};

/// How many lines from the top hold the header.
pub const HEADER_LINES: usize = 40;

/// The comment marks a header line may start with; `REM` in any case.
pub const COMMENT_MARKS: [&str; 5] = ["#", "//", "--", "::", "REM"];

/// The keys a header may set: the command's own, then the run family's
/// ([`RECIPE_KEYS`]).
pub const HEADER_KEYS: &[&str] = &[
    "name",
    "description",
    "key",
    "menu",
    "selection",
    "places",
    "needs",
    "os",
    "protocol",
];

/// Words a description should not start with: the meaning comes first.
const FILLER: &[&str] = &[
    "this script",
    "this command",
    "this plugin",
    "a script",
    "script that",
    "allows you to",
    "lets you",
    "used to",
    "this is",
];

/// What a script's header says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptHeader {
    /// The command's name; the file's name when absent.
    pub name: Option<String>,
    /// One sentence for the palette and F1 help.
    pub description: String,
    /// A proposed key, bound only when free.
    pub key: Option<String>,
    /// Where it appears; the Plugins menu when absent.
    pub menu: Option<String>,
    /// How many entries it needs.
    pub selection: Selection,
    /// Which places it works on.
    pub places: Places,
    /// The tools it needs, by name.
    pub needs: Vec<String>,
    /// The systems it exists on; empty means every system.
    pub os: Vec<Os>,
    /// The protocol's major version.
    pub protocol: u32,
    /// How it runs.
    pub recipe: Recipe,
}

impl Default for ScriptHeader {
    fn default() -> Self {
        Self {
            name: None,
            description: String::new(),
            key: None,
            menu: None,
            selection: Selection::Any,
            places: Places::Any,
            needs: Vec::new(),
            os: Vec::new(),
            protocol: PROTOCOL,
            recipe: Recipe::default(),
        }
    }
}

impl ScriptHeader {
    /// The command's name: the header's, or the file's stem with
    /// underscores read as spaces ("make_thumbs" says "make thumbs").
    pub fn name_or(&self, stem: &str) -> String {
        self.name.clone().unwrap_or_else(|| stem.replace('_', " "))
    }

    /// Whether the command exists on this system.
    pub fn exists_on(&self, os: Os) -> bool {
        self.os.is_empty() || self.os.contains(&os)
    }
}

/// What [`parse_header`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderReport {
    /// The header, with defaults where a line was wrong.
    pub header: ScriptHeader,
    /// Every problem, errors first.
    pub problems: Vec<Problem>,
    /// How many `diro:` lines were read.
    pub settings: usize,
}

impl HeaderReport {
    /// Whether any problem is an error: the script does not run.
    pub fn has_errors(&self) -> bool {
        self.problems.iter().any(|p| p.severity == Severity::Error)
    }

    /// The errors.
    pub fn errors(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
    }
}

/// One `diro:` line, as found.
struct Setting {
    line: usize,
    key: String,
    value: String,
}

/// The `diro:` lines at the top of a script, and the problems with the
/// lines themselves (no `=`, a key that is not a word).
fn settings(text: &str) -> (Vec<Setting>, Vec<Problem>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    for (index, raw) in text.lines().take(HEADER_LINES).enumerate() {
        let number = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || (number == 1 && trimmed.starts_with("#!"))
            || trimmed.eq_ignore_ascii_case("@echo off")
        {
            continue;
        }
        let Some(comment) = comment_body(trimmed) else {
            break;
        };
        let Some(setting) = comment.trim_start().strip_prefix("diro:") else {
            continue;
        };
        let setting = setting.trim();
        let line_field = format!("line {number}");
        let Some((key, value)) = setting.split_once('=') else {
            problems.push(Problem {
                severity: Severity::Error,
                field: line_field,
                message: "a diro: line has no value".to_owned(),
                fix: "Write it as diro: key = \"value\".".to_owned(),
            });
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            problems.push(Problem {
                severity: Severity::Error,
                field: line_field,
                message: format!("\"{key}\" is not a key"),
                fix: "Use a key such as name, description or timeout.".to_owned(),
            });
            continue;
        }
        found.push(Setting {
            line: number,
            key: key.to_owned(),
            value: value.trim().to_owned(),
        });
    }
    (found, problems)
}

/// The text after a comment mark, or `None` when the line is not a
/// comment.
fn comment_body(line: &str) -> Option<&str> {
    for mark in ["#", "//", "--", "::"] {
        if let Some(rest) = line.strip_prefix(mark) {
            return Some(rest);
        }
    }
    let head = line.get(..3)?;
    if head.eq_ignore_ascii_case("rem") {
        let rest = &line[3..];
        if rest.is_empty() || rest.starts_with([' ', '\t']) {
            return Some(rest);
        }
    }
    None
}

/// Reads one value as TOML, or says why not.
fn toml_value(text: &str) -> Result<Value, (String, String)> {
    let document = format!("value = {text}");
    match document.parse::<Table>() {
        Ok(mut table) => table
            .remove("value")
            .ok_or_else(|| ("is empty".to_owned(), "Put the value in quotes.".to_owned())),
        Err(_) => {
            let first = text.chars().next();
            let unquoted = match first {
                None => true,
                Some(c) => {
                    !(c == '"'
                        || c == '\''
                        || c == '['
                        || c == '{'
                        || c == '+'
                        || c == '-'
                        || c.is_ascii_digit())
                }
            };
            if unquoted {
                let shown = text.replace('"', "'");
                Err((
                    "is not in quotes".to_owned(),
                    format!("Put the value in quotes: \"{shown}\"."),
                ))
            } else {
                Err((
                    "is not a TOML value".to_owned(),
                    "Write quoted text, a whole number, true or false, or a list.".to_owned(),
                ))
            }
        }
    }
}

/// Reads a script's header: the `diro:` lines in its first 40 lines.
pub fn parse_header(text: &str) -> HeaderReport {
    let (found, mut problems) = settings(text);
    let count = found.len();
    let mut table = Table::new();
    let mut lines: BTreeMap<String, usize> = BTreeMap::new();
    for setting in found {
        let field = format!("{} (line {})", setting.key, setting.line);
        if let Some(first) = lines.get(&setting.key) {
            problems.push(Problem {
                severity: Severity::Error,
                field,
                message: format!("is set twice; line {first} sets it first"),
                fix: "Keep one of the two lines.".to_owned(),
            });
            continue;
        }
        let known = HEADER_KEYS.contains(&setting.key.as_str())
            || RECIPE_KEYS.contains(&setting.key.as_str());
        match toml_value(&setting.value) {
            Err((message, fix)) => problems.push(Problem {
                severity: Severity::Error,
                field,
                message,
                fix,
            }),
            Ok(_) if !known => problems.push(Problem {
                severity: Severity::Warning,
                field,
                message: "is not a header key, so it is ignored".to_owned(),
                fix: "Check its spelling, or remove the line.".to_owned(),
            }),
            Ok(value) => {
                lines.insert(setting.key.clone(), setting.line);
                table.insert(setting.key, value);
            }
        }
    }
    let label = |key: &str| match lines.get(key) {
        Some(line) => format!("{key} (line {line})"),
        None => key.to_owned(),
    };
    let mut header = ScriptHeader::default();
    let mut r = Reader::new(&table, &label);
    read_command_keys(&mut r, &mut header);
    problems.append(&mut r.problems);
    let (recipe, mut recipe_problems) = Recipe::read(&table, &label);
    header.recipe = recipe;
    problems.append(&mut recipe_problems);
    problems.sort_by_key(|p| p.severity);
    HeaderReport {
        header,
        problems,
        settings: count,
    }
}

/// Reads the command's own keys: name, description, key, menu, selection,
/// places, needs, os and protocol.
fn read_command_keys(r: &mut Reader<'_>, header: &mut ScriptHeader) {
    if let Some(name) = r.string("name") {
        match name_problem(&name) {
            Some(why) => r.error(
                "name",
                &why,
                "Use plain words, 40 characters at most, meaning first.",
            ),
            None => header.name = Some(name),
        }
    }
    if let Some(text) = r.string("description") {
        check_description(r, &text);
        header.description = text;
    }
    if let Some(key) = r.string("key") {
        if Key::parse(&key).is_err() {
            r.error(
                "key",
                &format!("\"{key}\" is not a key name"),
                "Write a key such as \"ctrl+alt+t\".",
            );
        } else {
            header.key = Some(key);
        }
    }
    header.menu = r.string("menu");
    if let Some(word) = r.word("selection", &["none", "one", "two", "any", "many"]) {
        header.selection = match word.as_str() {
            "none" => Selection::None,
            "one" => Selection::One,
            "two" => Selection::Two,
            "many" => Selection::Many,
            _ => Selection::Any,
        };
    }
    if let Some(word) = r.word("places", &["any", "local"]) {
        header.places = if word == "local" {
            Places::Local
        } else {
            Places::Any
        };
    }
    if let Some(needs) = r.strings("needs") {
        for tool in &needs {
            if !is_tool_name(tool) {
                r.error(
                    "needs",
                    &format!("\"{tool}\" is not a tool name"),
                    "Name the tool, not its path: letters, digits, - _ and . only.",
                );
            }
        }
        header.needs = needs.into_iter().filter(|t| is_tool_name(t)).collect();
    }
    if let Some(systems) = r.strings("os") {
        for word in systems {
            match word.as_str() {
                "windows" => header.os.push(Os::Windows),
                "linux" => header.os.push(Os::Linux),
                "macos" => header.os.push(Os::Macos),
                _ => r.error(
                    "os",
                    &format!("\"{word}\" is not a system"),
                    "Use windows, linux or macos.",
                ),
            }
        }
    }
    if let Some(n) = r.integer("protocol", 1, i64::from(u32::MAX)) {
        if n == i64::from(PROTOCOL) {
            header.protocol = PROTOCOL;
        } else {
            r.error(
                "protocol",
                &format!("the script speaks protocol {n}; diroxide speaks {PROTOCOL}"),
                "Use a version of the script made for this diroxide.",
            );
        }
    }
}

/// Why a name is not a plain name for people, or `None` when it is.
pub fn name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("is empty".to_owned());
    }
    if has_hidden(name) {
        return Some("holds a hidden or control character".to_owned());
    }
    if name.trim() != name {
        return Some("starts or ends with a space".to_owned());
    }
    let count = name.chars().count();
    (count > MAX_NAME_CHARS)
        .then(|| format!("is {count} characters, over the 40 of one Braille line"))
}

fn check_description(r: &mut Reader<'_>, text: &str) {
    if has_hidden(text) {
        r.error(
            "description",
            "holds a hidden or control character",
            "Use plain words.",
        );
        return;
    }
    let count = text.chars().count();
    if count > MAX_DESCRIPTION_CHARS {
        r.warn(
            "description",
            &format!("is {count} characters, over {MAX_DESCRIPTION_CHARS}"),
            "Keep it to one sentence.",
        );
    }
    let lower = text.to_lowercase();
    if let Some(filler) = FILLER.iter().find(|f| lower.starts_with(*f)) {
        r.warn(
            "description",
            &format!("starts with filler (\"{filler}\")"),
            "Say what it does first, within 40 characters.",
        );
    }
}

/// Whether text names a tool: letters, digits, `-`, `_` and `.`, at most
/// 40 characters.
pub fn is_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// The body of the `script_header` fuzz target: any bytes as a script.
/// The properties: no panic; every problem names its field and ends its
/// fix with a period; with no error, the name is a plain name of one
/// Braille line, and an unattended command has a timeout and asks
/// nothing.
pub fn fuzz_header(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let report = parse_header(&text);
    for problem in &report.problems {
        assert!(!problem.field.is_empty(), "{problem}");
        assert!(problem.fix.ends_with('.'), "{problem}");
        assert!(!problem.message.ends_with('.'), "{problem}");
    }
    if !report.has_errors() {
        if let Some(name) = &report.header.name {
            assert!(name_problem(name).is_none(), "{name}");
        }
        let recipe = &report.header.recipe;
        if recipe.unattended {
            assert!(recipe.timeout.is_some());
            assert!(recipe.prompt.is_none() && recipe.confirm.is_none());
            assert!(!recipe.mode.is_terminal());
        }
    }
    assert!(report.settings <= HEADER_LINES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::recipe::{ListForm, RunMode};
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A header in words, for the golden files: one line per value, then
    /// one line per problem.
    fn describe(report: &HeaderReport) -> String {
        let h = &report.header;
        let r = &h.recipe;
        let mut text = String::new();
        let _ = writeln!(text, "settings: {}", report.settings);
        let _ = writeln!(text, "name: {}", h.name.as_deref().unwrap_or("(file name)"));
        let _ = writeln!(text, "description: {}", h.description);
        let _ = writeln!(text, "key: {}", h.key.as_deref().unwrap_or("(none)"));
        let _ = writeln!(text, "selection: {:?}, places: {:?}", h.selection, h.places);
        let _ = writeln!(text, "needs: {:?}, os: {:?}", h.needs, h.os);
        let _ = writeln!(
            text,
            "mode: {}, per: {:?}, stdin: {:?}, cwd: {:?}",
            r.mode.word(),
            r.per,
            r.stdin,
            r.cwd
        );
        let _ = writeln!(
            text,
            "timeout: {}, selection_file: {}",
            crate::manifest::format_duration(r.timeout),
            r.selection_file.word()
        );
        let _ = writeln!(
            text,
            "confirm: {}, prompt: {}",
            r.confirm.as_deref().unwrap_or("(none)"),
            r.prompt.as_deref().unwrap_or("(none)")
        );
        let _ = writeln!(
            text,
            "unattended: {}, dry_run: {}, cursor_fallback: {}",
            r.unattended, r.dry_run, r.cursor_fallback
        );
        let _ = writeln!(
            text,
            "exit_words: {:?}, interpreter: {:?}",
            r.exit_words, r.interpreter
        );
        for problem in &report.problems {
            let _ = writeln!(text, "{problem}");
        }
        text
    }

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scripts/headers")
    }

    /// Every file under `fixtures/scripts/headers/` beside its `.golden`
    /// description; one per comment mark, and the problem cases.
    #[test]
    fn golden_headers_for_each_comment_mark() {
        let mut checked = 0;
        let mut names: Vec<PathBuf> = std::fs::read_dir(fixtures())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e != "golden"))
            .collect();
        names.sort();
        for script in names {
            let text = std::fs::read_to_string(&script).unwrap();
            let golden_path = script.with_extension(format!(
                "{}.golden",
                script.extension().unwrap().to_string_lossy()
            ));
            let made = describe(&parse_header(&text));
            // DIRO_BLESS=1 writes the golden files from what is made now,
            // for a reviewer to read in the diff.
            if std::env::var_os("DIRO_BLESS").is_some() {
                std::fs::write(&golden_path, &made).unwrap();
            }
            let golden = std::fs::read_to_string(&golden_path)
                .unwrap_or_else(|_| panic!("{} has no golden file", script.display()));
            assert_eq!(made, golden.replace("\r\n", "\n"), "{}", script.display());
            fuzz_header(text.as_bytes());
            checked += 1;
        }
        assert!(checked >= 6, "only {checked} fixtures");
    }

    #[test]
    fn each_comment_mark_reads() {
        for mark in ["#", "//", "--", "::", "REM", "rem", "Rem"] {
            let text = format!("{mark} diro: name = \"Make thumbnails\"\n");
            let report = parse_header(&text);
            assert_eq!(
                report.header.name.as_deref(),
                Some("Make thumbnails"),
                "{mark}"
            );
            assert!(report.problems.is_empty(), "{mark}: {:?}", report.problems);
        }
    }

    #[test]
    fn reading_stops_at_code_and_after_forty_lines() {
        let text =
            "#!/bin/sh\n\n# a comment\n# diro: name = \"A\"\necho hi\n# diro: key = \"ctrl+k\"\n";
        let report = parse_header(text);
        assert_eq!(report.header.name.as_deref(), Some("A"));
        assert_eq!(report.header.key, None);
        let mut long = "#\n".repeat(HEADER_LINES);
        long.push_str("# diro: name = \"Late\"\n");
        assert_eq!(parse_header(&long).header.name, None);
        let batch = "@echo off\r\nREM diro: name = \"Batch\"\r\n";
        assert_eq!(parse_header(batch).header.name.as_deref(), Some("Batch"));
        let bom = "\u{FEFF}# diro: name = \"Bom\"\n";
        assert_eq!(parse_header(bom).header.name.as_deref(), Some("Bom"));
        // A word that only starts like REM is code, not a comment.
        assert_eq!(parse_header("remove x\n# diro: name = \"N\"").settings, 0);
    }

    #[test]
    fn an_unquoted_word_says_put_it_in_quotes() {
        let report = parse_header("# diro: name = Make thumbnails\n");
        let problem = report.errors().next().unwrap();
        assert_eq!(
            problem.to_string(),
            "Error, name (line 1): is not in quotes. Put the value in quotes: \"Make thumbnails\"."
        );
        assert_eq!(report.header.name, None);
    }

    #[test]
    fn unknown_keys_warn_and_are_ignored() {
        let report = parse_header("# diro: colour = \"blue\"\n# diro: name = \"N\"\n");
        assert!(!report.has_errors());
        assert_eq!(
            report.problems[0].to_string(),
            "Warning, colour (line 1): is not a header key, so it is ignored. Check its spelling, or remove the line."
        );
    }

    #[test]
    fn every_key_of_section_two_six_reads() {
        let text = r#"#!/usr/bin/env bash
# diro: name = "Search the whole computer"
# diro: description = "Finds files by name with the index the system already keeps."
# diro: key = "ctrl+alt+f"
# diro: menu = "Plugins/Search"
# diro: selection = "none"
# diro: selection_file = "nul"
# diro: places = "local"
# diro: needs = ["plocate"]
# diro: mode = "lister"
# diro: per = "selection"
# diro: stdin = "none"
# diro: cwd = "other"
# diro: timeout = "60s"
# diro: interpreter = ["bash"]
# diro: stdout_directives = false
# diro: protocol = 1
# diro: prompt = "Search the whole computer for:"
# diro: dry_run = "supported"
# diro: cursor_fallback = false
# diro: os = ["linux"]
# diro: exit_words = { 8 = "Everything is not running. Start it, then try again." }
# diro: list = "nul"
# diro: max_rows = 5000
"#;
        let report = parse_header(text);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.settings, 23);
        let h = report.header;
        assert_eq!(h.selection, Selection::None);
        assert_eq!(h.places, Places::Local);
        assert_eq!(h.os, [Os::Linux]);
        assert_eq!(h.menu.as_deref(), Some("Plugins/Search"));
        assert_eq!(h.recipe.mode, RunMode::Lister);
        assert_eq!(h.recipe.list, ListForm::Nul);
        assert_eq!(h.recipe.timeout, Some(Duration::from_secs(60)));
        assert!(h.exists_on(Os::Linux) && !h.exists_on(Os::Windows));
        // confirm and unattended, the two keys left, read too.
        let more = parse_header(
            "# diro: confirm = \"Resize 3 images?\"\n# diro: unattended = \"ok\"\n# diro: timeout = \"1m\"\n",
        );
        assert!(more.problems.iter().any(|p| p.field.starts_with("confirm")));
        assert!(more.header.recipe.unattended);
    }

    #[test]
    fn a_key_set_twice_is_an_error() {
        let report = parse_header("# diro: name = \"A\"\n# diro: name = \"B\"\n");
        assert_eq!(report.header.name.as_deref(), Some("A"));
        assert_eq!(
            report.errors().next().unwrap().to_string(),
            "Error, name (line 2): is set twice; line 1 sets it first. Keep one of the two lines."
        );
    }

    #[test]
    fn a_name_over_forty_cells_is_refused() {
        let long = "A name that runs on past one Braille line of forty";
        let report = parse_header(&format!("# diro: name = \"{long}\""));
        assert!(report.has_errors());
        assert_eq!(report.header.name, None);
        assert_eq!(
            ScriptHeader::default().name_or("make_thumbs"),
            "make thumbs"
        );
    }
}
