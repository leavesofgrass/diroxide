//! carta, the pure-Rust converter built into diro (answer 27, pinned at
//! 0.0.13 by spike C2, answer 119), as the conversions' built-in tool
//! (R28 5.3; R30 commit 60). A recipe names it like any tool, `tool =
//! "carta"`, with the arguments `--from FORMAT --to FORMAT {input}
//! {output}`; the tools registry reports it found, "carta: 0.0.13, built
//! in", and refuses to start it, so the engine runs it here.
//!
//! - **Writers.** AsciiDoc, Typst, LaTeX, MediaWiki and Org first (whole
//!   documents, as textweaver writes them: a LaTeX preamble, an Org or
//!   AsciiDoc title), then plain text, GFM and CommonMark, then DOCX and
//!   EPUB, whose words matched pandoc's on read-back in spike C2. The ODT
//!   and RTF writers are off: they were not in C2's samples. The HTML
//!   writer is built for the quick view and not offered here.
//! - **Readers.** Markdown, Org, reStructuredText, MediaWiki, DokuWiki,
//!   Jira, HTML, DOCX, EPUB, ODT, CSV, TSV and LaTeX. The Typst reader is
//!   never built (it reads any file a document names), and every
//!   reStructuredText include is turned into a comment before carta sees
//!   the text ([`without_includes`], carried from textweaver's carta
//!   bridge under the owner's relicensing, answer 41). carta reads UTF-8
//!   only: other bytes are read with replacement characters.
//! - **Where it runs.** In process on a thread of its own with a time
//!   limit, or, for unattended runs, in a short-lived `diro` child the
//!   engine starts with [`CHILD_ARGUMENT`] and kills on its time limit or
//!   a cancel, since nothing can stop a thread: [`CartaRunner`]. The
//!   command line answers that argument with [`serve_child`].

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use diroxide_vfs::CancelToken;
use serde::{Deserialize, Serialize};

/// The hidden argument that makes `diro` a conversion child: it reads one
/// request on standard input, converts, answers on standard output and
/// ends. Not in the help: only the engine starts it.
pub const CHILD_ARGUMENT: &str = "--carta-convert-child";

/// What starts each answer line of the child, so the parent finds it
/// among anything else the program prints.
pub const ANSWER_MARK: &str = "diro-carta-child: ";

/// The carta writers conversions offer, by carta's name.
pub const WRITERS: [&str; 10] = [
    "asciidoc",
    "typst",
    "latex",
    "mediawiki",
    "org",
    "plain",
    "gfm",
    "commonmark",
    "docx",
    "epub",
];

/// The writers that make a whole document, title and preamble included.
const STANDALONE: [&str; 5] = ["asciidoc", "typst", "latex", "mediawiki", "org"];

/// The writers built but off for conversions, with the reason in words.
pub const WRITERS_OFF: [(&str, &str); 3] = [
    ("odt", "not in spike C2's samples"),
    ("rtf", "not in spike C2's samples"),
    ("html", "built for the quick view only"),
];

/// The formats conversions read with carta: a format word and carta's
/// reader for it. `markdown` is read as GitHub's flavor.
pub const READERS: [(&str, &str); 13] = [
    ("markdown", "gfm"),
    ("org", "org"),
    ("rst", "rst"),
    ("mediawiki", "mediawiki"),
    ("dokuwiki", "dokuwiki"),
    ("jira", "jira"),
    ("html", "html"),
    ("docx", "docx"),
    ("epub", "epub"),
    ("odt", "odt"),
    ("csv", "csv"),
    ("tsv", "tsv"),
    ("latex", "latex"),
];

/// The note a conversion gets when an include was left out.
pub const INCLUDE_NOTE: &str = "An include was left out, so the file it names was not read.";

/// One conversion carta makes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartaJob {
    /// The format read: a word of [`READERS`].
    pub from: String,
    /// The format written: one of [`WRITERS`].
    pub to: String,
    /// The input file.
    pub input: PathBuf,
    /// Where the output is written; it must not exist.
    pub output: PathBuf,
}

impl CartaJob {
    /// Reads a carta recipe's filled arguments: `--from FORMAT --to FORMAT
    /// INPUT OUTPUT`, the two options in either order, each checked
    /// against the formats offered.
    pub fn from_args(args: &[OsString]) -> Result<Self, String> {
        let mut from = None;
        let mut to = None;
        let mut paths = Vec::new();
        let mut rest = args.iter();
        while let Some(argument) = rest.next() {
            if argument == "--from" {
                from = rest.next().map(|a| a.to_string_lossy().into_owned());
            } else if argument == "--to" {
                to = rest.next().map(|a| a.to_string_lossy().into_owned());
            } else {
                paths.push(PathBuf::from(argument));
            }
        }
        let from = from.ok_or("carta's recipe has no --from")?;
        let to = to.ok_or("carta's recipe has no --to")?;
        if !READERS.iter().any(|(word, _)| *word == from) {
            return Err(format!("carta does not read {from} here"));
        }
        if !WRITERS.contains(&to.as_str()) {
            return Err(format!("carta does not write {to} here"));
        }
        let [input, output]: [PathBuf; 2] = paths
            .try_into()
            .map_err(|_| "carta's recipe needs one input and one output".to_owned())?;
        Ok(Self {
            from,
            to,
            input,
            output,
        })
    }
}

/// The source with every reStructuredText `.. include::` directive turned
/// into a comment, so carta never reads another file. Line based, so an
/// include written inside a literal block is commented out too.
pub fn without_includes(text: &str) -> Cow<'_, str> {
    let is_include = |line: &str| {
        line.trim_start()
            .strip_prefix("..")
            .map(str::trim_start)
            .and_then(|rest| rest.split_once("::"))
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("include"))
    };
    if !text.lines().any(is_include) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if is_include(line) {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(indent);
            out.push_str(".. diroxide left out an include\n");
        } else {
            out.push_str(line);
        }
    }
    Cow::Owned(out)
}

/// Converts bytes in carta, in this thread: decoded as UTF-8 first for a
/// text reader, includes left out of reStructuredText, read, then written
/// whole. The error is in words, for "Could not convert NAME: ".
pub fn convert_bytes(from: &str, to: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let reader = READERS
        .iter()
        .find(|(word, _)| *word == from)
        .map(|(_, reader)| *reader)
        .ok_or_else(|| format!("carta does not read {from} here"))?;
    if !WRITERS.contains(&to) {
        return Err(format!("carta does not write {to} here"));
    }
    let binary = matches!(reader, "docx" | "epub" | "odt");
    let text;
    let input: &[u8] = if binary {
        bytes
    } else {
        let decoded = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes));
        text = if reader == "rst" {
            without_includes(&decoded).into_owned()
        } else {
            decoded.into_owned()
        };
        text.as_bytes()
    };
    let failed = |e: carta::Error| format!("carta could not read it: {e}");
    let (document, media) =
        carta::read_document(reader, input, &carta::ReaderOptions::default()).map_err(failed)?;
    let mut options = carta::WriterOptions::default();
    options.standalone = STANDALONE.contains(&to);
    match carta::render_document(to, document, media, &options)
        .map_err(|e| format!("carta could not write {to}: {e}"))?
    {
        carta::Output::Text(text) => Ok(text.into_bytes()),
        carta::Output::Bytes(bytes) => Ok(bytes),
    }
}

/// Converts one file in this thread: the input read, the output written
/// as a new file, never over one.
pub fn convert_file(job: &CartaJob) -> Result<(), String> {
    let bytes = fs::read(&job.input).map_err(|e| format!("its input could not be read: {e}"))?;
    let made = convert_bytes(&job.from, &job.to, &bytes)?;
    write_new(&job.output, &made)
}

/// Writes a new file; a name that exists is never written over.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::File::create_new(path).map_err(|e| format!("its output: {e}"))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|e| format!("its output: {e}"))
}

/// The child's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildAnswer {
    /// The cause in words when the conversion failed; `None` when it
    /// succeeded.
    pub error: Option<String>,
}

/// The child's side, for the command line's hidden argument: reads one
/// [`CartaJob`] as a JSON line on `input`, converts, writes one answer
/// line starting with [`ANSWER_MARK`] on `output`, and gives the exit
/// code: 0 when converted, 1 when not, 2 for a request it cannot read.
pub fn serve_child(input: impl BufRead, mut output: impl Write) -> i32 {
    let mut line = String::new();
    let mut input = input;
    let job: Result<CartaJob, String> = input
        .read_line(&mut line)
        .map_err(|e| e.to_string())
        .and_then(|_| serde_json::from_str(line.trim()).map_err(|e| e.to_string()));
    let (code, answer) = match job {
        Err(error) => (
            2,
            ChildAnswer {
                error: Some(format!("the request could not be read: {error}")),
            },
        ),
        Ok(job) => match convert_file(&job) {
            Ok(()) => (0, ChildAnswer { error: None }),
            Err(error) => (1, ChildAnswer { error: Some(error) }),
        },
    };
    let text = serde_json::to_string(&answer).unwrap_or_else(|_| "{}".to_owned());
    let _ = writeln!(output, "{ANSWER_MARK}{text}");
    let _ = output.flush();
    code
}

/// Where carta runs for a conversion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CartaRunner {
    /// On a thread of the engine's, with the recipe's time limit; a run
    /// past it is abandoned, not stopped, and its output never written.
    #[default]
    InProcess,
    /// In a child the engine starts and can kill: `program` with `args`
    /// (the current `diro` and [`CHILD_ARGUMENT`]), and `env` set, for
    /// tests.
    Child {
        /// The program: the current executable.
        program: PathBuf,
        /// Its first arguments.
        args: Vec<OsString>,
        /// Extra environment, for tests; empty in the program.
        env: Vec<(OsString, OsString)>,
    },
}

impl CartaRunner {
    /// The child runner for this program: `program --carta-convert-child`.
    pub fn child(program: impl Into<PathBuf>) -> Self {
        CartaRunner::Child {
            program: program.into(),
            args: vec![OsString::from(CHILD_ARGUMENT)],
            env: Vec::new(),
        }
    }
}

/// How a carta run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CartaEnd {
    /// The output was written.
    Done,
    /// It failed, with the cause in words.
    Failed(String),
    /// It took longer than its limit and was stopped (or, in process,
    /// abandoned).
    TimedOut,
    /// The job was cancelled.
    Cancelled,
}

/// How often a waiting run looks at the clock and the cancel token.
const POLL: Duration = Duration::from_millis(20);

impl CartaRunner {
    /// Runs one conversion under a time limit and the job's cancel token.
    pub fn run(&self, job: &CartaJob, limit: Duration, cancel: &CancelToken) -> CartaEnd {
        match self {
            CartaRunner::InProcess => in_process(job, limit, cancel),
            CartaRunner::Child { program, args, env } => {
                in_child(program, args, env, job, limit, cancel)
            }
        }
    }
}

/// carta on a thread with a deep stack; the output is written here, only
/// when the thread answered in time, so an abandoned run never writes.
fn in_process(job: &CartaJob, limit: Duration, cancel: &CancelToken) -> CartaEnd {
    let bytes = match fs::read(&job.input) {
        Ok(bytes) => bytes,
        Err(error) => return CartaEnd::Failed(format!("its input could not be read: {error}")),
    };
    let (send, receive) = mpsc::channel();
    let (from, to) = (job.from.clone(), job.to.clone());
    let started = thread::Builder::new()
        .name("carta".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let _ = send.send(convert_bytes(&from, &to, &bytes));
        });
    if let Err(error) = started {
        return CartaEnd::Failed(format!("carta could not be started: {error}"));
    }
    let deadline = Instant::now() + limit;
    loop {
        if cancel.is_cancelled() {
            return CartaEnd::Cancelled;
        }
        match receive.recv_timeout(POLL) {
            Ok(Ok(made)) => {
                return match write_new(&job.output, &made) {
                    Ok(()) => CartaEnd::Done,
                    Err(error) => CartaEnd::Failed(error),
                };
            }
            Ok(Err(error)) => return CartaEnd::Failed(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return CartaEnd::Failed("carta stopped unexpectedly".to_owned());
            }
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                return CartaEnd::TimedOut;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// carta in a child: the request on its standard input, the answer read
/// from its standard output, the child killed on the time limit or a
/// cancel.
fn in_child(
    program: &Path,
    args: &[OsString],
    env: &[(OsString, OsString)],
    job: &CartaJob,
    limit: Duration,
    cancel: &CancelToken,
) -> CartaEnd {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CartaEnd::Failed(format!("carta's child could not be started: {error}"));
        }
    };
    let request = serde_json::to_string(job).unwrap_or_default();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{request}");
    }
    let (send, receive) = mpsc::channel::<Option<ChildAnswer>>();
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            let answer = BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .find_map(|line| {
                    line.strip_prefix(ANSWER_MARK)
                        .and_then(|json| serde_json::from_str::<ChildAnswer>(json).ok())
                });
            let _ = send.send(answer);
        });
    }
    let deadline = Instant::now() + limit;
    let end = loop {
        if cancel.is_cancelled() {
            break CartaEnd::Cancelled;
        }
        match receive.recv_timeout(POLL) {
            Ok(Some(ChildAnswer { error: None })) => break CartaEnd::Done,
            Ok(Some(ChildAnswer { error: Some(error) })) => break CartaEnd::Failed(error),
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                break CartaEnd::Failed("carta's child ended without an answer".to_owned());
            }
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                break CartaEnd::TimedOut;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    if end != CartaEnd::Done {
        let _ = child.kill();
    }
    let _ = child.wait();
    end
}

/// The words of a format for a recipe's name: "markdown" to "Markdown".
pub fn format_words(format: &str) -> &str {
    match format {
        "asciidoc" => "AsciiDoc",
        "typst" => "Typst",
        "latex" => "LaTeX",
        "mediawiki" => "MediaWiki",
        "org" => "Org",
        "plain" => "plain text",
        "gfm" => "GitHub Markdown",
        "commonmark" => "CommonMark",
        "docx" => "Word",
        "epub" => "EPUB",
        "markdown" => "Markdown",
        "rst" => "reStructuredText",
        other => other,
    }
}

/// Whether an argument names the hidden child mode.
pub fn is_child_argument(argument: &OsStr) -> bool {
    argument == CHILD_ARGUMENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::testing::scratch;
    use std::io::Read;

    /// A read of a child's whole answer stream.
    fn answer_of(stream: &mut dyn Read) -> Option<ChildAnswer> {
        let mut text = String::new();
        stream.read_to_string(&mut text).ok()?;
        text.lines()
            .find_map(|line| line.strip_prefix(ANSWER_MARK))
            .and_then(|json| serde_json::from_str(json).ok())
    }

    const NOTE: &str = "# Field notes\n\nHello from *Ada Example*.\n\n| Bird | Count |\n| --- | --- |\n| Crow | 3 |\n\n- one\n  - two\n";

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    /// Every writer conversions offer makes a document that keeps the
    /// words; the binary ones read back through carta's own readers.
    #[test]
    fn each_writer_keeps_the_words() {
        for to in WRITERS {
            let made = convert_bytes("markdown", to, NOTE.as_bytes()).unwrap();
            let words = match to {
                "docx" | "epub" => text(convert_bytes(to, "plain", &made).unwrap()),
                _ => text(made),
            };
            assert!(words.contains("Ada Example"), "{to}: {words}");
            assert!(words.contains("Crow"), "{to}: {words}");
        }
        let latex = text(convert_bytes("markdown", "latex", NOTE.as_bytes()).unwrap());
        assert!(
            latex.contains(r"\documentclass"),
            "a whole document: {latex}"
        );
        let org = text(convert_bytes("org", "gfm", b"* Heading\n\nText.\n").unwrap());
        assert!(org.contains("# Heading"), "{org}");
    }

    #[test]
    fn writers_and_readers_left_out_are_refused_in_words() {
        assert_eq!(
            convert_bytes("markdown", "odt", NOTE.as_bytes()),
            Err("carta does not write odt here".to_owned())
        );
        assert_eq!(
            convert_bytes("typst", "org", b"= Hi"),
            Err("carta does not read typst here".to_owned())
        );
        assert!(WRITERS_OFF.iter().any(|(w, _)| *w == "rtf"));
    }

    /// An include in reStructuredText is never followed: the file it names
    /// stays unread.
    #[test]
    fn a_restructured_text_include_is_left_out() {
        let root = scratch();
        let secret = root.path().join("secret.txt");
        fs::write(&secret, "the secret words").unwrap();
        let source = format!(
            "Title\n=====\n\n.. include:: {}\n\nAfter.\n",
            secret.to_string_lossy().replace('\\', "/")
        );
        assert!(matches!(without_includes(&source), Cow::Owned(_)));
        let made = text(convert_bytes("rst", "gfm", source.as_bytes()).unwrap());
        assert!(!made.contains("secret words"), "{made}");
        assert!(made.contains("After."), "{made}");
        assert!(matches!(
            without_includes("No include.\n"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn a_recipes_arguments_read_as_a_job() {
        let args: Vec<OsString> = ["--to", "org", "--from", "markdown", "in.md", "out.org"]
            .iter()
            .map(OsString::from)
            .collect();
        let job = CartaJob::from_args(&args).unwrap();
        assert_eq!(job.from, "markdown");
        assert_eq!(job.output, PathBuf::from("out.org"));
        let short: Vec<OsString> = ["--from", "markdown", "--to", "org", "in.md"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(
            CartaJob::from_args(&short),
            Err("carta's recipe needs one input and one output".to_owned())
        );
    }

    fn job(root: &Path, to: &str) -> CartaJob {
        fs::write(root.join("note.md"), NOTE).unwrap();
        CartaJob {
            from: "markdown".to_owned(),
            to: to.to_owned(),
            input: root.join("note.md"),
            output: root.join(format!("note.{to}")),
        }
    }

    #[test]
    fn in_process_writes_a_new_file_and_never_over_one() {
        let root = scratch();
        let job = job(root.path(), "org");
        let cancel = CancelToken::new();
        let runner = CartaRunner::InProcess;
        assert_eq!(
            runner.run(&job, Duration::from_secs(60), &cancel),
            CartaEnd::Done
        );
        assert!(text(fs::read(&job.output).unwrap()).contains("Ada Example"));
        match runner.run(&job, Duration::from_secs(60), &cancel) {
            CartaEnd::Failed(cause) => assert!(cause.starts_with("its output: "), "{cause}"),
            other => panic!("{other:?}"),
        }
        cancel.cancel();
        let again = CartaJob {
            output: root.path().join("again.org"),
            ..job
        };
        assert_eq!(
            runner.run(&again, Duration::from_secs(60), &cancel),
            CartaEnd::Cancelled
        );
        assert!(!again.output.exists());
    }

    /// The child's side through pipes in memory: one request in, one
    /// answer out.
    #[test]
    fn the_child_answers_one_request() {
        let root = scratch();
        let job = job(root.path(), "typst");
        let request = format!("{}\n", serde_json::to_string(&job).unwrap());
        let mut answer = Vec::new();
        assert_eq!(serve_child(request.as_bytes(), &mut answer), 0);
        assert_eq!(
            answer_of(&mut answer.as_slice()),
            Some(ChildAnswer { error: None })
        );
        assert!(job.output.exists());
        let mut again = Vec::new();
        assert_eq!(serve_child(request.as_bytes(), &mut again), 1);
        let said = answer_of(&mut again.as_slice()).unwrap().error.unwrap();
        assert!(said.starts_with("its output: "), "{said}");
        let mut bad = Vec::new();
        assert_eq!(serve_child(&b"not json\n"[..], &mut bad), 2);
    }

    /// What this test binary does when the test below starts it as the
    /// child: serve one request, or hang until killed. Run as an ordinary
    /// test it does nothing.
    #[test]
    fn fake_child() {
        match std::env::var("DIRO_CARTA_FAKE_CHILD").as_deref() {
            Ok("serve") => {
                let stdin = std::io::stdin();
                let code = serve_child(stdin.lock(), std::io::stdout());
                std::process::exit(code);
            }
            Ok("hang") => loop {
                thread::sleep(Duration::from_millis(50));
            },
            _ => {}
        }
    }

    /// This test binary, started as the fake child.
    fn fake_runner(mode: &str) -> CartaRunner {
        CartaRunner::Child {
            program: std::env::current_exe().unwrap(),
            args: [
                "convert::carta::tests::fake_child",
                "--exact",
                "--nocapture",
            ]
            .iter()
            .map(OsString::from)
            .collect(),
            env: vec![("DIRO_CARTA_FAKE_CHILD".into(), mode.into())],
        }
    }

    #[test]
    fn a_child_converts_and_a_stuck_one_is_killed() {
        let root = scratch();
        let job = job(root.path(), "asciidoc");
        let cancel = CancelToken::new();
        assert_eq!(
            fake_runner("serve").run(&job, Duration::from_secs(60), &cancel),
            CartaEnd::Done
        );
        assert!(text(fs::read(&job.output).unwrap()).contains("Ada Example"));
        let stuck = CartaJob {
            output: root.path().join("stuck.adoc"),
            ..job.clone()
        };
        let started = Instant::now();
        assert_eq!(
            fake_runner("hang").run(&stuck, Duration::from_millis(500), &cancel),
            CartaEnd::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(!stuck.output.exists());
        cancel.cancel();
        assert_eq!(
            fake_runner("hang").run(&stuck, Duration::from_secs(60), &cancel),
            CartaEnd::Cancelled
        );
        let program = CartaRunner::child("diro");
        assert!(matches!(&program, CartaRunner::Child { args, .. } if args[0] == CHILD_ARGUMENT));
        assert!(is_child_argument(OsStr::new("--carta-convert-child")));
    }
}
