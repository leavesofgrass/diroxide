//! Programs the window starts (R27's decision 8): the window has no
//! terminal to hand over, so Shell here opens the system's terminal in
//! the folder, Run command runs its line there and leaves the terminal
//! open on it, and F4 starts the editor directly; none of them is waited
//! for, so the window goes on while they run.
//!
//! [`WindowLauncher`] is the runtime's launcher in the window
//! (`Runtime::with_launcher`). It plans each program ([`plan`]) and hands
//! the plan to a starter: the real one ([`start`]) starts it detached and
//! reaps it on a thread of its own; the tests' starter records it, so no
//! test starts a program. It does not wait ([`Launcher::waits`] is
//! false), so the app hears that the program started, or why not, and
//! says it in its own words: "Started a shell in work", "Started the
//! editor on notes.txt" (lane A5's `AppEvent::ProgramStarted`). Its
//! causes are plain lowercase words, which the app says as they are
//! ([`plain_cause`]).
//!
//! - **Windows:** the shell, a command line, a console program and a
//!   `.cmd` or `.bat` file get a console of their own
//!   (`CREATE_NEW_CONSOLE`, started by the platform module, which passes
//!   the window's handles on to none), which is the system's terminal
//!   (Windows Terminal where it is the default). A command line keeps
//!   its window after the command (cmd's `/K`, PowerShell's `-NoExit`).
//!   A program with windows of its own, an editor such as Notepad, told
//!   apart by the subsystem its header names ([`pe_subsystem`]), starts
//!   through the shell (`ShellExecuteExW`), as File Explorer starts it
//!   (lane AF, from the owner's Notepad report of November 6, 2026: the
//!   Store Notepad's hand-off between its instances misbehaved on direct
//!   starts in a console, and opened cleanly through the shell). The
//!   shell passes no environment of its own, so such a program gets
//!   diroxide's environment without the `DIRO_` variables.
//! - **Linux and the BSDs:** the shell and a command line run in a
//!   terminal emulator: `$TERMINAL`, else the first of
//!   `x-terminal-emulator`, `gnome-terminal`, `konsole`,
//!   `xfce4-terminal` and `xterm` on the path; a command line keeps its
//!   shell open after it. Other programs start directly.
//! - **macOS** (not verified by a person): Terminal opens in the folder
//!   for the shell, and runs a command line through `osascript`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use diroxide_app::runtime::SystemLauncher;
use diroxide_app::{Launcher, Program};

/// What kind of program a plan starts, for how it is started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Started {
    /// The shell, in a terminal.
    Terminal,
    /// A command line, in a terminal left open on it.
    Command,
    /// A program on its own, the editor or a tool.
    Program,
}

/// The system the plan is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// Linux and the other Unix systems.
    Unix,
}

impl System {
    /// The system this runs on.
    pub const fn current() -> Self {
        if cfg!(windows) {
            System::Windows
        } else if cfg!(target_os = "macos") {
            System::MacOs
        } else {
            System::Unix
        }
    }
}

/// A program as the window starts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    /// The program.
    pub program: OsString,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// The working folder.
    pub folder: PathBuf,
    /// The environment the app gives it (`DIRO_ACTIVE` and the rest).
    pub env: Vec<(String, OsString)>,
    /// Whether it gets a console window of its own (Windows).
    pub new_console: bool,
    /// Whether it starts through the shell, as File Explorer starts a
    /// program with windows of its own (Windows; [`through_shell`]).
    pub through_shell: bool,
    /// What it is, for what is said.
    pub started: Started,
}

/// The subsystem a Windows program's header names
/// (`IMAGE_SUBSYSTEM_WINDOWS_GUI` for a program with windows of its own).
pub const WINDOW_SUBSYSTEM: u16 = 2;

/// The subsystem named in the header of a Windows program, from the
/// file's first bytes: 2 for a program with windows of its own, 3 for a
/// console program; `None` when the bytes are not a program's header or
/// stop short of the field.
pub fn pe_subsystem(head: &[u8]) -> Option<u16> {
    let word = |at: usize| -> Option<u16> {
        let bytes = head.get(at..at.checked_add(2)?)?;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    if head.get(0..2)? != b"MZ" {
        return None;
    }
    let pointer = head.get(0x3c..0x40)?;
    let header = u32::from_le_bytes([pointer[0], pointer[1], pointer[2], pointer[3]]) as usize;
    if head.get(header..header.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    // The optional header follows the four signature bytes and the
    // 20-byte file header; its magic says PE32 or PE32+, and the
    // subsystem is 68 bytes in for both.
    let optional = header.checked_add(24)?;
    if !matches!(word(optional)?, 0x10b | 0x20b) {
        return None;
    }
    word(optional.checked_add(68)?)
}

/// How far into a file [`read_subsystem`] reads at most: a header placed
/// further in is not a program the window starts through the shell.
const HEADER_LIMIT: u64 = 64 * 1024;

/// The subsystem of the program at `path`, read from its header;
/// `None` when it cannot be read or is not a program's header.
pub fn read_subsystem(path: &Path) -> Option<u16> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.take(HEADER_LIMIT).read_to_end(&mut head).ok()?;
    pe_subsystem(&head)
}

/// The file a program name starts on Windows: a name with a folder in it
/// as it is; else the first folder of `path_var` holding the name, or the
/// name with one of `path_ext`'s endings when it has no ending of its
/// own, as `is_file` says.
pub fn resolve_program(
    program: &std::ffi::OsStr,
    path_var: Option<&std::ffi::OsStr>,
    path_ext: Option<&str>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let name = Path::new(program);
    if name.components().count() > 1 || name.is_absolute() {
        return is_file(name).then(|| name.to_path_buf());
    }
    let endings: Vec<String> = if name.extension().is_some() {
        vec![String::new()]
    } else {
        path_ext
            .unwrap_or(".COM;.EXE;.BAT;.CMD")
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let folders = std::env::split_paths(path_var?).collect::<Vec<_>>();
    folders.iter().find_map(|folder| {
        endings.iter().find_map(|ending| {
            let mut file = program.to_os_string();
            file.push(ending);
            let candidate = folder.join(file);
            is_file(&candidate).then_some(candidate)
        })
    })
}

/// Whether a Windows program starts through the shell: a program on its
/// own (not the shell or a command line), found as an `.exe` file whose
/// header names windows of its own. A console program, a `.cmd` or
/// `.bat` file, or a program not found keeps its console of its own, so
/// a program not found is said as before.
pub fn through_shell(started: Started, file: Option<&Path>, subsystem: Option<u16>) -> bool {
    started == Started::Program
        && file
            .and_then(Path::extension)
            .is_some_and(|ending| ending.eq_ignore_ascii_case("exe"))
        && subsystem == Some(WINDOW_SUBSYSTEM)
}

/// A Windows plan for a program with windows of its own made to start
/// through the shell, on its full path, with no console of its own; any
/// other plan is left as it is.
pub fn use_shell_for_window_program(
    planned: &mut Planned,
    file: Option<PathBuf>,
    subsystem: Option<u16>,
) {
    if let Some(file) = file.filter(|f| through_shell(planned.started, Some(f), subsystem)) {
        planned.program = file.into_os_string();
        planned.new_console = false;
        planned.through_shell = true;
    }
}

/// What the runtime asked for, sorted: the shell alone is the shell; the
/// shell with arguments runs a command line; anything else is a program.
pub fn kind_of(program: &Program, shell: &OsString) -> Started {
    if &program.program == shell {
        if program.args.is_empty() {
            Started::Terminal
        } else {
            Started::Command
        }
    } else {
        Started::Program
    }
}

/// The terminal emulator on a Unix system: `terminal` (the `TERMINAL`
/// variable) when set, else the first of the usual ones that `found`
/// says is on the path.
pub fn unix_terminal(terminal: Option<OsString>, found: impl Fn(&str) -> bool) -> Option<OsString> {
    if let Some(terminal) = terminal.filter(|t| !t.is_empty()) {
        return Some(terminal);
    }
    [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "xterm",
    ]
    .into_iter()
    .find(|name| found(name))
    .map(OsString::from)
}

/// A text as an AppleScript string, its backslashes and quotes escaped.
fn apple_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A word as a POSIX shell takes it, in single quotes.
fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// How the window starts `program` on `system`: `shell` is the launcher's
/// shell, `terminal` the Unix terminal emulator, if any. An error says
/// why it cannot start, in words.
pub fn plan(
    program: &Program,
    shell: &OsString,
    system: System,
    terminal: Option<OsString>,
) -> Result<Planned, String> {
    let started = kind_of(program, shell);
    let mut planned = Planned {
        program: program.program.clone(),
        args: program.args.clone(),
        folder: program.folder.clone(),
        env: program.env.clone(),
        new_console: false,
        through_shell: false,
        started,
    };
    match system {
        System::Windows => {
            planned.new_console = true;
            if started == Started::Command {
                keep_open_on_windows(&mut planned.args);
            }
        }
        System::Unix => {
            if started != Started::Program {
                let terminal =
                    terminal.ok_or_else(|| "no terminal program was found".to_owned())?;
                let mut inner = vec![program.program.clone()];
                let mut args = program.args.clone();
                if started == Started::Command
                    && let Some(line) = args.pop()
                {
                    // The shell stays open after the line, so its output
                    // can be read.
                    let shell_text = program.program.to_string_lossy().into_owned();
                    args.push(OsString::from(format!(
                        "{}; exec {}",
                        line.to_string_lossy(),
                        shell_word(&shell_text)
                    )));
                }
                inner.extend(args);
                let separator = if Path::new(&terminal).file_name()
                    == Some(std::ffi::OsStr::new("gnome-terminal"))
                {
                    "--"
                } else {
                    "-e"
                };
                planned.program = terminal;
                planned.args = std::iter::once(OsString::from(separator))
                    .chain(inner)
                    .collect();
            }
        }
        System::MacOs => match started {
            Started::Terminal => {
                planned.program = OsString::from("open");
                planned.args = vec![
                    OsString::from("-a"),
                    OsString::from("Terminal"),
                    program.folder.clone().into_os_string(),
                ];
            }
            Started::Command => {
                let line = program
                    .args
                    .last()
                    .map(|l| l.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let folder = program.folder.to_string_lossy().into_owned();
                let script = format!(
                    "tell application \"Terminal\" to do script {}",
                    apple_string(&format!("cd {}; {line}", shell_word(&folder)))
                );
                planned.program = OsString::from("osascript");
                planned.args = vec![OsString::from("-e"), OsString::from(script)];
            }
            Started::Program => {}
        },
    }
    Ok(planned)
}

/// A command line's arguments made to keep its window open after it on
/// Windows: cmd's `/C` becomes `/K`, PowerShell gains `-NoExit`.
fn keep_open_on_windows(args: &mut Vec<OsString>) {
    for arg in args.iter_mut() {
        if arg.to_string_lossy().eq_ignore_ascii_case("/c") {
            *arg = OsString::from("/K");
            return;
        }
    }
    if let Some(at) = args
        .iter()
        .position(|a| a.to_string_lossy().eq_ignore_ascii_case("-command"))
    {
        args.insert(at, OsString::from("-NoExit"));
    }
}

/// Starts a planned program detached: no waiting, its handles not the
/// window's, reaped on a thread of its own when it ends.
pub fn start(planned: &Planned) -> Result<(), String> {
    #[cfg(windows)]
    if planned.through_shell {
        // As File Explorer starts it: the shell hands the file to the
        // program, a Store app's launcher included.
        return crate::platform::win::start_through_shell(
            &planned.program,
            &planned.args,
            &planned.folder,
        );
    }
    #[cfg(windows)]
    if planned.new_console {
        // The standard library always passes this process's handles, so
        // a shell in its new console would read and write elsewhere; the
        // platform call passes none, and the console owns them.
        return crate::platform::win::start_in_new_console(
            &planned.program,
            &planned.args,
            &planned.folder,
            &planned.env,
        );
    }
    let mut command = std::process::Command::new(&planned.program);
    command
        .args(&planned.args)
        .current_dir(&planned.folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (name, value) in &planned.env {
        command.env(name, value);
    }
    let mut child = command.spawn().map_err(|e| plain_cause(&e))?;
    let _ = std::thread::Builder::new()
        .name("diroxide-gui-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// What starts a planned program: the real [`start`], or a test's.
pub type Starter = Box<dyn Fn(&Planned) -> Result<(), String> + Send + Sync>;

/// Why a program could not start, in plain lowercase words, as the app
/// says a launcher's cause word for word: "the program was not found",
/// or the system's own words with their first letter lowered and the
/// error's number left off.
pub fn plain_cause(error: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => "the program was not found".to_owned(),
        ErrorKind::PermissionDenied => "the system did not let it run".to_owned(),
        _ => {
            let text = error.to_string();
            let text = text
                .split(" (os error")
                .next()
                .unwrap_or(&text)
                .trim()
                .trim_end_matches('.');
            let mut chars = text.chars();
            match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect(),
                None => "it could not start".to_owned(),
            }
        }
    }
}

/// The runtime's launcher in the window.
pub struct WindowLauncher {
    system: SystemLauncher,
    starter: Starter,
    /// The Unix terminal emulator, when a test chooses it; else
    /// `TERMINAL` or the first on the path.
    terminal: Option<OsString>,
}

impl WindowLauncher {
    /// A launcher that starts programs for real.
    pub fn new() -> Self {
        Self::with_starter(Box::new(start))
    }

    /// A launcher that hands each plan to `starter` (the tests record it).
    pub fn with_starter(starter: Starter) -> Self {
        Self {
            system: SystemLauncher,
            starter,
            terminal: None,
        }
    }

    /// The same launcher with this Unix terminal emulator (the tests'
    /// choice, so they never depend on what the machine has).
    pub fn with_terminal(mut self, terminal: impl Into<OsString>) -> Self {
        self.terminal = Some(terminal.into());
        self
    }
}

impl Default for WindowLauncher {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for WindowLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowLauncher").finish_non_exhaustive()
    }
}

/// Whether a program is on the path.
fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|folder| folder.join(name).is_file())
    })
}

impl Launcher for WindowLauncher {
    fn run(&self, program: &Program) -> Result<Option<i32>, String> {
        let (shell, _) = self.shell();
        let terminal = match System::current() {
            System::Unix => self
                .terminal
                .clone()
                .or_else(|| unix_terminal(std::env::var_os("TERMINAL"), on_path)),
            _ => None,
        };
        let mut planned = plan(program, &shell, System::current(), terminal)?;
        if System::current() == System::Windows && planned.started == Started::Program {
            let file = resolve_program(
                &planned.program,
                std::env::var_os("PATH").as_deref(),
                std::env::var("PATHEXT").ok().as_deref(),
                Path::is_file,
            );
            let subsystem = file.as_deref().and_then(read_subsystem);
            use_shell_for_window_program(&mut planned, file, subsystem);
        }
        // Nothing has ended: the app says what started, or why not.
        (self.starter)(&planned).map(|()| None)
    }

    fn waits(&self) -> bool {
        false
    }

    fn shell(&self) -> (OsString, Vec<OsString>) {
        self.system.shell()
    }

    fn editor(&self) -> OsString {
        self.system.editor()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(name: &str, args: &[&str]) -> Program {
        Program {
            program: OsString::from(name),
            args: args.iter().map(OsString::from).collect(),
            folder: PathBuf::from("work"),
            env: vec![("DIRO_ACTIVE".to_owned(), OsString::from("work"))],
        }
    }

    #[test]
    fn windows_gives_each_program_its_own_console_and_keeps_a_command_open() {
        let shell = OsString::from("cmd.exe");
        let p = plan(&program("cmd.exe", &[]), &shell, System::Windows, None).unwrap();
        assert!(p.new_console);
        assert_eq!(p.started, Started::Terminal);
        assert!(p.args.is_empty());
        let p = plan(
            &program("cmd.exe", &["/C", "dir"]),
            &shell,
            System::Windows,
            None,
        )
        .unwrap();
        assert_eq!(p.started, Started::Command);
        assert_eq!(p.args, [OsString::from("/K"), OsString::from("dir")]);
        let pwsh = OsString::from("pwsh.exe");
        let p = plan(
            &program("pwsh.exe", &["-NoLogo", "-Command", "ls"]),
            &pwsh,
            System::Windows,
            None,
        )
        .unwrap();
        assert_eq!(
            p.args,
            ["-NoLogo", "-NoExit", "-Command", "ls"].map(OsString::from)
        );
        // The editor starts directly, with its file, and the environment.
        let p = plan(
            &program("notepad.exe", &["notes.txt"]),
            &shell,
            System::Windows,
            None,
        )
        .unwrap();
        assert_eq!(p.started, Started::Program);
        assert_eq!(p.program, OsString::from("notepad.exe"));
        assert_eq!(p.args, [OsString::from("notes.txt")]);
        assert_eq!(p.env.len(), 1);
    }

    /// A made program header: the DOS stub's pointer at 0x3c to the
    /// signature at `at`, the file header, and the optional header's
    /// magic and subsystem.
    fn header(at: usize, magic: u16, subsystem: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; at + 24 + 70];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(at as u32).to_le_bytes());
        bytes[at..at + 4].copy_from_slice(b"PE\0\0");
        bytes[at + 24..at + 26].copy_from_slice(&magic.to_le_bytes());
        bytes[at + 24 + 68..at + 24 + 70].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }

    #[test]
    fn a_programs_header_says_whether_it_has_windows_of_its_own() {
        assert_eq!(
            pe_subsystem(&header(0x80, 0x20b, 2)),
            Some(WINDOW_SUBSYSTEM)
        );
        assert_eq!(pe_subsystem(&header(0xf8, 0x10b, 3)), Some(3));
        // Not a program, a short file, a pointer past the end, an
        // unknown magic: no subsystem, so the console start stays.
        assert_eq!(pe_subsystem(b"@echo off\r\n"), None);
        assert_eq!(pe_subsystem(b"MZ"), None);
        let mut past = header(0x80, 0x20b, 2);
        past[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(pe_subsystem(&past), None);
        assert_eq!(pe_subsystem(&header(0x80, 0x107, 2)), None);
        let short = header(0x80, 0x20b, 2);
        assert_eq!(pe_subsystem(&short[..0x80 + 24 + 69]), None);
    }

    #[test]
    fn a_program_name_is_found_on_the_path_with_its_endings() {
        let folders = std::env::join_paths([PathBuf::from("one"), PathBuf::from("two")]).unwrap();
        let there = |files: &'static [&'static str]| {
            move |p: &Path| files.iter().any(|f| p == Path::new(f))
        };
        let two = |name: &str| PathBuf::from("two").join(name);
        let found = resolve_program(
            std::ffi::OsStr::new("notepad.exe"),
            Some(&folders),
            None,
            there(&["two/notepad.exe", "two\\notepad.exe"]),
        );
        assert_eq!(found, Some(two("notepad.exe")));
        // No ending: PATHEXT's, in order; a .cmd is found as a .cmd.
        let found = resolve_program(
            std::ffi::OsStr::new("tw"),
            Some(&folders),
            Some(".EXE;.CMD"),
            there(&["two/tw.CMD", "two\\tw.CMD"]),
        );
        assert_eq!(found, Some(two("tw.CMD")));
        assert_eq!(
            resolve_program(std::ffi::OsStr::new("gone"), Some(&folders), None, |_| {
                false
            }),
            None
        );
        assert_eq!(
            resolve_program(std::ffi::OsStr::new("x.exe"), None, None, |_| true),
            None
        );
    }

    #[test]
    fn only_a_program_with_windows_of_its_own_starts_through_the_shell() {
        let shell = OsString::from("cmd.exe");
        let editor = plan(
            &program("notepad.exe", &["notes.txt"]),
            &shell,
            System::Windows,
            None,
        )
        .unwrap();
        let full = PathBuf::from("C:/Windows/System32/notepad.exe");
        let mut p = editor.clone();
        use_shell_for_window_program(&mut p, Some(full.clone()), Some(WINDOW_SUBSYSTEM));
        assert!(p.through_shell && !p.new_console);
        assert_eq!(p.program, full.clone().into_os_string());
        assert_eq!(p.args, [OsString::from("notes.txt")]);
        // A console program, a .cmd, a program not found: a console of
        // its own, as before.
        for (file, subsystem) in [
            (Some(PathBuf::from("C:/tools/vim.exe")), Some(3)),
            (
                Some(PathBuf::from("C:/tools/edit.cmd")),
                Some(WINDOW_SUBSYSTEM),
            ),
            (None, None),
        ] {
            let mut p = editor.clone();
            use_shell_for_window_program(&mut p, file, subsystem);
            assert!(p.new_console && !p.through_shell, "{p:?}");
            assert_eq!(p.program, OsString::from("notepad.exe"));
        }
        // The shell itself never goes through it.
        let mut p = plan(&program("cmd.exe", &[]), &shell, System::Windows, None).unwrap();
        use_shell_for_window_program(&mut p, Some(full), Some(WINDOW_SUBSYSTEM));
        assert!(p.new_console && !p.through_shell);
    }

    #[test]
    fn unix_runs_the_shell_in_a_terminal_and_the_editor_directly() {
        let shell = OsString::from("/bin/bash");
        let xterm = Some(OsString::from("xterm"));
        let p = plan(
            &program("/bin/bash", &[]),
            &shell,
            System::Unix,
            xterm.clone(),
        )
        .unwrap();
        assert_eq!(p.program, OsString::from("xterm"));
        assert_eq!(p.args, ["-e", "/bin/bash"].map(OsString::from));
        let p = plan(
            &program("/bin/bash", &["-c", "make"]),
            &shell,
            System::Unix,
            Some(OsString::from("gnome-terminal")),
        )
        .unwrap();
        assert_eq!(
            p.args,
            ["--", "/bin/bash", "-c", "make; exec '/bin/bash'"].map(OsString::from)
        );
        let p = plan(
            &program("gedit", &["notes.txt"]),
            &shell,
            System::Unix,
            None,
        )
        .unwrap();
        assert_eq!(p.program, OsString::from("gedit"));
        // No terminal: said in words, with a next step.
        let err = plan(&program("/bin/bash", &[]), &shell, System::Unix, None).unwrap_err();
        assert_eq!(err, "no terminal program was found");
        assert_eq!(
            unix_terminal(None, |name| name == "konsole"),
            Some(OsString::from("konsole"))
        );
        assert_eq!(
            unix_terminal(Some(OsString::from("foot")), |_| true),
            Some(OsString::from("foot"))
        );
    }

    #[test]
    fn macos_opens_terminal_in_the_folder() {
        let shell = OsString::from("/bin/zsh");
        let p = plan(&program("/bin/zsh", &[]), &shell, System::MacOs, None).unwrap();
        assert_eq!(p.program, OsString::from("open"));
        assert_eq!(p.args, ["-a", "Terminal", "work"].map(OsString::from));
        let p = plan(
            &program("/bin/zsh", &["-c", "echo \"hi\""]),
            &shell,
            System::MacOs,
            None,
        )
        .unwrap();
        assert_eq!(p.program, OsString::from("osascript"));
        let script = p.args[1].to_string_lossy().into_owned();
        assert!(script.contains("cd 'work'; echo \\\"hi\\\""), "{script}");
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_command_line_quotes_as_programs_split_it() {
        let line = |program: &str, args: &[&str]| {
            let args: Vec<OsString> = args.iter().map(OsString::from).collect();
            String::from_utf16(&crate::platform::win::command_line(
                std::ffi::OsStr::new(program),
                &args,
            ))
            .unwrap()
        };
        assert_eq!(line("cmd.exe", &["/K", "dir"]), "cmd.exe /K dir");
        assert_eq!(
            line(
                r"C:\Program Files\pwsh.exe",
                &["-Command", "Get-ChildItem 'a b'"]
            ),
            r#""C:\Program Files\pwsh.exe" -Command "Get-ChildItem 'a b'""#
        );
        assert_eq!(line("x", &[r#"say "hi""#]), r#"x "say \"hi\"""#);
        assert_eq!(line("x", &[r"D:\a b\"]), r#"x "D:\a b\\""#);
        assert_eq!(line("x", &[""]), r#"x """#);
    }

    #[test]
    fn a_cause_is_plain_lowercase_words_and_the_launcher_does_not_wait() {
        use std::io::{Error, ErrorKind};
        assert_eq!(
            plain_cause(&Error::from(ErrorKind::NotFound)),
            "the program was not found"
        );
        assert_eq!(
            plain_cause(&Error::from(ErrorKind::PermissionDenied)),
            "the system did not let it run"
        );
        assert_eq!(
            plain_cause(&Error::other("The pipe is being closed. (os error 232)")),
            "the pipe is being closed"
        );
        assert!(!WindowLauncher::new().waits());
    }
}
