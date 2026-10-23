//! The context a script command runs in (R3 sections 5.2 and 5.3, R22
//! section 2.6): the effective selection, the two selection files, the
//! context as JSON, the directives file, the cancel file, a private
//! temporary folder, and the `DIRO_*` environment that names them.
//!
//! - **The selection.** The marked entries, or the cursor's entry when
//!   nothing is marked and the command allows it (`cursor_fallback`);
//!   checked against the command's count (`none`, `one`, `two`, `any`,
//!   `many`) and places (`local`). A row that is not a file is refused:
//!   "This row is not a file."
//! - **Two selection files.** `DIRO_SELECTION_FILE` holds one place a
//!   line; `DIRO_SELECTION_FILE0` the same places, each ended by a NUL,
//!   byte for byte. A name with a line break cannot be written to the
//!   first, so a line reader is refused such a selection, and the
//!   variable is left empty for a NUL reader, which must not read it.
//! - **The run folder** is made fresh inside the state folder for each
//!   run and removed when the run ends, after checking that it lies
//!   inside the folder of runs; the cancel file appears in it when the
//!   owner cancels, for a script to poll before the job kill comes.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use diroxide_core::{CommandId, EntryKind, Place, PlaceKind};
use diroxide_tools::process::{ProcessSpec, Stream};

use crate::manifest::{Places, Selection};
use crate::protocol::PROTOCOL;
use crate::script::command::ScriptCommand;
use crate::script::interpreter::forward_slashes;
use crate::script::recipe::{ListForm, RunMode, StdinForm};
use crate::shapes::{PanelRef, WireContext, WireEntry};

/// The words of the refusal when a command does not use the cursor's
/// row (R22 section 2.6).
pub const MARK_FIRST: &str = "Mark something first: this command will not use the cursor row";

/// An entry a command gets: its place and its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    /// Where it is.
    pub place: Place,
    /// What it is.
    pub kind: EntryKind,
}

impl Picked {
    /// An entry of a known kind.
    pub fn new(place: Place, kind: EntryKind) -> Self {
        Self { place, kind }
    }

    /// A local entry, its kind read from the disk without following a
    /// link; a place that is not local, or not there, is a file.
    pub fn from_disk(place: Place) -> Self {
        let kind = place
            .local_path()
            .and_then(|p| fs::symlink_metadata(p).ok())
            .map_or(EntryKind::File, |m| {
                if m.is_symlink() {
                    EntryKind::Link
                } else if m.is_dir() {
                    EntryKind::Folder
                } else if m.is_file() {
                    EntryKind::File
                } else {
                    EntryKind::Special
                }
            });
        Self { place, kind }
    }
}

/// The panels and the selection when a command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunContext {
    /// The active panel's place.
    pub active: Place,
    /// The other panel's place, the destination by the orthodox rule.
    pub other: Place,
    /// The entry under the active panel's cursor.
    pub cursor: Option<Picked>,
    /// The entry under the other panel's cursor.
    pub other_cursor: Option<Picked>,
    /// The marked entries of the active panel.
    pub marked: Vec<Picked>,
    /// The answer to the command's `prompt`.
    pub input: Option<String>,
    /// A preview run: the command must change nothing.
    pub dry_run: bool,
    /// An unattended run (a hot folder, a task).
    pub unattended: bool,
    /// The owner's locale.
    pub locale: String,
    /// The host's version.
    pub version: String,
}

impl RunContext {
    /// Both panels at places, with nothing marked and no cursor.
    pub fn new(active: Place, other: Place) -> Self {
        Self {
            active,
            other,
            cursor: None,
            other_cursor: None,
            marked: Vec::new(),
            input: None,
            dry_run: false,
            unattended: false,
            locale: "en-US".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// Why a command will not run with this context, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Nothing is marked and the command does not use the cursor.
    MarkFirst,
    /// The selection's count does not fit the command.
    Count {
        /// What the command needs.
        needs: Selection,
        /// How many entries there are.
        have: usize,
    },
    /// A line reader was given a name with a line break.
    LineBreak,
    /// A panel is not a local folder, and the command works only there.
    NotLocal,
    /// A selected row is not a file.
    NotAFile,
}

impl Refusal {
    /// The sentence for a command named `title`, meaning first.
    pub fn sentence(&self, title: &str) -> String {
        match self {
            Refusal::MarkFirst => MARK_FIRST.to_owned(),
            Refusal::Count { needs, have } => {
                let wanted = match needs {
                    Selection::One => "one item",
                    Selection::Two => "two items",
                    Selection::Many => "two or more items",
                    Selection::Any | Selection::None => "an item",
                };
                let present = match have {
                    0 => "none is selected".to_owned(),
                    1 => "1 is selected".to_owned(),
                    n => format!("{n} are selected"),
                };
                format!("{title} needs {wanted}; {present}.")
            }
            Refusal::LineBreak => format!("{title}: a name has a line break."),
            Refusal::NotLocal => format!("{title} works on local folders only."),
            Refusal::NotAFile => "This row is not a file.".to_owned(),
        }
    }

    /// The longer sentence for the log and the standard failure form.
    pub fn detail(&self, title: &str) -> String {
        match self {
            Refusal::LineBreak => format!(
                "Could not run {title}: one selected name has a line break. Rename it, or use a script that reads the NUL list."
            ),
            other => other.sentence(title),
        }
    }
}

/// The selection a command works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effective {
    /// The entries.
    pub entries: Vec<Picked>,
    /// Whether they are the cursor's entry because nothing was marked.
    pub from_cursor: bool,
}

/// The selection a command gets, checked against its count, its places
/// and its rows.
pub fn effective(command: &ScriptCommand, context: &RunContext) -> Result<Effective, Refusal> {
    if command.places == Places::Local
        && [&context.active, &context.other]
            .iter()
            .any(|p| p.kind() != PlaceKind::Local)
    {
        return Err(Refusal::NotLocal);
    }
    if command.selection == Selection::None {
        return Ok(Effective {
            entries: Vec::new(),
            from_cursor: false,
        });
    }
    let (entries, from_cursor) = if !context.marked.is_empty() {
        (context.marked.clone(), false)
    } else if !command.recipe.cursor_fallback {
        return Err(Refusal::MarkFirst);
    } else {
        (context.cursor.iter().cloned().collect::<Vec<_>>(), true)
    };
    if entries.iter().any(|e| e.kind == EntryKind::Item) {
        return Err(Refusal::NotAFile);
    }
    let have = entries.len();
    let fits = match command.selection {
        Selection::None => true,
        Selection::One => have == 1,
        Selection::Two => have == 2,
        Selection::Any => have >= 1,
        Selection::Many => have >= 2,
    };
    if !fits {
        return Err(Refusal::Count {
            needs: command.selection,
            have,
        });
    }
    Ok(Effective {
        entries,
        from_cursor: from_cursor && have > 0,
    })
}

/// Why a list could not be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListError {
    /// A name holds a line break (`\n` or `\r`), which a line list cannot
    /// carry.
    LineBreak,
    /// A name holds a NUL, which no file name can.
    Nul,
}

/// The bytes of a list of names: each followed by `\n`, or by a NUL.
/// Names are written byte for byte; nothing is changed or escaped.
pub fn list_bytes<N: AsRef<[u8]>>(names: &[N], form: ListForm) -> Result<Vec<u8>, ListError> {
    let mut out = Vec::new();
    for name in names {
        let name = name.as_ref();
        if name.contains(&0) {
            return Err(ListError::Nul);
        }
        if form == ListForm::Lines && name.iter().any(|b| *b == b'\n' || *b == b'\r') {
            return Err(ListError::LineBreak);
        }
        out.extend_from_slice(name);
        out.push(match form {
            ListForm::Lines => b'\n',
            ListForm::Nul => 0,
        });
    }
    Ok(out)
}

/// A place as a script sees it: its text, with forward slashes for Git's
/// bash on Windows.
pub fn place_text(place: &Place, forward: bool) -> String {
    let text = place.to_string();
    if forward && place.kind() == PlaceKind::Local {
        forward_slashes(&text)
    } else {
        text
    }
}

static RUNS: AtomicU64 = AtomicU64::new(0);

/// The folder of one run, inside the folder of runs in the state folder:
/// the selection files, the context, the directives, the cancel file and
/// the script's temporary folder.
#[derive(Debug)]
pub struct RunFolder {
    root: PathBuf,
    path: PathBuf,
}

impl RunFolder {
    /// Makes a fresh run folder inside `runs_root` (made if missing).
    pub fn create(runs_root: &Path) -> io::Result<Self> {
        fs::create_dir_all(runs_root)?;
        // Absolute, never canonical: Windows' canonical form carries the
        // extended prefix, which no script and no listener should see.
        let root = std::path::absolute(runs_root)?;
        loop {
            let n = RUNS.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("run-{}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::create_dir(path.join("temp"))?;
                    fs::write(path.join("directives"), b"")?;
                    return Ok(Self { root, path });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// The run folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One place a line.
    pub fn selection_file(&self) -> PathBuf {
        self.path.join("selection.txt")
    }

    /// Each place ended by a NUL.
    pub fn selection_file0(&self) -> PathBuf {
        self.path.join("selection0")
    }

    /// The context as JSON.
    pub fn context_file(&self) -> PathBuf {
        self.path.join("context.json")
    }

    /// Where the script writes directives.
    pub fn directives_file(&self) -> PathBuf {
        self.path.join("directives")
    }

    /// The file that appears when the owner cancels.
    pub fn cancel_file(&self) -> PathBuf {
        self.path.join("cancel")
    }

    /// The script's own temporary folder, `DIRO_TEMP`.
    pub fn temp_dir(&self) -> PathBuf {
        self.path.join("temp")
    }

    /// Makes the cancel file appear.
    pub fn cancel(&self) -> io::Result<()> {
        fs::write(self.cancel_file(), b"cancel\n")
    }

    /// Whether the cancel file is there.
    pub fn cancelled(&self) -> bool {
        self.cancel_file().exists()
    }

    /// Removes the run folder and what the run left in it, after checking
    /// that its full path lies directly inside the folder of runs. Links
    /// inside it are removed, never followed.
    pub fn remove(self) -> io::Result<()> {
        let path = fs::canonicalize(&self.path)?;
        let root = fs::canonicalize(&self.root)?;
        if path.parent() != Some(root.as_path())
            || !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("run-"))
        {
            return Err(io::Error::other(format!(
                "{} is not a run folder of {}",
                path.display(),
                self.root.display()
            )));
        }
        fs::remove_dir_all(path)
    }
}

/// What a run needs, made ready in its run folder.
#[derive(Debug)]
pub struct Prepared {
    /// The run folder.
    pub folder: RunFolder,
    /// The selection.
    pub selection: Effective,
    /// The environment the child gets, after diroxide's own.
    pub env: Vec<(OsString, OsString)>,
    /// The folder the child starts in.
    pub cwd: PathBuf,
    /// Whether places reach the child with forward slashes.
    pub forward: bool,
}

/// Where a run's private folders are: the folder of runs and the
/// command's data folder, both inside the state folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateFolders {
    /// The folder holding each run's folder.
    pub runs: PathBuf,
    /// The command's own folder that lasts between runs, `DIRO_DATA_DIR`.
    pub data: PathBuf,
}

/// Checks the selection, writes the run folder's files, and builds the
/// environment of a run. `entry` is the one entry of a per-entry run.
/// `tools` are the paths of the tools the command needs, by name.
pub fn prepare(
    command: &ScriptCommand,
    context: &RunContext,
    folders: &StateFolders,
    forward: bool,
    tools: &BTreeMap<String, PathBuf>,
) -> Result<Prepared, PrepareError> {
    let selection = effective(command, context).map_err(PrepareError::Refused)?;
    let texts: Vec<String> = selection
        .entries
        .iter()
        .map(|e| place_text(&e.place, forward))
        .collect();
    let nul =
        list_bytes(&texts, ListForm::Nul).map_err(|_| PrepareError::Refused(Refusal::LineBreak))?;
    let lines = match list_bytes(&texts, ListForm::Lines) {
        Ok(bytes) => Some(bytes),
        Err(_) if command.recipe.selection_file == ListForm::Nul => None,
        Err(_) => return Err(PrepareError::Refused(Refusal::LineBreak)),
    };
    let folder = RunFolder::create(&folders.runs).map_err(PrepareError::Io)?;
    fs::write(folder.selection_file0(), &nul).map_err(PrepareError::Io)?;
    if let Some(lines) = &lines {
        fs::write(folder.selection_file(), lines).map_err(PrepareError::Io)?;
    }
    fs::create_dir_all(&folders.data).map_err(PrepareError::Io)?;
    let json = context_json(command, context, &selection);
    fs::write(folder.context_file(), json).map_err(PrepareError::Io)?;

    let path = |p: &Path| -> OsString {
        if forward {
            OsString::from(forward_slashes(&p.to_string_lossy()))
        } else {
            p.as_os_str().to_owned()
        }
    };
    let text = |p: &Place| OsString::from(place_text(p, forward));
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    let mut set = |name: &str, value: OsString| env.push((OsString::from(name), value));
    set("DIRO_PROTOCOL", PROTOCOL.to_string().into());
    set("DIRO_VERSION", context.version.clone().into());
    set("DIRO_COMMAND", command.id.clone().into());
    if let crate::script::command::Program::Script(script) = &command.program {
        set("DIRO_SCRIPT", path(script));
    }
    set("DIRO_LOCALE", context.locale.clone().into());
    set("DIRO_ACTIVE", text(&context.active));
    set("DIRO_OTHER", text(&context.other));
    set(
        "DIRO_ACTIVE_PROVIDER",
        PanelRef::for_place(&context.active).provider.into(),
    );
    set(
        "DIRO_OTHER_PROVIDER",
        PanelRef::for_place(&context.other).provider.into(),
    );
    let cursor = context.cursor.as_ref();
    set(
        "DIRO_CURSOR",
        cursor.map(|c| text(&c.place)).unwrap_or_default(),
    );
    set(
        "DIRO_CURSOR_NAME",
        cursor
            .and_then(|c| c.place.name())
            .map(OsString::from)
            .unwrap_or_default(),
    );
    set(
        "DIRO_OTHER_CURSOR",
        context
            .other_cursor
            .as_ref()
            .map(|c| text(&c.place))
            .unwrap_or_default(),
    );
    set(
        "DIRO_SELECTION_COUNT",
        selection.entries.len().to_string().into(),
    );
    set(
        "DIRO_SELECTION_FROM_CURSOR",
        if selection.from_cursor { "1" } else { "0" }.into(),
    );
    set(
        "DIRO_SELECTION_FILE",
        if lines.is_some() {
            path(&folder.selection_file())
        } else {
            OsString::new()
        },
    );
    set("DIRO_SELECTION_FILE0", path(&folder.selection_file0()));
    set("DIRO_CONTEXT_FILE", path(&folder.context_file()));
    set("DIRO_DIRECTIVES", path(&folder.directives_file()));
    set("DIRO_CANCEL_FILE", path(&folder.cancel_file()));
    set("DIRO_TEMP", path(&folder.temp_dir()));
    set("DIRO_DATA_DIR", path(&folders.data));
    if context.dry_run {
        set("DIRO_DRY_RUN", "1".into());
    }
    if context.unattended {
        set("DIRO_UNATTENDED", "1".into());
    }
    if let Some(input) = &context.input {
        set("DIRO_INPUT", input.clone().into());
    }
    for (name, tool) in tools {
        let variable: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        set(&format!("DIRO_TOOL_{variable}"), path(tool));
    }
    if !command.recipe.mode.is_terminal() {
        set("TERM", "dumb".into());
    }
    let cwd = match command.recipe.cwd {
        crate::script::recipe::CwdChoice::Active => context.active.local_path().map(PathBuf::from),
        crate::script::recipe::CwdChoice::Other => context.other.local_path().map(PathBuf::from),
        crate::script::recipe::CwdChoice::Script => Some(command.folder.clone()),
    }
    .unwrap_or_else(|| folder.temp_dir());
    Ok(Prepared {
        folder,
        selection,
        env,
        cwd,
        forward,
    })
}

/// Why a run could not be made ready.
#[derive(Debug)]
pub enum PrepareError {
    /// The context does not fit the command.
    Refused(Refusal),
    /// The run folder could not be written.
    Io(io::Error),
}

/// The context as the JSON a command reads on standard input and from
/// `DIRO_CONTEXT_FILE`.
pub fn context_json(
    command: &ScriptCommand,
    context: &RunContext,
    selection: &Effective,
) -> String {
    let id = CommandId::new(command.id.clone())
        .unwrap_or_else(|_| CommandId::from_static("script.command"));
    let wire = |p: &Picked| WireEntry {
        name: p.place.name().unwrap_or_default().to_owned(),
        name_raw: None,
        place: Some((&p.place).into()),
        kind: p.kind,
        size: None,
        modified: None,
        mode: None,
        attributes: Vec::new(),
        owner: None,
        link_target: None,
        mime: None,
        revision: None,
        extra: BTreeMap::new(),
    };
    let mut wire_context = WireContext::new(
        &id,
        PanelRef::for_place(&context.active),
        PanelRef::for_place(&context.other),
    );
    wire_context.locale = context.locale.clone();
    if let Some(input) = &context.input {
        wire_context
            .args
            .insert("input".to_owned(), serde_json::Value::String(input.clone()));
    }
    let cursor = context.cursor.as_ref().map(wire);
    let mut wire_context =
        wire_context.with_selection(None, selection.entries.iter().map(wire).collect());
    wire_context.cursor = cursor;
    wire_context.selection_from_cursor = selection.from_cursor;
    serde_json::to_string(&wire_context).unwrap_or_else(|_| "{}".to_owned())
}

/// The process for a run: the program and its arguments, the run's
/// environment after diroxide's own, the working folder, and the streams
/// its mode needs (a terminal mode shares the owner's terminal).
pub fn process_spec(
    program: &Path,
    args: &[OsString],
    prepared: &Prepared,
    command: &ScriptCommand,
) -> ProcessSpec {
    let mut spec = ProcessSpec::new(program)
        .args(args.iter().cloned())
        .cwd(&prepared.cwd)
        .diroxide_environment();
    for (name, value) in &prepared.env {
        spec = spec.env(name, value);
    }
    let mode = command.recipe.mode;
    if mode.is_terminal() {
        spec.stdin(Stream::Inherit)
            .stdout(Stream::Inherit)
            .stderr(Stream::Inherit)
            .tie(diroxide_tools::process::Tie::Terminal)
    } else if mode == RunMode::Launch {
        spec.stdin(Stream::Null)
            .stdout(Stream::Null)
            .stderr(Stream::Null)
            .tie(diroxide_tools::process::Tie::Detached)
    } else {
        let stdin = match command.recipe.stdin {
            StdinForm::Json => Stream::Piped,
            StdinForm::None => Stream::Null,
        };
        spec.stdin(stdin)
            .stdout(Stream::Piped)
            .stderr(Stream::Piped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::header::parse_header;
    use crate::script::recipe::Recipe;

    fn place(text: &str) -> Place {
        let full = if cfg!(windows) {
            format!("D:{}", text.replace('/', "\\"))
        } else {
            text.to_owned()
        };
        Place::parse(&full).unwrap()
    }

    fn file(text: &str) -> Picked {
        Picked::new(place(text), EntryKind::File)
    }

    fn command(header: &str) -> ScriptCommand {
        ScriptCommand::from_script(Path::new("total-size.sh"), &parse_header(header).header)
    }

    fn context() -> RunContext {
        let mut context = RunContext::new(place("/home/ada"), place("/backup"));
        context.cursor = Some(file("/home/ada/a.txt"));
        context
    }

    /// The odd-name corpus, as bytes in memory: never created on disk.
    fn odd_names() -> Vec<Vec<u8>> {
        vec![
            b"plain.txt".to_vec(),
            b"with space.txt".to_vec(),
            b"-starts-with-dash".to_vec(),
            b"say \"hi\".txt".to_vec(),
            "caf\u{e9} \u{fc}ber.txt".as_bytes().to_vec(),
            "private\u{f03a}use".as_bytes().to_vec(),
            "right\u{202e}txt.exe".as_bytes().to_vec(),
            b"tab\there".to_vec(),
            b"bell\x07.txt".to_vec(),
            vec![b'n', b'o', b't', 0xff, 0xfe, b'8'],
            b"trailing dot.".to_vec(),
        ]
    }

    #[test]
    fn the_nul_list_holds_the_exact_bytes_of_every_name() {
        let names = odd_names();
        let bytes = list_bytes(&names, ListForm::Nul).unwrap();
        let back: Vec<&[u8]> = bytes.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
        assert_eq!(back.len(), names.len());
        for (made, given) in back.iter().zip(&names) {
            assert_eq!(*made, given.as_slice());
        }
        assert_eq!(bytes.iter().filter(|b| **b == 0).count(), names.len());
    }

    #[test]
    fn a_line_break_cannot_go_in_the_line_list() {
        let mut names = odd_names();
        names.push(b"line\nbreak.txt".to_vec());
        assert_eq!(
            list_bytes(&names, ListForm::Lines),
            Err(ListError::LineBreak)
        );
        assert!(list_bytes(&names, ListForm::Nul).is_ok());
        assert_eq!(
            list_bytes(&[b"carriage\rreturn".to_vec()], ListForm::Lines),
            Err(ListError::LineBreak)
        );
        assert_eq!(
            list_bytes(&[b"nul\0".to_vec()], ListForm::Nul),
            Err(ListError::Nul)
        );
        assert_eq!(
            list_bytes(&odd_names()[..2], ListForm::Lines).unwrap(),
            b"plain.txt\nwith space.txt\n"
        );
    }

    #[test]
    fn the_cursor_counts_when_nothing_is_marked() {
        let chosen = effective(&command(""), &context()).unwrap();
        assert!(chosen.from_cursor);
        assert_eq!(chosen.entries, [file("/home/ada/a.txt")]);
        let mut marked = context();
        marked.marked = vec![file("/home/ada/b.txt"), file("/home/ada/c.txt")];
        let chosen = effective(&command(""), &marked).unwrap();
        assert!(!chosen.from_cursor);
        assert_eq!(chosen.entries.len(), 2);
    }

    #[test]
    fn counts_and_rows_are_checked_with_words() {
        let mut two = context();
        two.marked = vec![
            file("/home/ada/b.txt"),
            file("/home/ada/c.txt"),
            file("/home/ada/d.txt"),
        ];
        let one = command("# diro: selection = \"one\"\n");
        let refusal = effective(&one, &two).unwrap_err();
        assert_eq!(
            refusal.sentence("Rename by date"),
            "Rename by date needs one item; 3 are selected."
        );
        let many = command("# diro: selection = \"many\"\n");
        assert_eq!(
            effective(&many, &context()).unwrap_err().sentence("Join"),
            "Join needs two or more items; 1 is selected."
        );
        let strict = command("# diro: cursor_fallback = false\n");
        let refusal = effective(&strict, &context()).unwrap_err();
        assert_eq!(refusal.sentence("Delete old logs"), MARK_FIRST);
        assert!(diroxide_core::format::within_cells(
            MARK_FIRST,
            "Mark something first"
        ));
        let mut item = context();
        item.cursor = Some(Picked::new(place("/home/ada/x"), EntryKind::Item));
        assert_eq!(
            effective(&command(""), &item).unwrap_err().sentence("x"),
            "This row is not a file."
        );
        let none = command("# diro: selection = \"none\"\n");
        assert!(effective(&none, &context()).unwrap().entries.is_empty());
        let mut empty = context();
        empty.cursor = None;
        assert_eq!(
            effective(&command(""), &empty)
                .unwrap_err()
                .sentence("Total size"),
            "Total size needs an item; none is selected."
        );
    }

    #[test]
    fn a_local_command_refuses_other_places() {
        let local = command("# diro: places = \"local\"\n");
        let mut remote = context();
        remote.other = Place::parse("sftp://example.org/home").unwrap();
        assert_eq!(effective(&local, &remote).unwrap_err(), Refusal::NotLocal);
        assert!(effective(&command(""), &remote).is_ok());
    }

    fn folders(root: &Path) -> StateFolders {
        StateFolders {
            runs: root.join("runs"),
            data: root.join("data").join("total-size"),
        }
    }

    fn value<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
        env.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    #[test]
    fn a_run_folder_holds_the_files_the_environment_names() {
        let root = tempfile::tempdir().unwrap();
        let mut context = context();
        context.marked = vec![file("/home/ada/one.txt"), file("/home/ada/two words.txt")];
        context.input = Some("report".into());
        context.dry_run = true;
        let mut tools = BTreeMap::new();
        tools.insert("magick".to_owned(), PathBuf::from("/usr/bin/magick"));
        let prepared =
            prepare(&command(""), &context, &folders(root.path()), false, &tools).unwrap();
        let env = &prepared.env;
        let file0 = PathBuf::from(value(env, "DIRO_SELECTION_FILE0").unwrap());
        let bytes = fs::read(&file0).unwrap();
        let expected = list_bytes(
            &[
                place("/home/ada/one.txt").to_string(),
                place("/home/ada/two words.txt").to_string(),
            ],
            ListForm::Nul,
        )
        .unwrap();
        assert_eq!(bytes, expected);
        let lines = fs::read_to_string(value(env, "DIRO_SELECTION_FILE").unwrap()).unwrap();
        assert_eq!(lines.lines().count(), 2);
        assert_eq!(value(env, "DIRO_SELECTION_COUNT").unwrap(), "2");
        assert_eq!(value(env, "DIRO_SELECTION_FROM_CURSOR").unwrap(), "0");
        assert_eq!(value(env, "DIRO_DRY_RUN").unwrap(), "1");
        assert_eq!(value(env, "DIRO_INPUT").unwrap(), "report");
        assert_eq!(value(env, "DIRO_TOOL_MAGICK").unwrap(), "/usr/bin/magick");
        assert_eq!(value(env, "DIRO_COMMAND").unwrap(), "script.total-size");
        assert_eq!(value(env, "TERM").unwrap(), "dumb");
        assert!(value(env, "DIRO_UNATTENDED").is_none());
        let context_file = value(env, "DIRO_CONTEXT_FILE").unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(context_file).unwrap()).unwrap();
        assert_eq!(json["selection_count"], 2);
        assert_eq!(json["command"], "script.total-size");
        assert_eq!(json["args"]["input"], "report");
        assert!(PathBuf::from(value(env, "DIRO_TEMP").unwrap()).is_dir());
        assert!(PathBuf::from(value(env, "DIRO_DATA_DIR").unwrap()).is_dir());
        // The cancel file appears only on cancel.
        let cancel = PathBuf::from(value(env, "DIRO_CANCEL_FILE").unwrap());
        assert!(!cancel.exists() && !prepared.folder.cancelled());
        prepared.folder.cancel().unwrap();
        assert!(cancel.exists() && prepared.folder.cancelled());
        let run = prepared.folder.path().to_path_buf();
        prepared.folder.remove().unwrap();
        assert!(!run.exists());
        assert!(root.path().join("runs").is_dir());
    }

    #[test]
    fn a_line_reader_is_refused_a_name_with_a_line_break() {
        let root = tempfile::tempdir().unwrap();
        let mut context = context();
        context.marked = vec![Picked::new(
            Place::parse_with(
                "/home/ada/line\nbreak",
                diroxide_core::place::PathStyle::Unix,
            )
            .unwrap_or_else(|_| place("/home/ada/plain")),
            EntryKind::File,
        )];
        if !context.marked[0].place.to_string().contains('\n') {
            // This system's places cannot hold a line break; the byte
            // tests above cover the rule.
            return;
        }
        let refused = prepare(
            &command(""),
            &context,
            &folders(root.path()),
            false,
            &BTreeMap::new(),
        );
        match refused {
            Err(PrepareError::Refused(r)) => {
                assert_eq!(
                    r.sentence("Total size"),
                    "Total size: a name has a line break."
                );
            }
            other => panic!("{other:?}"),
        }
        let nul = command("# diro: selection_file = \"nul\"\n");
        let prepared = prepare(
            &nul,
            &context,
            &folders(root.path()),
            false,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(value(&prepared.env, "DIRO_SELECTION_FILE").unwrap(), "");
    }

    #[test]
    fn forward_slashes_reach_gits_bash() {
        let root = tempfile::tempdir().unwrap();
        let mut context = context();
        context.marked = vec![file("/home/ada/one.txt")];
        let prepared = prepare(
            &command(""),
            &context,
            &folders(root.path()),
            true,
            &BTreeMap::new(),
        )
        .unwrap();
        let active = value(&prepared.env, "DIRO_ACTIVE")
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(!active.contains('\\'), "{active}");
        let list = value(&prepared.env, "DIRO_SELECTION_FILE0")
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(!list.contains('\\'), "{list}");
        let bytes = fs::read(list).unwrap();
        assert!(!bytes.contains(&b'\\'));
        prepared.folder.remove().unwrap();
    }

    #[test]
    fn the_process_gets_diroxides_environment_and_counts_its_level() {
        let root = tempfile::tempdir().unwrap();
        let prepared = prepare(
            &command(""),
            &context(),
            &folders(root.path()),
            false,
            &BTreeMap::new(),
        )
        .unwrap();
        let spec = process_spec(Path::new("bash"), &[], &prepared, &command(""));
        let (_, set) = spec.environment_changes();
        let names: Vec<String> = set
            .iter()
            .map(|(n, _)| n.to_string_lossy().into_owned())
            .collect();
        for wanted in [
            "DIRO_LEVEL",
            "NO_COLOR",
            "PYTHONUTF8",
            "DIRO_DIRECTIVES",
            "DIRO_CANCEL_FILE",
        ] {
            assert!(names.iter().any(|n| n == wanted), "{wanted} in {names:?}");
        }
        // The run's own DIRO_ values come after the inherited ones are
        // removed, so a parent's cannot leak through.
        let level = names.iter().position(|n| n == "DIRO_LEVEL").unwrap();
        let ours = names.iter().position(|n| n == "DIRO_ACTIVE").unwrap();
        assert!(level < ours);
        let mut terminal = command("# diro: mode = \"terminal\"\n");
        terminal.recipe = Recipe {
            mode: RunMode::Terminal,
            ..Recipe::default()
        };
        let spec = process_spec(Path::new("bash"), &[], &prepared, &terminal);
        assert_eq!(spec.tied(), diroxide_tools::process::Tie::Terminal);
        prepared.folder.remove().unwrap();
    }

    #[test]
    fn a_run_folder_outside_its_root_is_not_removed() {
        let root = tempfile::tempdir().unwrap();
        let folder = RunFolder::create(&root.path().join("runs")).unwrap();
        let moved = RunFolder {
            root: root.path().join("elsewhere"),
            path: folder.path().to_path_buf(),
        };
        assert!(moved.remove().is_err());
        assert!(folder.path().is_dir());
        folder.remove().unwrap();
    }
}
