//! `diro watch PATH`: tell of each file that settles in a folder, one line
//! an arrival, until Ctrl+C (W3t, plan commit 34; R16's "tell" rule).
//!
//! - It says "Watching in for new files. Ctrl+C stops." once the folder is
//!   listed; what is there already is not an arrival. Then, for each file
//!   once it has stopped changing, "Arrived: report.pdf, 2.1 MB", or
//!   "Arrived again: ..." when a name settles a second time. Nothing is
//!   said while a file grows.
//! - A file is settled after 2 seconds with no change (10 on a place that
//!   cannot be watched, such as a network share, which is listed every 5
//!   seconds); on Windows, never while a program still holds it open for
//!   writing. `--settle SECONDS` sets the quiet period, for a producer
//!   that pauses longer.
//! - Unfinished and scratch names are never said (`.part`, `.crdownload`,
//!   `.tmp`, a leading dot, Office's `~$` lock files and the rest);
//!   folders and links are skipped; an online-only file is said as
//!   skipped.
//! - `--print0` (also spelled `--null` and `-0`, as `diro ls` and `diro
//!   search` spell it) writes each arrived file's full path and a NUL to
//!   standard output and nothing else, for `xargs -0`; the words go to
//!   standard error.
//! - Ctrl+C stops: "Stopped watching in: 3 arrived".
//!
//! Nothing is saved, moved or run. Exit codes, as `diro ls` has them: 0
//! watched until stopped, 1 the folder could not be watched, 2 wrong use,
//! 3 the watch ended early (the folder went away).

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use diroxide_core::{EntryKind, Message, Place, PlaceKind};
use diroxide_ops::tracker::{WatchNote, WatchOptions, spawn_watch, start_line, stopped_line};
use diroxide_vfs::{OpContext, Registry};

use crate::CliError;
use crate::open::current_place;

/// The shortest quiet period `--settle` takes, in seconds.
const SETTLE_LEAST: f64 = 0.5;

/// The longest quiet period `--settle` takes, in seconds: an hour.
const SETTLE_MOST: f64 = 3600.0;

/// What `diro watch` was asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchArgs {
    /// The folder, as typed; the current folder by default.
    pub place: Option<String>,
    /// Write NUL-separated paths only (`--print0`, `--null`, `-0`).
    pub print0: bool,
    /// The quiet period, instead of the default.
    pub settle: Option<Duration>,
    /// Print the help for `diro watch` and nothing else.
    pub help: bool,
}

/// The help for `diro watch`, one usage a line, meaning first.
pub fn help_text() -> String {
    [
        "diro watch: say each file that settles in a folder, one line each.",
        "",
        "Usage:",
        "diro watch [PATH]       watch a folder; the current folder by default",
        "",
        "Options:",
        "--settle SECONDS        wait this long with no change; 2 by default",
        "--print0, --null, -0    write NUL-separated paths only, for xargs -0",
        "",
        "Ctrl+C stops. Unfinished names such as .part and .crdownload are",
        "never said. Nothing is saved, moved or run.",
        "",
        "Exit codes:",
        "0  watched until stopped",
        "1  the folder could not be watched",
        "2  wrong use: a bad option",
        "3  the watch ended early: the folder went away",
    ]
    .join("\n")
}

/// Reads the arguments after `watch`.
pub fn parse(args: &[String]) -> Result<WatchArgs, CliError> {
    let mut parsed = WatchArgs::default();
    let mut only_places = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if only_places {
            if parsed.place.replace(arg.clone()).is_some() {
                return Err(CliError::TooMany("watch".to_owned()));
            }
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--" => only_places = true,
            "--print0" | "--null" | "-0" => parsed.print0 = true,
            "--help" | "-h" => parsed.help = true,
            "--settle" => {
                let value = inline
                    .or_else(|| rest.next().cloned())
                    .ok_or_else(|| CliError::Missing("--settle".to_owned()))?;
                parsed.settle = Some(settle(&value)?);
            }
            option if option.starts_with('-') && option.len() > 1 => {
                return Err(CliError::UnknownOption(option.to_owned()));
            }
            place => {
                if parsed.place.replace(place.to_owned()).is_some() {
                    return Err(CliError::TooMany("watch".to_owned()));
                }
            }
        }
    }
    Ok(parsed)
}

/// Reads `--settle`'s seconds: a number from 0.5 to 3600.
fn settle(text: &str) -> Result<Duration, CliError> {
    match text.trim().parse::<f64>() {
        Ok(seconds) if (SETTLE_LEAST..=SETTLE_MOST).contains(&seconds) => {
            Ok(Duration::from_secs_f64(seconds))
        }
        _ => Err(CliError::Failed(format!(
            "--settle takes seconds from 0.5 to 3600, such as 2 or 0.5, not {text}"
        ))),
    }
}

/// Runs `diro watch` until Ctrl+C. Returns the exit code.
pub fn run(args: &WatchArgs, out: &mut dyn Write) -> Result<i32, CliError> {
    if args.help {
        writeln!(out, "{}", help_text()).map_err(write_failed)?;
        return Ok(0);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    // Without the handler, Ctrl+C still ends the program, only without
    // the last line.
    let _ = ctrlc::set_handler(move || stopping.store(true, Ordering::SeqCst));
    let here = current_place()?;
    let registry = crate::cmd::archive::archive_registry().0;
    run_with(args, out, &mut std::io::stderr(), &registry, &here, &stop)
}

/// [`run`] with the providers, the current folder, the stream for words
/// beside `--print0`, and the flag Ctrl+C sets given.
pub fn run_with(
    args: &WatchArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    registry: &Registry,
    here: &Place,
    stop: &Arc<AtomicBool>,
) -> Result<i32, CliError> {
    let typed = args.place.as_deref().unwrap_or(".");
    let place = match Place::resolve(typed, here) {
        Ok(place) => place,
        Err(error) => {
            let text = Message::from_error("watch", typed, &error).text;
            say(args, out, err, &text)?;
            return Ok(2);
        }
    };
    let folder = name_of(&place);
    if let Some(problem) = refusal(registry, &place, &folder) {
        say(args, out, err, &problem)?;
        return Ok(1);
    }
    let provider = match registry.resolve(&place) {
        Ok(provider) => provider,
        Err(error) => {
            let text = Message::could_not("watch", &folder, &error.to_string(), "").text;
            say(args, out, err, &text)?;
            return Ok(1);
        }
    };
    let (sender, notes) = mpsc::channel();
    let options = WatchOptions {
        quiet: args.settle,
        pace: None,
    };
    let clock = Arc::new(diroxide_core::SystemClock::utc());
    let sink = Box::new(move |note| {
        let _ = sender.send(note);
    });
    let watch = spawn_watch(provider, place.clone(), options, clock, sink)
        .map_err(|error| CliError::Failed(format!("the watch could not start: {error}")))?;
    let code = report(&notes, args, &place, &folder, out, err, stop);
    drop(watch);
    code
}

/// The folder's name as the lines say it.
fn name_of(place: &Place) -> String {
    place.name().map_or_else(
        || place.to_string(),
        |name| diroxide_core::names::display_name(name).into_owned(),
    )
}

/// Why the place cannot be watched, in words, if it cannot: it is inside
/// an archive, it is not there, or it is not a folder.
fn refusal(registry: &Registry, place: &Place, folder: &str) -> Option<String> {
    if place.kind() == PlaceKind::Archive {
        return Some(
            Message::could_not(
                "watch",
                folder,
                "it is inside an archive, which changes only through Pack",
                "",
            )
            .text,
        );
    }
    let found = registry
        .resolve(place)
        .and_then(|provider| provider.stat(place, &OpContext::new()));
    match found {
        Ok(entry) if entry.kind == EntryKind::Folder => None,
        Ok(_) => Some(Message::could_not("watch", folder, "it is not a folder", "").text),
        Err(error) => Some(Message::could_not("watch", folder, &error.to_string(), "").text),
    }
}

/// Writes the watch's notes as they come, until Ctrl+C or the watch ends
/// by itself; returns the exit code.
fn report(
    notes: &Receiver<WatchNote>,
    args: &WatchArgs,
    place: &Place,
    folder: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
    stop: &AtomicBool,
) -> Result<i32, CliError> {
    let mut arrived = 0;
    loop {
        if stop.load(Ordering::SeqCst) {
            say(args, out, err, &stopped_line(folder, arrived))?;
            return Ok(0);
        }
        let note = match notes.recv_timeout(Duration::from_millis(100)) {
            Ok(note) => note,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                say(args, out, err, &stopped_line(folder, arrived))?;
                return Ok(3);
            }
        };
        match note {
            WatchNote::Started { polled, .. } => {
                say(args, out, err, &start_line(folder, polled, "Ctrl+C stops."))?;
            }
            WatchNote::Arrived(arrival) => {
                arrived += 1;
                if args.print0 {
                    let path = place
                        .join(&arrival.name)
                        .map_or_else(|_| arrival.name.clone(), |p| p.to_string());
                    write!(out, "{path}\0").map_err(write_failed)?;
                    out.flush().map_err(write_failed)?;
                } else {
                    say(
                        args,
                        out,
                        err,
                        &diroxide_ops::tracker::arrived_line(&arrival),
                    )?;
                }
            }
            WatchNote::Skipped { line, .. } | WatchNote::GaveUp { line, .. } => {
                say(args, out, err, &line)?;
            }
            WatchNote::Failed { line } => {
                say(args, out, err, &line)?;
                return Ok(1);
            }
            WatchNote::Ended { line } => {
                say(args, out, err, &line)?;
                return Ok(3);
            }
            _ => {}
        }
    }
}

/// Writes a line of words: to standard output, or to standard error beside
/// `--print0`, so a pipe gets only paths. Each line is flushed at once.
fn say(
    args: &WatchArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    text: &str,
) -> Result<(), CliError> {
    let target: &mut dyn Write = if args.print0 { err } else { out };
    writeln!(target, "{text}").map_err(write_failed)?;
    target.flush().map_err(write_failed)
}

fn write_failed(error: std::io::Error) -> CliError {
    CliError::Failed(format!("the watch could not write: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments_are_read() {
        assert_eq!(parse(&args(&[])).unwrap(), WatchArgs::default());
        let parsed = parse(&args(&["in", "--print0", "--settle", "0.5"])).unwrap();
        assert_eq!(parsed.place.as_deref(), Some("in"));
        assert!(parsed.print0);
        assert_eq!(parsed.settle, Some(Duration::from_millis(500)));
        assert!(parse(&args(&["--null"])).unwrap().print0);
        assert!(parse(&args(&["-0"])).unwrap().print0);
        assert_eq!(
            parse(&args(&["--settle=10"])).unwrap().settle,
            Some(Duration::from_secs(10))
        );
        assert!(parse(&args(&["--help"])).unwrap().help);
        assert_eq!(
            parse(&args(&["--", "-odd"])).unwrap().place.as_deref(),
            Some("-odd")
        );
        assert_eq!(
            parse(&args(&["--bogus"])),
            Err(CliError::UnknownOption("--bogus".to_owned()))
        );
        assert_eq!(
            parse(&args(&["a", "b"])),
            Err(CliError::TooMany("watch".to_owned()))
        );
        assert_eq!(
            parse(&args(&["--settle"])),
            Err(CliError::Missing("--settle".to_owned()))
        );
        for bad in ["0", "0.1", "3601", "soon", "-2"] {
            assert!(
                matches!(parse(&args(&["--settle", bad])), Err(CliError::Failed(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_help_says_what_it_does_first_and_the_exit_codes() {
        let help = help_text();
        assert!(diroxide_core::format::within_cells(
            help.lines().next().unwrap(),
            "diro watch: say each file that settles"
        ));
        assert!(help.contains("--print0, --null, -0"));
        assert!(help.contains("3  the watch ended early"));
        let mut out = Vec::new();
        let code = run(
            &WatchArgs {
                help: true,
                ..WatchArgs::default()
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(String::from_utf8(out).unwrap().starts_with("diro watch:"));
    }

    fn folder() -> (tempfile::TempDir, Place) {
        let dir = tempfile::tempdir().unwrap();
        let place = Place::parse(dir.path().to_str().unwrap()).unwrap();
        (dir, place)
    }

    #[test]
    fn a_folder_that_is_not_there_or_not_a_folder_cannot_be_watched() {
        let (dir, here) = folder();
        std::fs::write(dir.path().join("note.txt"), b"x").unwrap();
        let registry = Registry::with_local();
        let stop = Arc::new(AtomicBool::new(false));
        for (typed, words) in [
            ("nope", "Could not watch nope: "),
            ("note.txt", "Could not watch note.txt: it is not a folder."),
        ] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let watch = WatchArgs {
                place: Some(typed.to_owned()),
                ..WatchArgs::default()
            };
            let code = run_with(&watch, &mut out, &mut err, &registry, &here, &stop).unwrap();
            assert_eq!(code, 1, "{typed}");
            let text = String::from_utf8(out).unwrap();
            assert!(text.starts_with(words), "{text}");
            assert!(err.is_empty());
        }
    }

    #[test]
    fn notes_become_lines_and_exit_codes() {
        let place = Place::parse(if cfg!(windows) { r"D:\in" } else { "/in" }).unwrap();
        let arrival = |name: &str, again| diroxide_ops::tracker::Arrival {
            name: name.to_owned(),
            size: Some(2_100_000),
            again,
            waited: Duration::from_secs(2),
        };
        let cases: [(Vec<WatchNote>, i32, &str); 3] = [
            (
                vec![
                    WatchNote::Started {
                        already: 3,
                        polled: None,
                    },
                    WatchNote::Waiting(Some("Waiting for report.pdf to settle".to_owned())),
                    WatchNote::Arrived(arrival("report.pdf", false)),
                    WatchNote::Say("never printed".to_owned()),
                    WatchNote::Arrived(arrival("report.pdf", true)),
                    WatchNote::Ended {
                        line: "Stopped watching in: the folder is gone".to_owned(),
                    },
                ],
                3,
                "Watching in for new files. Ctrl+C stops.\nArrived: report.pdf, 2.1 MB\nArrived again: report.pdf, 2.1 MB\nStopped watching in: the folder is gone\n",
            ),
            (
                vec![WatchNote::Failed {
                    line: "Could not watch in: it does not exist.".to_owned(),
                }],
                1,
                "Could not watch in: it does not exist.\n",
            ),
            (
                vec![
                    WatchNote::Started {
                        already: 0,
                        polled: Some(Duration::from_secs(5)),
                    },
                    WatchNote::Skipped {
                        name: "a.pdf".to_owned(),
                        reason: diroxide_ops::tracker::SkipReason::OnlineOnly,
                        line: "Skipped a.pdf: online only".to_owned(),
                    },
                ],
                3,
                "Watching in for new files, checking every 5 seconds. Ctrl+C stops.\nSkipped a.pdf: online only\nStopped watching in\n",
            ),
        ];
        for (notes, code, text) in cases {
            let (sender, receiver) = mpsc::channel();
            for note in notes {
                sender.send(note).unwrap();
            }
            drop(sender);
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let stop = AtomicBool::new(false);
            let got = report(
                &receiver,
                &WatchArgs::default(),
                &place,
                "in",
                &mut out,
                &mut err,
                &stop,
            )
            .unwrap();
            assert_eq!(got, code);
            assert_eq!(String::from_utf8(out).unwrap(), text);
            assert!(err.is_empty());
        }
    }

    /// A buffer shared with the watching thread.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    #[test]
    fn print0_writes_each_arrived_path_and_a_nul_and_nothing_else() {
        // Real time, on the local disk: generous bounds, and the test
        // checks what is written, not when.
        let (dir, here) = folder();
        std::fs::create_dir(dir.path().join("in")).unwrap();
        let watched = dir.path().join("in");
        std::fs::write(watched.join("before.txt"), b"there first").unwrap();
        let (out, err) = (Shared::default(), Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (mut out, mut err, stop, here) =
                (out.clone(), err.clone(), Arc::clone(&stop), here.clone());
            std::thread::spawn(move || {
                let args = WatchArgs {
                    place: Some("in".to_owned()),
                    print0: true,
                    settle: Some(Duration::from_millis(500)),
                    help: false,
                };
                run_with(
                    &args,
                    &mut out,
                    &mut err,
                    &Registry::with_local(),
                    &here,
                    &stop,
                )
            })
        };
        let wait_for = |what: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(60);
            while Instant::now() < deadline && !what() {
                std::thread::sleep(Duration::from_millis(50));
            }
            what()
        };
        assert!(wait_for(&|| err
            .text()
            .starts_with("Watching in for new files.")));
        std::fs::write(watched.join("scan-004.pdf.part"), b"unfinished").unwrap();
        std::fs::write(watched.join("scan-004.pdf"), b"%PDF-1.7").unwrap();
        assert!(wait_for(&|| out.text().ends_with('\0')), "{}", err.text());
        stop.store(true, Ordering::SeqCst);
        let code = worker.join().unwrap().unwrap();
        assert_eq!(code, 0);
        let expected = format!(
            "{}\0",
            here.join("in").unwrap().join("scan-004.pdf").unwrap()
        );
        assert_eq!(out.text(), expected);
        assert_eq!(
            err.text(),
            "Watching in for new files. Ctrl+C stops.\nStopped watching in: 1 arrived\n"
        );
    }
}
