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
//! test starts a program. Each start is noted ([`Started`]) for the
//! frontend, which says what opened in place of the app's "Back in
//! diroxide", since nothing has ended yet.
//!
//! - **Windows:** every program gets a console of its own
//!   (`CREATE_NEW_CONSOLE`, started by the platform module, which passes
//!   the window's handles on to none), which is the system's terminal
//!   (Windows Terminal where it is the default); a program with its own
//!   window, an editor such as Notepad, simply opens. A command line
//!   keeps its window after the command (cmd's `/K`, PowerShell's
//!   `-NoExit`).
//! - **Linux and the BSDs:** the shell and a command line run in a
//!   terminal emulator: `$TERMINAL`, else the first of
//!   `x-terminal-emulator`, `gnome-terminal`, `konsole`,
//!   `xfce4-terminal` and `xterm` on the path; a command line keeps its
//!   shell open after it. Other programs start directly.
//! - **macOS** (not verified by a person): Terminal opens in the folder
//!   for the shell, and runs a command line through `osascript`.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_app::runtime::SystemLauncher;
use diroxide_app::{Launcher, Program};

/// What the window started, for what it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Started {
    /// The shell, in a terminal: "Opened a terminal in work".
    Terminal,
    /// A command line, in a terminal: "Started the command in work".
    Command,
    /// A program on its own, the editor or a tool: "Opened notes.txt".
    Program,
}

impl Started {
    /// What is said when it started: `what` is the app's name for it
    /// (the file for the editor, the tool's name), `place` the folder's.
    pub fn words(self, what: &str, place: &str) -> String {
        match self {
            Started::Terminal => format!("Opened a terminal in {place}"),
            Started::Command => format!("Started the command in {place}"),
            Started::Program => format!("Opened {what}"),
        }
    }
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
    /// What it is, for what is said.
    pub started: Started,
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
                let terminal = terminal.ok_or_else(|| {
                    "no terminal program was found; set the TERMINAL variable to one".to_owned()
                })?;
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
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let _ = std::thread::Builder::new()
        .name("diroxide-gui-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// What starts a planned program: the real [`start`], or a test's.
pub type Starter = Box<dyn Fn(&Planned) -> Result<(), String> + Send + Sync>;

/// The runtime's launcher in the window.
pub struct WindowLauncher {
    system: SystemLauncher,
    starter: Starter,
    started: Arc<Mutex<VecDeque<Started>>>,
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
            started: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// The starts not yet said, shared with the frontend.
    pub fn started(&self) -> Arc<Mutex<VecDeque<Started>>> {
        Arc::clone(&self.started)
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
            System::Unix => unix_terminal(std::env::var_os("TERMINAL"), on_path),
            _ => None,
        };
        let planned = plan(program, &shell, System::current(), terminal)?;
        (self.starter)(&planned)?;
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(planned.started);
        // Nothing has ended: the frontend says what opened.
        Ok(None)
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
        assert!(err.contains("TERMINAL"), "{err}");
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
    fn what_is_said_puts_the_meaning_first() {
        assert_eq!(
            Started::Terminal.words("Shell", "work"),
            "Opened a terminal in work"
        );
        assert_eq!(
            Started::Command.words("Command", "work"),
            "Started the command in work"
        );
        assert_eq!(
            Started::Program.words("notes.txt", "work"),
            "Opened notes.txt"
        );
    }
}
