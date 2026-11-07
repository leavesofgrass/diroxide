//! The folders a task may never name (R16 section 4, rule 4; R28 6.3):
//! a path written any way but literally and absolutely, and the folders
//! that are too broad or not the owner's to automate.
//!
//! **Literal and absolute.** A task's folders are written out in full: no
//! wildcard (`*`, `?`, `[`, `]`), no variable (`$`, `%`), no `~`, no
//! bare drive letter (`C:` without a slash means the current folder on
//! that drive), no `.` or `..`, no device prefix (`\\?\`, `\\.\`), no
//! address, no hidden character, and no name ending in a dot or a space
//! (Windows drops them). Nothing in a path is ever expanded.
//!
//! **Refused folders.** A drive root, the home folder itself, a folder
//! that holds diroxide's settings or state folder or lies inside one, a
//! trash folder, a system folder, and the folder the program lives in.
//! Downloads is allowed: its folder is not a drive root.

use std::path::Path;

use diroxide_core::names::classify;
use diroxide_core::place::PathStyle;
use diroxide_core::{Place, PlaceKind};

/// What a task's folders are checked against: the computer's own folders,
/// read once by the app or the command line ([`Surroundings::of_this_computer`]),
/// or given by a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surroundings {
    /// How paths are written here.
    pub style: PathStyle,
    /// The owner's home folder.
    pub home: Option<Place>,
    /// diroxide's own folders: the config and state folders.
    pub own: Vec<Place>,
    /// The folder the program lives in.
    pub program: Option<Place>,
    /// System folders: a task never lies in one.
    pub system: Vec<Place>,
    /// Trash folders known by path, beside the ones known by name
    /// (`$Recycle.Bin`, `.Trash`, `.Trash-1000`).
    pub trash: Vec<Place>,
}

impl Surroundings {
    /// Nothing known but the path style: only the drive root, the trash
    /// names and the literal rules refuse.
    pub fn none(style: PathStyle) -> Self {
        Self {
            style,
            home: None,
            own: Vec::new(),
            program: None,
            system: Vec::new(),
            trash: Vec::new(),
        }
    }

    /// This computer's folders: the home folder from the environment, the
    /// config and state folders given, the program's folder, the system
    /// folders and the owner's trash. A folder that cannot be read is left
    /// out; the literal rules and the drive root still hold.
    pub fn of_this_computer(config: &Path, state: &Path) -> Self {
        let style = PathStyle::native();
        let place = |path: &Path| Place::parse_with(&path.to_string_lossy(), style).ok();
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = std::env::var_os(home_var)
            .filter(|h| !h.is_empty())
            .and_then(|h| place(Path::new(&h)));
        let own = [config, state].into_iter().filter_map(place).collect();
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().and_then(place));
        let mut system: Vec<Place> = Vec::new();
        let mut trash: Vec<Place> = Vec::new();
        if cfg!(windows) {
            for var in [
                "SystemRoot",
                "windir",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
                "ProgramData",
            ] {
                if let Some(value) = std::env::var_os(var).filter(|v| !v.is_empty())
                    && let Some(folder) = place(Path::new(&value))
                    && !system.contains(&folder)
                {
                    system.push(folder);
                }
            }
        } else {
            let tops: &[&str] = if cfg!(target_os = "macos") {
                &[
                    "/System", "/Library", "/private", "/usr", "/bin", "/sbin", "/etc", "/var",
                    "/dev", "/cores", "/opt",
                ]
            } else {
                &[
                    "/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/opt",
                    "/proc", "/run", "/sbin", "/sys", "/usr", "/var",
                ]
            };
            system.extend(tops.iter().filter_map(|top| place(Path::new(top))));
            if let Some(home) = &home {
                let data = std::env::var_os("XDG_DATA_HOME")
                    .filter(|v| !v.is_empty())
                    .and_then(|v| place(Path::new(&v)))
                    .or_else(|| home.join(".local").and_then(|p| p.join("share")).ok());
                if let Some(data) = data
                    && let Ok(bin) = data.join("Trash")
                {
                    trash.push(bin);
                }
            }
        }
        Self {
            style,
            home,
            own,
            program,
            system,
            trash,
        }
    }
}

/// Whether a name is one the platforms give a trash folder:
/// `$Recycle.Bin` and the older `RECYCLER` on Windows, `.Trash` on macOS
/// and at the top of Linux mounts, `.Trash-1000` for a user's own.
pub fn is_trash_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "$recycle.bin"
        || lower == "recycler"
        || name == ".Trash"
        || name
            .strip_prefix(".Trash-")
            .is_some_and(|uid| !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()))
}

/// Checks that `text` is a literal, absolute folder path in `style` and
/// reads it as a place on this computer. The error is the rest of a
/// sentence that starts with `field`: "home has a wildcard".
pub fn literal_folder(field: &str, text: &str, style: PathStyle) -> Result<Place, String> {
    if text.trim().is_empty() {
        return Err(format!("{field} is missing"));
    }
    if text.trim() != text {
        return Err(format!("{field} has a space at its start or end"));
    }
    if text.contains("://") {
        return Err(format!("{field} must be a folder, not an address"));
    }
    if text.chars().any(|c| classify(c).is_hidden()) {
        return Err(format!("{field} holds a hidden character"));
    }
    let windows = style == PathStyle::Windows;
    if windows {
        let unified = text.replace('/', "\\");
        if unified.starts_with("\\\\?\\") || unified.starts_with("\\\\.\\") {
            return Err(format!("{field} has a device prefix"));
        }
    }
    if text.contains(['*', '?', '[', ']']) {
        return Err(format!("{field} has a wildcard"));
    }
    if text.contains(['$', '%']) {
        return Err(format!("{field} has a variable"));
    }
    if text.contains('~') {
        return Err(format!("{field} has a ~, which is never expanded"));
    }
    if text.contains('!') {
        return Err(format!("{field} has a !, which reads as an archive"));
    }
    let bytes = text.as_bytes();
    if windows
        && bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && !matches!(bytes.get(2), Some(b'\\' | b'/'))
    {
        return Err(format!(
            "{field} is a bare drive letter; write {}\\",
            &text[..2]
        ));
    }
    let absolute = if windows {
        (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
            || text.starts_with("\\\\")
            || text.starts_with("//")
    } else {
        text.starts_with('/')
    };
    if !absolute {
        return Err(format!("{field} is not a full path"));
    }
    let separators: &[char] = if windows { &['\\', '/'] } else { &['/'] };
    for part in text.split(separators) {
        if part == "." || part == ".." {
            return Err(format!("{field} has . or .. in it"));
        }
        if windows && !part.is_empty() && (part.ends_with('.') || part.ends_with(' ')) {
            return Err(format!("{field} has a name ending in a dot or space"));
        }
    }
    let place =
        Place::parse_with(text, style).map_err(|_| format!("{field} is not a folder path"))?;
    if place.kind() != PlaceKind::Local {
        return Err(format!("{field} is not a folder on this computer"));
    }
    Ok(place)
}

/// Why a task may not use `folder`, if it may not: the rest of a sentence
/// that starts with `field`, "home is too broad, a drive root".
pub fn refused(field: &str, folder: &Place, around: &Surroundings) -> Option<String> {
    if folder.is_root() {
        return Some(format!("{field} is too broad, a drive root"));
    }
    if around.home.as_ref().is_some_and(|home| same(folder, home)) {
        return Some(format!("{field} is too broad, your home folder"));
    }
    for own in &around.own {
        if own.is_within(folder) {
            return Some(format!("{field} holds diroxide's own folders"));
        }
        if folder.is_within(own) {
            return Some(format!("{field} is inside diroxide's own folders"));
        }
    }
    let mut up = Some(folder.clone());
    while let Some(place) = up {
        if place.name().is_some_and(is_trash_name) {
            return Some(format!("{field} is a trash folder"));
        }
        up = place.parent();
    }
    if around.trash.iter().any(|bin| folder.is_within(bin)) {
        return Some(format!("{field} is a trash folder"));
    }
    if around.system.iter().any(|system| folder.is_within(system)) {
        return Some(format!("{field} is a system folder"));
    }
    if around
        .program
        .as_ref()
        .is_some_and(|program| folder.is_within(program))
    {
        return Some(format!("{field} is diroxide's program folder"));
    }
    None
}

/// Whether two places are the same folder (Windows paths without regard
/// to case).
pub fn same(a: &Place, b: &Place) -> bool {
    a.is_within(b) && b.is_within(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn win(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Windows).unwrap()
    }

    fn unix(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    #[test]
    fn a_literal_full_path_reads_and_every_other_form_is_refused_in_words() {
        let w = PathStyle::Windows;
        assert_eq!(literal_folder("home", "D:\\scans", w), Ok(win("D:\\scans")));
        assert_eq!(literal_folder("home", "D:/scans", w), Ok(win("D:\\scans")));
        assert!(literal_folder("home", "\\\\nas\\share\\scans", w).is_ok());
        let refused = [
            ("", "home is missing"),
            (" D:\\scans", "home has a space at its start or end"),
            ("file://D:/scans", "home must be a folder, not an address"),
            ("D:\\sc\u{202e}ans", "home holds a hidden character"),
            ("D:\\sc\u{f03a}ans", "home holds a hidden character"),
            ("\\\\?\\D:\\scans", "home has a device prefix"),
            ("\\\\.\\PhysicalDrive0", "home has a device prefix"),
            ("D:\\scans\\*", "home has a wildcard"),
            ("D:\\scan?", "home has a wildcard"),
            ("D:\\[ab]", "home has a wildcard"),
            ("%USERPROFILE%\\scans", "home has a variable"),
            ("$HOME\\scans", "home has a variable"),
            ("~\\scans", "home has a ~, which is never expanded"),
            ("D:\\SCANS~1", "home has a ~, which is never expanded"),
            ("D:\\a.zip!x", "home has a !, which reads as an archive"),
            ("C:", "home is a bare drive letter; write C:\\"),
            ("C:scans", "home is a bare drive letter; write C:\\"),
            ("scans", "home is not a full path"),
            ("\\scans", "home is not a full path"),
            ("D:\\scans\\..\\x", "home has . or .. in it"),
            ("D:\\.\\scans", "home has . or .. in it"),
            ("D:\\scans.", "home has a name ending in a dot or space"),
            ("D:\\scans \\in", "home has a name ending in a dot or space"),
        ];
        for (text, words) in refused {
            assert_eq!(
                literal_folder("home", text, w),
                Err(words.to_owned()),
                "{text}"
            );
            // The field and its fault come first on the Braille line.
            let line = format!("Task scans: {words}.");
            let fault: String = words.split(' ').take(3).collect::<Vec<_>>().join(" ");
            assert!(within_cells(&line, &fault), "{line}");
        }
        let u = PathStyle::Unix;
        assert_eq!(
            literal_folder("to", "/srv/scans", u),
            Ok(unix("/srv/scans"))
        );
        assert_eq!(
            literal_folder("to", "srv/scans", u),
            Err("to is not a full path".to_owned())
        );
        assert_eq!(
            literal_folder("to", "/srv/../etc", u),
            Err("to has . or .. in it".to_owned())
        );
        // A trailing dot is a real name on Unix.
        assert!(literal_folder("to", "/srv/scans.", u).is_ok());
    }

    #[test]
    fn broad_own_trash_system_and_program_folders_are_refused() {
        let around = Surroundings {
            style: PathStyle::Windows,
            home: Some(win("C:\\Users\\Ada")),
            own: vec![
                win("C:\\Users\\Ada\\AppData\\Roaming\\diroxide"),
                win("D:\\diro\\state"),
            ],
            program: Some(win("D:\\tools\\diro")),
            system: vec![win("C:\\Windows"), win("C:\\Program Files")],
            trash: Vec::new(),
        };
        let cases = [
            ("D:\\", Some("home is too broad, a drive root")),
            ("\\\\nas\\share", Some("home is too broad, a drive root")),
            (
                "c:\\users\\ada",
                Some("home is too broad, your home folder"),
            ),
            ("C:\\Users", Some("home holds diroxide's own folders")),
            (
                "C:\\Users\\Ada\\AppData",
                Some("home holds diroxide's own folders"),
            ),
            ("D:\\diro", Some("home holds diroxide's own folders")),
            (
                "D:\\diro\\state\\hot",
                Some("home is inside diroxide's own folders"),
            ),
            ("D:\\$Recycle.Bin\\S-1-5", Some("home is a trash folder")),
            ("C:\\Windows\\Temp", Some("home is a system folder")),
            ("C:\\program files\\x", Some("home is a system folder")),
            ("D:\\tools\\diro", Some("home is diroxide's program folder")),
            (
                "D:\\tools\\diro\\scans",
                Some("home is diroxide's program folder"),
            ),
            ("C:\\Users\\Ada\\Downloads", None),
            ("D:\\tools", None),
            ("D:\\scans", None),
        ];
        for (text, words) in cases {
            assert_eq!(
                refused("home", &win(text), &around).as_deref(),
                words,
                "{text}"
            );
        }
        let linux = Surroundings {
            style: PathStyle::Unix,
            home: Some(unix("/home/ada")),
            own: vec![unix("/home/ada/.config/diroxide")],
            program: None,
            system: vec![unix("/etc"), unix("/usr")],
            trash: vec![unix("/home/ada/.local/share/Trash")],
        };
        assert_eq!(
            refused("to", &unix("/"), &linux).as_deref(),
            Some("to is too broad, a drive root")
        );
        assert_eq!(
            refused("to", &unix("/home/ada/.local/share/Trash/files"), &linux).as_deref(),
            Some("to is a trash folder")
        );
        assert_eq!(
            refused("to", &unix("/media/usb/.Trash-1000"), &linux).as_deref(),
            Some("to is a trash folder")
        );
        assert_eq!(
            refused("to", &unix("/usr/local/scans"), &linux).as_deref(),
            Some("to is a system folder")
        );
        // Case matters on Unix: /home/Ada is not the home folder.
        assert_eq!(refused("to", &unix("/home/Ada"), &linux), None);
        assert_eq!(refused("to", &unix("/home/ada/Downloads"), &linux), None);
    }

    #[test]
    fn trash_names_are_known_and_others_are_not() {
        for name in [
            "$Recycle.Bin",
            "$RECYCLE.BIN",
            "RECYCLER",
            ".Trash",
            ".Trash-1000",
        ] {
            assert!(is_trash_name(name), "{name}");
        }
        for name in ["Trash", ".Trash-", ".Trash-ada", "recycle", "scans"] {
            assert!(!is_trash_name(name), "{name}");
        }
    }

    #[test]
    fn this_computers_folders_refuse_its_own_settings() {
        // A path only, under the build's own folder; nothing is made.
        let exe = std::env::current_exe().unwrap();
        let config = exe.parent().unwrap().join("hot-config");
        let around = Surroundings::of_this_computer(&config, &config);
        let own = around.own.first().unwrap().clone();
        assert_eq!(
            refused("home", &own, &around).as_deref(),
            Some("home holds diroxide's own folders")
        );
        if let Some(home) = &around.home {
            assert_eq!(
                refused("home", home, &around).as_deref(),
                Some("home is too broad, your home folder")
            );
        }
        if let Some(program) = &around.program {
            assert!(refused("home", program, &around).is_some());
        }
    }
}
