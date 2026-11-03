//! What a frontend reaches beside keys and commands (lane A5): the job
//! history's dialogs, and a program started without waiting for it.
//! Every place is the fake provider's and every program a recorded one;
//! nothing touches a disk, a trash or the network, and nothing starts.

use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use diroxide_app::commands::ids;
use diroxide_app::dialog::Dialog;
use diroxide_app::{Keymap, Launcher, Program};
use diroxide_core::{KeyColumn, Place, Side};
use diroxide_ops::FakeProvider;
use diroxide_vfs::Registry;

use crate::support::{Session, Setup};

fn local(path: &str) -> Place {
    if cfg!(windows) {
        Place::parse(&format!("D:{}", path.replace('/', "\\"))).unwrap()
    } else {
        Place::parse(path).unwrap()
    }
}

/// Two files in one folder of the fake provider, both panels on it, set
/// up as given and with these keys.
fn world_with(setup: Setup, keymap: Keymap) -> (FakeProvider, Session) {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/a"));
    fake.add_file(&local("/a/f1.txt"), b"one");
    fake.add_file(&local("/a/f2.txt"), b"two");
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    let session = Session::open_with_keymap(local("/a"), local("/a"), registry, setup, keymap);
    (fake, session)
}

/// The same world with the test settings and the terminal's keys.
fn world() -> (FakeProvider, Session) {
    world_with(Setup::default(), Keymap::defaults())
}

/// A launcher that starts each program and returns without waiting, as
/// the GUI's does; it records the programs and starts none.
#[derive(Default)]
struct Starter {
    programs: Mutex<Vec<Program>>,
    /// Why every start fails, when it should.
    refusal: Option<String>,
}

impl Launcher for Starter {
    fn run(&self, program: &Program) -> Result<Option<i32>, String> {
        self.programs.lock().unwrap().push(program.clone());
        match &self.refusal {
            Some(cause) => Err(cause.clone()),
            None => Ok(None),
        }
    }
    fn shell(&self) -> (OsString, Vec<OsString>) {
        (OsString::from("testshell"), vec![OsString::from("-c")])
    }
    fn editor(&self) -> OsString {
        OsString::from("testeditor")
    }
    fn waits(&self) -> bool {
        false
    }
}

/// The world as the GUI runs it: its keys, and a launcher that does not
/// wait.
fn started(starter: Arc<Starter>) -> Session {
    let setup = Setup {
        launcher: Some(starter),
        ..Setup::default()
    };
    world_with(setup, Keymap::defaults_for(KeyColumn::Gui)).1
}

/// The labels of the top dialog's list.
fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.label.clone()).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

/// Job history is the Jobs list's Finished group (lane A2), so the
/// command opens the Jobs list there; a finished job's Show what failed
/// opens its items. The two are the history's only dialogs.
#[test]
fn job_history_opens_the_jobs_list_and_show_what_failed_the_items() {
    let (fake, mut s) = world();
    assert_eq!(s.command(ids::JOB_HISTORY), ["No jobs have run yet."]);
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    s.key("f8");
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Moved 1 item to the trash.")),
        "{said:?}"
    );
    assert_eq!(fake.trashed(), 1);

    let said = s.command(ids::JOB_HISTORY);
    assert!(said[0].starts_with("Jobs, Done: trash f1.txt"), "{said:?}");
    assert_eq!(s.dialog_title().as_deref(), Some("Jobs"));
    assert_eq!(rows(&s), ["Finished, 1 job", "Done: trash f1.txt"]);

    // Enter opens the job's actions, the first of them Show what failed.
    s.key("enter");
    assert_eq!(s.dialog_title().as_deref(), Some("Done: trash f1.txt"));
    assert_eq!(rows(&s)[0], "Show what failed");
    let said = s.key("enter");
    let title = s.dialog_title().unwrap();
    assert!(title.starts_with("Move to the trash finished"), "{title}");
    assert_eq!(rows(&s), ["Nothing failed or was skipped."]);
    assert!(
        said[0].starts_with("Move to the trash finished"),
        "{said:?}"
    );

    // Escape walks back to the Jobs list, then to the panel.
    s.key("escape");
    assert_eq!(s.dialog_title().as_deref(), Some("Jobs"));
    s.key("escape");
    assert!(s.app.dialog().is_none(), "{:?}", s.dialog_title());
}

/// The GUI's handover: a launcher that does not wait, so the app says
/// what started, in its own words, and never "Back in diroxide", since
/// nothing ended; Shell here says nothing first, since nothing is handed
/// over. Every line has its meaning in its first 40 cells.
#[test]
fn a_program_started_without_waiting_is_said_in_the_apps_words() {
    let starter = Arc::new(Starter::default());
    let mut s = started(starter.clone());
    assert_eq!(s.command(ids::SHELL_HERE), ["Started a shell in a"]);
    s.command(ids::RUN_COMMAND);
    s.type_text("echo hi");
    assert_eq!(s.key("enter"), ["Started the command in a"]);
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    assert_eq!(s.key("f4"), ["Started the editor on f1.txt"]);
    assert!(
        !s.said
            .iter()
            .any(|m| m.text.starts_with("Back in diroxide")),
        "{:?}",
        s.said
    );
    let programs = starter.programs.lock().unwrap();
    assert_eq!(programs.len(), 3);
    assert_eq!(programs[0].program, OsString::from("testshell"));
    assert_eq!(programs[1].args.last().unwrap(), "echo hi");
    assert_eq!(programs[2].program, OsString::from("testeditor"));
}

/// A start that fails says why and what to do: the cause the launcher
/// gives, with the TERMINAL variable when no terminal program was found.
#[test]
fn a_program_that_could_not_start_says_why_and_what_to_do() {
    let starter = Arc::new(Starter {
        refusal: Some("no terminal program was found".to_owned()),
        ..Starter::default()
    });
    let mut s = started(starter);
    assert_eq!(
        s.command(ids::SHELL_HERE),
        [
            "Could not start a shell in a: no terminal program was found. Set the TERMINAL variable to a terminal program, such as xterm."
        ]
    );
    let starter = Arc::new(Starter {
        refusal: Some("the program was not found".to_owned()),
        ..Starter::default()
    });
    let mut s = started(starter);
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    assert_eq!(
        s.key("f4"),
        [
            "Could not start the editor on f1.txt: the program was not found. Check the command, or the shell and editor settings."
        ]
    );
}

/// The terminal's handover is unchanged: Shell here says how to come
/// back, the launcher waits, and the app is back in diroxide after.
#[test]
fn the_terminal_still_hands_over_and_comes_back() {
    let (_, mut s) = world();
    assert_eq!(
        s.command(ids::SHELL_HERE),
        [
            "Shell here: exit the shell to come back.",
            "Back in diroxide, a"
        ]
    );
}
