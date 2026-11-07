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
//! Which frontend opens is [`choose_frontend`]'s decision table: the flag,
//! then the `frontend` setting, then the automatic choice (the GUI on a
//! desktop, the terminal over SSH, on a console or with a pipe). A GUI
//! that cannot start says one sentence and the terminal opens instead.
//! `--graphics NAME` chooses the GUI's graphics API for one start, and
//! `--background` opens the GUI off screen, never in front, for automated
//! checks. `--inline` draws below the prompt instead of on the alternate
//! screen; `--cwd-file FILE` writes the active panel's last local folder at
//! quit for the shell wrapper of `diro init`; `--theme NAME` sets the theme
//! for this session.
//!
//! In the terminal, the full screen runs until the app quits; when standard
//! input or output is not a terminal (a pipe, a file), one screen is
//! printed as plain lines instead. Subcommands (`diro ls`, `diro serve`
//! and the rest) never come here, so they never open the GUI.

use std::env;
use std::io::{IsTerminal, Write};
use std::path::Path;

use diroxide_app::keymap::Platform;
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_config::{Frontend as FrontendSetting, Graphics, Paths};
use diroxide_core::{KeyColumn, Message, Place, PlaceKind};
use diroxide_filetype::TypeRegistry;
use diroxide_gui::{GraphicsApi, GuiOptions};
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
    /// The GUI, even where the terminal would open.
    pub gui: bool,
    /// The GUI off screen, never activated, with no taskbar button: for
    /// automated checks, which must not take the focus.
    pub background: bool,
    /// The GUI's graphics API for this start, over the `graphics` setting.
    pub graphics: Option<GraphicsApi>,
    /// The inline viewport instead of the alternate screen (W1d-2).
    pub inline: bool,
    /// Where to write the folder the shell moves to at quit.
    pub cwd_file: Option<String>,
    /// The theme for this session.
    pub theme: Option<String>,
    /// `--log FILE`: the GUI's diagnostic lines (the graphics adapter and
    /// the drawing road taken) appended to this file.
    pub log: Option<String>,
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

/// Whether this is a remote session over SSH, where a window would open on
/// the far machine's desktop, not the user's: `SSH_CONNECTION` or
/// `SSH_TTY` is set.
pub fn remote_session() -> bool {
    ["SSH_CONNECTION", "SSH_TTY"]
        .iter()
        .any(|name| env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// The frontend that opens the panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frontend {
    /// The GUI's window.
    Gui,
    /// The terminal: the full screen, or plain lines on a pipe.
    Terminal,
}

/// What [`choose_frontend`] decides from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontendInputs {
    /// `--gui`.
    pub gui_flag: bool,
    /// `--tui`.
    pub tui_flag: bool,
    /// The `frontend` setting.
    pub setting: FrontendSetting,
    /// A remote session over SSH ([`remote_session`]).
    pub remote: bool,
    /// A display for a window ([`display_present`]).
    pub display: bool,
    /// Standard input and output are both a terminal (not a pipe or a
    /// file).
    pub interactive: bool,
    /// This `diro` has the GUI (its `gui` feature).
    pub gui_built: bool,
}

/// The frontend chosen, and the sentence said first when the choice is not
/// the one asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendChoice {
    /// The frontend.
    pub frontend: Frontend,
    /// Said before the terminal opens, when the GUI was asked for and
    /// cannot open.
    pub notice: Option<String>,
}

/// The sentence when the GUI was asked for and this build has none.
pub const NOT_BUILT_NOTICE: &str =
    "This diro was built without the GUI; the terminal opens instead.";

/// The sentence when the GUI was asked for and there is no display.
pub const NO_DISPLAY_NOTICE: &str =
    "There is no display for the GUI (no WAYLAND_DISPLAY or DISPLAY); the terminal opens instead.";

/// Whether a desktop opens the GUI with no flag and no setting: true from
/// the GUI alpha's last lane (R27's commit 34, Tuesday, November 3, 2026).
/// `diro` from a shell on a desktop, or a double-click on Windows, opens
/// the window; the decision table below keeps the terminal over SSH, with
/// no display, with a pipe or a file, and in a build without the GUI.
pub const GUI_BY_DEFAULT: bool = true;

/// Which frontend opens, in this order (the GUI alpha's contract step):
///
/// 1. `--tui` opens the terminal, whatever else is asked.
/// 2. `--gui` opens the GUI; where this build has no GUI or there is no
///    display, the terminal opens after a sentence that says why.
/// 3. The `frontend` setting: `gui` as `--gui`; `terminal` opens the
///    terminal.
/// 4. `auto`, the default: the terminal over SSH (`SSH_CONNECTION` or
///    `SSH_TTY`), with no display (Linux and the other Unix systems need
///    `WAYLAND_DISPLAY` or `DISPLAY`; Windows and macOS always have one),
///    when standard input or output is a pipe or a file, and in a build
///    without the GUI; the GUI everywhere else.
///
/// Subcommands never come here. A GUI that starts and then fails says one
/// sentence and opens the terminal ([`run`]).
pub fn choose_frontend(inputs: &FrontendInputs) -> FrontendChoice {
    let terminal = |notice: Option<&str>| FrontendChoice {
        frontend: Frontend::Terminal,
        notice: notice.map(str::to_owned),
    };
    if inputs.tui_flag {
        return terminal(None);
    }
    let gui_asked = inputs.gui_flag || inputs.setting == FrontendSetting::Gui;
    if gui_asked {
        if !inputs.gui_built {
            return terminal(Some(NOT_BUILT_NOTICE));
        }
        if !inputs.display {
            return terminal(Some(NO_DISPLAY_NOTICE));
        }
        return FrontendChoice {
            frontend: Frontend::Gui,
            notice: None,
        };
    }
    let automatic = inputs.setting == FrontendSetting::Auto
        && !inputs.remote
        && inputs.display
        && inputs.interactive
        && inputs.gui_built;
    if automatic && GUI_BY_DEFAULT {
        FrontendChoice {
            frontend: Frontend::Gui,
            notice: None,
        }
    } else {
        terminal(None)
    }
}

/// The GUI's graphics API: `--graphics` first, then the `graphics`
/// setting.
pub fn graphics_api(flag: Option<GraphicsApi>, setting: Graphics) -> GraphicsApi {
    flag.unwrap_or(match setting {
        Graphics::Auto => GraphicsApi::Auto,
        Graphics::Vulkan => GraphicsApi::Vulkan,
        Graphics::Dx12 => GraphicsApi::Dx12,
        Graphics::Metal => GraphicsApi::Metal,
        Graphics::Gl => GraphicsApi::Gl,
        Graphics::Software => GraphicsApi::Software,
    })
}

/// The sentence said when the GUI started and could not open: the cause
/// first, that the terminal opens instead, then what to try.
pub fn gui_failed_sentence(error: &diroxide_gui::GuiError) -> String {
    use diroxide_core::Explain;
    let cause = error.to_string();
    let next = error.next_step();
    format!("Could not open the GUI: {cause}; the terminal opens instead. {next}")
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

/// Opens the panels with the platform's config and state folders, in the
/// frontend [`choose_frontend`] picks: the GUI's window, or the full screen
/// on a terminal, one screen of plain lines otherwise.
pub fn run(options: &OpenOptions, out: &mut dyn Write) -> Result<i32, CliError> {
    let here = current_place()?;
    let paths = paths(options.portable);
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let settings = paths.as_ref().map(start_settings).unwrap_or_default();
    let choice = choose_frontend(&FrontendInputs {
        gui_flag: options.gui,
        tui_flag: options.tui,
        setting: settings.frontend,
        remote: remote_session(),
        display: display_present(),
        interactive,
        gui_built: diroxide_gui::built(),
    });
    let write_failed = |e: std::io::Error| CliError::Failed(e.to_string());
    if let Some(notice) = &choice.notice {
        writeln!(out, "{notice}").map_err(write_failed)?;
    }
    if choice.frontend == Frontend::Gui {
        let gui = GuiOptions {
            background: options.background,
            graphics: graphics_api(options.graphics, settings.graphics),
            log: options.log.as_ref().map(std::path::PathBuf::from),
            ..GuiOptions::default()
        };
        return run_gui(options, paths.as_ref(), &here, gui, out);
    }
    open_terminal(options, paths.as_ref(), &here, out)
}

/// The settings the start chooses by (`frontend`, `graphics`), read
/// quietly: a file that is missing or not TOML gives the defaults here,
/// and [`build`] reads it again, says its problems once and backs a broken
/// file up.
fn start_settings(paths: &Paths) -> diroxide_config::Settings {
    std::fs::read_to_string(paths.config_file(Paths::SETTINGS))
        .ok()
        .and_then(|text| diroxide_config::Settings::from_toml(&text).ok())
        .unwrap_or_default()
}

/// The terminal: the full screen when standard input and output are a
/// terminal, one screen of plain lines otherwise.
fn open_terminal(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        run_terminal(options, paths, here, out)
    } else {
        run_with(options, paths, here, out)
    }
}

/// Opens the GUI's window with these folders until the app quits, then
/// writes the `--cwd-file`. On Windows a console that only this start
/// opened (a double-click) is let go first, so it closes. A GUI that cannot
/// start says one sentence, and the terminal opens instead, on a new
/// console if the first was let go.
pub fn run_gui(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
    gui: GuiOptions,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    // Before the app and its runtime exist, so no other thread runs yet.
    diroxide_gui::prepare(&gui);
    let released = diroxide_gui::console::release_for_window();
    // The window reads the GUI column of the keymap.
    let registry = std::sync::Arc::new(diroxide_vfs::Registry::with_local());
    let (app, runtime) = match build_for(options, paths, here, registry, KeyColumn::Gui) {
        Ok(built) => built,
        Err(error) => {
            // The reason must reach the person: a message box when the
            // console was let go (a shortcut that names a folder that is
            // gone), and a console for the error line.
            diroxide_gui::console::tell_start_error(released, &error.to_string());
            diroxide_gui::console::regain_for_terminal(released);
            return Err(error);
        }
    };
    match diroxide_gui::run(app, runtime, gui) {
        Ok(app) => finish(&app, options, out),
        Err(error) => {
            let sentence = gui_failed_sentence(&error);
            // Started from a shortcut, the console is gone: a message box
            // says why before the terminal opens on a new one.
            diroxide_gui::console::tell_start_error(released, &sentence);
            diroxide_gui::console::regain_for_terminal(released);
            let write_failed = |e: std::io::Error| CliError::Failed(e.to_string());
            writeln!(out, "{sentence}").map_err(write_failed)?;
            open_terminal(options, paths, here, out)
        }
    }
}

/// Says what the options cannot do before the screen opens: an unknown
/// theme gives the default. A theme is known when it is built in or a
/// file of its name loads from the config's themes folder, as the app
/// reads them at start.
fn notices(
    options: &OpenOptions,
    paths: Option<&Paths>,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let write_failed = |e: std::io::Error| CliError::Failed(e.to_string());
    if let Some(theme) = &options.theme
        && !theme_is_known(theme, paths)
    {
        writeln!(out, "Unknown theme: {theme}. Galaxy is in use.").map_err(write_failed)?;
    }
    Ok(())
}

/// Whether a theme is built in or one of the owner's theme files.
fn theme_is_known(name: &str, paths: Option<&Paths>) -> bool {
    diroxide_app::settings::theme_names()
        .iter()
        .any(|n| *n == name)
        || paths.is_some_and(|paths| {
            diroxide_config::theme::registry(&paths.themes_dir(), Paths::SETTINGS)
                .0
                .get(name)
                .is_some()
        })
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
    notices(options, paths, out)?;
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
    build_for(options, paths, here, registry, KeyColumn::Terminal)
}

/// The app as [`build_on`] makes it, with the keys of one keymap column:
/// the terminal's, or the GUI's for the window (the GUI alpha's
/// foundation lane), whose keys a terminal cannot deliver (Ctrl+Tab,
/// F11, Shift with Enter).
pub fn build_for(
    options: &OpenOptions,
    paths: Option<&Paths>,
    here: &Place,
    registry: std::sync::Arc<diroxide_vfs::Registry>,
    column: KeyColumn,
) -> Result<(App, Runtime), CliError> {
    let resolve = |text: &str| {
        Place::resolve(text, here)
            .map_err(|e| CliError::Failed(Message::from_error("open", text, &e).text))
    };
    let platform = Platform::current();
    let config = paths.map(diroxide_config::load).unwrap_or_default();
    let keymap = match column {
        KeyColumn::Terminal => diroxide_app::startup::terminal_keymap(&config, platform),
        KeyColumn::Gui => {
            let preset = diroxide_app::keymap::presets::chosen(&config.settings.preset);
            diroxide_app::keymap::presets::keymaps(&config.keymap, preset, platform).1
        }
    };
    let startup = paths
        .map(|paths| diroxide_app::startup::load(paths, &config, platform))
        .unwrap_or_default();
    // Helper programs run through the tools registry: 7-Zip for a RAR, a
    // conversion's tool (lane AW).
    let tools = diroxide_tools::ToolRegistry::builtin()
        .with_overrides(config.tools.clone())
        .with_components(startup.helper_programs.clone());
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
    // The retro pack, read only while its setting is on (lane TL's note).
    app_options.types = TypeRegistry::builtin()
        .with_config(&config.filetypes)
        .with_retro(app_options.settings.filetypes.retro);
    app_options.clock = std::sync::Arc::new(diroxide_app::startup::local_clock());
    app_options.startup = startup;
    let app = App::new(app_options);
    let editor = app.settings().editor.clone();
    let mut runtime = Runtime::new(registry)
        .with_engine_options(engine)
        .with_tool_runner(std::sync::Arc::new(tools.clone()))
        .with_tool_versions(tools);
    // The watched folders' carta runs in a diro child (lane AW).
    if let Ok(program) = env::current_exe() {
        runtime = runtime.with_carta_child(program);
    }
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
    notices(options, paths, out)?;
    let (mut app, mut runtime) = build(options, paths, here)?;
    // One printed screen has no one to close a first-run list.
    app.set_headless(true);
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
            theme: Some("no-such".to_owned()),
            ..OpenOptions::default()
        };
        let mut out = Vec::new();
        run_with(&options, Some(&paths), &here, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Unknown theme: no-such. Galaxy is in use.");
        // A theme file of the name in the config's themes folder is known.
        std::fs::create_dir_all(paths.themes_dir()).unwrap();
        std::fs::write(
            paths.themes_dir().join("harbor.toml"),
            "[theme]\nname = \"harbor\"\ninherits = \"galaxy\"\n",
        )
        .unwrap();
        let options = OpenOptions {
            theme: Some("harbor".to_owned()),
            ..options
        };
        let mut out = Vec::new();
        run_with(&options, Some(&paths), &here, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("Unknown theme"),
            "a theme file is known: {text:?}"
        );
    }

    /// The inputs of a desktop start with nothing asked: Windows or macOS,
    /// or Linux with a display, from a shell or a double-click.
    fn desktop() -> FrontendInputs {
        FrontendInputs {
            gui_flag: false,
            tui_flag: false,
            setting: FrontendSetting::Auto,
            remote: false,
            display: true,
            interactive: true,
            gui_built: true,
        }
    }

    /// The decision table of [`choose_frontend`], row by row: the flag,
    /// then the setting, then the automatic choice.
    #[test]
    fn the_frontend_is_the_flag_then_the_setting_then_automatic() {
        use Frontend::{Gui, Terminal};
        let base = desktop();
        let rows: Vec<(&str, FrontendInputs, Frontend, Option<&str>)> = vec![
            ("a desktop opens the GUI", base, Gui, None),
            (
                "--tui wins over --gui and the setting",
                FrontendInputs {
                    tui_flag: true,
                    gui_flag: true,
                    setting: FrontendSetting::Gui,
                    ..base
                },
                Terminal,
                None,
            ),
            (
                "--gui opens it over SSH and with a pipe",
                FrontendInputs {
                    gui_flag: true,
                    remote: true,
                    interactive: false,
                    setting: FrontendSetting::Terminal,
                    ..base
                },
                Gui,
                None,
            ),
            (
                "--gui with no display says why",
                FrontendInputs {
                    gui_flag: true,
                    display: false,
                    ..base
                },
                Terminal,
                Some(NO_DISPLAY_NOTICE),
            ),
            (
                "--gui in a build without it says why",
                FrontendInputs {
                    gui_flag: true,
                    gui_built: false,
                    ..base
                },
                Terminal,
                Some(NOT_BUILT_NOTICE),
            ),
            (
                "the setting terminal",
                FrontendInputs {
                    setting: FrontendSetting::Terminal,
                    ..base
                },
                Terminal,
                None,
            ),
            (
                "the setting gui opens it with a pipe",
                FrontendInputs {
                    setting: FrontendSetting::Gui,
                    interactive: false,
                    ..base
                },
                Gui,
                None,
            ),
            (
                "the setting gui with no display says why",
                FrontendInputs {
                    setting: FrontendSetting::Gui,
                    display: false,
                    ..base
                },
                Terminal,
                Some(NO_DISPLAY_NOTICE),
            ),
            (
                "automatic over SSH",
                FrontendInputs {
                    remote: true,
                    ..base
                },
                Terminal,
                None,
            ),
            (
                "automatic with no display (a container, a console)",
                FrontendInputs {
                    display: false,
                    ..base
                },
                Terminal,
                None,
            ),
            (
                "automatic with a pipe or a file",
                FrontendInputs {
                    interactive: false,
                    ..base
                },
                Terminal,
                None,
            ),
            (
                "automatic in a build without the GUI, silently",
                FrontendInputs {
                    gui_built: false,
                    ..base
                },
                Terminal,
                None,
            ),
        ];
        for (row, inputs, frontend, notice) in rows {
            let choice = choose_frontend(&inputs);
            assert_eq!(choice.frontend, frontend, "{row}");
            assert_eq!(choice.notice.as_deref(), notice, "{row}");
        }
        // Each sentence carries its meaning first, within a Braille line.
        for notice in [NOT_BUILT_NOTICE, NO_DISPLAY_NOTICE] {
            assert!(notice.chars().take(40).collect::<String>().contains("GUI"));
        }
        // Wired to the real environment: whatever it holds, a choice is made.
        let _ = choose_frontend(&FrontendInputs {
            remote: remote_session(),
            display: display_present(),
            gui_built: diroxide_gui::built(),
            ..base
        });
    }

    /// The flip (R27's commit 34): with nothing asked, a desktop opens the
    /// GUI, from a shell or a double-click, and the setting `auto` says
    /// so; `diro --tui` and the setting `terminal` still open the
    /// terminal, with no sentence before it.
    #[test]
    fn a_desktop_opens_the_gui_by_default() {
        let choice = choose_frontend(&desktop());
        assert_eq!(choice.frontend, Frontend::Gui);
        assert_eq!(choice.notice, None);
        assert_eq!(FrontendSetting::default(), FrontendSetting::Auto);
        for terminal in [
            FrontendInputs {
                tui_flag: true,
                ..desktop()
            },
            FrontendInputs {
                setting: FrontendSetting::Terminal,
                ..desktop()
            },
        ] {
            let choice = choose_frontend(&terminal);
            assert_eq!(choice.frontend, Frontend::Terminal);
            assert_eq!(choice.notice, None);
        }
    }

    #[test]
    fn graphics_come_from_the_flag_then_the_setting() {
        assert_eq!(graphics_api(None, Graphics::Auto), GraphicsApi::Auto);
        assert_eq!(graphics_api(None, Graphics::Dx12), GraphicsApi::Dx12);
        assert_eq!(
            graphics_api(None, Graphics::Software),
            GraphicsApi::Software
        );
        assert_eq!(
            graphics_api(Some(GraphicsApi::Vulkan), Graphics::Dx12),
            GraphicsApi::Vulkan
        );
    }

    #[test]
    fn a_gui_that_cannot_start_says_so_and_what_to_try() {
        let said = gui_failed_sentence(&diroxide_gui::GuiError::Graphics(
            "no compatible WGPU device found".to_owned(),
        ));
        assert_eq!(
            said,
            "Could not open the GUI: the window could not start its graphics (no compatible WGPU device found); the terminal opens instead. Try another graphics API with diro --graphics, such as software, dx12, vulkan or gl. The terminal, diro --tui, needs none."
        );
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
