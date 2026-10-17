//! `diro archive list|pack|unpack|test`: archives from a shell, in plain
//! lines, the name first, everything to standard output in order, as
//! `diro ls` writes them (W2g, plan commit 55).
//!
//! - `diro archive list PLACE` lists every member of an archive, or of a
//!   folder inside one (`course.zip!week1`), one a line, the member's path
//!   first; `-0` writes only the paths, each ended by NUL, for `xargs -0`.
//! - `diro archive pack ARCHIVE ITEM...` packs into a new archive whose
//!   format comes from its name; `diro archive pack ARCHIVE -` packs the
//!   NUL-separated paths on standard input (`--lines` for one a line).
//! - `diro archive unpack ARCHIVE [--into FOLDER]` unpacks into the
//!   current folder, or FOLDER, with a folder named after the archive when
//!   it holds several things at its top.
//! - `diro archive test ARCHIVE` reads every member and checks it.
//!
//! What a command will do is said before anything is written: the pack
//! and unpack plans' rows, the same rows the app's dialogs show. The
//! outcome lines are the runners' own ("Pack finished: 3 done.",
//! "Checked 3 of 3 entries, all good."). Exit codes as `diro ls` defines
//! them: 0 done, 1 nothing done, 2 wrong use, 3 partly done.
//!
//! Questions and passwords are asked in the terminal, never taken from the
//! command line (ADR-0009): a y-or-n question starts on No, a password is
//! typed into a masked prompt ("Password for course.zip (hidden): "), one
//! asterisk a character unless the `secret_echo` setting is silent. Keys
//! are read from the terminal itself, so standard input stays free for a
//! list of paths. With no terminal, nothing is asked: a question is
//! answered No and said so, and a locked archive says a password is needed.
//!
//! The jobs run on a worker thread while this thread says their progress
//! (on standard error, in a terminal, at most every ten seconds, as the
//! jobs line does in screen-reader mode) and answers what they ask. The
//! split and join subcommands share this machinery ([`Session`],
//! [`Asker`], [`Relay`]).

use std::io::{self, IsTerminal, Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use diroxide_archive::{
    Allowances, ArchiveError, ArchiveFormat, ArchiveProvider, ArchiveQuestions, CompressionLevel,
    Index, JobEnv, JobSecrets, Limits, MAX_LISTED, PackRequest, Question, RecordKind,
    UnpackRequest, capped_line, new_password_request, plan_pack, plan_unpack, run_pack, run_unpack,
    test_archive, wrong_password,
};
use diroxide_config::SplitPreset;
use diroxide_core::clock::describe_time;
use diroxide_core::format::{count, grouped, human_size};
use diroxide_core::names::display_name;
use diroxide_core::{
    Clock, ConflictAnswer, ConflictPolicy, Entry, EntryKind, ErrorAction, JobId, JobState, Message,
    Place, Secret, SecretRequest, Timestamp, UtcOffset, catalog,
};
use diroxide_ops::{Conflict, Engine, ItemError, JobControl, JobProgress, JobReport, JobSink};
use diroxide_vfs::{CancelToken, OpContext, Registry, SecretSource};
use serde_json::json;

use crate::CliError;
use crate::cmd::ls::Separator;
use crate::open::current_place;

// ---------------------------------------------------------------------
// Who answers: the terminal, or nobody.

/// Who answers the questions and passwords a command asks, on the
/// command's own thread. The terminal in a real run; a script in tests.
pub trait Asker: Send + Sync {
    /// Whether a person is there to answer: false outside a terminal.
    fn present(&self) -> bool;

    /// Asks a question that ends "y or n"; the answer starts on No. True
    /// for yes.
    fn confirm(&self, question: &str) -> bool;

    /// Asks for a secret with this prompt ("Password for course.zip
    /// (hidden): "); `None` when none was given.
    fn secret(&self, prompt: &str) -> Option<Secret>;

    /// Says a line beside the prompts: "The two passwords differ.
    /// Nothing was changed. Try again."
    fn tell(&self, line: &str);
}

/// Nobody to ask: every question is answered No and no secret is given.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAsker;

impl Asker for NoAsker {
    fn present(&self) -> bool {
        false
    }

    fn confirm(&self, _question: &str) -> bool {
        false
    }

    fn secret(&self, _prompt: &str) -> Option<Secret> {
        None
    }

    fn tell(&self, _line: &str) {}
}

/// The terminal: prompts on standard error, keys read from the terminal
/// itself in raw mode, so nothing typed is echoed but the asterisks.
#[derive(Clone, Copy, Debug)]
pub struct TerminalAsker {
    asterisks: bool,
}

impl TerminalAsker {
    /// The terminal, showing one asterisk a typed character when
    /// `asterisks`, nothing otherwise (the `secret_echo` setting).
    pub fn new(asterisks: bool) -> Self {
        Self { asterisks }
    }

    /// The terminal when standard error is one, else [`NoAsker`].
    pub fn if_present(asterisks: bool) -> Box<dyn Asker> {
        if io::stderr().is_terminal() {
            Box::new(Self::new(asterisks))
        } else {
            Box::new(NoAsker)
        }
    }
}

/// Raw mode for as long as it is held.
struct RawMode;

impl RawMode {
    fn enter() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// A key pressed, with Control held or not; `None` when the terminal
/// cannot be read.
fn next_key() -> Option<(crossterm::event::KeyCode, bool)> {
    use crossterm::event::{Event, KeyEventKind, KeyModifiers};
    loop {
        match crossterm::event::read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                return Some((key.code, key.modifiers.contains(KeyModifiers::CONTROL)));
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

impl Asker for TerminalAsker {
    fn present(&self) -> bool {
        true
    }

    fn confirm(&self, question: &str) -> bool {
        use crossterm::event::KeyCode;
        let mut err = io::stderr();
        let _ = write!(err, "{question} ");
        let _ = err.flush();
        let yes = match RawMode::enter() {
            Ok(_raw) => loop {
                match next_key() {
                    Some((KeyCode::Char('y' | 'Y'), false)) => break true,
                    Some((KeyCode::Char('n' | 'N') | KeyCode::Enter | KeyCode::Esc, _))
                    | Some((KeyCode::Char('c' | 'd'), true))
                    | None => break false,
                    Some(_) => {}
                }
            },
            Err(_) => false,
        };
        let _ = writeln!(err, "{}", if yes { "yes" } else { "no" });
        yes
    }

    fn secret(&self, prompt: &str) -> Option<Secret> {
        use crossterm::event::KeyCode;
        let mut err = io::stderr();
        let _ = write!(err, "{prompt}");
        let _ = err.flush();
        let mut secret = Secret::default();
        let given = match RawMode::enter() {
            Ok(_raw) => loop {
                match next_key() {
                    Some((KeyCode::Enter, _)) => break true,
                    Some((KeyCode::Esc, _)) | Some((KeyCode::Char('c' | 'd'), true)) | None => {
                        break false;
                    }
                    Some((KeyCode::Backspace, _)) => {
                        if secret.pop().is_some() && self.asterisks {
                            let _ = write!(err, "\u{8} \u{8}");
                        }
                    }
                    Some((KeyCode::Char(c), false)) => {
                        secret.push(c);
                        if self.asterisks {
                            let _ = write!(err, "*");
                        }
                    }
                    Some(_) => {}
                }
                let _ = err.flush();
            },
            Err(_) => false,
        };
        let _ = writeln!(err);
        given.then_some(secret)
    }

    fn tell(&self, line: &str) {
        let _ = writeln!(io::stderr(), "{line}");
    }
}

// ---------------------------------------------------------------------
// The relay between a job's thread and the command's.

/// What a job's thread hands the command's thread.
enum Event {
    Confirm(String, Sender<bool>),
    Secret(String, Sender<Option<Secret>>),
    Progress(JobProgress),
}

/// Carries a job's questions, passwords and progress to the command's
/// thread, which asks the terminal and says the progress: the job itself
/// never writes to the terminal. It is the job's sink (conflicts are
/// settled by the request's policy, and an error skips the item, which the
/// report says), its limit questions and its secret source.
#[derive(Debug, Default)]
pub struct Relay {
    sender: Mutex<Option<Sender<Event>>>,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Event::Confirm(..) => "Confirm",
            Event::Secret(..) => "Secret",
            Event::Progress(_) => "Progress",
        })
    }
}

impl Relay {
    /// A relay with no command listening yet: every question is No.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn install(&self, sender: Option<Sender<Event>>) {
        *self.sender.lock().unwrap_or_else(|e| e.into_inner()) = sender;
    }

    fn send(&self, event: Event) -> bool {
        self.sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|sender| sender.send(event).is_ok())
    }

    /// Asks a question ending "y or n" through the command's thread.
    pub fn confirm(&self, question: &str) -> bool {
        let (reply, answer) = mpsc::channel();
        self.send(Event::Confirm(question.to_owned(), reply)) && answer.recv().unwrap_or(false)
    }

    /// Asks for a secret through the command's thread.
    pub fn secret(&self, prompt: &str) -> Option<Secret> {
        let (reply, answer) = mpsc::channel();
        if !self.send(Event::Secret(prompt.to_owned(), reply)) {
            return None;
        }
        answer.recv().ok().flatten()
    }
}

impl SecretSource for Relay {
    fn ask(&self, request: &SecretRequest, cancel: &CancelToken) -> Option<Secret> {
        if cancel.is_cancelled() {
            return None;
        }
        if request.attempt >= 2 && !self.confirm(&wrong_password(request).text) {
            return None;
        }
        self.secret(&request.prompt_line())
    }
}

impl ArchiveQuestions for Relay {
    fn ask(&self, question: &Question) -> bool {
        self.confirm(&question.message().text)
    }
}

impl JobSink for Relay {
    fn progress(&self, progress: &JobProgress) {
        self.send(Event::Progress(progress.clone()));
    }

    fn conflict(&self, _conflict: &Conflict) -> ConflictAnswer {
        ConflictAnswer::skip()
    }

    fn error(&self, _error: &ItemError) -> ErrorAction {
        ErrorAction::Skip
    }
}

/// How often a running job's progress is said: the jobs line's interval
/// in screen-reader mode.
const PROGRESS_EVERY: Duration = Duration::from_secs(10);

/// Says a job's progress in words: a change of state at once (stalled,
/// paused, verifying), the running line at most every ten seconds, never
/// the start or the finish, which the plan and the summary say.
struct ProgressWords {
    started: Instant,
    last: Option<Instant>,
    state: JobState,
}

impl ProgressWords {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            last: None,
            state: JobState::Running,
        }
    }

    fn line(&mut self, progress: &JobProgress) -> Option<String> {
        let changed = progress.state != self.state;
        self.state = progress.state;
        let now = Instant::now();
        let quiet = match progress.state {
            JobState::Finished => return None,
            JobState::Running | JobState::Counting => {
                let since = self.last.unwrap_or(self.started);
                now.duration_since(since) < PROGRESS_EVERY
            }
            _ => !changed,
        };
        if quiet {
            return None;
        }
        self.last = Some(now);
        Some(progress.describe())
    }
}

// ---------------------------------------------------------------------
// The session: what every command needs from outside.

/// What the archive, split and join commands need from outside: the
/// providers (the archive provider among them), the clock for times in
/// words, the current folder, who answers, whether progress is said, and
/// the split presets. Tests give their own.
pub struct Session<'a> {
    /// Every place, archives included.
    pub registry: Arc<Registry>,
    /// The archive provider in `registry`.
    pub archives: Arc<ArchiveProvider>,
    /// The clock, for times in words.
    pub clock: &'a dyn Clock,
    /// The current folder, for relative paths.
    pub here: &'a Place,
    /// Who answers questions and passwords.
    pub asker: &'a dyn Asker,
    /// Whether progress is said on standard error.
    pub progress: bool,
    /// The split presets, the built-in ones with the user's merged in.
    pub presets: Vec<SplitPreset>,
}

impl std::fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("here", &self.here)
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

/// The registry with the local provider and the archive provider at the
/// default limits, connected to each other.
pub fn archive_registry() -> (Arc<Registry>, Arc<ArchiveProvider>) {
    ArchiveProvider::install(Registry::with_local(), Limits::default())
}

/// What a terminal run reads from the settings: the split presets and
/// whether the masked prompt shows asterisks. The real config folder;
/// tests never come here.
pub fn from_settings() -> (Vec<SplitPreset>, bool) {
    let settings = crate::open::paths(false)
        .map(|paths| diroxide_config::load_settings(&paths).value)
        .unwrap_or_default();
    let asterisks = settings.secret_echo != diroxide_app::config::SecretEcho::Silent;
    (settings.split_presets, asterisks)
}

/// Runs `body` with a session over the real places, the local clock, the
/// current folder and the terminal.
pub fn with_terminal<T>(
    body: impl FnOnce(&Session<'_>) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let clock = diroxide_app::startup::local_clock();
    let here = current_place()?;
    let (presets, asterisks) = from_settings();
    let asker = TerminalAsker::if_present(asterisks);
    let (registry, archives) = archive_registry();
    let session = Session {
        registry,
        archives,
        clock: &clock,
        here: &here,
        asker: asker.as_ref(),
        progress: io::stderr().is_terminal(),
        presets,
    };
    body(&session)
}

/// What a job asked that nobody was there to answer.
#[derive(Debug, Default)]
pub struct Unanswered {
    /// The questions, answered No.
    pub questions: Vec<String>,
    /// Whether a password was asked for.
    pub secret: bool,
}

impl Unanswered {
    /// Says them: each question with "Answered no", and why no password
    /// was taken.
    pub fn write(&self, out: &mut dyn Write) -> Result<(), CliError> {
        for question in &self.questions {
            line(out, question)?;
            line(out, NOT_ASKED)?;
        }
        if self.secret {
            line(out, "No password taken: passwords are typed in a terminal.")?;
        }
        Ok(())
    }
}

/// The line after a question nobody was there to answer.
pub const NOT_ASKED: &str = "Answered no: there is no terminal to ask.";

impl Session<'_> {
    /// Resolves a typed place against the current folder; the error says
    /// what could not be done to it.
    pub fn resolve(&self, verb: &str, text: &str) -> Result<Place, CliError> {
        Place::resolve(text, self.here)
            .map_err(|e| CliError::Failed(Message::from_error(verb, &display_name(text), &e).text))
    }

    /// The engine over the session's places, with the default options.
    pub fn engine(&self) -> Engine {
        Engine::new(self.registry.clone())
    }

    /// Asks a question on this thread, or, with nobody there, says it and
    /// that it was answered No.
    pub fn confirm(&self, question: &str, out: &mut dyn Write) -> Result<bool, CliError> {
        if self.asker.present() {
            Ok(self.asker.confirm(question))
        } else {
            line(out, question)?;
            line(out, NOT_ASKED)?;
            Ok(false)
        }
    }

    /// Runs `job` on a worker thread while this thread answers what it
    /// asks through `relay` and says its progress. Returns what the job
    /// returns, and what nobody was there to answer.
    pub fn drive<T: Send>(
        &self,
        relay: &Relay,
        unanswered: &mut Unanswered,
        job: impl FnOnce() -> T + Send,
    ) -> T {
        let (sender, events) = mpsc::channel();
        relay.install(Some(sender));
        let result = thread::scope(|scope| {
            let handle = scope.spawn(job);
            let mut words = ProgressWords::new();
            loop {
                match events.recv_timeout(Duration::from_millis(50)) {
                    Ok(Event::Confirm(question, reply)) => {
                        let yes = if self.asker.present() {
                            self.asker.confirm(&question)
                        } else {
                            unanswered.questions.push(question);
                            false
                        };
                        let _ = reply.send(yes);
                    }
                    Ok(Event::Secret(prompt, reply)) => {
                        let secret = if self.asker.present() {
                            self.asker.secret(&prompt)
                        } else {
                            unanswered.secret = true;
                            None
                        };
                        let _ = reply.send(secret);
                    }
                    Ok(Event::Progress(progress)) => {
                        if self.progress
                            && let Some(said) = words.line(&progress)
                        {
                            let _ = writeln!(io::stderr(), "{said}");
                        }
                    }
                    Err(RecvTimeoutError::Timeout) if handle.is_finished() => break,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            handle
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
        relay.install(None);
        result
    }
}

/// Writes one line.
pub fn line(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    writeln!(out, "{text}").map_err(write_failed)
}

/// Writes lines, then flushes, so the terminal's prompts come after them.
pub fn lines(out: &mut dyn Write, texts: &[String]) -> Result<(), CliError> {
    for text in texts {
        line(out, text)?;
    }
    out.flush().map_err(write_failed)
}

fn write_failed(error: io::Error) -> CliError {
    CliError::Failed(format!("the output could not be written: {error}"))
}

/// A report's exit code: 0 when everything was done, 3 when some items
/// were skipped or failed but some were done, 1 when nothing was done or
/// the job was cancelled.
pub fn report_code(report: &JobReport) -> i32 {
    let problems = !report.failed.is_empty() || !report.skipped.is_empty();
    if report.cancelled || (report.done.is_empty() && problems) {
        1
    } else if problems {
        3
    } else {
        0
    }
}

/// The lines a report adds after its summary: each failure with its
/// cause, the name first.
pub fn failure_lines(report: &JobReport) -> Vec<String> {
    report
        .failed
        .iter()
        .map(|(place, cause)| {
            let name = place
                .name()
                .map_or_else(|| place.to_string(), str::to_owned);
            format!("{}: {cause}", display_name(&name))
        })
        .collect()
}

// ---------------------------------------------------------------------
// The arguments.

/// What `diro archive list` was asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListArgs {
    /// The archive, or a folder inside one.
    pub place: String,
    /// More facts on each line.
    pub long: bool,
    /// One JSON object a line.
    pub json: bool,
    /// Hidden members too.
    pub hidden: bool,
    /// Only the paths, each ended by NUL.
    pub null: bool,
}

/// What `diro archive pack` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackArgs {
    /// The new archive.
    pub archive: String,
    /// What goes in; empty with `-`.
    pub items: Vec<String>,
    /// The items come from standard input, separated this way.
    pub stdin: Option<Separator>,
    /// How hard to compress.
    pub level: CompressionLevel,
    /// Ask for a password.
    pub password: bool,
    /// Hide a 7z's member names behind the password too.
    pub hide_names: bool,
    /// A 7z that is not solid.
    pub not_solid: bool,
    /// Replace an archive of the same name (the old one to the trash).
    pub replace: bool,
}

/// What `diro archive unpack` was asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnpackArgs {
    /// The archive, or a member or folder inside one to unpack alone.
    pub archive: String,
    /// Where to unpack; the current folder by default.
    pub into: Option<String>,
    /// Replace files of the same name (the old ones to the trash).
    pub replace: bool,
}

/// What `diro archive` was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveArgs {
    /// List the members.
    List(ListArgs),
    /// Pack into a new archive.
    Pack(PackArgs),
    /// Unpack.
    Unpack(UnpackArgs),
    /// Read and check every member.
    Test(String),
    /// Print the help.
    Help,
}

/// The help for `diro archive`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro archive: list, pack, unpack and test archives, one line each.",
        "",
        "Usage:",
        "diro archive list PLACE           list every member, the path first",
        "diro archive pack ARCHIVE ITEM... pack items; the name gives the format",
        "diro archive pack ARCHIVE -       pack the NUL-separated paths on input",
        "diro archive unpack ARCHIVE       unpack into the current folder",
        "diro archive test ARCHIVE         read every member and check it",
        "",
        "Formats: zip, 7z, tar, tar.gz, tar.xz and xz. PLACE may be inside an",
        "archive, as course.zip!week1, or the first part of a split one.",
        "",
        "Options for list:",
        "--long             add exact sizes, the packed size, CRC-32 and mode",
        "--json             write one JSON object a line, real names kept",
        "--hidden           list hidden members too, such as __MACOSX",
        "--null, -0         write only the paths, each ended by NUL",
        "",
        "Options for pack:",
        "--level LEVEL      fastest, fast, normal (the default), maximum, ultra",
        "--password         ask for a password, typed twice (zip and 7z)",
        "--hide-names       with --password, hide a 7z's member names too",
        "--not-solid        write a 7z that is not solid",
        "--lines            with -, read one path a line instead of NUL",
        "--replace          replace an archive of that name; the old goes to the trash",
        "",
        "Options for unpack:",
        "--into FOLDER      unpack there instead of the current folder",
        "--replace          replace files of the same name; the old go to the trash",
        "",
        "A taken name is kept and the new one numbered, unless --replace.",
        "Passwords are asked in the terminal, never on the command line.",
        "",
        "Exit codes:",
        "0  done",
        "1  nothing done: the archive could not be read or written",
        "2  wrong use, such as an unknown option",
        "3  partly done: some members were skipped or failed; the output says which",
    ]
    .join("\n")
}

/// The level for a typed word.
pub fn level_from(word: &str) -> Option<CompressionLevel> {
    match word.to_ascii_lowercase().as_str() {
        "fastest" => Some(CompressionLevel::Fastest),
        "fast" => Some(CompressionLevel::Fast),
        "normal" => Some(CompressionLevel::Normal),
        "maximum" | "max" => Some(CompressionLevel::Maximum),
        "ultra" => Some(CompressionLevel::Ultra),
        _ => None,
    }
}

/// Splits arguments into options and positionals: `--` ends the options,
/// and an option that takes a value reads the next argument (or its own
/// `=value`). Returns the positionals.
struct Options<'a> {
    rest: std::slice::Iter<'a, String>,
    only_positionals: bool,
}

/// One argument, read.
enum Arg<'a> {
    /// An option, with the value it was given after `=`.
    Flag(&'a str, Option<&'a str>),
    /// A positional, `-` included.
    Positional(&'a str),
}

impl<'a> Options<'a> {
    fn new(args: &'a [String]) -> Self {
        Self {
            rest: args.iter(),
            only_positionals: false,
        }
    }

    fn next_arg(&mut self) -> Option<Arg<'a>> {
        let arg = self.rest.next()?;
        if self.only_positionals || arg == "-" || !arg.starts_with('-') {
            return Some(Arg::Positional(arg));
        }
        if arg == "--" {
            self.only_positionals = true;
            return self.next_arg();
        }
        Some(match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => Arg::Flag(name, Some(value)),
            _ => Arg::Flag(arg, None),
        })
    }

    fn value(&mut self, flag: &str, given: Option<&'a str>) -> Result<&'a str, CliError> {
        match given.or_else(|| self.rest.next().map(String::as_str)) {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(CliError::Missing(flag.to_owned())),
        }
    }
}

fn is_help(arg: &Arg<'_>) -> bool {
    matches!(arg, Arg::Flag("--help" | "-h", None))
}

/// Reads the arguments after `archive`.
pub fn parse(args: &[String]) -> Result<ArchiveArgs, CliError> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(CliError::Missing("archive".to_owned()));
    };
    match sub.as_str() {
        "--help" | "-h" => Ok(ArchiveArgs::Help),
        "list" | "ls" => parse_list(rest),
        "pack" => parse_pack(rest),
        "unpack" => parse_unpack(rest),
        "test" => parse_test(rest),
        other if other.starts_with('-') => Err(CliError::UnknownOption(other.to_owned())),
        other => Err(CliError::Failed(format!(
            "archive has no subcommand {other}; use list, pack, unpack or test"
        ))),
    }
}

fn parse_list(args: &[String]) -> Result<ArchiveArgs, CliError> {
    let mut parsed = ListArgs::default();
    let mut place = None;
    let mut options = Options::new(args);
    while let Some(arg) = options.next_arg() {
        if is_help(&arg) {
            return Ok(ArchiveArgs::Help);
        }
        match arg {
            Arg::Flag("--long", None) => parsed.long = true,
            Arg::Flag("--json", None) => parsed.json = true,
            Arg::Flag("--hidden", None) => parsed.hidden = true,
            Arg::Flag("--null" | "-0", None) => parsed.null = true,
            Arg::Flag(flag, _) => return Err(CliError::UnknownOption(flag.to_owned())),
            Arg::Positional(_) if place.is_some() => {
                return Err(CliError::TooMany("archive list".to_owned()));
            }
            Arg::Positional(text) => place = Some(text.to_owned()),
        }
    }
    parsed.place = place.ok_or_else(|| CliError::Missing("archive list".to_owned()))?;
    if parsed.json && parsed.null {
        return Err(CliError::Failed(
            "--json and --null cannot be used together".to_owned(),
        ));
    }
    Ok(ArchiveArgs::List(parsed))
}

fn parse_pack(args: &[String]) -> Result<ArchiveArgs, CliError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut level = CompressionLevel::Normal;
    let (mut password, mut hide_names, mut not_solid, mut replace, mut lines) =
        (false, false, false, false, false);
    let mut options = Options::new(args);
    while let Some(arg) = options.next_arg() {
        if is_help(&arg) {
            return Ok(ArchiveArgs::Help);
        }
        match arg {
            Arg::Flag("--level", given) => {
                let word = options.value("--level", given)?;
                level = level_from(word).ok_or_else(|| {
                    CliError::Failed(format!(
                        "{word} is not a level; use fastest, fast, normal, maximum or ultra"
                    ))
                })?;
            }
            Arg::Flag("--password", None) => password = true,
            Arg::Flag("--hide-names", None) => hide_names = true,
            Arg::Flag("--not-solid", None) => not_solid = true,
            Arg::Flag("--replace", None) => replace = true,
            Arg::Flag("--lines", None) => lines = true,
            Arg::Flag(flag, _) => return Err(CliError::UnknownOption(flag.to_owned())),
            Arg::Positional(text) => positionals.push(text.to_owned()),
        }
    }
    let Some((archive, items)) = positionals.split_first() else {
        return Err(CliError::Missing("archive pack".to_owned()));
    };
    if items.is_empty() {
        return Err(CliError::Missing("archive pack".to_owned()));
    }
    let stdin = match items {
        [one] if one == "-" => Some(if lines {
            Separator::Lines
        } else {
            Separator::Nul
        }),
        _ if items.iter().any(|item| item == "-") => {
            return Err(CliError::Failed(
                "- reads the items from standard input, so it comes alone after the archive"
                    .to_owned(),
            ));
        }
        _ => None,
    };
    if lines && stdin.is_none() {
        return Err(CliError::Failed(
            "--lines only works with -, which reads the items from standard input".to_owned(),
        ));
    }
    if hide_names && !password {
        return Err(CliError::Failed(
            "--hide-names needs --password: the names are hidden behind it".to_owned(),
        ));
    }
    Ok(ArchiveArgs::Pack(PackArgs {
        archive: archive.clone(),
        items: if stdin.is_some() {
            Vec::new()
        } else {
            items.to_vec()
        },
        stdin,
        level,
        password,
        hide_names,
        not_solid,
        replace,
    }))
}

fn parse_unpack(args: &[String]) -> Result<ArchiveArgs, CliError> {
    let mut parsed = UnpackArgs::default();
    let mut archive = None;
    let mut options = Options::new(args);
    while let Some(arg) = options.next_arg() {
        if is_help(&arg) {
            return Ok(ArchiveArgs::Help);
        }
        match arg {
            Arg::Flag("--into", given) => {
                parsed.into = Some(options.value("--into", given)?.to_owned());
            }
            Arg::Flag("--replace", None) => parsed.replace = true,
            Arg::Flag(flag, _) => return Err(CliError::UnknownOption(flag.to_owned())),
            Arg::Positional(_) if archive.is_some() => {
                return Err(CliError::TooMany("archive unpack".to_owned()));
            }
            Arg::Positional(text) => archive = Some(text.to_owned()),
        }
    }
    parsed.archive = archive.ok_or_else(|| CliError::Missing("archive unpack".to_owned()))?;
    Ok(ArchiveArgs::Unpack(parsed))
}

fn parse_test(args: &[String]) -> Result<ArchiveArgs, CliError> {
    let mut archive = None;
    let mut options = Options::new(args);
    while let Some(arg) = options.next_arg() {
        if is_help(&arg) {
            return Ok(ArchiveArgs::Help);
        }
        match arg {
            Arg::Flag(flag, _) => return Err(CliError::UnknownOption(flag.to_owned())),
            Arg::Positional(_) if archive.is_some() => {
                return Err(CliError::TooMany("archive test".to_owned()));
            }
            Arg::Positional(text) => archive = Some(text.to_owned()),
        }
    }
    archive
        .map(ArchiveArgs::Test)
        .ok_or_else(|| CliError::Missing("archive test".to_owned()))
}

// ---------------------------------------------------------------------
// Running.

/// Runs `diro archive`, in the terminal, over the real places.
pub fn run(args: &ArchiveArgs, input: &mut dyn Read, out: &mut dyn Write) -> Result<i32, CliError> {
    if *args == ArchiveArgs::Help {
        line(out, &help_text())?;
        return Ok(0);
    }
    with_terminal(|session| run_with(args, input, out, session))
}

/// [`run`] in a session the caller gives.
pub fn run_with(
    args: &ArchiveArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    session: &Session<'_>,
) -> Result<i32, CliError> {
    let code = match args {
        ArchiveArgs::Help => {
            line(out, &help_text())?;
            0
        }
        ArchiveArgs::List(list_args) => list(list_args, out, session)?,
        ArchiveArgs::Pack(pack_args) => pack(pack_args, input, out, session)?,
        ArchiveArgs::Unpack(unpack_args) => unpack(unpack_args, out, session)?,
        ArchiveArgs::Test(archive) => test(archive, out, session)?,
    };
    out.flush().map_err(write_failed)?;
    Ok(code)
}

/// The archive's file and the member path inside it ("" at its root).
fn container_of(place: &Place) -> (Place, String) {
    match place.archive() {
        Some((container, member)) => (container.clone(), member.to_owned()),
        None => (place.clone(), String::new()),
    }
}

/// A place's name as the lines say it.
fn name_of(place: &Place) -> String {
    place.name().map_or_else(
        || display_name(&place.to_string()).into_owned(),
        |name| display_name(name).into_owned(),
    )
}

/// "Could not VERB NAME: CAUSE. NEXT STEP." for an archive call, or the
/// limit's question when it is one.
fn archive_failed(verb: &str, name: &str, error: &ArchiveError) -> CliError {
    CliError::Failed(error.message(verb, name).text)
}

/// Opens an archive on a job thread, asking a limit's question and the
/// password (three tries) through the relay.
fn open(
    session: &Session<'_>,
    container: &Place,
    unanswered: &mut Unanswered,
) -> Result<Arc<diroxide_archive::Opened>, ArchiveError> {
    let relay = Relay::new();
    let source: Arc<dyn SecretSource> = relay.clone();
    let secrets = JobSecrets::new(source);
    let archives = &session.archives;
    session.drive(&relay, unanswered, || {
        let mut allow = Allowances::default();
        loop {
            let opened = secrets.with_tries(&OpContext::new(), |ctx| {
                archives.open_archive(container, ctx, allow)
            });
            match opened {
                Err(ArchiveError::Ask(question)) if relay.confirm(&question.message().text) => {
                    allow = allow.with(question.kind);
                }
                other => return other,
            }
        }
    })
}

/// The members under `start`, depth first, each folder before what it
/// holds, in archive order within a folder.
fn members_under(index: &Index, start: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut stack: Vec<u32> = index.children(start).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        found.push(id);
        if found.len() > MAX_LISTED {
            break;
        }
        stack.extend(index.children(id).iter().rev());
    }
    found
}

/// One member's line: its path, why its name needs care, its other facts,
/// its kind, size and date; with `long` the exact size, the packed size,
/// the CRC-32 and the mode after them.
pub fn member_line(path: &str, entry: &Entry, long: bool, clock: &dyn Clock) -> String {
    let mut parts = vec![display_name(path).into_owned()];
    if let Some(care) = entry.extra.get(diroxide_archive::index::EXTRA_CARE) {
        parts.push(format!("name needs care: {care}"));
    } else if entry.needs_care {
        parts.push("name needs care".to_owned());
    }
    if let Some(note) = entry.extra.get(diroxide_archive::index::EXTRA_NOTE) {
        parts.push(note.clone());
    }
    parts.push(entry.kind.word().to_owned());
    if let Some(size) = entry.size
        && entry.kind != EntryKind::Folder
    {
        parts.push(human_size(size));
    }
    if let Some(modified) = entry.modified {
        parts.push(format!("modified {}", describe_time(modified, clock)));
    }
    if long {
        if let Some(size) = entry.size
            && entry.kind != EntryKind::Folder
        {
            parts.push(exact_bytes(size));
        }
        if let Some(packed) = entry.packed_size {
            parts.push(format!("packed {}", human_size(packed)));
        }
        if let Some(crc) = entry.crc32 {
            parts.push(format!("CRC-32 {crc:08X}"));
        }
        if let Some(attributes) = entry.attributes {
            parts.push(format!(
                "permissions: {}",
                attributes.short_words(entry.kind)
            ));
        }
        if let Some(link) = &entry.link {
            parts.push(format!("points to {}", display_name(&link.target)));
        }
    }
    parts.join(", ")
}

/// "2,104,331 bytes", "1 byte".
fn exact_bytes(bytes: u64) -> String {
    if bytes == 1 {
        "1 byte".to_owned()
    } else {
        format!("{} bytes", grouped(bytes))
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

/// One member's JSON object: the real path and name, null where a fact is
/// not known.
pub fn member_json(path: &str, entry: &Entry) -> serde_json::Value {
    json!({
        "path": path,
        "name": entry.name,
        "display_name": entry.display_name(),
        "kind": entry.kind.word(),
        "size": entry.size,
        "packed_size": entry.packed_size,
        "modified": entry.modified.map(rfc3339),
        "crc32": entry.crc32.map(|crc| format!("{crc:08X}")),
        "hidden": entry.hidden,
        "needs_care": entry.needs_care,
        "care": entry.extra.get(diroxide_archive::index::EXTRA_CARE),
        "note": entry.extra.get(diroxide_archive::index::EXTRA_NOTE),
        "link_target": entry.link.as_ref().map(|link| link.target.clone()),
    })
}

fn list(args: &ListArgs, out: &mut dyn Write, session: &Session<'_>) -> Result<i32, CliError> {
    let place = session.resolve("list", &args.place)?;
    let (container, member) = container_of(&place);
    let name = name_of(&container);
    let mut unanswered = Unanswered::default();
    let opened = open(session, &container, &mut unanswered);
    unanswered.write(out)?;
    let opened = opened.map_err(|e| archive_failed("list", &name, &e))?;
    let index = opened.index();
    let Some(start) = index.find(&member) else {
        return Err(CliError::Failed(format!(
            "Could not list {}: it is not in {name}. Check the path.",
            display_name(&member)
        )));
    };
    let ids = match index.record(start) {
        Some(record) if record.kind != RecordKind::Folder => vec![start],
        _ => members_under(index, start),
    };
    let capped = ids.len() > MAX_LISTED;
    let (mut listed, mut hidden, mut bytes) = (0u64, 0u64, 0u64);
    for &id in ids.iter().take(MAX_LISTED) {
        let Some(entry) = index.entry(id) else {
            continue;
        };
        if entry.hidden && !args.hidden {
            hidden += 1;
            continue;
        }
        let path = index.path(id);
        listed += 1;
        if entry.kind != EntryKind::Folder {
            bytes = bytes.saturating_add(entry.size.unwrap_or(0));
        }
        if args.null {
            out.write_all(path.as_bytes())
                .and_then(|()| out.write_all(&[0]))
                .map_err(write_failed)?;
        } else if args.json {
            line(out, &member_json(&path, &entry).to_string())?;
        } else {
            line(out, &member_line(&path, &entry, args.long, session.clock))?;
        }
    }
    let summary = index.summary();
    if !args.null && !args.json {
        if capped {
            line(out, &capped_line())?;
        }
        for note in &summary.notes {
            line(out, note)?;
        }
        for refused in &summary.refused_lines {
            line(out, refused)?;
        }
        let shown = summary.refused_lines.len() as u64;
        if summary.refused > shown {
            line(
                out,
                &format!(
                    "And {} refused.",
                    count(summary.refused - shown, "more name", "more names")
                ),
            )?;
        }
        let within = if member.is_empty() {
            name.clone()
        } else {
            format!("{}, in {name}", display_name(&member))
        };
        let mut said = format!(
            "{} in {within}, {}",
            count(listed, "item", "items"),
            human_size(bytes)
        );
        if hidden > 0 {
            said.push_str(&format!(", {hidden} hidden not shown"));
        }
        said.push('.');
        line(out, &capitalized(&said))?;
    }
    Ok(if summary.refused > 0 || capped { 3 } else { 0 })
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The items of a pack from standard input: NUL-separated by default,
/// one a line with `--lines`, as `diro ls -` reads them.
fn stdin_items(
    input: &mut dyn Read,
    separator: Separator,
    out: &mut dyn Write,
) -> Result<Option<Vec<String>>, CliError> {
    let mut bytes = Vec::new();
    input
        .take(crate::cmd::ls::MAX_LIST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| CliError::Failed(format!("standard input could not be read: {e}")))?;
    if bytes.len() as u64 > crate::cmd::ls::MAX_LIST_BYTES {
        line(out, "List is too long: over 64 MB. Nothing was packed.")?;
        return Ok(None);
    }
    let pieces: Vec<&[u8]> = match separator {
        Separator::Nul => {
            if !bytes.contains(&0) && bytes.contains(&b'\n') {
                line(out, "No NUL separators found. Use --lines.")?;
                return Ok(None);
            }
            bytes.split(|b| *b == 0).filter(|i| !i.is_empty()).collect()
        }
        Separator::Lines => {
            if bytes.contains(&0) {
                line(out, "The list has NUL separators. Leave out --lines.")?;
                return Ok(None);
            }
            bytes
                .split(|b| *b == b'\n')
                .map(|piece| piece.strip_suffix(b"\r").unwrap_or(piece))
                .filter(|piece| !piece.is_empty())
                .collect()
        }
    };
    let mut items = Vec::with_capacity(pieces.len());
    for (n, piece) in pieces.iter().enumerate() {
        match std::str::from_utf8(piece) {
            Ok(text) => items.push(text.to_owned()),
            Err(_) => {
                line(
                    out,
                    &format!(
                        "Could not pack item {}: the name is not valid Unicode. Nothing was packed.",
                        n + 1
                    ),
                )?;
                return Ok(None);
            }
        }
    }
    if items.is_empty() {
        line(out, "List is empty. Nothing to pack.")?;
        return Ok(None);
    }
    Ok(Some(items))
}

/// A new password for `name`, typed twice; three tries. `Err` with the
/// sentence to say when none was set.
fn new_password(asker: &dyn Asker, name: &str) -> Result<Secret, String> {
    if !asker.present() {
        return Err(format!(
            "Could not pack {name}: a password is typed in a terminal. Nothing was packed."
        ));
    }
    let request = new_password_request(name);
    for _ in 0..SecretRequest::ATTEMPTS {
        let Some(first) = asker.secret(&request.prompt_line()) else {
            return Err(format!(
                "No password given. Nothing was packed into {name}."
            ));
        };
        if first.is_empty() {
            asker.tell(catalog::SECRET_EMPTY);
            continue;
        }
        let Some(again) = asker.secret("Same password again (hidden): ") else {
            return Err(format!(
                "No password given. Nothing was packed into {name}."
            ));
        };
        if again == first {
            return Ok(first);
        }
        asker.tell(catalog::SECRETS_DIFFER);
    }
    Err(format!("No password set. Nothing was packed into {name}."))
}

fn pack(
    args: &PackArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    session: &Session<'_>,
) -> Result<i32, CliError> {
    let destination = session.resolve("pack", &args.archive)?;
    let name = name_of(&destination);
    let Some(format) = destination.name().and_then(ArchiveFormat::from_name) else {
        return Err(CliError::Failed(format!(
            "Could not pack {name}: its name does not say a format. End it in .zip, .7z, .tar, .tar.gz, .tar.xz or .xz."
        )));
    };
    let texts = match args.stdin {
        Some(separator) => match stdin_items(input, separator, out)? {
            Some(items) => items,
            None => return Ok(1),
        },
        None => args.items.clone(),
    };
    let mut sources = Vec::with_capacity(texts.len());
    for text in &texts {
        sources.push(session.resolve("pack", text)?);
    }
    let mut request = PackRequest::new(sources, destination, format);
    request.level = args.level;
    request.solid = format == ArchiveFormat::SevenZ && !args.not_solid;
    request.encrypt_names = args.hide_names;
    request.conflicts = if args.replace {
        ConflictPolicy::Overwrite
    } else {
        ConflictPolicy::KeepBoth
    };
    if args.password {
        if !diroxide_archive::pack::takes_password(format) {
            return Err(CliError::Failed(format!(
                "Could not pack {name}: {} keeps no password. Use 7z or zip.",
                diroxide_archive::pack::format_word(format)
            )));
        }
        match new_password(session.asker, &name) {
            Ok(secret) => request.password = Some(secret),
            Err(sentence) => {
                line(out, &sentence)?;
                return Ok(1);
            }
        }
    }
    let job = JobId::new(1);
    let plan = plan_pack(&session.registry, &request, job, &OpContext::new())
        .map_err(|e| archive_failed("pack", &name, &e))?;
    lines(out, &plan.rows())?;
    let engine = session.engine();
    let control = JobControl::new();
    let relay = Relay::new();
    let mut unanswered = Unanswered::default();
    let archives = &session.archives;
    let password = request.password.clone();
    let outcome = session.drive(&relay, &mut unanswered, || {
        let source: Arc<dyn SecretSource> = relay.clone();
        let env = JobEnv::new(&engine, archives, job, &control, relay.as_ref())
            .with_questions(relay.as_ref())
            .with_secrets(source);
        run_pack(&plan, password.as_ref(), &env)
    });
    unanswered.write(out)?;
    lines(out, &outcome.lines)?;
    lines(out, &failure_lines(&outcome.report))?;
    Ok(match outcome.archive {
        None => 1,
        Some(_) => match report_code(&outcome.report) {
            1 if !outcome.report.cancelled => 3,
            code => code,
        },
    })
}

fn unpack(args: &UnpackArgs, out: &mut dyn Write, session: &Session<'_>) -> Result<i32, CliError> {
    let place = session.resolve("unpack", &args.archive)?;
    let (container, member) = container_of(&place);
    let name = name_of(&container);
    let destination = match &args.into {
        Some(text) => session.resolve("unpack into", text)?,
        None => session.here.clone(),
    };
    let mut request = UnpackRequest::new(container, destination);
    if !member.is_empty() {
        request.members = vec![member];
    }
    request.conflicts = if args.replace {
        ConflictPolicy::Overwrite
    } else {
        ConflictPolicy::KeepBoth
    };
    let job = JobId::new(1);
    let engine = session.engine();
    let control = JobControl::new();
    let relay = Relay::new();
    let source: Arc<dyn SecretSource> = relay.clone();
    let secrets = JobSecrets::new(source);
    let mut unanswered = Unanswered::default();
    let archives = &session.archives;
    let registry = &session.registry;
    let planned = session.drive(&relay, &mut unanswered, || {
        let mut allow = Allowances::default();
        loop {
            let plan = secrets.with_tries(&OpContext::new(), |ctx| {
                plan_unpack(archives, registry, &request, job, ctx, allow)
            });
            match plan {
                Err(ArchiveError::Ask(question)) if relay.confirm(&question.message().text) => {
                    allow = allow.with(question.kind);
                }
                other => return other.map(|plan| (plan, allow)),
            }
        }
    });
    unanswered.write(out)?;
    let (plan, mut allow) = planned.map_err(|e| archive_failed("unpack", &name, &e))?;
    lines(out, &plan.rows())?;
    if let Some(refusal) = plan.refusal() {
        line(out, &refusal.text)?;
        line(out, "Nothing was unpacked.")?;
        return Ok(1);
    }
    for question in &plan.questions {
        if session.confirm(&question.message().text, out)? {
            allow = allow.with(question.kind);
        }
    }
    let mut unanswered = Unanswered::default();
    let outcome = session.drive(&relay, &mut unanswered, || {
        // The plan's secrets hold a password only when the plan needed one
        // and it worked (a 7z with hidden names): the run reuses it, so the
        // job asks once. Otherwise the run asks through the relay itself:
        // wrapping the plan's secrets again would hand a wrong password
        // back to the runner's own tries, so one wrong password used up
        // all three.
        let shared: Arc<dyn SecretSource> = if secrets.holds_a_password() {
            secrets.clone()
        } else {
            relay.clone()
        };
        let env = JobEnv::new(&engine, archives, job, &control, relay.as_ref())
            .with_questions(relay.as_ref())
            .with_secrets(shared);
        run_unpack(&plan, None, allow, &env)
    });
    unanswered.write(out)?;
    lines(out, &outcome.lines)?;
    lines(out, &failure_lines(&outcome.report))?;
    Ok(report_code(&outcome.report))
}

fn test(archive: &str, out: &mut dyn Write, session: &Session<'_>) -> Result<i32, CliError> {
    let place = session.resolve("test", archive)?;
    let (container, _) = container_of(&place);
    let engine = session.engine();
    let control = JobControl::new();
    let relay = Relay::new();
    let mut unanswered = Unanswered::default();
    let archives = &session.archives;
    let outcome = session.drive(&relay, &mut unanswered, || {
        let source: Arc<dyn SecretSource> = relay.clone();
        let env = JobEnv::new(&engine, archives, JobId::new(1), &control, relay.as_ref())
            .with_questions(relay.as_ref())
            .with_secrets(source);
        test_archive(&container, None, &env)
    });
    unanswered.write(out)?;
    lines(out, &outcome.lines())?;
    Ok(if outcome.is_sound() {
        0
    } else if outcome.failed.is_some() || outcome.cancelled {
        1
    } else {
        3
    })
}

/// A scripted asker for tests: the answers and secrets in order, and
/// everything asked kept.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct Script {
    /// Whether a person is there.
    pub present: bool,
    answers: Mutex<std::collections::VecDeque<bool>>,
    secrets: Mutex<std::collections::VecDeque<Option<String>>>,
    asked: Mutex<Vec<String>>,
}

#[cfg(test)]
impl Script {
    /// A person who answers these questions and gives these secrets, in
    /// order; anything past them is No and none.
    pub(crate) fn answering(answers: &[bool], secrets: &[Option<&str>]) -> Self {
        Self {
            present: true,
            answers: Mutex::new(answers.iter().copied().collect()),
            secrets: Mutex::new(secrets.iter().map(|s| s.map(str::to_owned)).collect()),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Every question, prompt and line, in order.
    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn note(&self, text: &str) {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text.to_owned());
    }
}

#[cfg(test)]
impl Asker for Script {
    fn present(&self) -> bool {
        self.present
    }

    fn confirm(&self, question: &str) -> bool {
        self.note(question);
        self.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or(false)
    }

    fn secret(&self, prompt: &str) -> Option<Secret> {
        self.note(prompt);
        self.secrets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .flatten()
            .map(Secret::new)
    }

    fn tell(&self, text: &str) {
        self.note(text);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use diroxide_config::builtin_split_presets;
    use diroxide_core::FixedClock;
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub(crate) const CANARY: &str = "canary-7f3a-not-a-password";

    pub(crate) fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Friday, October 16, 2026, 3:00 PM UTC.
    pub(crate) fn now() -> Timestamp {
        Timestamp::from_civil(2026, 10, 16, 15, 0, 0, UtcOffset::UTC).unwrap()
    }

    /// 2:05 PM that day, an even second, as a zip's DOS time keeps it.
    fn at() -> SystemTime {
        let seconds = Timestamp::from_civil(2026, 10, 16, 14, 5, 0, UtcOffset::UTC)
            .unwrap()
            .unix_seconds();
        UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).unwrap())
    }

    /// Opens a file or a folder to set its times.
    fn open_for_times(path: &Path) -> io::Result<File> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_WRITE_ATTRIBUTES, and FILE_FLAG_BACKUP_SEMANTICS so a
            // folder opens too.
            fs::OpenOptions::new()
                .access_mode(0x100)
                .custom_flags(0x0200_0000)
                .open(path)
        }
        #[cfg(not(windows))]
        {
            File::open(path)
        }
    }

    pub(crate) fn set_time(path: &Path) {
        open_for_times(path).unwrap().set_modified(at()).unwrap();
    }

    /// A temporary folder holding `src` (the current folder of the
    /// commands) with two folders of text files and a note, every time
    /// 2:05 PM; archives are written beside `src`.
    pub(crate) struct Fixture {
        pub(crate) dir: tempfile::TempDir,
        pub(crate) here: Place,
    }

    impl Fixture {
        pub(crate) fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
    }

    pub(crate) fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        for folder in ["week1", "week2"] {
            fs::create_dir_all(src.join(folder)).unwrap();
        }
        let files = [
            ("week1/a.txt", "alpha one\n"),
            ("week1/b.txt", "beta two\n"),
            ("week2/c.txt", "gamma\n"),
            ("notes.txt", "notes here\n"),
        ];
        for (name, text) in files {
            let path = src.join(name);
            fs::write(&path, text).unwrap();
            set_time(&path);
        }
        for folder in ["week1", "week2"] {
            set_time(&src.join(folder));
        }
        let here = Place::parse(src.to_str().unwrap()).unwrap();
        Fixture { dir, here }
    }

    pub(crate) fn session_in<'a>(
        clock: &'a FixedClock,
        here: &'a Place,
        asker: &'a dyn Asker,
    ) -> Session<'a> {
        let (registry, archives) = archive_registry();
        Session {
            registry,
            archives,
            clock,
            here,
            asker,
            progress: false,
            presets: builtin_split_presets(),
        }
    }

    fn run_in(fixture: &Fixture, asker: &dyn Asker, list: &[&str], input: &[u8]) -> (i32, String) {
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let session = session_in(&clock, &fixture.here, asker);
        let parsed = parse(&args(list)).unwrap();
        let mut out = Vec::new();
        let code = run_with(&parsed, &mut &input[..], &mut out, &session);
        let text = String::from_utf8(out).unwrap();
        match code {
            Ok(code) => (code, text),
            Err(error) => (-1, format!("{text}{error}")),
        }
    }

    fn run(fixture: &Fixture, list: &[&str]) -> (i32, String) {
        run_in(fixture, &NoAsker, list, b"")
    }

    fn packed(fixture: &Fixture, archive: &str) {
        let (code, text) = run(fixture, &["pack", archive, "week1", "week2", "notes.txt"]);
        assert_eq!(code, 0, "{text}");
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(
            parse(&args(&["list", "a.zip", "--long", "-0"])),
            Ok(ArchiveArgs::List(ListArgs {
                place: "a.zip".into(),
                long: true,
                null: true,
                ..ListArgs::default()
            }))
        );
        let ArchiveArgs::Pack(pack) = parse(&args(&[
            "pack",
            "x.7z",
            "a",
            "b",
            "--level=ultra",
            "--password",
        ]))
        .unwrap() else {
            panic!("not a pack");
        };
        assert_eq!(pack.items, ["a", "b"]);
        assert_eq!(pack.level, CompressionLevel::Ultra);
        assert!(pack.password && pack.stdin.is_none());
        let ArchiveArgs::Pack(piped) = parse(&args(&["pack", "x.zip", "-", "--lines"])).unwrap()
        else {
            panic!("not a pack");
        };
        assert_eq!(piped.stdin, Some(Separator::Lines));
        assert_eq!(
            parse(&args(&["unpack", "a.zip", "--into", "out"])),
            Ok(ArchiveArgs::Unpack(UnpackArgs {
                archive: "a.zip".into(),
                into: Some("out".into()),
                replace: false,
            }))
        );
        assert_eq!(
            parse(&args(&["test", "--", "-odd.zip"])),
            Ok(ArchiveArgs::Test("-odd.zip".into()))
        );
        assert_eq!(parse(&args(&["--help"])), Ok(ArchiveArgs::Help));
        assert_eq!(parse(&args(&["pack", "--help"])), Ok(ArchiveArgs::Help));
        assert_eq!(parse(&args(&[])), Err(CliError::Missing("archive".into())));
        assert_eq!(
            parse(&args(&["list"])),
            Err(CliError::Missing("archive list".into()))
        );
        assert_eq!(
            parse(&args(&["pack", "x.zip"])),
            Err(CliError::Missing("archive pack".into()))
        );
        assert_eq!(
            parse(&args(&["test", "a", "b"])),
            Err(CliError::TooMany("archive test".into()))
        );
        assert_eq!(
            parse(&args(&["list", "a.zip", "--bogus"])),
            Err(CliError::UnknownOption("--bogus".into()))
        );
        for wrong in [
            &["pack", "x.zip", "a", "-"][..],
            &["pack", "x.zip", "a", "--lines"],
            &["pack", "x.7z", "a", "--hide-names"],
            &["pack", "x.zip", "a", "--level", "warp"],
            &["list", "a.zip", "--json", "-0"],
            &["shred", "a.zip"],
        ] {
            assert!(
                matches!(parse(&args(wrong)), Err(CliError::Failed(_))),
                "{wrong:?}"
            );
        }
    }

    #[test]
    fn pack_says_its_plan_first_then_the_runner_lines() {
        let fixture = fixture();
        let (code, text) = run(
            &fixture,
            &["pack", "../course.zip", "week1", "week2", "notes.txt"],
        );
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Pack 3 items into course.zip");
        assert_eq!(lines[1], "Format: zip, level normal (6 of 9)");
        assert!(lines[2].starts_with("Where: "), "{text}");
        assert_eq!(lines[3], "Password: none (zip names stay visible)");
        assert_eq!(lines[4], "Solid: not for zip");
        assert!(lines[5].starts_with("Reads 4 files, 36 bytes."), "{text}");
        assert_eq!(
            &lines[6..],
            [
                "Paths: relative to their folder",
                "Test after packing: on",
                "Pack finished: 6 done.",
                "Checked 6 of 6 entries, all good.",
            ]
        );
        assert!(fixture.path("course.zip").is_file());
    }

    #[test]
    fn list_says_every_member_path_first_in_archive_order() {
        let fixture = fixture();
        packed(&fixture, "../course.zip");
        let (code, text) = run(&fixture, &["list", "../course.zip"]);
        assert_eq!(code, 0, "{text}");
        assert_eq!(
            text,
            "week1, folder, modified today, 2:05 PM\n\
             week1/a.txt, file, 10 bytes, modified today, 2:05 PM\n\
             week1/b.txt, file, 9 bytes, modified today, 2:05 PM\n\
             week2, folder, modified today, 2:05 PM\n\
             week2/c.txt, file, 6 bytes, modified today, 2:05 PM\n\
             notes.txt, file, 11 bytes, modified today, 2:05 PM\n\
             6 items in course.zip, 36 bytes.\n"
        );
        let (code, text) = run(&fixture, &["list", "../course.zip!week1"]);
        assert_eq!(code, 0, "{text}");
        assert_eq!(
            text,
            "week1/a.txt, file, 10 bytes, modified today, 2:05 PM\n\
             week1/b.txt, file, 9 bytes, modified today, 2:05 PM\n\
             2 items in week1, in course.zip, 19 bytes.\n"
        );
        let (code, text) = run(&fixture, &["list", "../course.zip!week2/c.txt"]);
        assert_eq!(code, 0, "{text}");
        assert!(text.starts_with("week2/c.txt, file, 6 bytes"), "{text}");
    }

    #[test]
    fn list_long_json_and_nul_forms() {
        let fixture = fixture();
        packed(&fixture, "../course.zip");
        let (_, long) = run(&fixture, &["list", "--long", "../course.zip!notes.txt"]);
        let first = long.lines().next().unwrap();
        assert!(
            first.starts_with(
                "notes.txt, file, 11 bytes, modified today, 2:05 PM, 11 bytes, packed "
            ),
            "{first}"
        );
        assert!(first.contains(", CRC-32 "), "{first}");
        assert!(diroxide_core::format::within_cells(
            first,
            "notes.txt, file, 11 bytes"
        ));
        let (code, json) = run(&fixture, &["list", "--json", "../course.zip"]);
        assert_eq!(code, 0);
        assert_eq!(json.lines().count(), 6, "no summary in JSON");
        let value: serde_json::Value = serde_json::from_str(json.lines().nth(1).unwrap()).unwrap();
        assert_eq!(value["path"], "week1/a.txt");
        assert_eq!(value["name"], "a.txt");
        assert_eq!(value["modified"], "2026-10-16T14:05:00Z");
        let (code, nul) = run(&fixture, &["list", "-0", "../course.zip"]);
        assert_eq!(code, 0);
        assert_eq!(
            nul,
            "week1\0week1/a.txt\0week1/b.txt\0week2\0week2/c.txt\0notes.txt\0"
        );
    }

    #[test]
    fn pack_reads_nul_separated_items_from_standard_input() {
        let fixture = fixture();
        let (code, text) = run_in(
            &fixture,
            &NoAsker,
            &["pack", "../two.zip", "-"],
            b"week2\0notes.txt\0",
        );
        assert_eq!(code, 0, "{text}");
        assert!(text.starts_with("Pack 2 items into two.zip\n"), "{text}");
        let (code, text) = run_in(
            &fixture,
            &NoAsker,
            &["pack", "../two.zip", "-"],
            b"week2\nnotes.txt\n",
        );
        assert_eq!(
            (code, text.as_str()),
            (1, "No NUL separators found. Use --lines.\n")
        );
        let (code, text) = run_in(&fixture, &NoAsker, &["pack", "../none.zip", "-"], b"");
        assert_eq!(
            (code, text.as_str()),
            (1, "List is empty. Nothing to pack.\n")
        );
    }

    #[test]
    fn test_and_unpack_say_the_runners_lines() {
        let fixture = fixture();
        packed(&fixture, "../course.zip");
        let (code, text) = run(&fixture, &["test", "../course.zip"]);
        assert_eq!((code, text.as_str()), (0, "Archive is sound: 6 members.\n"));
        fs::create_dir(fixture.path("out")).unwrap();
        let (code, text) = run(&fixture, &["unpack", "../course.zip", "--into", "../out"]);
        assert_eq!(code, 0, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Unpack 6 members into course");
        assert!(lines[1].starts_with("Where: "));
        assert_eq!(lines[2], "Folder: course, 3 items at the top");
        assert_eq!(lines[3], "Paths: kept");
        assert!(lines[4].starts_with("Writes 4 files, 36 bytes."));
        assert_eq!(
            &lines[5..],
            [
                "When a name is taken: keep both",
                "Unpack finished: 6 done."
            ]
        );
        let unpacked = fixture.path("out").join("course");
        assert_eq!(
            fs::read_to_string(unpacked.join("week1").join("a.txt")).unwrap(),
            "alpha one\n"
        );
        assert_eq!(
            fs::read_to_string(unpacked.join("notes.txt")).unwrap(),
            "notes here\n"
        );
        // One member alone, into the folder as it is.
        fs::create_dir(fixture.path("one")).unwrap();
        let (code, text) = run(
            &fixture,
            &["unpack", "../course.zip!week2/c.txt", "--into", "../one"],
        );
        assert_eq!(code, 0, "{text}");
        assert_eq!(
            fs::read_to_string(fixture.path("one").join("c.txt")).unwrap(),
            "gamma\n"
        );
    }

    #[test]
    fn seven_z_and_tar_xz_pack_test_and_list() {
        let fixture = fixture();
        for (archive, format_row, solid_row) in [
            (
                "../backup.7z",
                "Format: 7z, level normal (6 of 9)",
                "Solid: on, blocks up to 1 GiB",
            ),
            (
                "../logs.tar.xz",
                "Format: tar.xz, level normal (6 of 9)",
                "Solid: not for tar.xz",
            ),
        ] {
            let (code, text) = run(&fixture, &["pack", archive, "week1", "week2", "notes.txt"]);
            assert_eq!(code, 0, "{text}");
            assert!(text.lines().any(|l| l == format_row), "{text}");
            assert!(text.lines().any(|l| l == solid_row), "{text}");
            assert!(
                text.lines()
                    .any(|l| l.starts_with("Checked ") && l.ends_with(", all good.")),
                "{text}"
            );
            let (code, text) = run(&fixture, &["test", archive]);
            assert_eq!(code, 0, "{archive}: {text}");
            assert!(text.starts_with("Archive is sound: "), "{text}");
            let (code, text) = run(&fixture, &["list", archive]);
            assert_eq!(code, 0, "{text}");
            assert!(text.contains("week1/a.txt, file, 10 bytes"), "{text}");
            assert!(text.contains("6 items in "), "{text}");
        }
        // A plain xz keeps no name: its member is named after the
        // archive, so the archive is named after the file.
        let (code, text) = run(&fixture, &["pack", "../notes.txt.xz", "notes.txt"]);
        assert_eq!(code, 0, "{text}");
        // W2g's finding (P2b): an xz named otherwise holds the file as its
        // stem, says so in its plan, and passes its own test.
        let (code, text) = run(&fixture, &["pack", "../one.xz", "notes.txt"]);
        assert_eq!(code, 0, "{text}");
        assert!(
            text.lines()
                .any(|l| l == "Unpacks as one: xz keeps no name"),
            "{text}"
        );
        assert!(text.contains("Checked 1 of 1 entries, all good."), "{text}");
        let (code, text) = run(&fixture, &["list", "../one.xz"]);
        assert_eq!(code, 0, "{text}");
        assert!(text.lines().any(|l| l.starts_with("one, file")), "{text}");
        let (code, text) = run(&fixture, &["pack", "../bad.xz", "week1"]);
        assert_eq!(code, -1, "xz holds one file: {text}");
        let (code, text) = run(&fixture, &["pack", "../plain.txt", "week1"]);
        assert_eq!(code, -1);
        assert!(text.contains("its name does not say a format"), "{text}");
    }

    #[test]
    fn a_locked_zip_asks_for_its_password_and_never_shows_it() {
        let fixture = fixture();
        let maker = Script::answering(&[], &[Some(CANARY), Some(CANARY)]);
        let (code, text) = run_in(
            &fixture,
            &maker,
            &["pack", "../locked.zip", "week1", "notes.txt", "--password"],
            b"",
        );
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains("Password: AES-256 (zip names visible)\n"),
            "{text}"
        );
        assert_eq!(
            maker.asked(),
            [
                "New password for locked.zip (hidden): ",
                "Same password again (hidden): "
            ]
        );
        // The names are visible, so listing needs no password.
        let (code, listing) = run(&fixture, &["list", "../locked.zip"]);
        assert_eq!(code, 0, "{listing}");
        assert!(
            listing.contains("week1/a.txt, locked, file, 10 bytes"),
            "{listing}"
        );
        // A wrong password first, then the right one.
        let tester = Script::answering(&[true], &[Some("wrong-one"), Some(CANARY)]);
        let (code, text) = run_in(&fixture, &tester, &["test", "../locked.zip"], b"");
        assert_eq!((code, text.as_str()), (0, "Archive is sound: 4 members.\n"));
        assert_eq!(
            tester.asked(),
            [
                "Password for locked.zip (hidden): ",
                "Wrong password for locked.zip. Try again? y or n",
                "Password for locked.zip (hidden): ",
            ]
        );
        // With nobody to ask, nothing is read and it says why.
        let (code, text) = run(&fixture, &["test", "../locked.zip"]);
        assert_eq!(code, 3, "{text}");
        assert!(
            text.starts_with("No password taken: passwords are typed in a terminal.\n"),
            "{text}"
        );
        assert!(text.contains("Not checked: 3 of 4 members."), "{text}");
        for said in [&text, &listing] {
            assert!(!said.contains(CANARY));
        }
        // Unpacked with the password after a wrong one, the bytes are the
        // same: the wrong one costs one try, not all three.
        fs::create_dir(fixture.path("out")).unwrap();
        let unpacker = Script::answering(&[true], &[Some("wrong-one"), Some(CANARY)]);
        let (code, text) = run_in(
            &fixture,
            &unpacker,
            &["unpack", "../locked.zip", "--into", "../out"],
            b"",
        );
        assert_eq!(code, 0, "{text}");
        assert!(!text.contains(CANARY));
        assert_eq!(
            unpacker.asked(),
            [
                "Password for locked.zip (hidden): ",
                "Wrong password for locked.zip. Try again? y or n",
                "Password for locked.zip (hidden): ",
            ]
        );
        assert_eq!(
            fs::read_to_string(fixture.path("out").join("locked").join("notes.txt")).unwrap(),
            "notes here\n"
        );
    }

    #[test]
    fn a_new_password_is_typed_twice_and_needs_a_terminal() {
        let fixture = fixture();
        let (code, text) = run(
            &fixture,
            &["pack", "../locked.zip", "notes.txt", "--password"],
        );
        assert_eq!(
            (code, text.as_str()),
            (
                1,
                "Could not pack locked.zip: a password is typed in a terminal. Nothing was packed.\n"
            )
        );
        assert!(!fixture.path("locked.zip").exists());
        let typist = Script::answering(
            &[],
            &[Some("one"), Some("two"), Some(""), Some("ok"), Some("ok")],
        );
        let (code, text) = run_in(
            &fixture,
            &typist,
            &[
                "pack",
                "../locked.7z",
                "notes.txt",
                "--password",
                "--hide-names",
            ],
            b"",
        );
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("Password: AES-256, names hidden\n"), "{text}");
        let asked = typist.asked();
        assert!(
            asked.contains(&catalog::SECRETS_DIFFER.to_owned()),
            "{asked:?}"
        );
        assert!(
            asked.contains(&catalog::SECRET_EMPTY.to_owned()),
            "{asked:?}"
        );
        let (code, text) = run(
            &fixture,
            &["pack", "../locked.tar", "notes.txt", "--password"],
        );
        assert_eq!(code, -1);
        assert!(
            text.contains("tar keeps no password. Use 7z or zip."),
            "{text}"
        );
        // A 7z with its names hidden asks before it lists.
        let lister = Script::answering(&[], &[Some("ok")]);
        let (code, text) = run_in(&fixture, &lister, &["list", "../locked.7z"], b"");
        assert_eq!(code, 0, "{text}");
        assert!(text.starts_with("notes.txt, "), "{text}");
        assert_eq!(lister.asked(), ["Password for locked.7z (hidden): "]);
        let (code, text) = run(&fixture, &["list", "../locked.7z"]);
        assert_eq!(code, -1);
        assert!(
            text.contains("Could not list locked.7z: a password is needed."),
            "{text}"
        );
    }

    /// A zip of two stored members built in memory, the first named so it
    /// would leave the archive: the hostile name never touches the disk.
    pub(crate) fn hostile_zip() -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let members: [(&str, &[u8]); 2] = [("../evil.txt", b"evil\n"), ("good.txt", b"good\n")];
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in members {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            let header = |signature: u32, central: bool| {
                let mut h = signature.to_le_bytes().to_vec();
                if central {
                    h.extend_from_slice(&20u16.to_le_bytes());
                }
                h.extend_from_slice(&20u16.to_le_bytes());
                h.extend_from_slice(&0x0800u16.to_le_bytes());
                h.extend_from_slice(&0u16.to_le_bytes());
                h.extend_from_slice(&0u16.to_le_bytes());
                h.extend_from_slice(&0x5D50u16.to_le_bytes());
                h.extend_from_slice(&crc.to_le_bytes());
                h.extend_from_slice(&size.to_le_bytes());
                h.extend_from_slice(&size.to_le_bytes());
                h.extend_from_slice(&(name.len() as u16).to_le_bytes());
                h.extend_from_slice(&0u16.to_le_bytes());
                if central {
                    // The comment's length, the disk, the attributes.
                    h.extend_from_slice(&[0; 6]);
                    h.extend_from_slice(&0u32.to_le_bytes());
                    h.extend_from_slice(&offset.to_le_bytes());
                }
                h.extend_from_slice(name.as_bytes());
                h
            };
            out.extend(header(0x0403_4B50, false));
            out.extend_from_slice(data);
            central.extend(header(0x0201_4B50, true));
        }
        let start = out.len() as u32;
        let length = central.len() as u32;
        out.extend(central);
        out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn a_name_that_leaves_the_archive_is_said_and_never_written() {
        let fixture = fixture();
        fs::write(fixture.path("hostile.zip"), hostile_zip()).unwrap();
        let (code, text) = run(&fixture, &["list", "../hostile.zip"]);
        assert_eq!(code, 3, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("good.txt, file, 5 bytes"), "{text}");
        assert!(
            lines.contains(&"Skipped ../evil.txt: name leaves the archive."),
            "{text}"
        );
        assert_eq!(lines.last(), Some(&"1 item in hostile.zip, 5 bytes."));
        fs::create_dir(fixture.path("out")).unwrap();
        let (code, text) = run(&fixture, &["unpack", "../hostile.zip", "--into", "../out"]);
        assert_eq!(code, 3, "{text}");
        assert!(
            text.contains("Skipped ../evil.txt: name leaves the archive."),
            "{text}"
        );
        assert!(fixture.path("out").join("good.txt").is_file());
        assert!(!fixture.path("evil.txt").exists());
    }

    #[test]
    fn what_cannot_be_read_says_so_in_a_sentence() {
        let fixture = fixture();
        let (code, text) = run(&fixture, &["list", "../missing.zip"]);
        assert_eq!(code, -1);
        assert!(text.starts_with("Could not list missing.zip: "), "{text}");
        packed(&fixture, "../course.zip");
        let (code, text) = run(&fixture, &["list", "../course.zip!nowhere"]);
        assert_eq!(code, -1);
        assert_eq!(
            text,
            "Could not list nowhere: it is not in course.zip. Check the path."
        );
    }

    #[test]
    fn a_question_with_nobody_to_ask_is_said_and_answered_no() {
        let mut out = Vec::new();
        let clock = FixedClock::new(now(), UtcOffset::UTC);
        let here = Place::parse(if cfg!(windows) { "D:\\x" } else { "/x" }).unwrap();
        let session = session_in(&clock, &here, &NoAsker);
        assert!(!session.confirm("Go on anyway? y or n", &mut out).unwrap());
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("Go on anyway? y or n\n{NOT_ASKED}\n")
        );
        let script = Script::answering(&[true], &[]);
        let session = session_in(&clock, &here, &script);
        let mut out = Vec::new();
        assert!(session.confirm("Go on anyway? y or n", &mut out).unwrap());
        assert!(out.is_empty(), "the terminal asked it");
    }

    #[test]
    fn progress_is_said_on_a_change_and_at_most_every_ten_seconds() {
        let mut words = ProgressWords::new();
        let mut progress = JobProgress::start(diroxide_core::JobKind::Unpack, 10, Some(100));
        progress.state = JobState::Running;
        assert_eq!(words.line(&progress), None, "not at the start");
        progress.state = JobState::Stalled;
        assert!(words.line(&progress).is_some(), "a change at once");
        assert_eq!(words.line(&progress), None, "not twice");
        progress.state = JobState::Finished;
        assert_eq!(words.line(&progress), None, "the summary says it");
    }

    #[test]
    fn every_fixed_sentence_says_its_meaning_in_40_cells() {
        for (sentence, meaning) in [
            (NOT_ASKED, "Answered no: there is no terminal"),
            (
                "No password taken: passwords are typed in a terminal.",
                "No password taken",
            ),
            (
                "List is empty. Nothing to pack.",
                "List is empty. Nothing to pack.",
            ),
            (
                "Same password again (hidden): ",
                "Same password again (hidden):",
            ),
        ] {
            assert!(
                diroxide_core::format::within_cells(sentence, meaning),
                "{sentence}"
            );
        }
        for line in help_text().lines() {
            assert!(line.chars().count() <= 80, "{line}");
        }
    }
}
