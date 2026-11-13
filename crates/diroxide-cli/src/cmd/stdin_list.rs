//! `diro -`: a list of paths from standard input, as a list that is not a
//! folder (alpha 5, lane CX; FC's one path-list reader).
//!
//! - **What it reads.** The whole of standard input first, before
//!   anything opens: NUL-separated paths by default (`find -print0`,
//!   `fd -0`, a `paths.nul` file through cmd.exe's `<`), one a line with
//!   `--lines`, FC's limits (16,384 paths, 64 MiB). A relative path is
//!   read against the current folder; a path given twice is kept once; a
//!   name that is not Unicode is said by its position.
//! - **What it says first:** "3 paths from standard input" (27 cells),
//!   then what could not be read, one line each.
//! - **The opening options** (`--portable`, `--tui`, `--gui`,
//!   `--no-plugins`, `--theme` and the rest) are taken as `diro [PATH]`
//!   takes them, before or after the `-`; they apply when the list opens
//!   in the app.
//! - **Where the list goes.** On a terminal the list opens in the app's
//!   results lister after the start, "3 paths from standard input, 1
//!   missing" (lane AS, through `App::open_place_list_at_start`), in the
//!   window or the full screen as `diro` chooses. Keys reach the
//!   terminal frontend with a pipe on standard input: crossterm reads the
//!   console (`CONIN$`) on Windows and `/dev/tty` on Linux and macOS, so
//!   the screen opens after the pipe is read (spike X1). When standard
//!   output is not a terminal, the places are said as plain lines, one
//!   entry a line in `diro ls`'s words, and nothing opens.
//!
//! Exit codes: 0 every path read; 1 none could be read, or the list is
//! too long; 2 wrong use, or a list whose separators are not the ones
//! asked for; 3 some could not be read.

use std::io::{IsTerminal, Read, Write};

use diroxide_config::Paths;
use diroxide_core::{KeyColumn, Place};
use diroxide_search::{PathLimits, PathListError, PathSeparator, read_path_list};
use diroxide_tui::{Files, Tui, TuiOptions};
use diroxide_vfs::{OpContext, Registry};

use crate::cmd::archive::{line, lines};
use crate::open::{
    Frontend, FrontendInputs, OpenOptions, TERMINAL_LOG, build, build_for, choose_frontend,
    current_place, display_present, graphics_api, gui_failed_sentence, remote_session,
};
use crate::{CliError, parse_open};

/// `diro -`'s arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StdinListArgs {
    /// The opening options, as `diro [PATH]` reads them; no places.
    pub open: OpenOptions,
    /// How standard input is separated.
    pub separator: PathSeparator,
}

/// Whether the arguments ask for `diro -`: a `-` among them, before any
/// `--`.
pub fn wants(args: &[String]) -> bool {
    args.iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "-")
}

/// Reads `diro -`'s arguments: `-`, `--lines` or `--null`, and the
/// opening options.
pub fn parse(args: &[String]) -> Result<StdinListArgs, CliError> {
    let mut lines_asked = false;
    let mut null = false;
    let mut rest = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-" => {}
            "--lines" => lines_asked = true,
            "--null" | "-0" => null = true,
            _ => rest.push(arg.clone()),
        }
    }
    if lines_asked && null {
        return Err(CliError::Failed(
            "--lines and --null cannot be used together".to_owned(),
        ));
    }
    let open = parse_open(&rest)?;
    if let Some(place) = open.places.first() {
        return Err(CliError::Failed(format!(
            "diro - opens the paths on standard input; {place} cannot be opened beside them"
        )));
    }
    Ok(StdinListArgs {
        open,
        separator: if lines_asked {
            PathSeparator::Lines
        } else {
            PathSeparator::Nul
        },
    })
}

/// The list's name in the app.
pub const TITLE: &str = "Standard input";

/// Runs `diro -` over this computer's places: on a terminal the list
/// opens in the app (the window or the full screen, as `diro` chooses);
/// when standard output is not a terminal its lines are printed.
pub fn run(
    args: &StdinListArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
) -> Result<i32, CliError> {
    let here = current_place()?;
    if std::io::stdout().is_terminal() {
        return open_in_app(args, input, out, &here);
    }
    let registry = crate::cmd::ls::providers();
    let clock = diroxide_app::startup::local_clock();
    run_with(args, input, out, &registry, &here, &clock)
}

/// Reads the list whole; `Err` carries the exit code once its lines are
/// said.
fn read_list(
    args: &StdinListArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
) -> Result<Result<diroxide_search::PathList, i32>, CliError> {
    match read_path_list(input, args.separator, PathLimits::default()) {
        Ok(list) => Ok(Ok(list)),
        Err(PathListError::Read(e)) => Err(CliError::Failed(format!(
            "standard input could not be read: {e}"
        ))),
        Err(error @ PathListError::TooLong(_)) => {
            lines(out, &[error.to_string(), NOTHING_OPENED.to_owned()])?;
            Ok(Err(1))
        }
        Err(error) => {
            line(out, &error.to_string())?;
            Ok(Err(2))
        }
    }
}

/// Opens the list in the app after the start (lane AS): the start line
/// is the count, "3 paths from standard input", and the app adds ", 1
/// missing" when it looks each place up. What could not be read is said
/// before the screen opens.
fn open_in_app(
    args: &StdinListArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    here: &Place,
) -> Result<i32, CliError> {
    let list = match read_list(args, input, out)? {
        Ok(list) => list,
        Err(code) => return Ok(code),
    };
    if list.is_empty() {
        line(out, "List is empty. Nothing to open.")?;
        return Ok(0);
    }
    let read = list.places(here);
    let start = read.words();
    for (item, why) in &read.refused {
        line(out, &format!("Could not read {}: {why}.", item.what()))?;
    }
    out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
    let options = &args.open;
    let paths = crate::open::paths(options.portable);
    let setting = paths
        .as_ref()
        .map(|p| diroxide_config::load(p).settings.frontend)
        .unwrap_or_default();
    let choice = choose_frontend(&FrontendInputs {
        gui_flag: options.gui,
        tui_flag: options.tui,
        setting,
        remote: remote_session(),
        display: display_present(),
        interactive: true,
        gui_built: diroxide_gui::built(),
    });
    if let Some(notice) = &choice.notice {
        line(out, notice)?;
    }
    if choice.frontend == Frontend::Gui {
        let gui = diroxide_gui::GuiOptions {
            background: options.background,
            graphics: graphics_api(options.graphics, Default::default()),
            log: options.log.as_ref().map(std::path::PathBuf::from),
            ..diroxide_gui::GuiOptions::default()
        };
        diroxide_gui::prepare(&gui);
        let registry = std::sync::Arc::new(Registry::with_local());
        let (mut app, runtime) =
            build_for(options, paths.as_ref(), here, registry, KeyColumn::Gui)?;
        app.open_place_list_at_start(TITLE, &start, read.places.clone());
        match diroxide_gui::run(app, runtime, gui) {
            Ok(_) => return Ok(0),
            Err(error) => line(out, &gui_failed_sentence(&error))?,
        }
    }
    let (mut app, mut runtime) = build(options, paths.as_ref(), here)?;
    app.open_place_list_at_start(TITLE, &start, read.places);
    let files = Files {
        themes: paths.as_ref().map(Paths::themes_dir),
        log: paths.as_ref().map(|p| p.state_file(TERMINAL_LOG)),
    };
    let tui = TuiOptions {
        screen_reader: app.screen_reader(),
        inline: options.inline,
        ..TuiOptions::default()
    };
    Tui::new(tui)
        .run_terminal(&mut app, &mut runtime, &files)
        .map_err(|e| CliError::Failed(e.to_string()))?;
    Ok(0)
}

/// [`run`] with the providers, the current folder and the clock given.
pub fn run_with(
    args: &StdinListArgs,
    input: &mut dyn Read,
    out: &mut dyn Write,
    registry: &Registry,
    here: &Place,
    clock: &dyn diroxide_core::Clock,
) -> Result<i32, CliError> {
    let list = match read_list(args, input, out)? {
        Ok(list) => list,
        Err(code) => return Ok(code),
    };
    if list.is_empty() {
        line(out, "List is empty. Nothing to open.")?;
        return Ok(0);
    }
    let read = list.places(here);
    line(out, &read.words())?;
    let mut failed = read.refused.len();
    for (item, why) in &read.refused {
        line(out, &format!("Could not read {}: {why}.", item.what()))?;
    }
    let ctx = OpContext::new();
    let mut said = 0usize;
    for place in &read.places {
        let entry = registry
            .resolve(place)
            .and_then(|provider| provider.stat(place, &ctx));
        match entry {
            Ok(entry) => {
                said += 1;
                line(
                    out,
                    &crate::cmd::ls::entry_line(&entry, false, place.parent().as_ref(), clock),
                )?;
            }
            Err(error) => {
                failed += 1;
                line(out, &missing_line(place, &error))?;
            }
        }
    }
    out.flush().map_err(|e| CliError::Failed(e.to_string()))?;
    Ok(match (said, failed) {
        (_, 0) => 0,
        (0, _) => 1,
        _ => 3,
    })
}

/// The last line when a list was refused whole.
pub const NOTHING_OPENED: &str = "Nothing was opened.";

/// A place the list names that cannot be read, the word first:
/// "Missing: gone.txt, in D:\old\place"; "Not read: secret.txt, in D:\a:
/// access is denied".
pub fn missing_line(place: &Place, error: &diroxide_vfs::VfsError) -> String {
    match error {
        diroxide_vfs::VfsError::NotFound => format!("Missing: {}", place.title()),
        other => format!("Not read: {}: {other}", place.title()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::BRAILLE_CELLS;
    use diroxide_core::{FixedClock, UtcOffset};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn run_in(dir: &tempfile::TempDir, list: &[&str], input: &[u8]) -> (i32, String) {
        let here = Place::parse(dir.path().to_str().unwrap()).unwrap();
        let registry = Registry::with_local();
        let clock = FixedClock::new(
            diroxide_core::Timestamp::from_unix(1_794_477_600, 0),
            UtcOffset::UTC,
        );
        let parsed = parse(&args(list)).unwrap();
        let mut out = Vec::new();
        let code = run_with(&parsed, &mut &input[..], &mut out, &registry, &here, &clock).unwrap();
        (code, String::from_utf8(out).unwrap())
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("open")).unwrap();
        std::fs::write(dir.path().join("open").join("report.md"), "# Report\n").unwrap();
        std::fs::write(dir.path().join("open").join("notes.txt"), "notes\n").unwrap();
        dir
    }

    #[test]
    fn the_opening_options_are_taken_before_or_after_the_dash() {
        assert!(wants(&args(&["-"])));
        assert!(wants(&args(&["--portable", "-"])));
        assert!(!wants(&args(&["--", "-"])));
        assert!(!wants(&args(&["here"])));
        let parsed = parse(&args(&["--portable", "-", "--tui", "--lines"])).unwrap();
        assert!(parsed.open.portable);
        assert!(parsed.open.tui);
        assert_eq!(parsed.separator, PathSeparator::Lines);
        assert!(matches!(
            parse(&args(&["-", "--lines", "--null"])),
            Err(CliError::Failed(_))
        ));
        assert!(matches!(
            parse(&args(&["-", "D:\\here"])),
            Err(CliError::Failed(_))
        ));
        assert!(matches!(
            parse(&args(&["-", "--bogus"])),
            Err(CliError::UnknownOption(_))
        ));
    }

    #[test]
    fn three_paths_are_said_count_first_then_one_a_line() {
        let dir = fixture();
        let input = b"open/report.md\0open/notes.txt\0open/report.md\0open/gone.txt\0";
        let (code, text) = run_in(&dir, &["-"], input);
        let lines: Vec<&str> = text.lines().collect();
        // 27 cells, as the owner's check quotes it.
        assert_eq!(lines[0], "3 paths from standard input");
        assert_eq!(lines[0].chars().count(), 27);
        assert!(lines[1].starts_with("report.md, file, "), "{text}");
        assert!(lines[2].starts_with("notes.txt, file, "), "{text}");
        assert_eq!(
            lines[3],
            format!(
                "Missing: gone.txt, in {}",
                dir.path().join("open").display()
            )
        );
        assert_eq!(code, 3);
    }

    #[test]
    fn separators_must_be_the_ones_asked_for() {
        let dir = fixture();
        assert_eq!(
            run_in(&dir, &["-"], b"open/report.md\nopen/notes.txt\n"),
            (2, "No NUL separators found. Use --lines.\n".to_owned())
        );
        let (code, text) = run_in(
            &dir,
            &["-", "--lines"],
            b"open/report.md\r\nopen/notes.txt\n",
        );
        assert_eq!(code, 0, "{text}");
        assert!(text.starts_with("2 paths from standard input\n"));
        assert_eq!(
            run_in(&dir, &["-"], b""),
            (0, "List is empty. Nothing to open.\n".to_owned())
        );
        let (code, text) = run_in(&dir, &["-"], b"open/report.md\0\xff\xfe\0");
        assert_eq!(code, 3, "{text}");
        assert!(
            text.starts_with("1 path from standard input. 1 could not be read\n"),
            "{text}"
        );
        assert!(text.contains("Could not read item 2: the name is not valid Unicode.\n"));
    }

    #[test]
    fn every_fixed_sentence_says_its_meaning_in_40_cells() {
        for sentence in [NOTHING_OPENED, "List is empty. Nothing to open."] {
            assert!(sentence.chars().count() <= BRAILLE_CELLS, "{sentence}");
        }
    }
}
