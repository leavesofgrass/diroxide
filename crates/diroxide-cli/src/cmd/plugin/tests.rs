//! Tests of `diro plugin`, in folders under a temporary folder, with a
//! scripted asker; scripts run through the system's bash and are skipped,
//! with a line saying so, where there is none.

use std::io::Cursor;
use std::sync::Mutex;

use diroxide_core::Secret;

use super::*;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Answers questions from a list, and keeps what was asked.
pub(super) struct Script {
    answers: Mutex<Vec<bool>>,
    pub(super) asked: Mutex<Vec<String>>,
}

impl Script {
    pub(super) fn new(answers: &[bool]) -> Self {
        Self {
            answers: Mutex::new(answers.iter().rev().copied().collect()),
            asked: Mutex::new(Vec::new()),
        }
    }
}

impl Asker for Script {
    fn present(&self) -> bool {
        true
    }
    fn confirm(&self, question: &str) -> bool {
        self.asked.lock().unwrap().push(question.to_owned());
        self.answers.lock().unwrap().pop().unwrap_or(false)
    }
    fn secret(&self, _prompt: &str) -> Option<Secret> {
        None
    }
    fn tell(&self, _line: &str) {}
}

/// A config folder, a state folder and a work folder.
pub(super) struct Folders {
    pub(super) root: tempfile::TempDir,
}

impl Folders {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for folder in ["config/plugins/scripts", "state", "work"] {
            fs::create_dir_all(root.path().join(folder)).unwrap();
        }
        Self { root }
    }

    pub(super) fn paths(&self) -> Paths {
        Paths::new(
            self.root.path().join("config"),
            self.root.path().join("state"),
        )
    }

    pub(super) fn work(&self) -> PathBuf {
        self.root.path().join("work")
    }

    pub(super) fn scripts(&self) -> PathBuf {
        self.root.path().join("config/plugins/scripts")
    }

    /// Runs `diro plugin ARGS` here with these answers; the exit code,
    /// standard output and the questions asked.
    pub(super) fn run(
        &self,
        words: &[&str],
        answers: &[bool],
        input: &str,
    ) -> (i32, String, Vec<String>) {
        let asker = Script::new(answers);
        let mut input = Cursor::new(input.as_bytes().to_vec());
        let mut err = Vec::new();
        let mut out = Vec::new();
        let env = Env {
            paths: self.paths(),
            asker: &asker,
            input: &mut input,
            err: &mut err,
            lookup: Lookup::system(),
            registry: ToolRegistry::builtin(),
            here: Place::parse(&self.work().to_string_lossy()).unwrap(),
            hash: sha256_hex,
        };
        let parsed = parse(&args(words)).unwrap();
        let code = match run_in(&parsed, env, &mut out) {
            Ok(code) => code,
            Err(CliError::Failed(sentence)) => {
                out.extend_from_slice(sentence.as_bytes());
                out.push(b'\n');
                1
            }
            Err(other) => panic!("{other:?}"),
        };
        let asked = asker.asked.lock().unwrap().clone();
        (code, String::from_utf8(out).unwrap(), asked)
    }
}

pub(super) fn has_bash() -> bool {
    let found = Lookup::system().bash().is_ok();
    if !found {
        eprintln!("bash was not found; this test is skipped here");
    }
    found
}

const TOTAL: &str = "#!/usr/bin/env bash\n# diro: name = \"Count marked\"\n# diro: selection_file = \"nul\"\ncount=0\nwhile IFS= read -r -d '' name; do count=$((count + 1)); done < \"${DIRO_SELECTION_FILE0:?}\"\nprintf 'announce Selected: %s\\n' \"$count\" >> \"${DIRO_DIRECTIVES:?}\"\n";

#[test]
fn subcommands_are_read() {
    let parsed = parse(&args(&[
        "run",
        "total-size",
        "--yes",
        "--",
        "a b.txt",
        "--odd",
    ]))
    .unwrap();
    assert_eq!(
        parsed.action,
        PluginAction::Run {
            name: "total-size".into(),
            command: None,
            paths: vec!["a b.txt".into(), "--odd".into()],
        }
    );
    assert!(parsed.yes);
    assert_eq!(parse(&args(&[])).unwrap().action, PluginAction::Help);
    assert_eq!(
        parse(&args(&["check"])).unwrap().action,
        PluginAction::Check(None)
    );
    assert_eq!(
        parse(&args(&["enable"])),
        Err(CliError::Missing("plugin enable".into()))
    );
    assert_eq!(
        parse(&args(&["list", "--bogus"])),
        Err(CliError::UnknownOption("--bogus".into()))
    );
    assert!(matches!(parse(&args(&["dance"])), Err(CliError::Failed(_))));
    assert!(matches!(
        parse(&args(&["list", "--explain"])),
        Err(CliError::Failed(_))
    ));
    assert_eq!(
        parse(&args(&["log", "a", "b"])),
        Err(CliError::TooMany("plugin b".into()))
    );
}

#[test]
fn a_script_is_enabled_only_after_a_yes_and_asks_again_after_a_change() {
    let folders = Folders::new();
    let script = folders.scripts().join("count-marked.sh");
    fs::write(&script, TOTAL).unwrap();
    let (code, out, _) = folders.run(&["list"], &[], "");
    assert_eq!(code, 0);
    assert_eq!(out, "Count marked: not enabled, script, id count-marked\n");
    let (code, out, asked) = folders.run(&["enable", "count-marked"], &[false], "");
    assert_eq!(code, 1);
    assert_eq!(asked, ["Enable it? y or n"]);
    assert!(out.starts_with("Enable Count marked?\nIt runs: "), "{out}");
    assert!(
        out.contains("While it runs it can do what you can.\n"),
        "{out}"
    );
    assert!(out.ends_with("Not enabled.\n"), "{out}");
    let (code, out, _) = folders.run(&["enable", "count-marked"], &[true], "");
    assert_eq!(code, 0);
    assert!(out.ends_with("Enabled: Count marked.\n"), "{out}");
    let (_, out, _) = folders.run(&["list"], &[], "");
    assert_eq!(out, "Count marked: enabled, script, id count-marked\n");
    fs::write(&script, format!("{TOTAL}\n")).unwrap();
    let (_, out, _) = folders.run(&["list"], &[], "");
    assert_eq!(
        out,
        "Count marked: changed, asks again, script, id count-marked\n"
    );
    let (code, out, asked) = folders.run(&["run", "count-marked"], &[false], "");
    assert_eq!(code, 1);
    assert_eq!(asked, ["Enable it again? y or n"]);
    assert!(
        out.starts_with("Count marked changed since you enabled it.\n"),
        "{out}"
    );
    let (code, out, _) = folders.run(&["disable", "count-marked"], &[], "");
    assert_eq!((code, out.as_str()), (0, "Disabled: Count marked.\n"));
}

#[test]
fn run_once_asks_one_question_then_runs() {
    if !has_bash() {
        return;
    }
    let folders = Folders::new();
    fs::write(folders.scripts().join("count-marked.sh"), TOTAL).unwrap();
    for name in ["a.txt", "b.txt"] {
        fs::write(folders.work().join(name), b"x").unwrap();
    }
    let (code, out, asked) = folders.run(
        &["run", "count-marked", "--", "a.txt", "b.txt"],
        &[true],
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(asked, ["Run it once? y or n"]);
    assert!(out.starts_with("Count marked is not enabled.\n"), "{out}");
    assert!(out.ends_with("Selected: 2. By Count marked.\n"), "{out}");
    let (code, out, _) = folders.run(&["run", "count-marked"], &[false], "");
    assert_eq!(code, 1);
    assert!(out.ends_with("Not enabled.\n"), "{out}");
    let (code, out, _) = folders.run(&["run", "count-marked", "--yes", "--", "a.txt"], &[], "");
    assert_eq!(code, 0);
    assert!(
        out.contains("Run it once? y or n yes, from --yes\n"),
        "{out}"
    );
}

#[test]
fn explain_needs_no_question_and_runs_nothing() {
    if !has_bash() {
        return;
    }
    let folders = Folders::new();
    fs::write(folders.scripts().join("count-marked.sh"), TOTAL).unwrap();
    let (code, out, asked) = folders.run(
        &["run", "count-marked", "--explain", "--", "a.txt"],
        &[],
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert!(asked.is_empty());
    assert!(out.starts_with("Would run "), "{out}");
    assert!(out.ends_with("Nothing was run.\n"), "{out}");
}

#[test]
fn check_says_no_problems_or_each_problem() {
    let folders = Folders::new();
    fs::write(folders.scripts().join("count-marked.sh"), TOTAL).unwrap();
    let (code, out, _) = folders.run(&["check", "count-marked"], &[], "");
    if has_bash() {
        assert_eq!(
            (code, out.as_str()),
            (0, "Checked 1 plugin: no problems.\n")
        );
    }
    fs::write(
        folders.scripts().join("bad.sh"),
        "# diro: name = Bad one\n# diro: colour = \"x\"\n",
    )
    .unwrap();
    let (code, out, _) = folders.run(&["check", "bad"], &[], "");
    assert_eq!(code, 1);
    assert!(
        out.starts_with(
            "Error, name (line 1): is not in quotes. Put the value in quotes: \"Bad one\". (bad)\n"
        ),
        "{out}"
    );
    assert!(
        out.ends_with("Checked 1 plugin: 1 error, 1 warning.\n"),
        "{out}"
    );
    let (code, out, _) = folders.run(&["enable", "bad"], &[true], "");
    assert_eq!(code, 1);
    assert!(out.starts_with("Not enabled: bad has errors.\n"), "{out}");
}

#[test]
fn install_copies_and_never_enables() {
    let folders = Folders::new();
    let source = folders.work().join("tidy.sh");
    fs::write(&source, "# diro: name = \"Tidy\"\nexit 0\n").unwrap();
    let path = source.to_string_lossy().into_owned();
    let (code, out, _) = folders.run(&["install", &path], &[], "");
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        "Installed: Tidy. It is not enabled.\nNext: diro plugin enable tidy\n"
    );
    assert!(folders.scripts().join("tidy.sh").is_file());
    let (_, out, _) = folders.run(&["list"], &[], "");
    assert!(out.starts_with("Tidy: not enabled, script"), "{out}");
    let (code, out, _) = folders.run(&["install", &path], &[], "");
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "Could not install tidy: it exists. Remove it first, or choose another name.\n"
    );
    let zip = folders
        .work()
        .join("plugin.zip")
        .to_string_lossy()
        .into_owned();
    assert_eq!(folders.run(&["install", &zip], &[], "").0, 2);
}

#[test]
fn log_restart_and_unknown_names_say_so() {
    let folders = Folders::new();
    fs::write(
        folders.scripts().join("tidy.sh"),
        "# diro: name = \"Tidy\"\nexit 0\n",
    )
    .unwrap();
    let (code, out, _) = folders.run(&["log", "tidy"], &[], "");
    assert_eq!((code, out.as_str()), (0, "No log yet for Tidy.\n"));
    let (code, out, _) = folders.run(&["restart", "tidy"], &[], "");
    assert_eq!(
        (code, out.as_str()),
        (0, "Nothing to restart: Tidy starts fresh each run.\n")
    );
    let (code, out, _) = folders.run(&["enable", "nothing-here"], &[], "");
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "No plugin or script is called nothing-here. Run diro plugin list for their names.\n"
    );
    let empty = Folders::new();
    let (_, out, _) = empty.run(&["list"], &[], "");
    assert!(out.starts_with("No plugins or scripts in "), "{out}");
}

#[test]
fn new_writes_a_skeleton_that_checks_clean() {
    let folders = Folders::new();
    let (code, out, _) = folders.run(&["new", "total-size", "--language", "bash"], &[], "");
    assert_eq!(code, 0, "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("Wrote total-size.sh in "), "{out}");
    assert_eq!(lines[1], "Check it: diro plugin check total-size");
    assert_eq!(lines[2], "Try it once: diro plugin run total-size -- FILE");
    assert!(folders.scripts().join("total-size.sh").is_file());
    if has_bash() {
        let (code, out, _) = folders.run(&["check", "total-size"], &[], "");
        assert_eq!(
            (code, out.as_str()),
            (0, "Checked 1 plugin: no problems.\n")
        );
    }
    let (code, out, _) = folders.run(&["new", "line-counter", "--language=python"], &[], "");
    assert_eq!(code, 0, "{out}");
    assert!(
        out.starts_with(
            "Wrote line-counter with 4 files: plugin.toml, main.py, test_main.py, README.txt.\n"
        ),
        "{out}"
    );
    let (code, out, _) = folders.run(&["check", "line-counter"], &[], "");
    assert_eq!(
        (code, out.as_str()),
        (0, "Checked 1 plugin: no problems.\n")
    );
    let (code, out, _) = folders.run(&["list"], &[], "");
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "Line counter: not enabled, plugin 0.1.0, id line-counter\nTotal size: not enabled, script, id total-size\n"
    );
}

#[test]
fn new_refuses_odd_names_existing_ones_and_wrong_kinds() {
    let folders = Folders::new();
    let (code, out, _) = folders.run(&["new", "My Tool", "--language", "bash"], &[], "");
    assert_eq!(code, 2);
    assert_eq!(
        out,
        "Could not make My Tool: U+004D is an uppercase letter; use lowercase. Use lowercase letters, digits, - and _.\n"
    );
    assert_eq!(
        folders
            .run(&["new", "tidy", "--language", "bash"], &[], "")
            .0,
        0
    );
    let (code, out, _) = folders.run(&["new", "tidy", "--language", "bash"], &[], "");
    assert_eq!(
        (code, out.as_str()),
        (1, "Could not make tidy: it exists. Choose another name.\n")
    );
    assert_eq!(
        folders.run(&["new", "x1", "--language", "lua"], &[], "").0,
        2
    );
    assert_eq!(
        folders
            .run(
                &["new", "lst", "--language", "bash", "--kind", "lister"],
                &[],
                ""
            )
            .0,
        2
    );
    let elsewhere = folders.work().to_string_lossy().into_owned();
    let (code, out, _) = folders.run(
        &[
            "new",
            "my-store",
            "--language",
            "python",
            "--kind",
            "provider",
            "--dir",
            &elsewhere,
        ],
        &[],
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.lines()
            .nth(2)
            .unwrap()
            .starts_with("Install it: diro plugin install "),
        "{out}"
    );
    assert!(folders.work().join("my-store").join("main.py").is_file());
    assert!(matches!(
        parse(&args(&["list", "--kind", "lister"])),
        Err(CliError::Failed(_))
    ));
}

/// A plugin whose previewer and converter run the system's bash with a
/// fake tool, so no pdftotext or pandoc is needed.
fn fake_recipes(folders: &Folders) -> bool {
    let Ok(bash) = Lookup::system().bash() else {
        eprintln!("bash was not found; this test is skipped here");
        return false;
    };
    let folder = folders.root.path().join("config/plugins/fake-tools");
    fs::create_dir_all(&folder).unwrap();
    let preview = folder.join("preview.sh");
    fs::write(&preview, "printf 'Title line\nbody text\n'\n").unwrap();
    let convert = folder.join("convert.sh");
    fs::write(&convert, "printf 'converted' > \"$2\"\n").unwrap();
    let path = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "/"));
    let program = path(&bash.program);
    let manifest = format!(
        "[plugin]\nid = \"fake-tools\"\nname = \"Fake tools\"\nversion = \"0.1.0\"\nprotocol = 1\n\n[[previewer]]\nid = \"first_page\"\nname = \"First page\"\nargv = [{program}, \"--noprofile\", \"--norc\", {}, \"{{input}}\"]\n\n[previewer.match]\nextensions = [\"pdf\"]\n\n[[converter]]\nid = \"to_docx\"\nname = \"To Word\"\nargv = [{program}, \"--noprofile\", \"--norc\", {}, \"{{input}}\", \"{{output}}\"]\n\n[converter.from]\nextensions = [\"md\"]\n\n[[converter.to]]\nid = \"docx\"\nname = \"Word document\"\nextension = \"docx\"\n",
        path(&preview),
        path(&convert)
    );
    fs::write(folder.join("plugin.toml"), manifest).unwrap();
    true
}

#[test]
fn preview_and_convert_run_a_recipe_once_after_the_question() {
    let folders = Folders::new();
    if !fake_recipes(&folders) {
        return;
    }
    fs::write(folders.work().join("a.pdf"), b"%PDF").unwrap();
    let (code, out, asked) = folders.run(
        &["preview", "fake-tools", "first_page", "a.pdf"],
        &[true],
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(asked, ["Run it once? y or n"]);
    assert!(
        out.ends_with("Title line\nTitle line\nbody text\n"),
        "{out}"
    );
    fs::write(folders.work().join("notes.md"), b"# x").unwrap();
    let out_dir = folders.root.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let to = out_dir.to_string_lossy().into_owned();
    let (code, out, _) = folders.run(
        &[
            "convert",
            "fake-tools",
            "to_docx",
            "notes.md",
            "--to",
            &to,
            "--yes",
        ],
        &[],
        "",
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.ends_with("Converted: notes.md to notes.docx.\n"),
        "{out}"
    );
    assert_eq!(fs::read(out_dir.join("notes.docx")).unwrap(), b"converted");
    let (code, out, _) = folders.run(
        &["preview", "fake-tools", "nope", "a.pdf", "--yes"],
        &[],
        "",
    );
    assert_eq!(code, 2);
    assert!(
        out.ends_with("Could not preview a.pdf: there is no previewer nope.\n"),
        "{out}"
    );
    assert!(matches!(
        parse(&args(&["list", "--to", "y"])),
        Err(CliError::Failed(_))
    ));
}
