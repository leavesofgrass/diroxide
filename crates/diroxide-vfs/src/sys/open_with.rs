//! Open with: the programs the system lists for a file, read without a
//! shell (R28 4.3; R30's commits 26 to 28).
//!
//! - **Linux and the BSDs** (`xdg_apps`): `mimeapps.list` in the
//!   config and data folders, the `mimeinfo.cache` files, and each
//!   program's `.desktop` file; its `Exec` line becomes an argument list.
//! - **Windows** (`windows_open_with`, `windows_command`): per extension, `OpenWithList` and
//!   `OpenWithProgids` under the user's `FileExts` key and the classes,
//!   and `Applications\NAME\shell\open\command`, the user's hive before the
//!   machine's; the command becomes an argument list with the file where
//!   `%1` is. A handler with no command (a Store app) is started through
//!   the system by its class. "Choose another program" hands the file to
//!   the system's own dialog, started and never waited for.
//! - **macOS** (`mac_apps`): the application folders' bundles,
//!   each `Info.plist` read for the document types it declares, started
//!   with `open -a`.
//!
//! A program's name and the file's name never pass through a shell: each
//! argument reaches the program whole, so a name with a space, a quote or
//! a dollar sign arrives as it is.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// One piece of an argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgPart {
    /// Text as it is.
    Text(String),
    /// The file's place (`%1` on Windows, `%f` or `%u` in a desktop file).
    File,
    /// Every file, each its own argument (`%F` or `%U`); with one file,
    /// that file.
    Files,
}

/// A program and its arguments, the file's place marked: what a
/// command line becomes, so nothing is ever run as a shell string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCommand {
    /// The program, a path or a name looked up on PATH.
    pub program: String,
    /// The arguments, each a list of pieces.
    pub args: Vec<Vec<ArgPart>>,
}

impl OpenCommand {
    /// The program and its arguments for one file. An argument that is
    /// the file's place alone becomes the file whole; one with text
    /// around the place gets the file's text put in it. A command that
    /// names no place gets the file as its last argument.
    pub fn for_file(&self, file: &OsStr) -> (OsString, Vec<OsString>) {
        let mut args = Vec::with_capacity(self.args.len() + 1);
        let mut placed = false;
        for arg in &self.args {
            match arg.as_slice() {
                [ArgPart::File | ArgPart::Files] => {
                    args.push(file.to_owned());
                    placed = true;
                }
                parts => {
                    let mut out = OsString::new();
                    for part in parts {
                        match part {
                            ArgPart::Text(text) => out.push(text),
                            ArgPart::File | ArgPart::Files => {
                                out.push(file);
                                placed = true;
                            }
                        }
                    }
                    args.push(out);
                }
            }
        }
        if !placed {
            args.push(file.to_owned());
        }
        (OsString::from(&self.program), args)
    }

    /// The program's file name, for a name when nothing better is known:
    /// `notepad` from `C:\Windows\notepad.exe`.
    pub fn program_stem(&self) -> String {
        let name = self
            .program
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&self.program);
        let lower = name.to_ascii_lowercase();
        match lower.strip_suffix(".exe") {
            Some(_) => name[..name.len() - 4].to_owned(),
            None => name.to_owned(),
        }
    }
}

/// How a listed program is started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// Run this program with these arguments.
    Command(OpenCommand),
    /// Hand the file to the system with this class (a Windows ProgID
    /// whose handler has no command line, such as a Store app); the
    /// system starts its handler.
    Class(String),
}

/// One program that can open a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWithApp {
    /// The name said and listed: "Notepad", "Text Editor".
    pub name: String,
    /// How it is started.
    pub launch: Launch,
    /// Where the system keeps it: the executable's name, the class, the
    /// desktop file's id, or the bundle's path. Two rows with one id are
    /// one program.
    pub id: String,
    /// Whether it is the system's default for this kind of file.
    pub default: bool,
}

impl OpenWithApp {
    /// Starts it on one file, never waited for: its command with the file
    /// in its place, or the system's handler for its class.
    pub fn start(&self, file: &Path) -> std::io::Result<()> {
        match &self.launch {
            Launch::Command(command) => {
                let (program, args) = command.for_file(file.as_os_str());
                std::process::Command::new(program)
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map(drop)
            }
            Launch::Class(class) => start_by_class(class, file),
        }
    }
}

/// The programs for one file, in the system's order, the default first
/// when the system names one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenWithList {
    /// The programs.
    pub apps: Vec<OpenWithApp>,
    /// Whether the system has its own dialog for the rest ("Choose
    /// another program"): Windows.
    pub choose_another: bool,
}

impl OpenWithList {
    /// The first row of an Open with list, the system's own choice.
    pub const DEFAULT_PROGRAM: &str = "Default program";

    /// The last row on systems with their own Open with dialog.
    pub const CHOOSE_ANOTHER: &str = "Choose another program";

    /// The list's title for a file: "Open report.pdf with:".
    pub fn title(name: &str) -> String {
        format!("Open {name} with:")
    }

    /// Adds a program unless one with the same id or the same command is
    /// in already; a later default marks the one in.
    pub fn push(&mut self, app: OpenWithApp) {
        let same = |a: &OpenWithApp| {
            a.id.eq_ignore_ascii_case(&app.id)
                || match (&a.launch, &app.launch) {
                    (Launch::Command(x), Launch::Command(y)) => {
                        x.program.eq_ignore_ascii_case(&y.program)
                    }
                    _ => false,
                }
        };
        match self.apps.iter_mut().find(|a| same(a)) {
            Some(known) => known.default |= app.default,
            None => self.apps.push(app),
        }
    }

    /// The default first, the rest in the order found.
    pub fn default_first(mut self) -> Self {
        self.apps.sort_by_key(|a| !a.default);
        self
    }
}

/// The programs this system lists for a file, read without a shell and
/// without opening it: by its extension on Windows, by its media type
/// (the type registry's `mime`) on Linux and the BSDs, by both on macOS.
/// An empty list where the system says nothing.
pub fn open_with_list(file: &Path, mime: Option<&str>) -> OpenWithList {
    #[cfg(windows)]
    {
        let _ = mime;
        super::windows_open_with::open_with_list(file)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        super::xdg_apps::open_with_list(file, mime)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = (file, mime);
        OpenWithList::default()
    }
}

/// Hands the file to the system's own Open with dialog ("Choose another
/// program"), started and never waited for. Windows only; elsewhere it
/// says the system has none.
pub fn choose_another_program(file: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        super::windows_open_with::choose_another_program(file)
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "This system has no Open with dialog of its own.",
        ))
    }
}

fn start_by_class(class: &str, file: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        super::windows_open_with::start_by_class(class, file)
    }
    #[cfg(not(windows))]
    {
        let _ = (class, file);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Starting a program by its class is for Windows.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_keeps_one_row_a_program_and_the_default_first() {
        let command = |program: &str| {
            Launch::Command(OpenCommand {
                program: program.to_owned(),
                args: vec![vec![ArgPart::File]],
            })
        };
        let mut list = OpenWithList::default();
        list.push(OpenWithApp {
            name: "Notepad".into(),
            launch: command(r"C:\Windows\notepad.exe"),
            id: "notepad.exe".into(),
            default: false,
        });
        list.push(OpenWithApp {
            name: "Text Editor".into(),
            launch: command(r"C:\Tools\edit.exe"),
            id: "edit.exe".into(),
            default: false,
        });
        list.push(OpenWithApp {
            name: "Notepad".into(),
            launch: command(r"c:\windows\NOTEPAD.EXE"),
            id: "txtfile".into(),
            default: true,
        });
        let list = list.default_first();
        assert_eq!(list.apps.len(), 2);
        assert_eq!(list.apps[0].name, "Notepad");
        assert!(list.apps[0].default);
        assert_eq!(OpenWithList::title("report.pdf"), "Open report.pdf with:");
        for line in [
            OpenWithList::title("report.pdf").as_str(),
            OpenWithList::DEFAULT_PROGRAM,
            OpenWithList::CHOOSE_ANOTHER,
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
    }
}
