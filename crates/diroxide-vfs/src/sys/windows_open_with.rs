//! Open with on Windows (R30's commit 27): the registry read with the
//! `windows-sys` calls alone, the user's hive before the machine's.
//!
//! For an extension, in this order:
//!
//! 1. the default: the user's choice (`FileExts\.ext\UserChoice`), else
//!    the class the extension names (`Classes\.ext`);
//! 2. the programs the user opened it with (`FileExts\.ext\OpenWithList`,
//!    most recent first, and `Classes\.ext\OpenWithList`), each an
//!    executable whose `Applications\NAME\shell\open\command` says how to
//!    start it;
//! 3. the classes that offer to open it (`OpenWithProgids` under the
//!    user's `FileExts` key and both hives' classes), each started by its
//!    own command or, with none (a Store app), through the system by its
//!    class.
//!
//! A program marked `NoOpenWith` is left out. Names come from the
//! program's own `FriendlyAppName`, the shell's cache of the names it
//! showed (`MuiCache`), or a Store app's name, each resolved when it is a
//! resource reference; failing those, the executable's own name. The
//! registry is only read: nothing here writes it, and nothing opens a
//! window except "Choose another program" and starting a listed program,
//! each only when asked.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
    RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_CLASSNAME, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW, SHLoadIndirectString,
    ShellExecuteExW,
};

use super::open_with::{Launch, OpenWithApp, OpenWithList};
use super::windows_command::parse_windows_command;

/// The user's per-extension choices.
const FILE_EXTS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";

/// The shell's cache of the program names it showed.
const MUI_CACHE: &str =
    r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache";

/// `SW_SHOWNORMAL`: a started program's window as it chooses.
const SHOW_NORMAL: i32 = 1;

/// The longest value name the registry allows, plus its NUL.
const MAX_NAME: usize = 16_384;

/// A NUL-terminated wide string.
fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Text from a wide buffer, up to its first NUL.
fn text_of(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end])
        .to_string_lossy()
        .into_owned()
}

/// An open registry key, closed when dropped.
struct Key(HKEY);

impl Key {
    /// Opens a key to read, or `None` when it is not there.
    fn open(root: HKEY, path: &str) -> Option<Key> {
        let path = wide(path);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `path` is NUL-terminated and outlives the call; `key` is
        // a valid place for the handle the call writes on success.
        let status = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then_some(Key(key))
    }

    /// A text value (`None` for the key's default), the expanding kind
    /// left as written: [`parse_windows_command`] expands it.
    fn string(&self, name: Option<&str>) -> Option<String> {
        let name = name.map(wide);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
        let mut kind = 0u32;
        let mut bytes = 0u32;
        // SAFETY: the key is open; a null data pointer asks only for the
        // value's type and size, written to the two locals.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name_ptr,
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
            return None;
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2) + 1];
        let mut size = u32::try_from(buffer.len() * 2).ok()?;
        // SAFETY: `buffer` holds `size` bytes, which the call writes at
        // most; the name and the key are as above.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name_ptr,
                std::ptr::null(),
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        (status == ERROR_SUCCESS)
            .then(|| text_of(&buffer))
            .filter(|t| !t.is_empty())
    }

    /// Whether a value of this name is there, whatever its kind.
    fn has_value(&self, name: &str) -> bool {
        let name = wide(name);
        // SAFETY: the key is open and the name NUL-terminated; no data or
        // size is asked for.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS || status == ERROR_MORE_DATA
    }

    /// The names of the key's subkeys, in the registry's order.
    fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut buffer = vec![0u16; 256];
        for index in 0u32..10_000 {
            let mut length = buffer.len() as u32;
            // SAFETY: `buffer` holds `length` wide characters, which the
            // call writes at most (a key name is at most 255); no class or
            // time is asked for.
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let name = text_of(&buffer[..(length as usize).min(buffer.len())]);
            if !name.is_empty() {
                names.push(name);
            }
        }
        names
    }

    /// The names of the key's values, in the registry's order, the
    /// default (unnamed) value left out.
    fn value_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut buffer = vec![0u16; MAX_NAME];
        for index in 0u32.. {
            let mut length = MAX_NAME as u32;
            // SAFETY: `buffer` holds `length` wide characters, which the
            // call writes at most; no type or data is asked for.
            let status = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status == ERROR_SUCCESS {
                let name = text_of(&buffer[..(length as usize).min(MAX_NAME)]);
                if !name.is_empty() {
                    names.push(name);
                }
            } else if index > 100_000 {
                break;
            }
        }
        names
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle came from RegOpenKeyExW and is closed once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

/// A key in the user's classes, then the machine's.
fn class_key(path: &str) -> Option<Key> {
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .find_map(|root| Key::open(root, &format!(r"Software\Classes\{path}")))
}

/// A resource reference (`@%SystemRoot%\system32\shell32.dll,-22985`,
/// `@{Package?ms-resource://...}`) resolved to its text; other text as it
/// is.
fn resolve_name(text: &str) -> Option<String> {
    if !text.starts_with('@') {
        return Some(text.to_owned());
    }
    let source = wide(text);
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `source` is NUL-terminated; `buffer` holds the count of
    // wide characters passed, which the call writes at most.
    let result = unsafe {
        SHLoadIndirectString(
            source.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            std::ptr::null(),
        )
    };
    (result == 0)
        .then(|| text_of(&buffer))
        .filter(|t| !t.is_empty())
}

/// The environment, for the commands' `%NAME%`.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The facts the registry holds about one extension.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Associations {
    /// The user's choice of class.
    pub(crate) user_choice: Option<String>,
    /// The class the extension names.
    pub(crate) class_default: Option<String>,
    /// Executables, most recent first.
    pub(crate) executables: Vec<String>,
    /// Classes, the user's first.
    pub(crate) classes: Vec<String>,
}

/// One handler as the registry describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Handler {
    /// Its command, as written.
    pub(crate) command: Option<String>,
    /// Its own name, as written (maybe a resource reference).
    pub(crate) name: Option<String>,
    /// Marked `NoOpenWith`.
    pub(crate) hidden: bool,
    /// A class the system starts itself (a Store app, `DelegateExecute`):
    /// its command, if any, is not run directly, since a Store app's
    /// program cannot be started from outside its package.
    pub(crate) by_class: bool,
    /// A Store app's id (`Microsoft.Windows.Photos_8wekyb3d8bbwe!App`):
    /// two classes with one id are one program.
    pub(crate) app_id: Option<String>,
}

/// The list from the facts: the default first, then the executables,
/// then the classes; the names from `name_of` (the program's path to its
/// shown name), else the handler's own, else the executable's.
pub(crate) fn build_list(
    facts: &Associations,
    executable: &dyn Fn(&str) -> Option<Handler>,
    class: &dyn Fn(&str) -> Option<Handler>,
    name_of: &dyn Fn(&str) -> Option<String>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> OpenWithList {
    let mut list = OpenWithList {
        apps: Vec::new(),
        choose_another: true,
        unread: 0,
    };
    let default = facts
        .user_choice
        .clone()
        .or_else(|| facts.class_default.clone());
    let mut add = |id: &str, handler: Option<Handler>, is_default: bool| {
        let Some(handler) = handler.filter(|h| !h.hidden) else {
            return;
        };
        let command = handler
            .command
            .as_deref()
            .and_then(|c| parse_windows_command(c, lookup));
        let launch = match command {
            _ if handler.by_class => Launch::Class(id.to_owned()),
            Some(command) => Launch::Command(command),
            None => return,
        };
        let name = match &launch {
            Launch::Command(command) => name_of(&command.program)
                .or_else(|| handler.name.clone())
                .unwrap_or_else(|| command.program_stem()),
            Launch::Class(_) => match &handler.name {
                Some(name) => name.clone(),
                None => return,
            },
        };
        list.push(OpenWithApp {
            name,
            launch,
            id: handler.app_id.clone().unwrap_or_else(|| id.to_owned()),
            default: is_default,
        });
    };
    if let Some(id) = &default {
        add(id, class(id), true);
    }
    for id in &facts.executables {
        add(id, executable(id), false);
    }
    for id in &facts.classes {
        add(id, class(id), false);
    }
    list.default_first()
}

/// What the registry says about an extension (`.txt`), read.
fn read_associations(extension: &str) -> Associations {
    let mut facts = Associations::default();
    let user = Key::open(HKEY_CURRENT_USER, &format!(r"{FILE_EXTS}\{extension}"));
    if let Some(user) = &user {
        facts.user_choice = Key::open(user.0, "UserChoice").and_then(|k| k.string(Some("ProgId")));
        if let Some(list) = Key::open(user.0, "OpenWithList") {
            let order = list.string(Some("MRUList")).unwrap_or_default();
            let mut names: Vec<(usize, String)> = list
                .value_names()
                .into_iter()
                .filter(|n| n.chars().count() == 1 && n != "MRUList")
                .filter_map(|n| {
                    let place = order.find(&n).unwrap_or(usize::MAX);
                    list.string(Some(&n)).map(|exe| (place, exe))
                })
                .collect();
            names.sort_by_key(|(place, _)| *place);
            facts
                .executables
                .extend(names.into_iter().map(|(_, exe)| exe));
        }
        if let Some(progids) = Key::open(user.0, "OpenWithProgids") {
            facts.classes.extend(progids.value_names());
        }
    }
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Some(key) = Key::open(root, &format!(r"Software\Classes\{extension}")) else {
            continue;
        };
        if facts.class_default.is_none() {
            facts.class_default = key.string(None);
        }
        if let Some(progids) = Key::open(key.0, "OpenWithProgids") {
            for id in progids.value_names() {
                if !facts.classes.iter().any(|c| c.eq_ignore_ascii_case(&id)) {
                    facts.classes.push(id);
                }
            }
        }
        if let Some(list) = Key::open(key.0, "OpenWithList") {
            for exe in list.subkey_names() {
                if !facts
                    .executables
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(&exe))
                {
                    facts.executables.push(exe);
                }
            }
        }
    }
    facts
}

/// A handler under a key: its open command (the shell's default verb,
/// else `open`), its own name, `NoOpenWith`, and whether the system
/// starts it by its class.
fn read_handler(key: &Key) -> Handler {
    let verb = Key::open(key.0, "shell")
        .and_then(|shell| shell.string(None))
        .unwrap_or_else(|| "open".to_owned());
    let verb_key =
        Key::open(key.0, &format!(r"shell\{verb}")).or_else(|| Key::open(key.0, r"shell\open"));
    let command_key = verb_key.as_ref().and_then(|v| Key::open(v.0, "command"));
    let command = command_key.as_ref().and_then(|c| c.string(None));
    let delegate = command_key
        .as_ref()
        .is_some_and(|c| c.has_value("DelegateExecute"));
    let application = Key::open(key.0, "Application");
    let name = application
        .as_ref()
        .and_then(|a| a.string(Some("ApplicationName")))
        .or_else(|| key.string(Some("FriendlyAppName")))
        .or_else(|| {
            verb_key
                .as_ref()
                .and_then(|v| v.string(Some("FriendlyAppName")))
        })
        .and_then(|n| resolve_name(&n));
    Handler {
        command,
        name,
        hidden: key.has_value("NoOpenWith"),
        by_class: delegate || application.is_some(),
        app_id: application
            .as_ref()
            .and_then(|a| a.string(Some("AppUserModelID"))),
    }
}

/// The shell's cached name for a program's path, then its own
/// `FriendlyAppName` under `Applications`.
fn name_of_program(program: &str) -> Option<String> {
    let cached = Key::open(HKEY_CURRENT_USER, MUI_CACHE)
        .and_then(|k| k.string(Some(&format!("{program}.FriendlyAppName"))));
    cached.or_else(|| {
        let exe = program.rsplit(['\\', '/']).next()?;
        class_key(&format!(r"Applications\{exe}"))
            .and_then(|k| k.string(Some("FriendlyAppName")))
            .and_then(|n| resolve_name(&n))
    })
}

/// The programs Windows lists for a file, by its extension.
pub(crate) fn open_with_list(file: &Path) -> OpenWithList {
    let Some(extension) = file
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
    else {
        return OpenWithList {
            apps: Vec::new(),
            choose_another: true,
            unread: 0,
        };
    };
    let facts = read_associations(&extension);
    build_list(
        &facts,
        &|exe| class_key(&format!(r"Applications\{exe}")).map(|k| read_handler(&k)),
        &|class| class_key(class).map(|k| read_handler(&k)),
        &name_of_program,
        &env,
    )
}

/// Hands the file to Windows's own Open with dialog: `rundll32` with
/// `shell32.dll,OpenAs_RunDLL` and the path, started and never waited
/// for. The path goes on the command line as it is, the one argument the
/// dialog reads.
pub(crate) fn choose_another_program(file: &Path) -> std::io::Result<()> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
    let rundll32 = Path::new(&root).join("System32").join("rundll32.exe");
    let mut line = OsString::from("shell32.dll,OpenAs_RunDLL ");
    line.push(file.as_os_str());
    std::process::Command::new(rundll32)
        .raw_arg(line)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
}

/// Starts the handler of a class on a file through the system, as
/// Explorer does for a Store app: `ShellExecuteExW` with the class named,
/// the open verb, and no message box of its own on failure.
pub(crate) fn start_by_class(class: &str, file: &Path) -> std::io::Result<()> {
    let class = wide(class);
    let verb = wide("open");
    let file: Vec<u16> = file.as_os_str().encode_wide().chain([0]).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_CLASSNAME | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpClass: class.as_ptr(),
        nShow: SHOW_NORMAL,
        ..Default::default()
    };
    // SAFETY: every string is NUL-terminated and outlives the call; the
    // structure's size is its own.
    let started = unsafe { ShellExecuteExW(&mut info) };
    if started == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(name: &str) -> Option<String> {
        (name == "SystemRoot").then(|| r"C:\Windows".to_owned())
    }

    /// The list from captured values, as a registry with the user's
    /// choice of a Store app, two recent programs, one marked
    /// NoOpenWith, and classes that repeat a program.
    #[test]
    fn the_list_is_built_from_the_registrys_values() {
        let facts = Associations {
            user_choice: Some("AppX43hnxtbyyps62jhe9sqpdzxn1790zetc".to_owned()),
            class_default: Some("jpegfile".to_owned()),
            executables: vec![
                "mspaint.exe".to_owned(),
                "hidden.exe".to_owned(),
                "unknown.exe".to_owned(),
            ],
            classes: vec![
                "jpegfile".to_owned(),
                "Paint.Picture".to_owned(),
                "AppX4mntx4h978m1v9gtzv0ewksfd6pmwsre".to_owned(),
                "AppXcesbfs704v2mjbts9dkr42s9vmrhxbkj".to_owned(),
            ],
        };
        let executable = |exe: &str| match exe {
            "mspaint.exe" => Some(Handler {
                command: Some(r#""%SystemRoot%\system32\mspaint.exe" "%1""#.to_owned()),
                ..Handler::default()
            }),
            "hidden.exe" => Some(Handler {
                command: Some(r#""C:\hidden.exe" "%1""#.to_owned()),
                hidden: true,
                ..Handler::default()
            }),
            _ => None,
        };
        let class = |id: &str| match id {
            "AppX43hnxtbyyps62jhe9sqpdzxn1790zetc" | "AppX4mntx4h978m1v9gtzv0ewksfd6pmwsre" => {
                Some(Handler {
                    name: Some("Photos".to_owned()),
                    by_class: true,
                    app_id: Some("Microsoft.Windows.Photos_8wekyb3d8bbwe!App".to_owned()),
                    ..Handler::default()
                })
            }
            "AppXcesbfs704v2mjbts9dkr42s9vmrhxbkj" => Some(Handler {
                command: Some(
                    r#""C:\Program Files\WindowsApps\Paint\mspaint.exe" "%1""#.to_owned(),
                ),
                name: Some("Paint, the Store app".to_owned()),
                by_class: true,
                app_id: Some("Microsoft.Paint_8wekyb3d8bbwe!App".to_owned()),
                ..Handler::default()
            }),
            "jpegfile" => Some(Handler {
                command: Some(r"%SystemRoot%\System32\mspaint.exe %1".to_owned()),
                ..Handler::default()
            }),
            "Paint.Picture" => Some(Handler {
                command: Some(r#""C:\Tools\Viewer Two\viewer.exe" /view "%1""#.to_owned()),
                name: Some("Viewer Two".to_owned()),
                ..Handler::default()
            }),
            _ => None,
        };
        let name_of = |program: &str| {
            program
                .to_ascii_lowercase()
                .ends_with("mspaint.exe")
                .then(|| "Paint".to_owned())
        };
        let list = build_list(&facts, &executable, &class, &name_of, &lookup);
        let rows: Vec<(&str, bool)> = list
            .apps
            .iter()
            .map(|a| (a.name.as_str(), a.default))
            .collect();
        assert_eq!(
            rows,
            [
                ("Photos", true),
                ("Paint", false),
                ("Viewer Two", false),
                ("Paint, the Store app", false)
            ],
            "one row for the Store app's two classes"
        );
        assert_eq!(
            list.apps[3].launch,
            Launch::Class("AppXcesbfs704v2mjbts9dkr42s9vmrhxbkj".to_owned()),
            "a Store app with a command is still started by its class"
        );
        assert!(list.choose_another);
        assert_eq!(
            list.apps[0].launch,
            Launch::Class("AppX43hnxtbyyps62jhe9sqpdzxn1790zetc".to_owned())
        );
        match &list.apps[2].launch {
            Launch::Command(command) => {
                assert_eq!(command.program, r"C:\Tools\Viewer Two\viewer.exe");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A live, read-only look at this computer's registry: whatever it
    /// lists for `.txt`, every row has a name and a way to start, and the
    /// list offers the system's own dialog. Nothing is started.
    #[test]
    fn this_computers_registry_reads() {
        let list = open_with_list(Path::new(r"C:\nowhere\notes.txt"));
        assert!(list.choose_another);
        for app in &list.apps {
            assert!(!app.name.is_empty(), "{app:?}");
            assert!(!app.name.starts_with('@'), "{app:?}");
            match &app.launch {
                Launch::Command(command) => assert!(!command.program.is_empty()),
                Launch::Class(class) => assert!(!class.is_empty()),
            }
        }
        assert!(open_with_list(Path::new("no-extension")).apps.is_empty());
    }
}
