//! `diro doctor`: what this diro has and finds, one fact a line, the
//! name first (alpha.7, R30 commit 20; R28 1.3), in this order: the
//! program, the screen-reader mode and how it was decided, the terminal,
//! the config, state, components and cache folders, each tool (the
//! registry's report, the lines the Settings Tools page will say), each
//! component, each plugin, and the startup problems line.
//!
//! - **Nothing private.** No secret, no environment dump, and no home
//!   folder: a folder under it is said from `~` unless `--paths` asks for
//!   it in full. Tools are said by where they were found ("on PATH",
//!   "in Program Files"), never by their paths.
//! - **Exit code 0,** unless `--check` is given and a needed tool is
//!   missing (1): the tools named after it, by name or by id, or with none
//!   named the tools the owner's `conversions.toml` recipes run.
//! - **Nothing is run but the tools' version commands,** each with its
//!   version arguments only, two seconds at most, side by side.

use std::io::{self, IsTerminal, Write};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use diroxide_components::manifest::Manifest;
use diroxide_components::{Kind, Status, host_target, sha256_hex};
use diroxide_config::{Config, Paths, ScreenReaderMode, problems_line};
use diroxide_plugins::script::installed;
use diroxide_plugins::script::interpreter::Lookup;
use diroxide_tools::{BUILTIN, ToolKind, ToolRegistry, ToolsReport};

use crate::CliError;
use crate::cmd::archive::NoAsker;
use crate::cmd::plugin::{self, PluginAction, PluginArgs};

/// `diro doctor` as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctorArgs {
    /// `--check [TOOL...]`: `None` without it; the tools' ids it names,
    /// empty when it names none (the recipes' tools are checked).
    pub check: Option<Vec<String>>,
    /// `--paths`: the folders in full, the home folder included.
    pub paths: bool,
    /// `--portable`: the settings and components beside the program.
    pub portable: bool,
    /// `--help`.
    pub help: bool,
}

/// The help for `diro doctor`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro doctor: what this diro has and finds, one fact a line, the name first.",
        "",
        "Usage:",
        "diro doctor                    the program, screen reader mode, terminal, folders, tools, components, plugins",
        "diro doctor --check [TOOL...]  the same; exit 1 when a needed tool is missing",
        "diro doctor --paths            say the folders in full; else the home folder is said as ~",
        "",
        "With --check, the needed tools are those named, by name or id, or else the tools",
        "your conversions.toml recipes run.",
        "",
        "Options:",
        "--portable  use the settings and components beside the program",
        "",
        "Exit codes:",
        "0  done, and nothing needed is missing",
        "1  with --check, a needed tool is missing",
        "2  wrong use, such as an unknown option or tool",
    ]
    .join("\n")
}

/// The id of a tool named by its id or its name, ignoring case; built-in
/// tools are not programs and are not checked.
fn tool_id(word: &str) -> Option<&'static str> {
    BUILTIN
        .iter()
        .filter(|spec| spec.kind == ToolKind::Program)
        .find(|spec| spec.id.eq_ignore_ascii_case(word) || spec.name.eq_ignore_ascii_case(word))
        .map(|spec| spec.id)
}

/// Reads the arguments after `doctor`.
pub fn parse(args: &[String]) -> Result<DoctorArgs, CliError> {
    let mut parsed = DoctorArgs::default();
    let mut checking = false;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                parsed.help = true;
                return Ok(parsed);
            }
            "--paths" => {
                parsed.paths = true;
                checking = false;
            }
            "--portable" => {
                parsed.portable = true;
                checking = false;
            }
            "--check" => {
                parsed.check.get_or_insert_with(Vec::new);
                checking = true;
            }
            option if option.starts_with('-') => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            word if checking => {
                let id = tool_id(word).ok_or_else(|| {
                    CliError::Failed(format!(
                        "no tool is called {word}; diro doctor lists the tools"
                    ))
                })?;
                if let Some(check) = parsed.check.as_mut()
                    && !check.iter().any(|known| known == id)
                {
                    check.push(id.to_owned());
                }
            }
            _ => return Err(CliError::TooMany("doctor".to_owned())),
        }
    }
    Ok(parsed)
}

/// What the terminal answers now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalFacts {
    /// Whether standard output is a terminal.
    pub is_terminal: bool,
    /// Its columns and rows, when it is one.
    pub size: Option<(u16, u16)>,
}

impl TerminalFacts {
    /// Asks the process's own standard output.
    pub fn detect() -> Self {
        let is_terminal = io::stdout().is_terminal();
        Self {
            is_terminal,
            size: if is_terminal {
                crossterm::terminal::size().ok()
            } else {
                None
            },
        }
    }
}

/// What a run looks at: the real folders, tools and terminal in `diro`,
/// fake ones in tests.
pub struct Env {
    /// The config and state folders.
    pub paths: Paths,
    /// Whether they are the `--portable` ones.
    pub portable: bool,
    /// The settings, the recipes and `tools.toml`, read once.
    pub config: Config,
    /// The tools, with `tools.toml` and the installed helper components.
    pub tools: ToolRegistry,
    /// The components this diro knows.
    pub components: Manifest,
    /// The computer the components are for.
    pub target: String,
    /// The home folder, said as `~` unless `--paths`.
    pub home: Option<PathBuf>,
    /// The terminal now.
    pub terminal: TerminalFacts,
    /// The plugins' lines, already made ([`plugin_lines`]).
    pub plugins: Vec<String>,
}

/// Runs `diro doctor` with the real folders, tools and terminal.
pub fn run(args: &DoctorArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_error)?;
        return Ok(0);
    }
    let Some(paths) = crate::open::paths(args.portable) else {
        return Err(CliError::Failed(
            "Could not find the settings folder. Try --portable.".to_owned(),
        ));
    };
    let config = diroxide_config::load(&paths);
    let components = Manifest::embedded();
    let target = host_target().to_owned();
    let tools = ToolRegistry::builtin()
        .with_overrides(config.tools.clone())
        .with_components(helper_programs(
            &components,
            &paths.components_dir(),
            &target,
        ));
    let plugins = plugin_lines(&paths, &tools);
    let env = Env {
        paths,
        portable: args.portable,
        config,
        tools,
        components,
        target,
        home: home_folder(),
        terminal: TerminalFacts::detect(),
        plugins,
    };
    run_in(args, &env, out)
}

fn write_error(e: io::Error) -> CliError {
    CliError::Failed(e.to_string())
}

/// The home folder, from the environment the system gives every program.
fn home_folder() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

/// The programs of the installed helper components, for the registry's
/// component step: each file with an `exec`, in its component's folder.
pub fn helper_programs(manifest: &Manifest, components_dir: &Path, target: &str) -> Vec<PathBuf> {
    manifest
        .components
        .iter()
        .filter(|c| c.kind == Kind::Helper)
        .filter(|c| matches!(c.status_in(components_dir, target), Status::Installed))
        .flat_map(|c| {
            let dir = c.dir_in(components_dir);
            c.files_for(target)
                .into_iter()
                .filter_map(|file| file.exec.clone())
                .map(move |exec| {
                    exec.split('/')
                        .filter(|part| !part.is_empty())
                        .fold(dir.clone(), |path, part| path.join(part))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The plugins' lines: the count, then `diro plugin list`'s line for each
/// ("Line counter: enabled, a plugin"), or "Plugins: none".
pub fn plugin_lines(paths: &Paths, tools: &ToolRegistry) -> Vec<String> {
    let lookup = Lookup::system();
    if installed::scan(&paths.plugins_dir(), lookup.os).is_empty() {
        return vec!["Plugins: none".to_owned()];
    }
    let Ok(here) = crate::open::current_place() else {
        return vec!["Plugins: could not be listed".to_owned()];
    };
    let mut listed = Vec::new();
    let mut input = io::empty();
    let mut refusals = io::sink();
    let env = plugin::Env {
        paths: paths.clone(),
        asker: &NoAsker,
        input: &mut input,
        err: &mut refusals,
        lookup,
        registry: tools.clone(),
        here,
        hash: sha256_hex,
    };
    let args = PluginArgs {
        action: PluginAction::List,
        yes: false,
        portable: false,
        explain: false,
        language: None,
        kind: None,
        dir: None,
        to: None,
    };
    if plugin::run_in(&args, env, &mut listed).is_err() {
        return vec!["Plugins: could not be listed".to_owned()];
    }
    let rows: Vec<String> = String::from_utf8_lossy(&listed)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let mut lines = vec![format!("Plugins: {}", rows.len())];
    lines.extend(rows);
    lines
}

/// Runs `diro doctor` in `env`, writing its lines to `out`.
pub fn run_in(args: &DoctorArgs, env: &Env, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_error)?;
        return Ok(0);
    }
    let report = env.tools.report();
    let mut lines = program_lines();
    lines.push(screen_reader_line(env.config.settings.screen_reader_mode));
    lines.extend(terminal_lines(env));
    lines.extend(folder_lines(env, args.paths));
    lines.extend(report.lines());
    lines.extend(component_lines(env));
    lines.extend(env.plugins.iter().cloned());
    lines.push(startup_line(&env.config));
    let mut missing = false;
    if let Some(named) = &args.check {
        let (line, any_missing) = needed_line(named, env, &report);
        lines.push(line);
        missing = any_missing;
    }
    for line in &lines {
        writeln!(out, "{line}").map_err(write_error)?;
    }
    Ok(i32::from(missing))
}

/// The program, its system and what is built in.
fn program_lines() -> Vec<String> {
    let system = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    let mut built_in = vec!["terminal"];
    if cfg!(feature = "gui") {
        built_in.push("GUI");
    }
    if cfg!(feature = "fetch") {
        built_in.push("downloader");
    }
    if cfg!(feature = "syntax-set") {
        built_in.push("syntax set");
    }
    vec![
        format!("diro: {}", env!("CARGO_PKG_VERSION")),
        format!("System: {system} on {}", std::env::consts::ARCH),
        format!("Built in: {}", built_in.join(", ")),
    ]
}

/// The screen-reader mode and how it was decided: auto, the default,
/// counts as on.
fn screen_reader_line(mode: ScreenReaderMode) -> String {
    match mode {
        ScreenReaderMode::Auto => "Screen reader mode: on, the default".to_owned(),
        ScreenReaderMode::On => "Screen reader mode: on, from settings".to_owned(),
        ScreenReaderMode::Off => "Screen reader mode: off, from settings".to_owned(),
    }
}

/// The terminal now, then what it answered in the last terminal session,
/// from the terminal's log in the state folder (never queried here, so
/// no escape sequence is sent).
fn terminal_lines(env: &Env) -> Vec<String> {
    let mut lines = vec![if env.terminal.is_terminal {
        "Terminal: standard output is a terminal".to_owned()
    } else {
        "Terminal: output goes to a file or pipe".to_owned()
    }];
    if let Some((columns, rows)) = env.terminal.size {
        lines.push(format!("Terminal size: {columns} by {rows}"));
    }
    let log = env.paths.state_file(crate::open::TERMINAL_LOG);
    let answers: Vec<String> = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .take(50)
        .filter(|line| line.starts_with("Keyboard enhancement:") || line.starts_with("Color:"))
        .map(|line| diroxide_tools::version::clean_line(line, 200))
        .collect();
    if answers.is_empty() {
        lines.push("Last terminal session: none recorded".to_owned());
    } else {
        lines.push("Last terminal session: its answers".to_owned());
        lines.extend(answers);
    }
    lines
}

/// The folders, a home folder said as `~` unless `full`.
fn folder_lines(env: &Env, full: bool) -> Vec<String> {
    let say = |path: &Path| -> String {
        if !full
            && let Some(home) = &env.home
            && let Ok(rest) = path.strip_prefix(home)
        {
            return if rest.as_os_str().is_empty() {
                "~".to_owned()
            } else {
                format!("~{MAIN_SEPARATOR}{}", rest.display())
            };
        }
        path.display().to_string()
    };
    let mut lines = Vec::new();
    if env.portable {
        lines.push("Settings: beside the program, --portable".to_owned());
    }
    lines.push(format!("Config folder: {}", say(&env.paths.config_dir)));
    lines.push(format!("State folder: {}", say(&env.paths.state_dir)));
    lines.push(format!(
        "Components folder: {}",
        say(&env.paths.components_dir())
    ));
    lines.push("Cache folder: none, kept in memory".to_owned());
    lines
}

/// The count, then each component: its title and its state.
fn component_lines(env: &Env) -> Vec<String> {
    let dir = env.paths.components_dir();
    let states: Vec<(String, Status)> = env
        .components
        .components
        .iter()
        .map(|c| (c.title.clone(), c.status_in(&dir, &env.target)))
        .collect();
    let installed = states
        .iter()
        .filter(|(_, status)| matches!(status, Status::Installed))
        .count();
    let mut lines = vec![format!(
        "Components: {installed} of {} installed",
        states.len()
    )];
    lines.extend(
        states
            .iter()
            .map(|(title, status)| format!("{title}: {}", status.words())),
    );
    lines
}

/// The startup problems line: "none", or each file and its count.
fn startup_line(config: &Config) -> String {
    let problems = problems_line(&config.problems);
    if problems.is_empty() {
        "Startup problems: none".to_owned()
    } else {
        format!("Startup problems: {problems}")
    }
}

/// The needed tools' line, and whether one is missing: the tools named,
/// or the tools the owner's recipes run and those `tools.toml` marks
/// `needed`.
fn needed_line(named: &[String], env: &Env, report: &ToolsReport) -> (String, bool) {
    let mut needed: Vec<String> = named.to_vec();
    if needed.is_empty() {
        let marked = env
            .config
            .tools
            .tools
            .iter()
            .filter(|(_, own)| own.needed)
            .map(|(id, _)| id.as_str());
        let recipes = env.config.conversions.recipes.iter().map(|r| r.tool.trim());
        for tool in recipes.chain(marked) {
            let built_in = BUILTIN
                .iter()
                .any(|spec| spec.id == tool && spec.kind == ToolKind::BuiltIn);
            if !tool.is_empty() && !built_in && !needed.iter().any(|known| known == tool) {
                needed.push(tool.to_owned());
            }
        }
    }
    if needed.is_empty() {
        return ("Needed tools: none named".to_owned(), false);
    }
    let missing: Vec<String> = needed
        .iter()
        .filter(|id| {
            !report
                .tools
                .iter()
                .any(|tool| tool.id == **id && tool.found())
        })
        .map(|id| env.tools.name_of(id))
        .collect();
    if missing.is_empty() {
        (format!("Needed tools: all {} found", needed.len()), false)
    } else {
        (
            format!("Needed tools missing: {}", missing.join(", ")),
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use diroxide_config::{ConversionsConfig, RecipeConfig, ToolOverride, ToolsConfig};
    use diroxide_core::format::within_cells;
    use diroxide_tools::Platform;

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(&[]), Ok(DoctorArgs::default()));
        assert_eq!(
            parse(&args(&["--check", "pandoc", "FFmpeg", "pandoc", "--paths"])),
            Ok(DoctorArgs {
                check: Some(vec!["pandoc".to_owned(), "ffmpeg".to_owned()]),
                paths: true,
                ..DoctorArgs::default()
            })
        );
        assert_eq!(parse(&args(&["--check"])).unwrap().check, Some(Vec::new()));
        assert_eq!(
            parse(&args(&["--check", "nope"])),
            Err(CliError::Failed(
                "no tool is called nope; diro doctor lists the tools".to_owned()
            ))
        );
        assert_eq!(
            parse(&args(&["--bogus"])),
            Err(CliError::UnknownOption("--bogus".to_owned()))
        );
        assert_eq!(
            parse(&args(&["extra"])),
            Err(CliError::TooMany("doctor".to_owned()))
        );
        assert!(parse(&args(&["--help"])).unwrap().help);
        assert!(within_cells(
            help_text().lines().next().unwrap(),
            "diro doctor: what this diro has"
        ));
    }

    /// A bench: config, state and components folders under a test folder,
    /// a fake tool folder as the only PATH, and no install folders.
    struct Bench {
        root: tempfile::TempDir,
        bin: PathBuf,
    }

    impl Bench {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            fs::create_dir_all(&bin).unwrap();
            Self { root, bin }
        }

        fn paths(&self) -> Paths {
            Paths::new(
                self.root.path().join("home").join("config"),
                self.root.path().join("home").join("state"),
            )
        }

        /// A fake pandoc that says its version.
        fn pandoc(&self) {
            if cfg!(windows) {
                fs::write(
                    self.bin.join("pandoc.cmd"),
                    "@echo off\r\necho pandoc 3.13\r\n",
                )
                .unwrap();
            } else {
                let path = self.bin.join("pandoc");
                fs::write(&path, "#!/bin/sh\necho 'pandoc 3.13'\n").unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                }
            }
        }

        fn env(&self, config: Config) -> Env {
            let paths = self.paths();
            let tools = ToolRegistry::builtin()
                .with_path(Some(self.bin.as_os_str().to_owned()))
                .with_platform(Platform::Windows, None)
                .with_overrides(config.tools.clone());
            Env {
                plugins: plugin_lines(&paths, &tools),
                paths,
                portable: false,
                config,
                tools,
                components: Manifest::embedded(),
                target: host_target().to_owned(),
                home: Some(self.root.path().join("home")),
                terminal: TerminalFacts {
                    is_terminal: true,
                    size: Some((120, 30)),
                },
            }
        }
    }

    fn run_lines(args: &DoctorArgs, env: &Env) -> (i32, Vec<String>) {
        let mut out = Vec::new();
        let code = run_in(args, env, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        (code, text.lines().map(str::to_owned).collect())
    }

    /// The report's sections in order, each line its name first; the
    /// folders under the home folder said from ~; no path of a tool.
    #[test]
    fn the_report_says_one_fact_a_line_name_first() {
        let bench = Bench::new();
        bench.pandoc();
        let (code, lines) = run_lines(&DoctorArgs::default(), &bench.env(Config::default()));
        assert_eq!(code, 0);
        assert_eq!(lines[0], format!("diro: {}", env!("CARGO_PKG_VERSION")));
        assert!(lines[1].starts_with("System: "), "{}", lines[1]);
        assert!(lines[2].starts_with("Built in: terminal"), "{}", lines[2]);
        let expect_in_order = [
            "Screen reader mode: on, the default",
            "Terminal: standard output is a terminal",
            "Terminal size: 120 by 30",
            "Last terminal session: none recorded",
            &format!("Config folder: ~{MAIN_SEPARATOR}config"),
            &format!("State folder: ~{MAIN_SEPARATOR}state"),
            &format!("Components folder: ~{MAIN_SEPARATOR}state{MAIN_SEPARATOR}components"),
            "Cache folder: none, kept in memory",
            "pandoc: 3.13, on PATH",
            "Plugins: none",
            "Startup problems: none",
        ];
        let mut from = 0;
        for wanted in expect_in_order {
            let at = lines[from..]
                .iter()
                .position(|l| l == wanted)
                .unwrap_or_else(|| panic!("{wanted} after line {from}: {lines:#?}"));
            from += at + 1;
        }
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Tools: ") && l.ends_with(" found"))
        );
        assert!(lines.iter().any(|l| l == "carta: 0.0.13, built in"));
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Components: ") && l.ends_with(" installed"))
        );
        assert!(lines.iter().any(|l| l == "Example plugins: not installed"));
        let root = bench.root.path().display().to_string();
        assert!(
            lines.iter().all(|l| !l.contains(&root)),
            "a full path was said: {lines:#?}"
        );
        // Each line's subject (before its colon, or a next step's first
        // three words) is within its first 40 cells.
        for line in &lines {
            let subject = match line.split_once(':') {
                Some((name, _)) => name.to_owned(),
                None => line.split(' ').take(3).collect::<Vec<_>>().join(" "),
            };
            assert!(within_cells(line, &subject), "{line}");
        }
        for line in [
            "Screen reader mode: on, the default",
            "Terminal: standard output is a terminal",
            "Cache folder: none, kept in memory",
            "pandoc: 3.13, on PATH",
            "Startup problems: none",
        ] {
            assert!(line.chars().count() <= 40, "{line}");
        }
        let (_, full) = run_lines(
            &DoctorArgs {
                paths: true,
                ..DoctorArgs::default()
            },
            &bench.env(Config::default()),
        );
        assert!(
            full.iter()
                .any(|l| l.starts_with("Config folder: ") && l.contains(&root))
        );
    }

    /// --check: exit 1 when a needed tool is missing, the missing ones
    /// named; with none named, the recipes' tools are needed.
    #[test]
    fn check_exits_one_when_a_needed_tool_is_missing() {
        let bench = Bench::new();
        bench.pandoc();
        let check = |names: &[&str]| DoctorArgs {
            check: Some(names.iter().map(|n| (*n).to_owned()).collect()),
            ..DoctorArgs::default()
        };
        let (code, lines) = run_lines(&check(&["pandoc"]), &bench.env(Config::default()));
        assert_eq!(code, 0);
        assert_eq!(lines.last().unwrap(), "Needed tools: all 1 found");
        let (code, lines) = run_lines(&check(&["pandoc", "ffmpeg"]), &bench.env(Config::default()));
        assert_eq!(code, 1);
        assert_eq!(lines.last().unwrap(), "Needed tools missing: FFmpeg");
        let (code, lines) = run_lines(&check(&[]), &bench.env(Config::default()));
        assert_eq!(code, 0);
        assert_eq!(lines.last().unwrap(), "Needed tools: none named");
        let recipes = Config {
            conversions: ConversionsConfig {
                recipes: vec![
                    RecipeConfig {
                        tool: "magick".to_owned(),
                        ..RecipeConfig::default()
                    },
                    RecipeConfig {
                        tool: "carta".to_owned(),
                        ..RecipeConfig::default()
                    },
                ],
            },
            ..Config::default()
        };
        let (code, lines) = run_lines(&check(&[]), &bench.env(recipes));
        assert_eq!(code, 1);
        assert_eq!(lines.last().unwrap(), "Needed tools missing: ImageMagick");
        // A tool tools.toml marks needed is checked with none named (K's
        // request 1).
        let marked = Config {
            tools: ToolsConfig::from_toml("[ffmpeg]\nneeded = true\n\n[pandoc]\nneeded = true\n")
                .unwrap(),
            ..Config::default()
        };
        let (code, lines) = run_lines(&check(&[]), &bench.env(marked));
        assert_eq!(code, 1);
        assert_eq!(lines.last().unwrap(), "Needed tools missing: FFmpeg");
        // Without --check, a missing tool never changes the exit code.
        let (code, _) = run_lines(&DoctorArgs::default(), &bench.env(Config::default()));
        assert_eq!(code, 0);
    }

    /// tools.toml's words, the settings' mode, a config problem and the
    /// last terminal session's answers.
    #[test]
    fn the_settings_and_the_terminal_log_are_said() {
        let bench = Bench::new();
        let paths = bench.paths();
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(
            paths.state_file(crate::open::TERMINAL_LOG),
            "Terminal: alternate screen, raw mode, mouse capture off.\nKeyboard enhancement: the terminal answered no.\nColor: TrueColor.\n",
        )
        .unwrap();
        let mut config = Config::default();
        config.settings.screen_reader_mode = ScreenReaderMode::Off;
        config.tools = ToolsConfig::default();
        config.tools.tools.insert(
            "ffmpeg".to_owned(),
            ToolOverride {
                path: None,
                disabled: true,
                needed: false,
            },
        );
        config.problems.push(diroxide_config::ConfigProblem::new(
            "keymap.toml",
            "a problem",
        ));
        let mut env = bench.env(config);
        env.terminal = TerminalFacts::default();
        let (_, lines) = run_lines(&DoctorArgs::default(), &env);
        for wanted in [
            "Screen reader mode: off, from settings",
            "Terminal: output goes to a file or pipe",
            "Last terminal session: its answers",
            "Keyboard enhancement: the terminal answered no.",
            "Color: TrueColor.",
            "FFmpeg: turned off in tools.toml",
            "pandoc: not found",
            "Install: winget install JohnMacFarlane.Pandoc",
            "Startup problems: keymap.toml: 1 problem",
        ] {
            assert!(lines.iter().any(|l| l == wanted), "{wanted}: {lines:#?}");
        }
        assert!(!lines.iter().any(|l| l.starts_with("Terminal size:")));
    }
}
