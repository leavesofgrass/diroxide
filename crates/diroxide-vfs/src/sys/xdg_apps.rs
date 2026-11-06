//! Open with on Linux and the BSDs: freedesktop's association files read
//! by hand, with no crate and no shell (R28 4.3, R30's commit 26).
//!
//! - **Where:** each `mimeapps.list` in the order the specification
//!   gives (the desktop's own file before the plain one; the config home,
//!   the config folders, the data home's and the data folders'
//!   `applications`), then each data folder's `applications/mimeinfo.cache`.
//! - **What:** `[Default Applications]` names the default, `[Added
//!   Associations]` adds programs, `[Removed Associations]` takes them away;
//!   the caches list every installed program that declares the media type.
//! - **Each program:** its `.desktop` file, found by its id, gives `Name`
//!   (in the language the environment asks for when the file has it),
//!   `Exec`, `TryExec`, `NoDisplay`, `Hidden`, `Terminal` and `Icon`. A
//!   hidden program, one whose `TryExec` is not found, and one that wants a
//!   terminal are left out. `Exec` becomes an argument list: the
//!   specification's quoting undone, `%f` and `%u` the file, `%F` and `%U`
//!   the files, `%i` the icon's two arguments, `%c` the name, `%k` the
//!   desktop file, `%%` a percent sign, and the deprecated codes dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::open_with::{ArgPart, Launch, OpenCommand, OpenWithApp, OpenWithList};

/// The folders the specification reads, from the environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct XdgDirs {
    /// `$XDG_CONFIG_HOME`, or `~/.config`.
    pub(crate) config_home: Option<PathBuf>,
    /// `$XDG_CONFIG_DIRS`, or `/etc/xdg`.
    pub(crate) config_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_HOME`, or `~/.local/share`.
    pub(crate) data_home: Option<PathBuf>,
    /// `$XDG_DATA_DIRS`, or `/usr/local/share` and `/usr/share`.
    pub(crate) data_dirs: Vec<PathBuf>,
    /// `$XDG_CURRENT_DESKTOP`'s names, in lowercase: `gnome`, `kde`.
    pub(crate) desktops: Vec<String>,
    /// The languages to look for in a localized name, best first:
    /// `de_AT`, then `de`.
    pub(crate) languages: Vec<String>,
}

impl XdgDirs {
    /// The folders as this session's environment names them.
    #[cfg_attr(test, allow(dead_code, reason = "the tests name their folders"))]
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let home = var("HOME").map(PathBuf::from);
        let list = |name: &str, default: &[&str]| -> Vec<PathBuf> {
            match var(name) {
                Some(value) => std::env::split_paths(&value)
                    .filter(|p| p.is_absolute())
                    .collect(),
                None => default.iter().map(PathBuf::from).collect(),
            }
        };
        let language = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| var(name))
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            config_home: var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".config"))),
            config_dirs: list("XDG_CONFIG_DIRS", &["/etc/xdg"]),
            data_home: var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".local").join("share"))),
            data_dirs: list("XDG_DATA_DIRS", &["/usr/local/share", "/usr/share"]),
            desktops: var("XDG_CURRENT_DESKTOP")
                .map(|v| {
                    v.to_string_lossy()
                        .split(':')
                        .filter(|d| !d.is_empty())
                        .map(str::to_lowercase)
                        .collect()
                })
                .unwrap_or_default(),
            languages: languages(&language),
        }
    }

    /// The `mimeapps.list` files, in the specification's order.
    fn mimeapps_files(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = Vec::new();
        folders.extend(self.config_home.iter().cloned());
        folders.extend(self.config_dirs.iter().cloned());
        folders.extend(self.data_home.iter().map(|d| d.join("applications")));
        folders.extend(self.data_dirs.iter().map(|d| d.join("applications")));
        let mut files = Vec::new();
        for folder in folders {
            for desktop in &self.desktops {
                files.push(folder.join(format!("{desktop}-mimeapps.list")));
            }
            files.push(folder.join("mimeapps.list"));
        }
        files
    }

    /// The `applications` folders, the data home's first.
    fn application_folders(&self) -> Vec<PathBuf> {
        self.data_home
            .iter()
            .chain(&self.data_dirs)
            .map(|d| d.join("applications"))
            .collect()
    }
}

/// `de_AT.UTF-8@euro` becomes `de_AT@euro`, `de_AT`, `de@euro`, `de`: the
/// keys a localized name is looked up under, best first.
fn languages(locale: &str) -> Vec<String> {
    if locale.is_empty() || locale == "C" || locale == "POSIX" {
        return Vec::new();
    }
    let (rest, modifier) = match locale.split_once('@') {
        Some((rest, modifier)) => (rest, Some(modifier)),
        None => (locale, None),
    };
    let rest = rest.split('.').next().unwrap_or(rest);
    let (lang, country) = match rest.split_once('_') {
        Some((lang, country)) => (lang, Some(country)),
        None => (rest, None),
    };
    let mut out = Vec::new();
    if let (Some(country), Some(modifier)) = (country, modifier) {
        out.push(format!("{lang}_{country}@{modifier}"));
    }
    if let Some(country) = country {
        out.push(format!("{lang}_{country}"));
    }
    if let Some(modifier) = modifier {
        out.push(format!("{lang}@{modifier}"));
    }
    out.push(lang.to_owned());
    out
}

/// A key file's groups: group name to its keys, in the format both
/// `mimeapps.list` and `.desktop` files share. Comments and blank lines
/// are skipped; the first of a repeated key wins.
fn read_key_file(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut groups: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(name.to_owned());
            groups.entry(name.to_owned()).or_default();
            continue;
        }
        if let (Some(group), Some((key, value))) = (&current, line.split_once('=')) {
            groups
                .entry(group.clone())
                .or_default()
                .entry(key.trim().to_owned())
                .or_insert_with(|| value.trim().to_owned());
        }
    }
    groups
}

/// The ids a list value names: `a.desktop;b.desktop;`.
fn ids(value: &str) -> impl Iterator<Item = &str> {
    value.split(';').map(str::trim).filter(|id| !id.is_empty())
}

/// A string value's escapes undone: `\s` a space, `\n`, `\t`, `\r`, `\\`.
fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// What an `Exec` line's field codes need besides the file.
pub(crate) struct ExecContext<'a> {
    /// The program's name, for `%c`.
    pub(crate) name: &'a str,
    /// Its icon, for `%i`.
    pub(crate) icon: Option<&'a str>,
    /// Its desktop file, for `%k`.
    pub(crate) location: &'a str,
}

/// An `Exec` value (its string escapes already undone) into a program
/// and arguments; `None` when it is empty or a quote is not closed.
pub(crate) fn parse_exec(exec: &str, context: &ExecContext<'_>) -> Option<OpenCommand> {
    // Split into words, each remembering whether it was quoted.
    let mut words: Vec<(String, bool)> = Vec::new();
    let mut chars = exec.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c == ' ' || c == '\t' {
            chars.next();
            continue;
        }
        if c == '"' {
            chars.next();
            let mut word = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some(e @ ('"' | '`' | '$' | '\\')) => word.push(e),
                        Some(other) => {
                            word.push('\\');
                            word.push(other);
                        }
                        None => word.push('\\'),
                    },
                    c => word.push(c),
                }
            }
            if !closed {
                return None;
            }
            words.push((word, true));
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\t' {
                    break;
                }
                word.push(c);
                chars.next();
            }
            words.push((word, false));
        }
    }
    let mut words = words.into_iter();
    let (program, _) = words.next()?;
    let mut args = Vec::new();
    for (word, quoted) in words {
        if quoted {
            args.push(vec![ArgPart::Text(word)]);
            continue;
        }
        match word.as_str() {
            "%F" | "%U" => {
                args.push(vec![ArgPart::Files]);
                continue;
            }
            "%i" => {
                if let Some(icon) = context.icon {
                    args.push(vec![ArgPart::Text("--icon".to_owned())]);
                    args.push(vec![ArgPart::Text(icon.to_owned())]);
                }
                continue;
            }
            _ => {}
        }
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut only_dropped = true;
        let mut word_chars = word.chars();
        while let Some(c) = word_chars.next() {
            if c != '%' {
                text.push(c);
                only_dropped = false;
                continue;
            }
            match word_chars.next() {
                Some('f' | 'u') => {
                    if !text.is_empty() {
                        parts.push(ArgPart::Text(std::mem::take(&mut text)));
                    }
                    parts.push(ArgPart::File);
                    only_dropped = false;
                }
                Some('F' | 'U') => {
                    if !text.is_empty() {
                        parts.push(ArgPart::Text(std::mem::take(&mut text)));
                    }
                    parts.push(ArgPart::File);
                    only_dropped = false;
                }
                Some('c') => {
                    text.push_str(context.name);
                    only_dropped = false;
                }
                Some('k') => {
                    text.push_str(context.location);
                    only_dropped = false;
                }
                Some('%') => {
                    text.push('%');
                    only_dropped = false;
                }
                // Deprecated and unknown codes are dropped.
                Some(_) | None => {}
            }
        }
        if !text.is_empty() {
            parts.push(ArgPart::Text(text));
        }
        if parts.is_empty() && only_dropped {
            continue;
        }
        if parts.is_empty() {
            parts.push(ArgPart::Text(String::new()));
        }
        args.push(parts);
    }
    Some(OpenCommand { program, args })
}

/// Whether a program named by `TryExec` is there: a full path that
/// exists, or a name found in a folder of `path`.
fn program_found(program: &str, path: Option<&std::ffi::OsStr>) -> bool {
    let program = Path::new(program);
    if program.is_absolute() {
        return program.is_file();
    }
    path.is_some_and(|path| std::env::split_paths(path).any(|dir| dir.join(program).is_file()))
}

/// A desktop file by its id: `org.gnome.TextEditor.desktop` in an
/// `applications` folder, or `kde4-kate.desktop` as `kde4/kate.desktop`.
fn find_desktop_file(id: &str, folders: &[PathBuf]) -> Option<PathBuf> {
    for folder in folders {
        let direct = folder.join(id);
        if direct.is_file() {
            return Some(direct);
        }
        let mut prefix = String::new();
        for (i, piece) in id.split('-').enumerate() {
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(piece);
            let rest = &id[prefix.len()..];
            if let Some(rest) = rest.strip_prefix('-') {
                let nested = folder.join(&prefix).join(rest);
                if nested.is_file() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

/// One program from its desktop file, or `None` when it is hidden, not
/// an application, not installed, wants a terminal, or has no command.
fn read_app(id: &str, dirs: &XdgDirs, path_var: Option<&std::ffi::OsStr>) -> Option<OpenWithApp> {
    let folders = dirs.application_folders();
    let file = find_desktop_file(id, &folders)?;
    let text = std::fs::read_to_string(&file).ok()?;
    let groups = read_key_file(&text);
    let entry = groups.get("Desktop Entry")?;
    let truthy = |key: &str| entry.get(key).is_some_and(|v| v == "true");
    if entry.get("Type").is_some_and(|t| t != "Application")
        || truthy("Hidden")
        || truthy("Terminal")
    {
        return None;
    }
    if let Some(try_exec) = entry.get("TryExec")
        && !program_found(&unescape_value(try_exec), path_var)
    {
        return None;
    }
    let name = dirs
        .languages
        .iter()
        .find_map(|lang| entry.get(&format!("Name[{lang}]")))
        .or_else(|| entry.get("Name"))
        .map(|n| unescape_value(n))
        .filter(|n| !n.is_empty())?;
    let icon = entry.get("Icon").map(|i| unescape_value(i));
    let location = file.to_string_lossy().into_owned();
    let exec = unescape_value(entry.get("Exec")?);
    let command = parse_exec(
        &exec,
        &ExecContext {
            name: &name,
            icon: icon.as_deref(),
            location: &location,
        },
    )?;
    Some(OpenWithApp {
        name,
        launch: Launch::Command(command),
        id: id.to_owned(),
        default: false,
    })
}

/// The programs for a media type, read from these folders: the default
/// first, then the added ones, then the installed ones the caches list,
/// less the removed ones.
pub(crate) fn apps_for(
    mime: &str,
    dirs: &XdgDirs,
    path_var: Option<&std::ffi::OsStr>,
) -> OpenWithList {
    let mut default: Option<String> = None;
    let mut named: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for file in dirs.mimeapps_files() {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let groups = read_key_file(&text);
        let list = |group: &str| -> Vec<String> {
            groups
                .get(group)
                .and_then(|g| g.get(mime))
                .map(|v| ids(v).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        // A file's own removals hide what lower files add, never what it
        // adds itself.
        for id in list("Added Associations") {
            if !removed.contains(&id) && !named.contains(&id) {
                named.push(id);
            }
        }
        if default.is_none() {
            default = list("Default Applications")
                .into_iter()
                .find(|id| !removed.contains(id) && read_app(id, dirs, path_var).is_some());
        }
        removed.extend(list("Removed Associations"));
    }
    for folder in dirs.application_folders() {
        let Ok(text) = std::fs::read_to_string(folder.join("mimeinfo.cache")) else {
            continue;
        };
        let groups = read_key_file(&text);
        if let Some(value) = groups.get("MIME Cache").and_then(|g| g.get(mime)) {
            for id in ids(value) {
                if !removed.iter().any(|r| r == id) && !named.iter().any(|n| n == id) {
                    named.push(id.to_owned());
                }
            }
        }
    }
    let mut list = OpenWithList::default();
    if let Some(id) = &default
        && let Some(mut app) = read_app(id, dirs, path_var)
    {
        app.default = true;
        list.push(app);
    }
    for id in named {
        if let Some(app) = read_app(&id, dirs, path_var) {
            list.push(app);
        }
    }
    list
}

/// The programs this session's freedesktop files list for a file of
/// this media type.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn open_with_list(_file: &Path, mime: Option<&str>) -> OpenWithList {
    match mime {
        Some(mime) => apps_for(
            mime,
            &XdgDirs::from_env(),
            std::env::var_os("PATH").as_deref(),
        ),
        None => OpenWithList::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::test_support::scratch;

    fn context() -> ExecContext<'static> {
        ExecContext {
            name: "Text Editor",
            icon: Some("accessories-text-editor"),
            location: "/usr/share/applications/editor.desktop",
        }
    }

    fn argv(exec: &str, file: &str) -> Vec<String> {
        let command = parse_exec(exec, &context()).unwrap();
        let (program, args) = command.for_file(OsStr::new(file));
        std::iter::once(program)
            .chain(args)
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// The `Exec` rules (the specification's own page): each code in its
    /// place, quoting undone, and a name with a space or a dollar sign
    /// arriving whole.
    #[test]
    fn exec_lines_become_argument_lists() {
        let file = "/home/ada/My Files/report $HOME (1).pdf";
        assert_eq!(argv("evince %U", file), ["evince", file]);
        assert_eq!(
            argv("gedit --new-window %f", file),
            ["gedit", "--new-window", file]
        );
        assert_eq!(
            argv("viewer --open=%f %i -c %c", file),
            [
                "viewer",
                &format!("--open={file}"),
                "--icon",
                "accessories-text-editor",
                "-c",
                "Text Editor"
            ]
        );
        assert_eq!(
            argv(r#""/opt/My App/app" "--title=a \"b\" \$x \\ c" %u"#, file),
            ["/opt/My App/app", r#"--title=a "b" $x \ c"#, file]
        );
        assert_eq!(argv("app %d %D %n %N %v %m %F", file), ["app", file]);
        assert_eq!(
            argv("app 100%% %k", file),
            [
                "app",
                "100%",
                "/usr/share/applications/editor.desktop",
                file
            ]
        );
        assert_eq!(
            argv("app", file),
            ["app", file],
            "no code: the file goes last"
        );
        assert!(parse_exec(r#"app "unclosed"#, &context()).is_none());
        assert!(parse_exec("   ", &context()).is_none());
        assert_eq!(unescape_value(r"sh -c a\sb\\c"), r"sh -c a b\c");
    }

    #[test]
    fn locales_become_name_keys() {
        assert_eq!(
            languages("de_AT.UTF-8@euro"),
            ["de_AT@euro", "de_AT", "de@euro", "de"]
        );
        assert_eq!(languages("fr_FR.UTF-8"), ["fr_FR", "fr"]);
        assert!(languages("C").is_empty());
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A small system of its own, under the build's target folder: the
    /// default, an added program, a cached one, a removed one, a hidden
    /// one, one whose TryExec is missing, one wanting a terminal, and a
    /// nested id.
    #[test]
    fn the_association_files_are_read_in_order() {
        let root = scratch();
        let home = root.path().join("config");
        let data_home = root.path().join("data-home");
        let data = root.path().join("data");
        let apps = data.join("applications");
        let bin = root.path().join("bin");
        write(&bin.join("present"), "");
        let desktop = |name: &str, extra: &str| {
            format!(
                "[Desktop Entry]\nType=Application\nName={name}\nName[de]={name} auf Deutsch\nExec={} %U\n{extra}",
                name.to_lowercase().replace(' ', "-")
            )
        };
        write(&apps.join("reader.desktop"), &desktop("Reader", ""));
        write(
            &apps.join("viewer.desktop"),
            &desktop("Viewer", "NoDisplay=true\n"),
        );
        write(&apps.join("cached.desktop"), &desktop("Cached", ""));
        write(&apps.join("unwanted.desktop"), &desktop("Unwanted", ""));
        write(
            &apps.join("hidden.desktop"),
            &desktop("Hidden", "Hidden=true\n"),
        );
        write(
            &apps.join("missing.desktop"),
            &desktop("Missing", "TryExec=not-installed-anywhere\n"),
        );
        write(
            &apps.join("tried.desktop"),
            &desktop("Tried", "TryExec=present\n"),
        );
        write(
            &apps.join("console.desktop"),
            &desktop("Console", "Terminal=true\n"),
        );
        write(
            &apps.join("kde4").join("okular.desktop"),
            &desktop("Okular", ""),
        );
        write(
            &home.join("mimeapps.list"),
            "[Default Applications]\napplication/pdf=gone.desktop;viewer.desktop;\n\n[Added Associations]\napplication/pdf=reader.desktop;hidden.desktop;\n\n[Removed Associations]\napplication/pdf=unwanted.desktop;\n",
        );
        write(
            &home.join("gnome-mimeapps.list"),
            "# The desktop's own file comes first.\n[Added Associations]\napplication/pdf=kde4-okular.desktop;\n",
        );
        write(
            &apps.join("mimeinfo.cache"),
            "[MIME Cache]\napplication/pdf=cached.desktop;unwanted.desktop;missing.desktop;tried.desktop;console.desktop;\ntext/plain=reader.desktop;\n",
        );
        let dirs = XdgDirs {
            config_home: Some(home),
            config_dirs: Vec::new(),
            data_home: Some(data_home),
            data_dirs: vec![data],
            desktops: vec!["gnome".to_owned()],
            languages: Vec::new(),
        };
        let path = std::env::join_paths([&bin]).unwrap();
        let list = apps_for("application/pdf", &dirs, Some(&path));
        let names: Vec<&str> = list.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Viewer", "Okular", "Reader", "Cached", "Tried"]);
        assert!(list.apps[0].default);
        assert!(!list.choose_another);
        let german = XdgDirs {
            languages: languages("de_DE.UTF-8"),
            ..dirs.clone()
        };
        let list = apps_for("text/plain", &german, Some(&path));
        assert_eq!(list.apps[0].name, "Reader auf Deutsch");
        assert!(apps_for("image/png", &dirs, Some(&path)).apps.is_empty());
    }
}
