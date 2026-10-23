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

/// Copies a folder's tree into `to`, made here, skipping Python's
/// bytecode folders and never following a link; on Linux and macOS the
/// copy is the owner's alone (0700 for folders, 0600 for files), as an
/// installed plugins folder is.
fn copy_private(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for item in fs::read_dir(from).unwrap().flatten() {
        let kind = item.file_type().unwrap();
        let name = item.file_name();
        if kind.is_symlink() || name == "__pycache__" {
            continue;
        }
        let target = to.join(&name);
        if kind.is_dir() {
            copy_private(&item.path(), &target);
        } else if kind.is_file() {
            fs::copy(item.path(), &target).unwrap();
            owner_only(&target, 0o600);
        }
    }
    owner_only(to, 0o700);
}

#[cfg(unix)]
fn owner_only(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn owner_only(_path: &Path, _mode: u32) {}

/// `diro plugin check` over a private copy of the examples, never over
/// the repository's tree in place: there the files may be anyone's to
/// change (a bind mount from Windows shows every file as mode 777), and
/// the script-folder trust check rightly refuses such a script.
#[test]
fn every_example_passes_check() {
    let lookup = Lookup::system();
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("plugins");
    copy_private(&examples(), &copy);
    let all = installed::scan(&copy, Os::current());
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

// Section 3.4: the system-index listers, with a fake index program.

/// The system-index command of this system, its program a fake on the
/// bench's PATH: a shell script elsewhere, a batch file on Windows.
fn index_command(bench: &mut Bench, body: &str) -> Option<ScriptCommand> {
    let text = fs::read_to_string(examples().join("system-index").join("plugin.toml")).unwrap();
    let manifest = Manifest::check_for(&text, Os::current()).manifest.unwrap();
    let wanted = match Os::current() {
        Os::Windows => "everything",
        Os::Linux => "plocate",
        Os::Macos => "spotlight",
    };
    let index = manifest.commands.iter().position(|c| c.id == wanted)?;
    let (command, problems) = ScriptCommand::from_manifest(
        &manifest,
        index,
        &manifest.commands[index],
        &examples().join("system-index"),
    )?;
    assert!(problems.is_empty(), "{problems:?}");
    let tool = &command.needs[0];
    let bin = bench.root.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let fake = if cfg!(windows) {
        bin.join(format!("{tool}.cmd"))
    } else {
        bin.join(tool)
    };
    fs::write(&fake, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::join_paths([&bin]).unwrap();
    bench.env.lookup.path = Some(path.clone());
    bench.env.registry = bench.env.registry.clone().with_path(Some(path));
    Some(command)
}

fn search(bench: &Bench, command: &ScriptCommand, query: &str) -> (RunReport, Recorder) {
    let mut context = bench.context();
    context.input = Some(query.to_owned());
    bench.run(command, &context)
}

/// A fake that prints the places, one a line on Windows, NUL-ended
/// elsewhere, then ends with `code`.
fn printing(places: &[&str], code: i32) -> String {
    if cfg!(windows) {
        let mut text = String::from("@echo off\r\n");
        for place in places {
            text.push_str(&format!("echo {place}\r\n"));
        }
        text.push_str(&format!("exit /b {code}\r\n"));
        text
    } else {
        let mut text = String::from("#!/bin/sh\n");
        for place in places {
            text.push_str(&format!("printf '%s\\0' '{place}'\n"));
        }
        text.push_str(&format!("exit {code}\n"));
        text
    }
}

#[test]
fn a_search_opens_a_lister_of_the_places_found() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let places: &[&str] = if cfg!(windows) {
        &[
            "C:\\Users\\Ada Example\\report.pdf",
            "D:\\old\\report-old.pdf",
        ]
    } else {
        &["/home/ada/report.pdf", "/home/ada/odd name report.txt"]
    };
    let Some(command) = index_command(&mut bench, &printing(places, 0)) else {
        return;
    };
    let (report, _) = search(&bench, &command, "report");
    assert_eq!(report.ending, Ending::Exited(0), "{:?}", report.stderr);
    assert_eq!(report.places, places);
    assert!(report.said.is_empty());
    assert!(!report.capped);
    assert_eq!(
        crate::script::words::results_title(report.places.len(), "report", "Everything"),
        "2 results for report, from Everything"
    );
}

#[test]
fn a_search_stops_at_its_cap_and_says_so() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let body = if cfg!(windows) {
        "@echo off\r\nfor /l %%i in (1,1,6000) do @echo C:\\data\\r%%i.txt\r\n".to_owned()
    } else {
        "#!/bin/sh\ni=0\nwhile [ $i -lt 6000 ]; do printf '/data/r%s\\0' \"$i\"; i=$((i+1)); done\n"
            .to_owned()
    };
    let Some(command) = index_command(&mut bench, &body) else {
        return;
    };
    let (report, _) = search(&bench, &command, "r");
    assert!(report.capped);
    assert_eq!(report.places.len(), 5000);
    assert_eq!(
        crate::script::words::first_shown(command.recipe.max_rows),
        "First 5,000 shown, narrow the search."
    );
}

#[test]
fn a_search_says_its_exit_words_and_its_missing_tool() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let Some(command) = index_command(&mut bench, &printing(&[], 8)) else {
        return;
    };
    let (report, _) = search(&bench, &command, "report");
    if cfg!(windows) {
        assert_eq!(
            report.said[0].text,
            "Everything is not running. Start it, then try again. By Search the whole computer."
        );
    } else {
        assert_eq!(
            report.said[0].text,
            format!("Could not run {}: code 8.", command.title)
        );
    }
    let quiet = index_command(&mut bench, &printing(&[], 0)).unwrap();
    let (report, _) = search(&bench, &quiet, "nothing");
    assert!(report.places.is_empty());
    assert_eq!(
        crate::script::words::no_results("nothing"),
        "No results for nothing."
    );
    bench.env.lookup.path = None;
    bench.env.registry = bench.env.registry.clone().with_path(None);
    let mut sink = Recorder::default();
    let mut context = bench.context();
    context.input = Some("report".to_owned());
    let error = run(&quiet, &context, &bench.env, &mut sink, &Cancel::new()).unwrap_err();
    assert_eq!(
        error.sentence(&quiet.title),
        format!("Needs {}, not found.", quiet.needs[0])
    );
    // An empty answer to the prompt cancels.
    let (report, _) = search(&bench, &quiet, "");
    assert_eq!(report.ending, Ending::Cancelled);
}

#[test]
fn a_real_index_when_asked() {
    if std::env::var_os("DIRO_TOOLS").is_none() {
        eprintln!("set DIRO_TOOLS=1 to search with this system's real index");
        return;
    }
    let Some(bench) = Bench::new() else { return };
    let text = fs::read_to_string(examples().join("system-index").join("plugin.toml")).unwrap();
    let manifest = Manifest::check_for(&text, Os::current()).manifest.unwrap();
    let Some((index, spec)) = manifest
        .commands
        .iter()
        .enumerate()
        .find(|(_, c)| c.os.contains(&Os::current()))
    else {
        return;
    };
    let (command, _) =
        ScriptCommand::from_manifest(&manifest, index, spec, &examples().join("system-index"))
            .unwrap();
    let (report, _) = search(&bench, &command, "diroxide");
    assert!(
        matches!(report.ending, Ending::Exited(0)),
        "{:?}",
        report.said
    );
}

// Section 3.5: two files to the diff tool, started on its own.

/// The compare command named `id`, its tool a recorder: the system's
/// bash writing its arguments to a file, then lingering past the run.
fn compare(bench: &mut Bench, id: &str) -> (ScriptCommand, PathBuf) {
    let text = fs::read_to_string(examples().join("compare-files").join("plugin.toml")).unwrap();
    let manifest = Manifest::check_for(&text, Os::current()).manifest.unwrap();
    let index = manifest.commands.iter().position(|c| c.id == id).unwrap();
    let (command, problems) = ScriptCommand::from_manifest(
        &manifest,
        index,
        &manifest.commands[index],
        &examples().join("compare-files"),
    )
    .unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let record = bench.root.path().join("work").join("arguments.txt");
    let recorder = bench.root.path().join("scripts").join("recorder.sh");
    fs::write(
        &recorder,
        format!(
            "printf '%s\\n' \"$@\" > '{}'\nsleep 3\n",
            record.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let bash = bench.env.lookup.bash().unwrap();
    let mut tool = vec![json!(bash.program.to_string_lossy())];
    tool.extend(bash.args.iter().map(|a| json!(a.to_string_lossy())));
    tool.push(json!(recorder.to_string_lossy()));
    bench.env.settings = json!({ "tool": tool }).as_object().unwrap().clone();
    (command, record)
}

fn recorded(record: &Path) -> Vec<String> {
    for _ in 0..200 {
        if let Ok(text) = fs::read_to_string(record)
            && text.ends_with('\n')
        {
            return text.lines().map(str::to_owned).collect();
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the diff tool never ran");
}

#[test]
fn compare_panels_launches_the_tool_detached_with_both_files() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let (command, record) = compare(&mut bench, "compare_panels");
    let mut context = bench.context();
    let left = missing(&bench, "say \"hi\" left.txt").unwrap();
    let right = missing(&bench, "-right side.txt").unwrap();
    context.cursor = Some(left.clone());
    context.other_cursor = Some(right.clone());
    let started = std::time::Instant::now();
    let (report, _) = bench.run(&command, &context);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "it waited for the tool"
    );
    assert_eq!(report.said[0].text, "Started bash.");
    let arguments = recorded(&record);
    assert_eq!(arguments, [left.place.to_string(), right.place.to_string()]);
}

#[test]
fn compare_says_what_is_missing() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let (command, _) = compare(&mut bench, "compare_panels");
    let mut sink = Recorder::default();
    let error = run(
        &command,
        &bench.context(),
        &bench.env,
        &mut sink,
        &Cancel::new(),
    )
    .unwrap_err();
    assert_eq!(
        error.sentence(&command.title),
        "Both panels need a file under the cursor."
    );
    bench.env.settings = json!({ "tool": [] }).as_object().unwrap().clone();
    let error = run(
        &command,
        &bench.context(),
        &bench.env,
        &mut sink,
        &Cancel::new(),
    )
    .unwrap_err();
    assert_eq!(
        error.sentence(&command.title),
        "Choose tool in the plugin's settings first."
    );
}

#[test]
fn compare_marked_takes_exactly_two() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let (command, record) = compare(&mut bench, "compare_marked");
    let two = vec![
        work_file(&bench, "one.txt", 1),
        work_file(&bench, "two.txt", 1),
    ];
    let (report, _) = run_with(&bench, &command, two.clone());
    assert_eq!(report.ending, Ending::Exited(0));
    assert_eq!(recorded(&record).len(), 2);
    let mut three = two;
    three.push(work_file(&bench, "three.txt", 1));
    let mut context = bench.context();
    context.marked = three;
    let mut sink = Recorder::default();
    let error = run(&command, &context, &bench.env, &mut sink, &Cancel::new()).unwrap_err();
    assert_eq!(
        error.sentence(&command.title),
        "Compare the two marked files needs two items; 3 are selected."
    );
}

// Sections 3.6 and 3.7: the lister and the toy provider through the real
// host. The host serves the plugin's own requests from W4a-2's commits
// 10 to 14; until then the provider methods, which go from the host to
// the plugin, run now, and the lister's run waits.

fn plugin_host(id: &str, state: &Path) -> Host {
    let folder = examples().join(id);
    let text = fs::read_to_string(folder.join("plugin.toml")).unwrap();
    let manifest = Manifest::from_toml(&text).unwrap();
    let mut host = Host::new(&PluginSettings::default()).with_state_dir(state);
    host.add(PluginInfo {
        manifest,
        folder,
        enabled: true,
    });
    host
}

fn b64(bytes: &[u8]) -> String {
    crate::base64::encode(bytes)
}

fn from_b64(text: &str) -> Vec<u8> {
    crate::base64::decode(text).unwrap()
}

fn provider_error(host: &Host, method: &str, params: serde_json::Value) -> i64 {
    match host.call("memfs", method, params) {
        Err(crate::PluginError::Remote { code, .. }) => code,
        other => panic!("{method}: {other:?}"),
    }
}

/// Writes a file through a write session in two chunks, committed.
fn memfs_write(host: &Host, place: &str, data: &[u8]) {
    let session = host
        .call(
            "memfs",
            "provider/write_begin",
            json!({ "place": place, "conflict": "fail" }),
        )
        .unwrap();
    let id = session["write_id"].as_str().unwrap().to_owned();
    let half = data.len() / 2;
    for (offset, part) in [(0, &data[..half]), (half, &data[half..])] {
        host.call(
            "memfs",
            "provider/write_chunk",
            json!({ "write_id": id, "offset": offset, "data": b64(part) }),
        )
        .unwrap();
    }
    host.call(
        "memfs",
        "provider/write_end",
        json!({ "write_id": id, "commit": true }),
    )
    .unwrap();
}

#[test]
fn memfs_keeps_the_provider_contract_through_the_host() {
    if !python_present() {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let host = plugin_host("memfs", &scratch.path().join("state"));
    let connected = host
        .call("memfs", "provider/connect", json!({ "place": "memfs:/" }))
        .unwrap();
    assert_eq!(connected["display_name"], "Memory files (toy)");
    let empty = host
        .call("memfs", "provider/list", json!({ "place": "memfs:/" }))
        .unwrap();
    assert_eq!(empty["entries"], json!([]));
    host.call(
        "memfs",
        "provider/create_dir",
        json!({ "place": "memfs:/docs", "parents": false }),
    )
    .unwrap();
    memfs_write(&host, "memfs:/docs/a.txt", b"0123456789");
    let range = host
        .call(
            "memfs",
            "provider/read",
            json!({ "place": "memfs:/docs/a.txt", "offset": 3, "length": 4 }),
        )
        .unwrap();
    assert_eq!(from_b64(range["data"].as_str().unwrap()), b"3456");
    assert_eq!(range["eof"], false);
    let session = host
        .call(
            "memfs",
            "provider/write_begin",
            json!({ "place": "memfs:/docs/gone.txt", "conflict": "fail" }),
        )
        .unwrap();
    host.call(
        "memfs",
        "provider/write_end",
        json!({ "write_id": session["write_id"], "commit": false }),
    )
    .unwrap();
    assert_eq!(
        provider_error(
            &host,
            "provider/stat",
            json!({ "place": "memfs:/docs/gone.txt" })
        ),
        -32010
    );
    memfs_write(&host, "memfs:/taken.txt", b"t");
    assert_eq!(
        provider_error(
            &host,
            "provider/rename",
            json!({ "from": "memfs:/docs", "to": "memfs:/taken.txt" })
        ),
        -32011
    );
    assert_eq!(
        provider_error(&host, "provider/remove", json!({ "place": "memfs:/docs" })),
        -32015
    );
    assert_eq!(
        provider_error(
            &host,
            "provider/create_dir",
            json!({ "place": "memfs:/../up", "parents": false })
        ),
        -32016
    );
    assert_eq!(
        provider_error(
            &host,
            "provider/trash",
            json!({ "place": "memfs:/taken.txt" })
        ),
        -32017
    );
    host.shutdown_all(QUIT_BUDGET);
    assert!(!host.is_running("memfs"));
}

/// A local folder copied into memfs:/ and back, file by file, the way
/// the job engine will through `PluginProvider`, with equal bytes.
#[test]
fn memfs_copies_a_folder_both_ways_with_equal_bytes() {
    if !python_present() {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("source");
    fs::create_dir_all(source.join("inner")).unwrap();
    let files: Vec<(String, Vec<u8>)> = vec![
        ("one.txt".into(), b"first file".to_vec()),
        (
            "inner/two.bin".into(),
            (0..=255u8).cycle().take(70_000).collect(),
        ),
        ("empty.txt".into(), Vec::new()),
    ];
    for (name, data) in &files {
        fs::write(source.join(name), data).unwrap();
    }
    let host = plugin_host("memfs", &scratch.path().join("state"));
    host.call(
        "memfs",
        "provider/create_dir",
        json!({ "place": "memfs:/copy/inner", "parents": true }),
    )
    .unwrap();
    for (name, data) in &files {
        memfs_write(&host, &format!("memfs:/copy/{name}"), data);
    }
    let back = scratch.path().join("back");
    for (name, data) in &files {
        let mut read = Vec::new();
        loop {
            let part = host
                .call(
                    "memfs",
                    "provider/read",
                    json!({ "place": format!("memfs:/copy/{name}"), "offset": read.len(), "length": 32_768 }),
                )
                .unwrap();
            read.extend(from_b64(part["data"].as_str().unwrap()));
            if part["eof"] == true {
                break;
            }
        }
        let target = back.join(name);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, &read).unwrap();
        assert_eq!(&fs::read(&target).unwrap(), data, "{name}");
    }
    let listed = host
        .call("memfs", "provider/list", json!({ "place": "memfs:/copy" }))
        .unwrap();
    let names: Vec<&str> = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["empty.txt", "inner", "one.txt"]);
    host.shutdown_all(QUIT_BUDGET);
}

/// What a lister plugin's host needs of the app: the active panel, and
/// a record of the lister and documents it is shown.
#[derive(Default)]
struct RepositoryPanel {
    place: Option<Place>,
    listers: std::sync::Mutex<Vec<crate::listers::ListerView>>,
    documents: std::sync::Mutex<Vec<crate::TextDocument>>,
    said: std::sync::Mutex<Vec<String>>,
}

impl crate::host::HostServices for RepositoryPanel {
    fn announce(&self, message: diroxide_core::Message) {
        self.said.lock().unwrap().push(message.text);
    }

    fn ask(&self, _question: &crate::Question) -> crate::Answer {
        crate::Answer::Cancelled
    }

    fn panel(&self, which: crate::PanelChoice) -> Option<crate::PanelSnapshot> {
        let side = match which {
            crate::PanelChoice::Right => diroxide_core::Side::Right,
            _ => diroxide_core::Side::Left,
        };
        self.place
            .clone()
            .map(|place| crate::PanelSnapshot::at(side, place))
    }

    fn open_lister(&self, lister: &crate::listers::ListerView) -> Result<(), crate::RpcError> {
        self.listers.lock().unwrap().push(lister.clone());
        Ok(())
    }

    fn show_text(&self, document: &crate::TextDocument) -> Result<(), crate::RpcError> {
        self.documents.lock().unwrap().push(document.clone());
        Ok(())
    }
}

#[test]
fn changed_files_shows_a_lister_through_the_host() {
    if !python_present() {
        return;
    }
    let Some(git) = Lookup::system().which_all("git").into_iter().next() else {
        eprintln!("git was not found; this test is skipped here");
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let repository = scratch.path().join("repository");
    fs::create_dir_all(&repository).unwrap();
    let git_in = |args: &[&str]| {
        Command::new(&git)
            .args(args)
            .current_dir(&repository)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    };
    assert!(git_in(&["init", "--quiet"]).status.success());
    fs::write(repository.join("notes.txt"), b"new").unwrap();
    let folder = examples().join("changed-files");
    let text = fs::read_to_string(folder.join("plugin.toml")).unwrap();
    let manifest = Manifest::from_toml(&text).unwrap();
    let services = std::sync::Arc::new(RepositoryPanel {
        place: Some(Place::parse(&repository.to_string_lossy()).unwrap()),
        ..RepositoryPanel::default()
    });
    let mut host = Host::new(&PluginSettings::default())
        .with_state_dir(scratch.path().join("state"))
        .with_services(services.clone());
    host.add(PluginInfo {
        manifest,
        folder,
        enabled: true,
    });
    let result = host
        .call(
            "changed-files",
            methods::COMMAND_RUN,
            json!({ "command": "show", "context": {} }),
        )
        .unwrap();
    assert_eq!(result["done"], 1);
    let listers = services.listers.lock().unwrap().clone();
    assert_eq!(listers.len(), 1, "{:?}", services.said.lock().unwrap());
    let lister = &listers[0];
    assert!(
        lister.title.starts_with("Changed files: 1, 1 untracked"),
        "{}",
        lister.title
    );
    assert_eq!(lister.rows[0].name, "Untracked, notes.txt");
    assert_eq!(lister.rows[0].extra["id"], "notes.txt");
    assert_eq!(lister.activate(), Some("diff"));
    // Enter on the row: its diff, here none for an untracked file.
    let context = json!({ "selection": [
        { "name": "Untracked, notes.txt", "kind": "item",
          "extra": { "id": "notes.txt", "state": "Untracked", "path": "notes.txt" } }
    ] });
    let result = host
        .call(
            "changed-files",
            methods::COMMAND_RUN,
            json!({ "command": "diff", "context": context }),
        )
        .unwrap();
    assert_eq!(result["done"], 1);
    assert_eq!(
        *services.said.lock().unwrap(),
        ["No changes to show for notes.txt. By Changed files."]
    );
    host.shutdown_all(QUIT_BUDGET);
}

// Every Python example through W4a-2's conformance suite (R22 section
// 8.4: each shipped example is a conformance subject).

fn conformance_of(name: &str) {
    if !python_present() {
        return;
    }
    let report = crate::conformance::run(&examples().join(name));
    assert!(report.passed(), "{name}:\n{}", report.text());
}

#[test]
fn the_line_counter_passes_the_conformance_suite() {
    conformance_of("line-counter");
}

#[test]
fn the_changed_files_lister_passes_the_conformance_suite() {
    conformance_of("changed-files");
}

#[test]
fn the_toy_provider_passes_the_conformance_suite() {
    conformance_of("memfs");
}
