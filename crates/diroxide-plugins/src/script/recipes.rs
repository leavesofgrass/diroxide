//! A manifest's previewer and converter recipes, run the way the host
//! will run them (R22 sections 3.8 and 3.9): `diro plugin preview` and
//! `diro plugin convert` use them now; the preview region and the
//! conversions dialog come with a later alpha.
//!
//! - **Preview:** the input within the previewer's `max_size`, the tool
//!   run with no shell, its output cut at 64 KiB and cleaned of escape
//!   sequences and control characters, then the summary line (the first
//!   line with words) and the body. A failure is one sentence, the same
//!   the preview region will show.
//! - **Convert:** `{output}` is a staging path in the destination folder
//!   with the output's extension, so the last step is a rename, and a
//!   failed or refused conversion leaves nothing; an existing output is
//!   replaced only when the caller's question says yes.

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use diroxide_core::format::human_size;
use diroxide_tools::process::{ProcessSpec, Stream};
use toml::Value;

use crate::lines;
use crate::manifest::{Limit, Manifest};
use crate::script::placeholders::{Fill, PlaceholderError, expand};
use crate::script::runner::RunEnv;

/// The most preview text kept, in bytes.
pub const MAX_PREVIEW_BYTES: usize = 64 * 1024;

/// A previewer's input limit when its manifest gives none.
pub const DEFAULT_MAX_SIZE: u64 = 50 * 1000 * 1000;

/// A preview's time limit when the manifest gives none.
pub const PREVIEW_TIMEOUT: Duration = Duration::from_secs(5);

/// A preview, ready to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// The first line with words: the summary.
    pub summary: String,
    /// The whole text, a line a row.
    pub body: Vec<String>,
    /// Whether the text was cut at the limit.
    pub cut: bool,
}

/// Why a recipe gave nothing, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipeError {
    /// No previewer or converter has this id.
    NoSuch(String),
    /// A tool it needs is missing.
    Needs(String),
    /// The input is over the previewer's limit.
    TooLarge {
        /// The input's size.
        size: u64,
        /// The limit.
        max: u64,
    },
    /// The tool could not start.
    Start(String),
    /// The tool ended with a code other than 0.
    Failed {
        /// The tool's name.
        tool: String,
        /// Its exit code, if it had one.
        code: Option<i32>,
        /// The last line it wrote to standard error.
        last_error: Option<String>,
    },
    /// The tool ran past the time limit.
    TimedOut(Duration),
    /// The tool gave nothing.
    NoOutput(String),
    /// A placeholder could not be filled.
    Placeholder(PlaceholderError),
    /// The output is there already and was not replaced.
    Exists(String),
    /// Reading or writing failed.
    Io(String),
}

impl RecipeError {
    /// The sentences, meaning first: "Could not preview a.pdf: pdftotext
    /// ended with code 1." then "Last error: ...".
    pub fn lines(&self, verb: &str, file: &str) -> Vec<String> {
        match self {
            RecipeError::NoSuch(id) => vec![format!("Could not {verb} {file}: there is no {id}.")],
            RecipeError::Needs(tool) => vec![format!("Needs {tool}, not found.")],
            RecipeError::TooLarge { size, max } => vec![format!(
                "Could not {verb} {file}: it is {}, over the {} limit.",
                human_size(*size),
                human_size(*max)
            )],
            RecipeError::Start(cause) => vec![format!("Could not {verb} {file}: {cause}.")],
            RecipeError::Failed {
                tool,
                code,
                last_error,
            } => {
                let ended = match code {
                    Some(code) => format!("{tool} ended with code {code}"),
                    None => format!("{tool} was stopped"),
                };
                let mut out = vec![format!("Could not {verb} {file}: {ended}.")];
                if let Some(last) = last_error {
                    out.push(lines::last_error(last));
                }
                out
            }
            RecipeError::TimedOut(after) => vec![lines::timed_out(
                &format!("{} {file}", capitalized(verb)),
                *after,
            )],
            RecipeError::NoOutput(tool) => vec![format!("Nothing to show: {tool} gave no text.")],
            RecipeError::Placeholder(e) => vec![format!("Could not {verb} {file}: {}.", e.cause())],
            RecipeError::Exists(name) => vec![format!("Not converted: {name} exists.")],
            RecipeError::Io(cause) => vec![format!("Could not {verb} {file}: {cause}.")],
        }
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A recipe table's `needs`, read from the manifest's text (the
/// previewer's shape does not keep them).
pub fn needs_of(manifest_text: &str, table: &str, id: &str) -> Vec<String> {
    let Ok(document) = manifest_text.parse::<toml::Table>() else {
        return Vec::new();
    };
    document
        .get(table)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_table)
        .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|t| t.get("needs"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// What a tool wrote and how it ended.
struct Captured {
    code: Option<i32>,
    stdout: Vec<u8>,
    cut: bool,
    stderr: String,
    timed_out: bool,
}

/// Runs a program with an argument list (no shell, no console window, a
/// tree), keeping at most `cap` bytes of its output.
fn run_captured(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    limit: Limit,
    cap: usize,
) -> Result<Captured, String> {
    let spec = ProcessSpec::new(program)
        .args(args.iter().cloned())
        .cwd(cwd)
        .diroxide_environment()
        .stdin(Stream::Null)
        .stdout(Stream::Piped)
        .stderr(Stream::Piped);
    let mut child = spec.spawn().map_err(|e| e.to_string())?;
    let (out_send, out_receive) = mpsc::channel();
    if let Some(stdout) = child.take_stdout() {
        let _ = thread::Builder::new()
            .name("recipe-stdout".to_owned())
            .spawn(move || {
                let mut kept = Vec::new();
                let mut stream = stdout;
                let mut buffer = [0u8; 16 * 1024];
                let mut cut = false;
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let room = cap.saturating_sub(kept.len());
                            kept.extend_from_slice(&buffer[..n.min(room)]);
                            cut |= n > room;
                        }
                    }
                }
                let _ = out_send.send((kept, cut));
            });
    }
    let (err_send, err_receive) = mpsc::channel();
    if let Some(stderr) = child.take_stderr() {
        let _ = thread::Builder::new()
            .name("recipe-stderr".to_owned())
            .spawn(move || {
                let mut kept = Vec::new();
                let _ = stderr.take(64 * 1024).read_to_end(&mut kept);
                let _ = err_send.send(String::from_utf8_lossy(&kept).into_owned());
            });
    }
    let (code, timed_out) = match limit {
        Some(limit) => match child.wait_timeout(limit).map_err(|e| e.to_string())? {
            Some(status) => (status.code(), false),
            None => {
                let _ = child.kill_tree();
                (None, true)
            }
        },
        None => (child.wait().map_err(|e| e.to_string())?.code(), false),
    };
    let (stdout, cut) = out_receive
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    let stderr = err_receive
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    Ok(Captured {
        code,
        stdout,
        cut,
        stderr,
        timed_out,
    })
}

/// Text from a tool, with escape sequences and control characters
/// removed (tabs and line ends kept), carriage returns dropped.
pub fn clean_document(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1B}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7E}').contains(&c) {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\n' | '\t' => out.push(c),
            '\r' => {}
            c if c.is_control() || diroxide_core::names::classify(c).is_hidden() => {}
            c => out.push(c),
        }
    }
    out
}

/// The program of a filled argument list, found by the tools rule.
fn program_of(run: &[OsString], env: &RunEnv) -> Result<(PathBuf, Vec<OsString>), RecipeError> {
    let (first, rest) = run
        .split_first()
        .ok_or_else(|| RecipeError::Start("its argv is empty".to_owned()))?;
    let name = first.to_string_lossy().into_owned();
    let path = env.find_tool(&name).ok_or(RecipeError::Needs(name))?;
    Ok((path, rest.to_vec()))
}

fn tool_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Runs a previewer on a local file.
pub fn preview(
    manifest: &Manifest,
    manifest_text: &str,
    id: &str,
    file: &Path,
    env: &RunEnv,
) -> Result<Preview, RecipeError> {
    let previewer = manifest
        .previewers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| RecipeError::NoSuch(format!("previewer {id}")))?;
    for tool in needs_of(manifest_text, "previewer", id) {
        env.find_tool(&tool).ok_or(RecipeError::Needs(tool))?;
    }
    let size = fs::metadata(file)
        .map_err(|e| RecipeError::Io(e.to_string()))?
        .len();
    let max = previewer
        .matches
        .get("max_size")
        .and_then(Value::as_integer)
        .and_then(|n| u64::try_from(n).ok())
        .unwrap_or(DEFAULT_MAX_SIZE);
    if size > max {
        return Err(RecipeError::TooLarge { size, max });
    }
    let mut fill = Fill::new();
    fill.set("input", file.as_os_str());
    let run = expand(&previewer.argv, &fill).map_err(RecipeError::Placeholder)?;
    let (program, args) = program_of(&run, env)?;
    let folder = file.parent().unwrap_or(Path::new("."));
    let limit = manifest.timeouts.preview.or(Some(PREVIEW_TIMEOUT));
    let captured = run_captured(&program, &args, folder, limit, MAX_PREVIEW_BYTES)
        .map_err(RecipeError::Start)?;
    let tool = tool_name(&program);
    if captured.timed_out {
        return Err(RecipeError::TimedOut(limit.unwrap_or(PREVIEW_TIMEOUT)));
    }
    if captured.code != Some(0) {
        return Err(failed(tool, captured.code, &captured.stderr));
    }
    let text = clean_document(&String::from_utf8_lossy(&captured.stdout));
    let body: Vec<String> = text.lines().map(|l| l.trim_end().to_owned()).collect();
    let Some(summary) = body.iter().map(|l| l.trim()).find(|l| !l.is_empty()) else {
        return Err(RecipeError::NoOutput(tool));
    };
    Ok(Preview {
        summary: summary.to_owned(),
        body,
        cut: captured.cut,
    })
}

fn failed(tool: String, code: Option<i32>, stderr: &str) -> RecipeError {
    let last_error = stderr
        .lines()
        .map(|l| crate::services::clean_text(l, 120))
        .rfind(|l| !l.is_empty());
    RecipeError::Failed {
        tool,
        code,
        last_error,
    }
}

static STAGED: AtomicU64 = AtomicU64::new(0);

/// Runs a converter on a local file into `to_dir`. `replace` is asked,
/// with the output's path, when the output is there already. Returns the
/// output's path.
pub fn convert(
    manifest: &Manifest,
    id: &str,
    file: &Path,
    to_dir: &Path,
    replace: &mut dyn FnMut(&Path) -> bool,
    env: &RunEnv,
) -> Result<PathBuf, RecipeError> {
    let converter = manifest
        .converters
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| RecipeError::NoSuch(format!("converter {id}")))?;
    for tool in &converter.needs {
        env.find_tool(tool)
            .ok_or_else(|| RecipeError::Needs(tool.clone()))?;
    }
    let extension = converter
        .to
        .first()
        .and_then(|t| t.get("extension"))
        .and_then(Value::as_str)
        .unwrap_or("out")
        .trim_start_matches('.')
        .to_owned();
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".to_owned());
    let output = to_dir.join(format!("{stem}.{extension}"));
    let n = STAGED.fetch_add(1, Ordering::Relaxed);
    let staging = to_dir.join(format!(
        "{stem}-converting-{}-{n}.{extension}",
        std::process::id()
    ));
    let mut fill = Fill::new();
    fill.set("input", file.as_os_str())
        .set("output", staging.as_os_str());
    let run = expand(&converter.argv, &fill).map_err(RecipeError::Placeholder)?;
    let (program, args) = program_of(&run, env)?;
    let captured = run_captured(
        &program,
        &args,
        to_dir,
        manifest.timeouts.convert,
        64 * 1024,
    )
    .map_err(RecipeError::Start)?;
    let tool = tool_name(&program);
    let cleanup = |path: &Path| {
        if path.is_file() {
            let _ = fs::remove_file(path);
        }
    };
    if captured.timed_out {
        cleanup(&staging);
        return Err(RecipeError::TimedOut(
            manifest.timeouts.convert.unwrap_or_default(),
        ));
    }
    if captured.code != Some(0) {
        cleanup(&staging);
        return Err(failed(tool, captured.code, &captured.stderr));
    }
    if !staging.is_file() {
        return Err(RecipeError::NoOutput(tool));
    }
    if output.exists() && !replace(&output) {
        cleanup(&staging);
        return Err(RecipeError::Exists(
            output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));
    }
    fs::rename(&staging, &output).map_err(|e| {
        cleanup(&staging);
        RecipeError::Io(e.to_string())
    })?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Os;
    use crate::script::examples::examples;
    use crate::script::runner::tests::Bench;

    /// A manifest whose recipe runs a fake tool: the system's bash with a
    /// script of the bench, and an empty file named after the real tool
    /// on the bench's PATH, so `needs` finds it.
    fn fake(bench: &mut Bench, tool: &str, body: &str, table: &str) -> (Manifest, String) {
        let bin = bench.root.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) {
            format!("{tool}.exe")
        } else {
            tool.to_owned()
        };
        fs::write(bin.join(name), b"").unwrap();
        let path = std::env::join_paths([&bin]).unwrap();
        bench.env.lookup.path = Some(path.clone());
        bench.env.registry = bench.env.registry.clone().with_path(Some(path));
        let script = bench
            .root
            .path()
            .join("scripts")
            .join(format!("fake-{tool}.sh"));
        fs::write(&script, body).unwrap();
        let bash = system_bash();
        let quote = |s: &str| format!("\"{}\"", s.replace('\\', "/"));
        let argv = format!(
            "[{}, \"--noprofile\", \"--norc\", {}, {}]",
            quote(&bash),
            quote(&script.to_string_lossy()),
            table
        );
        let text = match tool {
            "pdftotext" => format!(
                "[plugin]\nid = \"fake\"\nname = \"Fake\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[[previewer]]\nid = \"first_page\"\nname = \"First page\"\nneeds = [\"pdftotext\"]\nargv = {argv}\n\n[previewer.match]\nextensions = [\"pdf\"]\nmax_size = 1000\n"
            ),
            _ => format!(
                "[plugin]\nid = \"fake\"\nname = \"Fake\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[[converter]]\nid = \"md_to_docx\"\nname = \"Markdown to Word\"\nneeds = [\"pandoc\"]\nargv = {argv}\n\n[converter.from]\nextensions = [\"md\"]\n\n[[converter.to]]\nid = \"docx\"\nname = \"Word document\"\nextension = \"docx\"\n"
            ),
        };
        let manifest = Manifest::check_for(&text, Os::current())
            .manifest
            .unwrap_or_else(|| panic!("{}", Manifest::check(&text).text()));
        (manifest, text)
    }

    /// The system's bash, as a path string, for the fake tools.
    fn system_bash() -> String {
        crate::script::interpreter::Lookup::system()
            .bash()
            .map(|b| b.program.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn input(bench: &Bench, name: &str, bytes: usize) -> PathBuf {
        let path = bench.root.path().join("work").join(name);
        fs::write(&path, vec![b'x'; bytes]).unwrap();
        path
    }

    #[test]
    fn a_preview_says_its_summary_first_and_cleans_the_text() {
        let Some(mut bench) = Bench::new() else {
            return;
        };
        let (manifest, text) = fake(
            &mut bench,
            "pdftotext",
            "printf '\\n  Annual report\\033[1m 2026\\033[0m\\r\\nPage one text\\n'\n",
            "\"{input}\"",
        );
        let file = input(&bench, "report.pdf", 100);
        let shown = preview(&manifest, &text, "first_page", &file, &bench.env).unwrap();
        assert_eq!(shown.summary, "Annual report 2026");
        assert_eq!(shown.body[2], "Page one text");
        assert!(!shown.cut);
    }

    #[test]
    fn a_preview_refuses_with_words() {
        let Some(mut bench) = Bench::new() else {
            return;
        };
        let (manifest, text) = fake(
            &mut bench,
            "pdftotext",
            "echo 'Syntax Error: not a PDF' >&2\nexit 1\n",
            "\"{input}\"",
        );
        let big = input(&bench, "big.pdf", 5000);
        let error = preview(&manifest, &text, "first_page", &big, &bench.env).unwrap_err();
        assert_eq!(
            error.lines("preview", "big.pdf"),
            ["Could not preview big.pdf: it is 5.0 KB, over the 1.0 KB limit."]
        );
        let small = input(&bench, "small.pdf", 10);
        let error = preview(&manifest, &text, "first_page", &small, &bench.env).unwrap_err();
        assert_eq!(
            error.lines("preview", "small.pdf"),
            [
                "Could not preview small.pdf: bash ended with code 1.",
                "Last error: Syntax Error: not a PDF."
            ]
        );
        let (quiet, quiet_text) = fake(&mut bench, "pdftotext", "exit 0\n", "\"{input}\"");
        let error = preview(&quiet, &quiet_text, "first_page", &small, &bench.env).unwrap_err();
        assert_eq!(
            error.lines("preview", "small.pdf"),
            ["Nothing to show: bash gave no text."]
        );
        bench.env.lookup.path = None;
        bench.env.registry = bench.env.registry.clone().with_path(None);
        let error = preview(&quiet, &quiet_text, "first_page", &small, &bench.env).unwrap_err();
        assert_eq!(
            error.lines("preview", "small.pdf"),
            ["Needs pdftotext, not found."]
        );
    }

    #[test]
    fn a_conversion_renames_into_place_and_leaves_nothing_on_failure() {
        let Some(mut bench) = Bench::new() else {
            return;
        };
        let (manifest, _) = fake(
            &mut bench,
            "pandoc",
            "printf 'docx from %s' \"$1\" > \"$2\"\n",
            "\"{input}\", \"{output}\"",
        );
        let source = input(&bench, "notes.md", 5);
        let work = bench.root.path().join("work");
        let mut never = |_: &Path| false;
        let made = convert(
            &manifest,
            "md_to_docx",
            &source,
            &work,
            &mut never,
            &bench.env,
        )
        .unwrap();
        assert_eq!(made, work.join("notes.docx"));
        assert!(fs::read_to_string(&made).unwrap().starts_with("docx from "));
        // A second run meets the output: asked, refused, nothing left.
        let error = convert(
            &manifest,
            "md_to_docx",
            &source,
            &work,
            &mut never,
            &bench.env,
        )
        .unwrap_err();
        assert_eq!(
            error.lines("convert", "notes.md"),
            ["Not converted: notes.docx exists."]
        );
        let mut always = |_: &Path| true;
        convert(
            &manifest,
            "md_to_docx",
            &source,
            &work,
            &mut always,
            &bench.env,
        )
        .unwrap();
        let (broken, _) = fake(
            &mut bench,
            "pandoc",
            "printf 'half' > \"$2\"\necho 'pandoc: cannot parse' >&2\nexit 64\n",
            "\"{input}\", \"{output}\"",
        );
        let other = input(&bench, "other.md", 5);
        let error = convert(
            &broken,
            "md_to_docx",
            &other,
            &work,
            &mut always,
            &bench.env,
        )
        .unwrap_err();
        assert_eq!(
            error.lines("convert", "other.md")[0],
            "Could not convert other.md: bash ended with code 64."
        );
        let left: Vec<String> = fs::read_dir(&work)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("converting"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        assert!(!work.join("other.docx").exists());
    }

    #[test]
    fn documents_lose_escapes_and_hidden_characters() {
        assert_eq!(
            clean_document("a\u{1b}[31mred\u{1b}[0m\r\nb\u{202e}c\u{7}\td"),
            "ared\nbc\td"
        );
    }

    #[test]
    fn the_example_recipes_read() {
        let pdf =
            fs::read_to_string(examples().join("pdf-first-page").join("plugin.toml")).unwrap();
        assert_eq!(needs_of(&pdf, "previewer", "first_page"), ["pdftotext"]);
        let manifest = Manifest::check_for(&pdf, Os::current()).manifest.unwrap();
        assert_eq!(manifest.previewers[0].argv[4], "{input}");
        let word =
            fs::read_to_string(examples().join("markdown-to-word").join("plugin.toml")).unwrap();
        let manifest = Manifest::check_for(&word, Os::current()).manifest.unwrap();
        assert_eq!(manifest.converters[0].needs, ["pandoc"]);
    }

    /// The real pdftotext and pandoc, when DIRO_TOOLS=1 says they are
    /// there (the container's CI list).
    #[test]
    fn the_real_tools_when_asked() {
        if std::env::var_os("DIRO_TOOLS").is_none() {
            eprintln!("set DIRO_TOOLS=1 to run the real pdftotext and pandoc");
            return;
        }
        let Some(bench) = Bench::new() else { return };
        let word =
            fs::read_to_string(examples().join("markdown-to-word").join("plugin.toml")).unwrap();
        let manifest = Manifest::check_for(&word, Os::current()).manifest.unwrap();
        let source = bench.root.path().join("work").join("notes.md");
        fs::write(&source, "# Notes\n\nSome words.\n").unwrap();
        let work = bench.root.path().join("work");
        let mut never = |_: &Path| false;
        let made = convert(
            &manifest,
            "md_to_docx",
            &source,
            &work,
            &mut never,
            &bench.env,
        )
        .unwrap();
        assert!(fs::metadata(made).unwrap().len() > 0);
    }
}
