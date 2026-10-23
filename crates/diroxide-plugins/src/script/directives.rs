//! Directives: what a script asks the host to do while it runs (R3
//! section 5.4).
//!
//! A script writes them to the file named by `DIRO_DIRECTIVES`, one a
//! line, UTF-8: `VERB` or `VERB arguments`, the arguments being the rest
//! of the line. A file channel cannot collide with output: a script that
//! lists a folder prints names, and a file called `diro:cd /elsewhere`
//! must not steer the panel. The `diro:` lines on standard output act as
//! directives only for a script whose header opts in with
//! `stdout_directives = true` (answer 94, R22's question 7).
//!
//! - `announce TEXT`, `warn TEXT`, `error TEXT`: said at once, at that
//!   importance, cleaned, with the script's name last ("Selected: 2,
//!   size: 12.4 MB. By Total size of selection."). `error` does not
//!   change the exit status.
//! - `progress N of M [TEXT]`: the job's sentence, at once.
//! - `refresh`: both panels list again, at once.
//! - `cd PLACE`, `cd_other PLACE`, `select PLACE`, `unselect PLACE`,
//!   `cursor PLACE`: wait until the script ends, because a panel that
//!   moves while the owner listens is hard to follow by ear.
//! - An unknown verb is logged and ignored.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use diroxide_core::Message;

use crate::lines;
use crate::services::{MAX_ANNOUNCE_CHARS, attribute, clean_text};

/// The longest directive line read, in bytes; a longer one is ignored.
pub const MAX_DIRECTIVE_BYTES: usize = 8 * 1024;

/// The prefix of a directive on standard output, for a script that opts
/// in.
pub const STDOUT_PREFIX: &str = "diro:";

/// One directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Directive {
    /// Say something.
    Announce(String),
    /// Say a warning.
    Warn(String),
    /// Say an error; the exit status is unchanged.
    Error(String),
    /// Report progress.
    Progress {
        /// Done so far.
        done: u64,
        /// In all.
        total: u64,
        /// What is being done, if said.
        text: Option<String>,
    },
    /// List both panels again.
    Refresh,
    /// Open a place in the active panel, when the script ends.
    Cd(String),
    /// Open a place in the other panel, when the script ends.
    CdOther(String),
    /// Mark an entry, when the script ends.
    Select(String),
    /// Unmark an entry, when the script ends.
    Unselect(String),
    /// Move the cursor to an entry, when the script ends.
    Cursor(String),
}

impl Directive {
    /// Whether it waits until the script ends.
    pub fn is_deferred(&self) -> bool {
        matches!(
            self,
            Directive::Cd(_)
                | Directive::CdOther(_)
                | Directive::Select(_)
                | Directive::Unselect(_)
                | Directive::Cursor(_)
        )
    }

    /// The place a deferred directive names.
    pub fn place(&self) -> Option<&str> {
        match self {
            Directive::Cd(p)
            | Directive::CdOther(p)
            | Directive::Select(p)
            | Directive::Unselect(p)
            | Directive::Cursor(p) => Some(p),
            _ => None,
        }
    }

    /// What the owner hears for it: cleaned, with `source` (the script's
    /// or the plugin's name) last. `None` for directives that only act.
    pub fn message(&self, source: &str) -> Option<Message> {
        let said = |text: &str| {
            let clean = clean_text(text, MAX_ANNOUNCE_CHARS);
            (!clean.is_empty()).then(|| attribute(&clean, source))
        };
        match self {
            Directive::Announce(text) => said(text).map(Message::info),
            Directive::Warn(text) => said(text).map(Message::warning),
            Directive::Error(text) => said(text).map(Message::error),
            Directive::Progress { done, total, text } => {
                let count = format!("{done} of {total}");
                let words = match text {
                    Some(t) => {
                        let clean = clean_text(t, 80);
                        if clean.is_empty() {
                            count
                        } else {
                            format!("{count}, {clean}")
                        }
                    }
                    None => count,
                };
                Some(Message::progress(lines::progress_line(&words, source)))
            }
            _ => None,
        }
    }
}

/// Why a line is not a directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectiveError {
    /// A verb this version does not know: logged and ignored.
    UnknownVerb(String),
    /// A known verb with arguments it cannot use.
    Malformed(String),
    /// Longer than [`MAX_DIRECTIVE_BYTES`].
    TooLong,
}

impl std::fmt::Display for DirectiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DirectiveError::UnknownVerb(verb) => {
                write!(f, "unknown directive {verb}, ignored")
            }
            DirectiveError::Malformed(verb) => {
                write!(
                    f,
                    "the {verb} directive is not written as it should be, ignored"
                )
            }
            DirectiveError::TooLong => write!(f, "a directive line is too long, ignored"),
        }
    }
}

/// Reads one directive line: `VERB` or `VERB arguments`. One space after
/// the verb is the separator; the rest is kept as written, except a final
/// carriage return.
pub fn parse_directive(line: &str) -> Result<Directive, DirectiveError> {
    if line.len() > MAX_DIRECTIVE_BYTES {
        return Err(DirectiveError::TooLong);
    }
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (verb, argument) = match line.split_once(' ') {
        Some((verb, rest)) => (verb, rest),
        None => (line, ""),
    };
    let verb = verb.trim_start();
    let text = |make: fn(String) -> Directive| {
        if argument.trim().is_empty() {
            Err(DirectiveError::Malformed(verb.to_owned()))
        } else {
            Ok(make(argument.to_owned()))
        }
    };
    match verb {
        "announce" => text(Directive::Announce),
        "warn" => text(Directive::Warn),
        "error" => text(Directive::Error),
        "cd" => text(Directive::Cd),
        "cd_other" => text(Directive::CdOther),
        "select" => text(Directive::Select),
        "unselect" => text(Directive::Unselect),
        "cursor" => text(Directive::Cursor),
        "refresh" if argument.trim().is_empty() => Ok(Directive::Refresh),
        "refresh" => Err(DirectiveError::Malformed(verb.to_owned())),
        "progress" => parse_progress(argument).ok_or(DirectiveError::Malformed(verb.to_owned())),
        other => Err(DirectiveError::UnknownVerb(clean_text(other, 40))),
    }
}

fn parse_progress(argument: &str) -> Option<Directive> {
    let mut words = argument.trim().splitn(4, ' ');
    let done: u64 = words.next()?.parse().ok()?;
    if words.next()? != "of" {
        return None;
    }
    let total: u64 = words.next()?.parse().ok()?;
    let text = words
        .next()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    Some(Directive::Progress { done, total, text })
}

/// A line of standard output as a directive: only when the script opted
/// in, and only a line that starts with exactly `diro:`. `None` means it
/// is output.
pub fn stdout_directive(line: &str, opted_in: bool) -> Option<Result<Directive, DirectiveError>> {
    if !opted_in {
        return None;
    }
    line.strip_prefix(STDOUT_PREFIX).map(parse_directive)
}

/// Follows the directives file while the script runs: each poll returns
/// the whole lines written since the last; [`DirectiveReader::finish`]
/// also returns a last line with no line end.
#[derive(Debug, Default)]
pub struct DirectiveReader {
    offset: u64,
    partial: Vec<u8>,
    skipping: bool,
}

impl DirectiveReader {
    /// A reader at the start of the file.
    pub fn new() -> Self {
        Self::default()
    }

    /// The whole lines written since the last poll.
    pub fn poll(&mut self, path: &Path) -> io::Result<Vec<String>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let length = file.metadata()?.len();
        if length < self.offset {
            // The script truncated the file: start again from its top.
            self.offset = 0;
            self.partial.clear();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        let mut lines = Vec::new();
        for byte in bytes {
            if byte == b'\n' {
                if !self.skipping {
                    lines.push(String::from_utf8_lossy(&self.partial).into_owned());
                }
                self.partial.clear();
                self.skipping = false;
            } else if self.partial.len() >= MAX_DIRECTIVE_BYTES {
                // Too long: dropped up to its line end.
                self.skipping = true;
                self.partial.clear();
            } else if !self.skipping {
                self.partial.push(byte);
            }
        }
        Ok(lines)
    }

    /// The last lines, once the script has ended, including one with no
    /// line end.
    pub fn finish(&mut self, path: &Path) -> io::Result<Vec<String>> {
        let mut lines = self.poll(path)?;
        if !self.partial.is_empty() && !self.skipping {
            lines.push(String::from_utf8_lossy(&self.partial).into_owned());
        }
        self.partial.clear();
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::{Importance, MessageKind};
    use proptest::prelude::*;
    use std::io::Write;

    #[test]
    fn every_verb_reads() {
        let cases = [
            (
                "announce Selected: 3",
                Directive::Announce("Selected: 3".into()),
            ),
            (
                "warn Skipped 2 items",
                Directive::Warn("Skipped 2 items".into()),
            ),
            (
                "error cannot read a.txt",
                Directive::Error("cannot read a.txt".into()),
            ),
            ("refresh", Directive::Refresh),
            ("cd /tmp/x y", Directive::Cd("/tmp/x y".into())),
            (
                "cd_other D:\\backup",
                Directive::CdOther("D:\\backup".into()),
            ),
            ("select /a/ b ", Directive::Select("/a/ b ".into())),
            ("unselect /a", Directive::Unselect("/a".into())),
            ("cursor /a/c.txt\r", Directive::Cursor("/a/c.txt".into())),
            (
                "progress 3 of 40 resizing images",
                Directive::Progress {
                    done: 3,
                    total: 40,
                    text: Some("resizing images".into()),
                },
            ),
            (
                "progress 50 of 3000",
                Directive::Progress {
                    done: 50,
                    total: 3000,
                    text: None,
                },
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_directive(line), Ok(expected), "{line}");
        }
    }

    #[test]
    fn wrong_lines_are_named_and_ignored() {
        assert_eq!(
            parse_directive("dance now"),
            Err(DirectiveError::UnknownVerb("dance".into()))
        );
        assert_eq!(
            parse_directive("progress three of 4"),
            Err(DirectiveError::Malformed("progress".into()))
        );
        assert_eq!(
            parse_directive("announce   "),
            Err(DirectiveError::Malformed("announce".into()))
        );
        assert_eq!(
            parse_directive("refresh now"),
            Err(DirectiveError::Malformed("refresh".into()))
        );
        let long = format!("announce {}", "x".repeat(MAX_DIRECTIVE_BYTES));
        assert_eq!(parse_directive(&long), Err(DirectiveError::TooLong));
        assert_eq!(
            DirectiveError::UnknownVerb("dance".into()).to_string(),
            "unknown directive dance, ignored"
        );
    }

    #[test]
    fn panel_moves_wait_and_words_go_at_once() {
        assert!(Directive::Cd("/x".into()).is_deferred());
        assert!(Directive::Select("/x".into()).is_deferred());
        assert!(Directive::Cursor("/x".into()).is_deferred());
        assert!(!Directive::Announce("x".into()).is_deferred());
        assert!(!Directive::Refresh.is_deferred());
        assert_eq!(Directive::Cd("/x".into()).place(), Some("/x"));
    }

    #[test]
    fn what_is_said_carries_the_source_last() {
        let source = "Total size of selection";
        let said = Directive::Announce("Selected: 2, size: 12.4 MB".into())
            .message(source)
            .unwrap();
        assert_eq!(
            said.text,
            "Selected: 2, size: 12.4 MB. By Total size of selection."
        );
        assert!(diroxide_core::format::within_cells(
            &said.text,
            "Selected: 2, size:"
        ));
        let warning = Directive::Warn("Skipped 2 items it could not read".into())
            .message(source)
            .unwrap();
        assert_eq!(warning.kind, MessageKind::Warning);
        let error = Directive::Error("cannot read a.txt".into())
            .message(source)
            .unwrap();
        assert_eq!(error.importance, Importance::High);
        let spoof = Directive::Announce("\u{1b}[2JMoved to the trash\u{202e}".into())
            .message(source)
            .unwrap();
        assert_eq!(
            spoof.text,
            "Moved to the trash. By Total size of selection."
        );
        let progress = Directive::Progress {
            done: 50,
            total: 3000,
            text: None,
        }
        .message(source)
        .unwrap();
        assert_eq!(progress.text, "50 of 3000 (Total size of selection)");
        assert!(Directive::Refresh.message(source).is_none());
        assert!(
            Directive::Announce("\u{7}".into())
                .message(source)
                .is_none()
        );
    }

    #[test]
    fn standard_output_steers_only_when_the_script_opts_in() {
        assert_eq!(stdout_directive("diro:cd /x", false), None);
        assert_eq!(
            stdout_directive("diro:cd /x", true),
            Some(Ok(Directive::Cd("/x".into())))
        );
        assert_eq!(stdout_directive("a file called diro:cd /x", true), None);
    }

    #[test]
    fn the_reader_follows_the_file_line_by_line() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("directives");
        let mut reader = DirectiveReader::new();
        assert!(reader.poll(&path).unwrap().is_empty());
        let mut file = File::create(&path).unwrap();
        file.write_all(b"announce one\nprogress 1 of").unwrap();
        file.flush().unwrap();
        assert_eq!(reader.poll(&path).unwrap(), ["announce one"]);
        file.write_all(b" 2\nselect /a\n").unwrap();
        file.flush().unwrap();
        assert_eq!(
            reader.poll(&path).unwrap(),
            ["progress 1 of 2", "select /a"]
        );
        file.write_all(b"cursor /b").unwrap();
        file.flush().unwrap();
        assert!(reader.poll(&path).unwrap().is_empty());
        assert_eq!(reader.finish(&path).unwrap(), ["cursor /b"]);
        let long = vec![b'x'; MAX_DIRECTIVE_BYTES + 10];
        file.write_all(&long).unwrap();
        file.write_all(b"\nrefresh\n").unwrap();
        file.flush().unwrap();
        assert_eq!(reader.poll(&path).unwrap(), ["refresh"]);
    }

    proptest! {
        #[test]
        fn any_line_reads_without_panic(line in ".{0,200}") {
            let _ = parse_directive(&line);
            let _ = stdout_directive(&line, true);
        }

        #[test]
        fn a_place_comes_back_as_written(place in "[^\r\n]{1,100}") {
            prop_assume!(!place.trim().is_empty());
            let line = format!("cd {place}");
            prop_assert_eq!(parse_directive(&line), Ok(Directive::Cd(place.clone())));
        }
    }
}
