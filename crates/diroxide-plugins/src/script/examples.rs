//! The shipped examples under `examples/plugins/` as test subjects (R22
//! sections 3 and 8.4): each passes `diro plugin check`, the scripts run
//! through the real runner with the cases of sections 3.1 and 3.2, the
//! Python examples pass their own tests, and the line counter answers
//! through the real host. Interpreters are the system's; a test whose
//! interpreter is missing says so and passes. Hostile names live in
//! memory and in the NUL files, never on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use diroxide_config::PluginSettings;
use diroxide_core::{CommandId, EntryKind, Place};
use serde_json::json;

use crate::host::{Host, HostEvent, PluginInfo, QUIT_BUDGET};
use crate::manifest::{Manifest, Os, Severity};
use crate::protocol::methods;
use crate::script::command::ScriptCommand;
use crate::script::context::{Picked, list_bytes};
use crate::script::directives::Directive;
use crate::script::header::parse_header;
use crate::script::installed::{self, Installed};
use crate::script::interpreter::Lookup;
use crate::script::recipe::ListForm;
use crate::script::runner::tests::Bench;
use crate::script::runner::{Cancel, Recorder, RunReport, run};
use crate::script::words::Ending;
use crate::shapes::{PanelRef, WireContext, WireEntry};

pub(crate) fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins")
}

fn script(name: &str) -> ScriptCommand {
    let path = examples().join("scripts").join(name);
    let text = fs::read_to_string(&path).unwrap();
    ScriptCommand::from_script(&path, &parse_header(&text).header)
}

/// What a run said at once, in order, as the owner hears it.
fn said(command: &ScriptCommand, sink: &Recorder) -> Vec<String> {
    sink.acted
        .iter()
        .filter_map(|(_, d)| d.message(&command.source))
        .map(|m| m.text)
        .collect()
}

/// A size as the total-size script writes it: a whole number, or one
/// decimal, then a unit.
fn is_size_words(text: &str) -> bool {
    let Some((number, unit)) = text.split_once(' ') else {
        return false;
    };
    ["bytes", "KB", "MB", "GB"].contains(&unit)
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[test]
fn every_example_passes_check() {
    let lookup = Lookup::system();
    let all = installed::scan(&examples(), Os::current());
    let ids: Vec<&str> = all.iter().map(Installed::id).collect();
    for wanted in [
        "changed-files",
        "line-counter",
        "memfs",
        "select-empty-folders",
        "total-size",
    ] {
        assert!(ids.contains(&wanted), "{wanted} in {ids:?}");
    }
    for item in &all {
        let problems = installed::check(item, &lookup);
        let serious: Vec<String> = problems
            .iter()
            .filter(|p| p.severity == Severity::Error || p.field != "file")
            .map(ToString::to_string)
            .collect();
        assert!(serious.is_empty(), "{}: {serious:?}", item.id());
    }
    for os in [Os::Windows, Os::Linux, Os::Macos] {
        for item in installed::scan(&examples(), os) {
            if let Installed::Plugin(plugin) = item {
                assert!(
                    plugin.report.manifest.is_some(),
                    "{} on {os:?}: {}",
                    plugin.id,
                    plugin.report.text()
                );
            }
        }
    }
}

#[test]
fn every_line_an_example_names_fits_one_braille_line() {
    for item in installed::scan(&examples(), Os::current()) {
        assert!(item.name().chars().count() <= 40, "{}", item.name());
        if let Some(manifest) = item.manifest() {
            for command in &manifest.commands {
                assert!(command.name.chars().count() <= 40, "{}", command.name);
            }
        }
    }
}

// Section 3.1: total size of the selection.

fn work_file(bench: &Bench, name: &str, bytes: usize) -> Picked {
    let path = bench.root.path().join("work").join(name);
    fs::write(&path, vec![b'x'; bytes]).unwrap();
    Picked::new(
        Place::parse(&path.to_string_lossy()).unwrap(),
        EntryKind::File,
    )
}

fn missing(bench: &Bench, name: &str) -> Option<Picked> {
    let path = bench.root.path().join("work").join(name);
    Place::parse(&path.to_string_lossy())
        .ok()
        .map(|place| Picked::new(place, EntryKind::File))
}

fn run_with(bench: &Bench, command: &ScriptCommand, marked: Vec<Picked>) -> (RunReport, Recorder) {
    let mut context = bench.context();
    context.marked = marked;
    bench.run(command, &context)
}

#[test]
fn total_size_counts_one_and_three_files() {
    let Some(bench) = Bench::new() else { return };
    let command = script("total-size.sh");
    let (report, sink) = run_with(&bench, &command, vec![work_file(&bench, "one.txt", 5000)]);
    assert_eq!(report.ending, Ending::Exited(0), "{:?}", report.stderr);
    let words = said(&command, &sink);
    let size = words[0]
        .strip_prefix("Selected: 1, size: ")
        .and_then(|rest| rest.strip_suffix(". By Total size of selection."))
        .unwrap_or_else(|| panic!("{words:?}"));
    assert!(is_size_words(size), "{size}");
    let three = vec![
        work_file(&bench, "a.txt", 10),
        work_file(&bench, "b.txt", 2000),
        work_file(&bench, "-starts-with-dash.txt", 30),
    ];
    let (report, sink) = run_with(&bench, &command, three);
    assert_eq!(report.ending, Ending::Exited(0), "{:?}", report.stderr);
    assert!(said(&command, &sink)[0].starts_with("Selected: 3, size: "));
    assert!(diroxide_core::format::within_cells(
        &said(&command, &sink)[0],
        "Selected: 3, size:"
    ));
}

#[test]
fn total_size_reads_odd_names_whole_and_counts_what_it_cannot_read() {
    let Some(bench) = Bench::new() else { return };
    let command = script("total-size.sh");
    // Never created on disk: each must still arrive as one name.
    let mut marked: Vec<Picked> = ["with space.txt", "say \"hi\".txt", "-dash-gone.txt"]
        .iter()
        .filter_map(|n| missing(&bench, n))
        .collect();
    marked.push(work_file(&bench, "real.txt", 100));
    if let Some(odd) = missing(&bench, "line\nbreak.txt")
        && odd.place.to_string().contains('\n')
    {
        marked.push(odd);
    }
    let names = marked.len();
    let (report, sink) = run_with(&bench, &command, marked);
    assert_eq!(report.ending, Ending::Exited(3));
    let words = said(&command, &sink);
    assert!(
        words[0].starts_with(&format!("Selected: {names}, size: ")),
        "{words:?}"
    );
    assert_eq!(
        words[1],
        format!(
            "Skipped {} items it could not read. By Total size of selection.",
            names - 1
        )
    );
    assert_eq!(
        report
            .said
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        ["Total size of selection partly done: code 3."]
    );
}

#[test]
fn total_size_reports_progress_on_a_long_list() {
    let Some(bench) = Bench::new() else { return };
    let command = script("total-size.sh");
    let count = if cfg!(windows) { 120 } else { 3000 };
    let marked: Vec<Picked> = (0..count)
        .filter_map(|n| missing(&bench, &format!("gone-{n:04}.txt")))
        .collect();
    let (report, sink) = run_with(&bench, &command, marked);
    assert_eq!(report.ending, Ending::Exited(3));
    let progress = sink
        .acted
        .iter()
        .filter(|(_, d)| matches!(d, Directive::Progress { .. }))
        .count();
    assert_eq!(progress, count / 50);
    let selected = format!("Selected: {count}, size: 0 bytes. By Total size of selection.");
    assert!(said(&command, &sink).contains(&selected));
}

#[cfg(unix)]
#[test]
fn total_size_skips_a_folder_it_cannot_read() {
    use std::os::unix::fs::PermissionsExt;
    let Some(bench) = Bench::new() else { return };
    let command = script("total-size.sh");
    let locked = bench.root.path().join("work").join("locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&locked).is_ok();
    let picked = Picked::new(
        Place::parse(&locked.to_string_lossy()).unwrap(),
        EntryKind::Folder,
    );
    let (report, _) = run_with(&bench, &command, vec![picked]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        eprintln!("running with rights that read any folder; the check is skipped");
        return;
    }
    assert_eq!(report.ending, Ending::Exited(3));
}

#[test]
fn the_nul_list_of_the_odd_names_is_byte_exact() {
    let names: Vec<&[u8]> = vec![b"with space", b"say \"hi\"", b"-dash", b"line\nbreak"];
    let bytes = list_bytes(&names, ListForm::Nul).unwrap();
    assert_eq!(bytes, b"with space\0say \"hi\"\0-dash\0line\nbreak\0");
}

// Section 3.2: select the empty folders.

#[test]
fn select_empty_folders_marks_them_when_it_ends() {
    let Some(bench) = Bench::new() else { return };
    let command = script("select-empty-folders.sh");
    let work = bench.root.path().join("work");
    for name in ["e1", "e2", "e3", "e4", ".hidden"] {
        fs::create_dir_all(work.join(name)).unwrap();
    }
    fs::create_dir_all(work.join("outer").join("inner")).unwrap();
    fs::create_dir_all(work.join("full")).unwrap();
    fs::write(work.join("full").join("x.txt"), b"x").unwrap();
    let (report, sink) = run_with(&bench, &command, Vec::new());
    assert_eq!(report.ending, Ending::Exited(0), "{:?}", report.stderr);
    let mut marked: Vec<String> = report
        .deferred
        .iter()
        .map(|d| match d {
            Directive::Select(place) => Path::new(place)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            other => panic!("{other:?}"),
        })
        .collect();
    marked.sort();
    assert_eq!(marked, [".hidden", "e1", "e2", "e3", "e4"]);
    assert_eq!(
        said(&command, &sink),
        ["5 empty folders selected. By Select empty folders."]
    );
    let none = Bench::new().unwrap();
    fs::write(none.root.path().join("work").join("only.txt"), b"x").unwrap();
    let (report, sink) = run_with(&none, &command, Vec::new());
    assert!(report.deferred.is_empty());
    assert_eq!(
        said(&command, &sink),
        ["No empty folders here. By Select empty folders."]
    );
}

// Section 3.3: the line counter, through the real host.

fn python_present() -> bool {
    let found = Lookup::system().python(&[]).is_ok();
    if !found {
        eprintln!("Python 3 was not found; this test is skipped here");
    }
    found
}

fn line_counter_host(folder: &Path, state: &Path) -> Host {
    let text = fs::read_to_string(folder.join("plugin.toml")).unwrap();
    let manifest = Manifest::from_toml(&text).unwrap();
    let mut host = Host::new(&PluginSettings::default()).with_state_dir(state);
    host.add(PluginInfo {
        manifest,
        folder: folder.to_path_buf(),
        enabled: true,
    });
    host
}

fn count_params(places: &[PathBuf]) -> serde_json::Value {
    let id = CommandId::new("line-counter.count_lines").unwrap();
    let here = Place::parse(&places[0].parent().unwrap().to_string_lossy()).unwrap();
    let entries: Vec<WireEntry> = places
        .iter()
        .map(|p| {
            let place = Place::parse(&p.to_string_lossy()).unwrap();
            let entry =
                diroxide_core::Entry::new(place.name().unwrap_or_default(), EntryKind::File);
            WireEntry::from_entry(&entry, Some(&place))
        })
        .collect();
    let context = WireContext::new(&id, PanelRef::for_place(&here), PanelRef::for_place(&here))
        .with_selection(None, entries);
    json!({ "command": "count_lines", "context": context })
}

fn said_by(host: &Host) -> Vec<String> {
    host.take_events()
        .into_iter()
        .filter_map(|e| match e {
            HostEvent::Said { message, .. } => Some(message.text),
            _ => None,
        })
        .collect()
}

#[test]
fn the_line_counter_answers_through_the_host() {
    if !python_present() {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let file = scratch.path().join("a.txt");
    fs::write(&file, "one\ntwo\r\nthree\n\nfive").unwrap();
    let host = line_counter_host(
        &examples().join("line-counter"),
        &scratch.path().join("state"),
    );
    let result = host
        .call("line-counter", methods::COMMAND_RUN, count_params(&[file]))
        .unwrap();
    assert_eq!(result["done"], 1);
    assert_eq!(said_by(&host), ["Lines: 5, files: 1. By Line counter."]);
    host.shutdown_all(QUIT_BUDGET);
    assert!(!host.is_running("line-counter"));
}

#[test]
fn a_stray_print_is_tolerated_and_said_once() {
    if !python_present() {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("line-counter");
    fs::create_dir_all(&copy).unwrap();
    fs::copy(
        examples().join("line-counter").join("plugin.toml"),
        copy.join("plugin.toml"),
    )
    .unwrap();
    let main = fs::read_to_string(examples().join("line-counter").join("main.py")).unwrap();
    let noisy = main.replace(
        "def count_lines(request_id, params):\n",
        "def count_lines(request_id, params):\n    print(\"a stray line\", flush=True)\n",
    );
    assert_ne!(noisy, main);
    fs::write(copy.join("main.py"), noisy).unwrap();
    let file = scratch.path().join("b.txt");
    fs::write(&file, "x\n").unwrap();
    let host = line_counter_host(&copy, &scratch.path().join("state"));
    for _ in 0..2 {
        host.call(
            "line-counter",
            methods::COMMAND_RUN,
            count_params(std::slice::from_ref(&file)),
        )
        .unwrap();
    }
    let words = said_by(&host);
    let stray = words
        .iter()
        .filter(|w| w.as_str() == "Line counter wrote stray text; see its log.")
        .count();
    assert_eq!(stray, 1, "{words:?}");
    assert_eq!(
        words
            .iter()
            .filter(|w| w.starts_with("Lines: 1, files: 1"))
            .count(),
        2
    );
    host.shutdown_all(QUIT_BUDGET);
}

#[test]
fn the_python_examples_pass_their_own_tests() {
    let Ok(python) = Lookup::system().python(&[]) else {
        eprintln!("Python 3 was not found; this test is skipped here");
        return;
    };
    for name in ["line-counter", "changed-files", "memfs"] {
        let folder = examples().join(name);
        let output = Command::new(&python.program)
            .args(python.args.iter().filter(|a| *a != "-P"))
            .args(["-I", "-B", "-m", "unittest", "discover", "-s"])
            .arg(&folder)
            .arg("-t")
            .arg(&folder)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn the_bash_examples_pass_bash_n() {
    let Ok(bash) = Lookup::system().bash() else {
        eprintln!("bash was not found; this test is skipped here");
        return;
    };
    for name in ["total-size.sh", "select-empty-folders.sh"] {
        let (program, mut args) = bash.command_for(&examples().join("scripts").join(name));
        args.insert(args.len() - 1, "-n".into());
        let status = Command::new(program).args(args).status().unwrap();
        assert!(status.success(), "{name}");
    }
}

/// The runner's own cancel reaches an example script too.
#[test]
fn an_example_script_is_cancelled_in_words() {
    let Some(bench) = Bench::new() else { return };
    let command = script("total-size.sh");
    let cancel = Cancel::new();
    cancel.cancel();
    let mut sink = Recorder::default();
    let report = run(&command, &bench.context(), &bench.env, &mut sink, &cancel).unwrap();
    assert_eq!(report.ending, Ending::Cancelled);
}
