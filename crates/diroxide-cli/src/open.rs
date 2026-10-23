//! `diro [PATH] [OTHER]`: open the panels.
//!
//! The start, in order (W1d): the config folder is read, its problems kept
//! for one startup line; the keymap problems are added; the read-only
//! references are written beside the owner's files; the session is
//! restored (the layout from the session unless the setting changed since
//! it was saved); the undo journal gives the interrupted jobs' notices and
//! the undo records; the clock reads the local offset. `diro` alone opens
//! the current folder in the left panel and restores the right one (the
//! owner's answer 74); `diro PATH` and `diro LEFT RIGHT` open those.
//!
//! `--tui` and `--gui` are accepted (the GUI comes with alpha.6, so
//! `--gui` says so and opens the terminal); `--inline` draws below the
//! prompt instead of on the alternate screen; `--cwd-file FILE` writes the
//! active panel's last local folder at quit for the shell wrapper of
//! `diro init`; `--theme NAME` sets the theme for this session.
//!
//! On a terminal the full screen runs until the app quits; when standard
//! input or output is not a terminal (a pipe, a file), one screen is
//! printed as plain lines instead.

use std::env;
use std::io::{IsTerminal, Write};
use std::path::Path;

use diroxide_app::keymap::Platform;
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_config::Paths;
use diroxide_core::{Message, Place, PlaceKind};
use diroxide_filetype::TypeRegistry;
use diroxide_tui::{Files, Tui, TuiOptions};

/// The terminal's log in the state folder: what it found at start.
pub const TERMINAL_LOG: &str = "terminal.log";

use crate::CliError;

/// How the panels are opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// Up to two places, as typed.
    pub places: Vec<String>,
    /// Keep the configuration beside the program.
    pub portable: bool,
    /// Start without plugins.
    pub no_plugins: bool,
    /// The terminal, even where a display is there.
    pub tui: bool,
    /// The GUI (it comes with alpha.6).
    pub gui: bool,
    /// The inline viewport instead of the alternate screen (W1d-2).
    pub inline: bool,
    /// Where to write the folder the shell moves to at quit.
    pub cwd_file: Option<String>,
    /// The theme for this session.
    pub theme: Option<String>,
}

/// The current folder as a place.
pub fn current_place() -> Result<Place, CliError> {
    let folder = env::current_dir()
        .map_err(|e| CliError::Failed(format!("the current folder could not be read: {e}")))?;
    let text = folder
        .into_os_string()
        .into_string()
        .map_err(|raw| CliError::NotUnicode(raw.to_string_lossy().into_owned()))?;
    Place::parse(&text).map_err(|e| CliError::Failed(Message::from_error("open", &text, &e).text))
}

/// The config and state folders: beside the program with `--portable`,
/// else the platform's.
pub fn paths(portable: bool) -> Option<Paths> {
    if portable {
        Paths::beside_program()
    } else {
        Paths::from_environment()
    }
}

/// Whether a display is there for the GUI (plan section 5.10):
/// always on Windows and macOS; on Linux and the other Unix systems when a
/// Wayland or X11 display is set. The GUI alpha calls this to choose
/// between the GUI and the terminal.
pub fn display_present() -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|name| env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// The folder a shell can move to for a place: the place itself when it
/// is local, the folder an archive sits in, nothing for a remote place.
fn local_folder(place: &Place) -> Option<Place> {
    match place.kind() {
        PlaceKind::Local => Some(place.clone()),
        PlaceKind::Archive => place
            .archive()
            .and_then(|(container, _)| local_folder(container))
            .and_then(|archive| archive.parent()),
        PlaceKind::Remote => None,
    }
}

/// Writes the `--cwd-file`: the folder as UTF-8 with no line end, or an
/// empty file when there is none or its path holds a control character
/// (the shell would misread it). Returns the reason in words when the
/// file is left empty.
pub fn write_cwd_file(file: &Path, folder: Option<&Place>) -> Result<Option<String>, CliError> {
    let (text, why) = match folder.and_then(local_folder) {
        None => (
            String::new(),
            Some("The panel is not on this computer; the shell stays where it was.".to_owned()),
        ),
        Some(folder) => {
            let path =
                diroxide_core::place::strip_extended_prefix(&folder.to_string()).into_owned();
            if path.chars().any(char::is_control) {
                (
                    String::new(),
                    Some(
                        "The folder's name has control characters; the shell stays where it was."
                            .to_owned(),
                    ),
                )
            } else {
                (path, None)
            }
        }
    };
    std::fs::write(file, text).map_err(|e| {
        CliError::Failed(format!(
            "the folder for the shell could not be written to {}: {e}",
            file.display()
        ))
    })?;
    Ok(why)
}

/// Opens the panels with the platform's config and state folders: the
/// full screen on a terminal, one screen of plain lines otherwise.
pub fn run(options: &OpenOptions, out: &mut dyn Write) -> Result<i32, CliError> {
    let here = current_place()?;
    let paths = paths(options.portable);
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        run_terminal(options, paths.as_ref(), &here, out)
    } else {
        run_with(options, paths.as_ref(), &here, out)
    }
}

/// Says what the options cannot do before the screen opens: the GUI
/// comes with alpha.6; an unknown theme gives the default.
fn notices(options: &OpenOptions, out: &mut dyn Write) -> Result<(), CliError> {
    let write_failed = |e: std::io::Error| CliError::Failed(e.to_string());
    if options.gui {
        writeln!(
            out,
            "The GUI comes with a later alpha; the terminal opens instead."
        )
        .map_err(write_failed)?;
    }
    if let Some(theme) = &options.theme
        && !diroxide_app::settings::theme_names().contains(theme)
    {
        writeln!(out, "Unknown theme: {theme}. Galaxy is in use.").map_err(write_failed)?;
    }
    Ok(())
}

/// The terminal's options for an app as built.
fn tui_options(app: &App, options: &OpenOptions) -> TuiOptions {
    TuiOptions {
        screen_reader: app.screen_reader(),
        inline: options.inline,
        ..TuiOptions::default()
    }
}

/// Runs the full screen on the terminal with these folders until the app
/// quits, then writes the `--cwd-file`.
pub fn run_terminal(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    notices(options, out)?;
    out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
    let (mut app, mut runtime) = build(options, paths, here)?;
    let files = Files {
        themes: paths.map(Paths::themes_dir),
        log: paths.map(|p| p.state_file(TERMINAL_LOG)),
    };
    Tui::new(tui_options(&app, options))
        .run_terminal(&mut app, &mut runtime, &files)
        .map_err(|e| CliError::Failed(Message::from_error("run", "the screen", &e).text))?;
    finish(&app, options, out)
}

/// After the screen: the `--cwd-file` for the shell wrapper, empty when
/// the shell keeps its own folder.
fn finish(app: &App, options: &OpenOptions, out: &mut dyn Write) -> Result<i32, CliError> {
    if let Some(file) = &options.cwd_file {
        if app.keeps_shell_folder() {
            // Empty: the wrapper leaves the shell where it was.
            write_cwd_file(Path::new(file), None)?;
            return Ok(0);
        }
        let folder = app
            .quit_folder()
            .cloned()
            .unwrap_or_else(|| app.active_panel().place().clone());
        if let Some(why) = write_cwd_file(Path::new(file), Some(&folder))? {
            writeln!(out, "{why}").map_err(|e| CliError::Failed(e.to_string()))?;
        }
    }
    Ok(0)
}

/// The app as `diro` starts it: config, keys, startup, session and clock.
/// Tests pass their own folders, never the real ones.
pub fn build(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
) -> Result<(App, Runtime), CliError> {
    build_on(
        options,
        paths,
        here,
        std::sync::Arc::new(diroxide_vfs::Registry::with_local()),
    )
}

/// The app as [`build`] makes it, over a registry of providers: the tests'
/// way to a provider whose trash is a test folder, never the system's.
pub fn build_on(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
    registry: std::sync::Arc<diroxide_vfs::Registry>,
) -> Result<(App, Runtime), CliError> {
    let resolve = |text: &str| {
        Place::resolve(text, here)
            .map_err(|e| CliError::Failed(Message::from_error("open", text, &e).text))
    };
    let platform = Platform::current();
    let config = paths.map(diroxide_config::load).unwrap_or_default();
    let keymap = diroxide_app::startup::terminal_keymap(&config, platform);
    let startup = paths
        .map(|paths| diroxide_app::startup::load(paths, &config, platform))
        .unwrap_or_default();
    let left = match options.places.first() {
        Some(text) => resolve(text)?,
        None => here.clone(),
    };
    let right = match options.places.get(1) {
        Some(text) => resolve(text)?,
        None if options.places.is_empty() => startup
            .session
            .as_ref()
            .and_then(|s| s.panel(diroxide_core::Side::Right).place.clone())
            .unwrap_or_else(|| left.clone()),
        None => left.clone(),
    };
    let mut settings = config.settings.clone();
    if let Some(theme) = &options.theme {
        settings.theme = theme.clone();
    }
    let mut protected: Vec<Place> = startup.home.iter().cloned().collect();
    if let Some(paths) = paths {
        for folder in [&paths.config_dir, &paths.state_dir] {
            if let Some(place) = folder.to_str().and_then(|f| Place::parse(f).ok()) {
                protected.push(place);
            }
        }
    }
    let engine = diroxide_app::runtime::engine_options(&settings, protected);
    let mut app_options = AppOptions::new(left, right);
    app_options.settings = settings;
    app_options.keymap = keymap;
    app_options.types = TypeRegistry::builtin().with_config(&config.filetypes);
    app_options.clock = std::sync::Arc::new(diroxide_app::startup::local_clock());
    app_options.startup = startup;
    let app = App::new(app_options);
    let editor = app.settings().editor.clone();
    let mut runtime = Runtime::new(registry).with_engine_options(engine);
    if options.no_plugins {
        // `--no-plugins`: nothing starts, and the Plugins menu says so
        // (W4d).
        runtime = runtime.without_plugins();
    }
    runtime.set_editor(&editor);
    if let Some(paths) = paths {
        runtime = runtime.with_paths(paths.clone());
    }
    Ok((app, runtime))
}

/// Opens the panels with these folders and prints one screen of plain
/// lines: the way out when standard output is not a terminal, and the
/// tests' way in.
pub fn run_with(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    notices(options, out)?;
    let (mut app, mut runtime) = build(options, paths, here)?;
    Tui::new(tui_options(&app, options))
        .run(&mut app, &mut runtime, out)
        .map_err(|e| CliError::Failed(e.to_string()))?;
    finish(&app, options, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(dir: &tempfile::TempDir) -> Paths {
        Paths::new(dir.path().join("config"), dir.path().join("state"))
    }

    #[test]
    fn opening_this_crate_prints_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = folders(&dir);
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let options = OpenOptions {
            places: vec![env!("CARGO_MANIFEST_DIR").to_owned()],
            ..OpenOptions::default()
        };
        let mut out = Vec::new();
        let code = run_with(&options, Some(&paths), &here, &mut out).unwrap();
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("diroxide-cli, left panel, in "), "{text}");
        assert!(text.contains("src, folder"));
        assert!(paths.config_file(Paths::KEYMAP_DEFAULTS).exists());
    }

    #[test]
    fn diro_alone_restores_the_right_panel_and_a_theme_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let paths = folders(&dir);
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let src = here.join("src").unwrap();
        let mut session = diroxide_config::SessionState::default();
        session.panel_mut(diroxide_core::Side::Right).place = Some(src.clone());
        diroxide_config::save_session(&paths, &session).unwrap();
        let (app, _) = build(&OpenOptions::default(), Some(&paths), &here).unwrap();
        assert_eq!(app.panel(diroxide_core::Side::Left).place(), &here);
        assert_eq!(app.panel(diroxide_core::Side::Right).place(), &src);
        let options = OpenOptions {
            places: vec![".".to_owned(), "src".to_owned()],
            gui: true,
            theme: Some("no-such".to_owned()),
            ..OpenOptions::default()
        };
        let mut out = Vec::new();
        run_with(&options, Some(&paths), &here, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "The GUI comes with a later alpha; the terminal opens instead."
        );
        assert_eq!(lines[1], "Unknown theme: no-such. Galaxy is in use.");
    }

    #[test]
    fn the_cwd_file_holds_a_local_folder_or_nothing_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cwd.txt");
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        assert_eq!(write_cwd_file(&file, Some(&here)).unwrap(), None);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            env!("CARGO_MANIFEST_DIR")
        );
        let inside = here.join("data.zip").unwrap().into_archive();
        write_cwd_file(&file, Some(&inside.join("x").unwrap())).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            env!("CARGO_MANIFEST_DIR")
        );
        let remote = Place::parse("sftp://example.org/home").unwrap();
        let why = write_cwd_file(&file, Some(&remote)).unwrap().unwrap();
        assert!(why.starts_with("The panel is not on this computer"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        let odd = here.join("bad\u{7}name").unwrap();
        let why = write_cwd_file(&file, Some(&odd)).unwrap().unwrap();
        assert!(why.contains("control characters"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        let _ = display_present();
    }
}
