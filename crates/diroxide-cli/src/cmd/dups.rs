//! `diro dups PATH...`: find duplicate files from a shell, in the
//! lister's words (alpha 5, lane CX; lane DU's engine).
//!
//! - The count line first ("3 groups, 7 files, 44 KB to free"), the
//!   notes on what was left out ("2 empty files left out"), then each
//!   group's row and its files, the name first.
//! - `--keep RULE` (oldest, the default helper, newest, shortest,
//!   name, first) or `--keep-in FOLDER` prints what a helper would mark,
//!   one line a file, and the groups it left alone. Nothing is ever
//!   removed from a shell in this alpha: the marked files are a list,
//!   for the trash in the app.
//! - `--format tsv` writes one line a file (group, size, path) and
//!   `--format json` one object a group; `-0` writes only the paths,
//!   NUL-separated, every member or with `--keep` the marked ones.
//! - A search that would read more than 10 GB asks first, starting on
//!   No; `--yes` reads without asking. Ctrl+C stops it.
//!
//! The full hash is BLAKE3, through `diroxide-meta`'s checksum reader,
//! the one the Checksum command uses. A file is called a duplicate only
//! on its full hash, or its bytes with `--bytes`.
//!
//! Exit codes: 0 done; 1 could not run, stopped, or a large read not
//! confirmed; 2 wrong use; 3 done, but some files could not be read.

use std::io::Write;
use std::sync::Mutex;

use diroxide_core::Place;
use diroxide_meta::checksum::{Algorithm, compute_reader};
use diroxide_meta::reader::MetaError;
use diroxide_ops::dups::{
    ContentHasher, DupEnv, DupError, DupEstimate, DupOptions, DupProgress, DupResult, DupSink,
    KeepRule, select,
};
use diroxide_ops::{JobControl, SystemPacer};
use diroxide_vfs::{CancelToken, VfsError};
use serde_json::json;

use crate::CliError;
use crate::cmd::archive::{NOT_ASKED, Session, line, lines, with_terminal};

/// How `diro dups` writes what it found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// The lister's words: the default.
    #[default]
    Text,
    /// One line a file: group, size in bytes, path, tab-separated.
    Tsv,
    /// One JSON object a group.
    Json,
}

/// `diro dups`'s arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DupsArgs {
    /// The folders, as typed.
    pub paths: Vec<String>,
    /// `--keep RULE` or `--keep-in FOLDER`, as typed.
    pub keep: Option<Keep>,
    /// `--format`.
    pub format: Format,
    /// `-0`: the paths only, NUL-separated.
    pub null: bool,
    /// `--empty`: group empty files too.
    pub empty: bool,
    /// `--bytes`: compare the bytes after the hash.
    pub bytes: bool,
    /// `--yes`: read without asking, however much.
    pub yes: bool,
    /// Print the help for `diro dups` and nothing else.
    pub help: bool,
}

/// A keep rule as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Keep {
    /// One of the named rules.
    Rule(KeepRule),
    /// `--keep-in FOLDER`, the folder as typed.
    InFolder(String),
}

/// The help for `diro dups`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro dups: find duplicate files, the count first, then each group.",
        "",
        "Usage:",
        "diro dups PATH...          find files with the same contents under PATH",
        "",
        "Options:",
        "--keep RULE                say what would be marked, keeping one copy a group:",
        "                           oldest, newest, shortest, name or first",
        "--keep-in FOLDER           keep the copies in FOLDER, mark the others",
        "--format FORMAT            text (the default), tsv or json",
        "-0, --null                 print only the paths, NUL-separated",
        "--empty                    group empty files too; left out by default",
        "--bytes                    compare the bytes too, after the hash",
        "--yes, -y                  read without asking, however much",
        "",
        "Nothing is removed: the marked files are a list, for the trash in the app.",
        "",
        "Exit codes:",
        "0  done",
        "1  could not run, stopped, or a large read not confirmed",
        "2  wrong use",
        "3  done, but some files could not be read",
    ]
    .join("\n")
}

/// A keep rule by its name.
pub fn keep_named(text: &str) -> Option<KeepRule> {
    match text.to_lowercase().as_str() {
        "oldest" => Some(KeepRule::Oldest),
        "newest" => Some(KeepRule::Newest),
        "shortest" | "shortest-path" => Some(KeepRule::ShortestPath),
        "name" | "first-by-name" => Some(KeepRule::FirstByName),
        "first" | "first-found" => Some(KeepRule::FirstFound),
        _ => None,
    }
}

/// Reads the arguments after `dups`.
pub fn parse(args: &[String]) -> Result<DupsArgs, CliError> {
    let mut parsed = DupsArgs::default();
    let mut only_paths = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_paths {
            parsed.paths.push(arg.clone());
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
            "--" => only_paths = true,
            "--help" | "-h" => parsed.help = true,
            "--keep" => {
                let text = value("--keep")?;
                let rule = keep_named(&text).ok_or_else(|| {
                    CliError::Failed(format!(
                        "there is no keep rule {text}; use oldest, newest, shortest, name or first"
                    ))
                })?;
                if parsed.keep.is_some() {
                    return Err(CliError::Failed("give one keep rule".to_owned()));
                }
                parsed.keep = Some(Keep::Rule(rule));
            }
            "--keep-in" => {
                let folder = value("--keep-in")?;
                if parsed.keep.is_some() {
                    return Err(CliError::Failed("give one keep rule".to_owned()));
                }
                parsed.keep = Some(Keep::InFolder(folder));
            }
            "--format" => {
                parsed.format = match value("--format")?.as_str() {
                    "text" | "words" => Format::Text,
                    "tsv" => Format::Tsv,
                    "json" => Format::Json,
                    other => {
                        return Err(CliError::Failed(format!(
                            "there is no {other} format; use text, tsv or json"
                        )));
                    }
                }
            }
            "-0" | "--null" => parsed.null = true,
            "--empty" => parsed.empty = true,
            "--bytes" => parsed.bytes = true,
            "--yes" | "-y" => parsed.yes = true,
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            path => parsed.paths.push(path.to_owned()),
        }
    }
    if parsed.help {
        return Ok(parsed);
    }
    if parsed.paths.is_empty() {
        return Err(CliError::Missing("dups".to_owned()));
    }
    if parsed.null && parsed.format != Format::Text {
        return Err(CliError::Failed(
            "-0 prints paths only; leave out --format".to_owned(),
        ));
    }
    Ok(parsed)
}

/// BLAKE3 over `diroxide-meta`'s checksum reader: the duplicate finder's
/// full hash, the same the Checksum command computes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Blake3Hasher;

impl ContentHasher for Blake3Hasher {
    fn name(&self) -> &'static str {
        "BLAKE3"
    }

    fn hash(
        &self,
        reader: &mut dyn std::io::Read,
        cancel: &CancelToken,
        on_step: &mut dyn FnMut(u64),
    ) -> Result<Vec<u8>, VfsError> {
        let digests =
            compute_reader(reader, &[Algorithm::Blake3], cancel, on_step).map_err(|e| match e {
                MetaError::Place(error) => error,
                other => VfsError::Corrupt {
                    what: other.to_string(),
                },
            })?;
        Ok(digests
            .into_iter()
            .next()
            .map(|digest| digest.bytes)
            .unwrap_or_default())
    }
}

/// The search's frontend at the shell: progress on standard error when
/// it is a terminal, the large-read question to whoever answers.
struct ShellSink<'a> {
    session: &'a Session<'a>,
    asked: Mutex<Option<String>>,
}

impl DupSink for ShellSink<'_> {
    fn progress(&self, progress: &DupProgress) {
        if self.session.progress {
            let _ = writeln!(std::io::stderr(), "{}", progress.describe());
        }
    }

    fn confirm(&self, estimate: &DupEstimate) -> bool {
        let question = estimate.question();
        if self.session.asker.present() {
            return self.session.asker.confirm(&question);
        }
        if let Ok(mut asked) = self.asked.lock() {
            *asked = Some(question);
        }
        false
    }
}

/// Runs `diro dups` on this computer's files and the terminal; Ctrl+C
/// stops the search.
pub fn run(args: &DupsArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    let control = JobControl::new();
    let cancel = control.cancel.clone();
    let _ = ctrlc::set_handler(move || cancel.cancel());
    with_terminal(|session| run_with(args, out, session, &control))
}

/// Runs `diro dups` in a session, stopped through `control`.
pub fn run_with(
    args: &DupsArgs,
    out: &mut dyn Write,
    session: &Session<'_>,
    control: &JobControl,
) -> Result<i32, CliError> {
    if args.help {
        line(out, &help_text())?;
        return Ok(0);
    }
    let mut roots = Vec::new();
    for typed in &args.paths {
        let root = session.resolve("find duplicates in", typed)?;
        // A root that is not there stops the search before it starts,
        // rather than counting as a file that could not be read.
        let found = session
            .registry
            .resolve(&root)
            .and_then(|provider| provider.stat(&root, &diroxide_vfs::OpContext::new()));
        if let Err(error) = found {
            let next = match error {
                VfsError::NotFound => "Check the path.".to_owned(),
                ref other => diroxide_core::Explain::next_step(other),
            };
            line(
                out,
                &format!(
                    "Could not find duplicates in {}: {error}. {next}",
                    diroxide_core::names::display_name(typed)
                ),
            )?;
            return Ok(1);
        }
        roots.push(root);
    }
    let keep = match &args.keep {
        Some(Keep::Rule(rule)) => Some(rule.clone()),
        Some(Keep::InFolder(typed)) => Some(KeepRule::InFolder(
            session.resolve("keep the copies in", typed)?,
        )),
        None => None,
    };
    let options = DupOptions {
        include_empty: args.empty,
        byte_compare: args.bytes,
        ask_above: if args.yes {
            None
        } else {
            DupOptions::default().ask_above
        },
        ..DupOptions::default()
    };
    let sink = ShellSink {
        session,
        asked: Mutex::new(None),
    };
    let pacer = SystemPacer::new();
    let hasher = Blake3Hasher;
    let env = DupEnv {
        registry: &session.registry,
        sink: &sink,
        hasher: &hasher,
        pacer: &pacer,
    };
    let result = match diroxide_ops::dups::find_duplicates(&roots, &options, control, &env) {
        Ok(result) => result,
        Err(error) => {
            if let Some(question) = sink.asked.lock().ok().and_then(|mut q| q.take()) {
                line(out, &question)?;
                line(out, NOT_ASKED)?;
            }
            let mut said = format!("{error}.");
            if matches!(error, DupError::Declined(_)) {
                said.push_str(" Add --yes to read it.");
            }
            line(out, &said)?;
            return Ok(1);
        }
    };
    let marked = keep.as_ref().map(|rule| select(&result, rule));
    if args.null {
        let places: Vec<&Place> = match &marked {
            Some(selection) => selection.marked.iter().collect(),
            None => result
                .groups
                .iter()
                .flat_map(|group| group.files.iter().map(|file| &file.place))
                .collect(),
        };
        for place in places {
            write!(out, "{place}\0").map_err(|e| CliError::Failed(e.to_string()))?;
        }
        out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
        return Ok(exit_code(&result));
    }
    match args.format {
        Format::Text => {
            lines(out, &text_lines(&result))?;
            if let Some(selection) = &marked {
                lines(out, &marked_lines(selection))?;
            }
        }
        Format::Tsv => {
            for (number, group) in result.groups.iter().enumerate() {
                let marked_here = |place: &Place| {
                    marked
                        .as_ref()
                        .is_some_and(|selection| selection.marked.contains(place))
                };
                for file in &group.files {
                    let state = if marked.is_none() {
                        String::new()
                    } else if marked_here(&file.place) {
                        "\tmarked".to_owned()
                    } else {
                        "\tkept".to_owned()
                    };
                    line(
                        out,
                        &format!("{}\t{}\t{}{state}", number + 1, group.size, file.place),
                    )?;
                }
            }
        }
        Format::Json => {
            for (number, group) in result.groups.iter().enumerate() {
                let files: Vec<serde_json::Value> = group
                    .files
                    .iter()
                    .map(|file| {
                        let mut object = json!({ "path": file.place.to_string() });
                        if let Some(selection) = &marked {
                            object["marked"] = json!(selection.marked.contains(&file.place));
                        }
                        object
                    })
                    .collect();
                let digest: String = group.digest.iter().map(|b| format!("{b:02x}")).collect();
                line(
                    out,
                    &json!({
                        "group": number + 1,
                        "size": group.size,
                        "hash": result.hasher,
                        "digest": digest,
                        "files": files,
                    })
                    .to_string(),
                )?;
            }
        }
    }
    Ok(exit_code(&result))
}

fn exit_code(result: &DupResult) -> i32 {
    if result.unreadable > 0 { 3 } else { 0 }
}

/// The words of a result: the count line, the notes, then each group's
/// row followed by its files' rows.
pub fn text_lines(result: &DupResult) -> Vec<String> {
    let mut said = vec![result.count_line()];
    said.extend(result.notes());
    let rows = result.group_rows();
    for (number, (group, row)) in result.groups.iter().zip(rows).enumerate() {
        said.push(row);
        said.extend((0..group.files.len()).filter_map(|file| group.file_row(file, number + 1)));
    }
    said
}

/// What a helper would mark: its summary, the groups it left alone, then
/// one line a marked file, the word first.
pub fn marked_lines(selection: &diroxide_ops::dups::Selection) -> Vec<String> {
    let mut said = vec![selection.summary()];
    said.extend(selection.left_alone_lines());
    said.extend(
        selection
            .marked
            .iter()
            .map(|place| format!("Marked: {}", place.title())),
    );
    said.push("Nothing was removed.".to_owned());
    said
}

#[cfg(test)]
mod tests;
