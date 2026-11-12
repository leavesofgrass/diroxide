//! What a frontend reaches beside keys and commands (lane A5): the job
//! history's dialogs and the open dialog's purpose, a program started
//! without waiting for it, and a click's row focused by its place in a
//! dialog or by its id in the preview. Every place is the fake
//! provider's and every program a recorded one;
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

/// Three files in one folder of the fake provider, both panels on it,
/// set up as given and with these keys.
fn world_with(setup: Setup, keymap: Keymap) -> (FakeProvider, Session) {
    let fake = FakeProvider::new();
    fake.add_folder(&local("/a"));
    fake.add_file(&local("/a/f1.txt"), b"one");
    fake.add_file(&local("/a/f2.txt"), b"two");
    fake.add_file(&local("/a/notes.md"), b"# One\n\ntwo\n\nthree\n");
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
    assert_eq!(s.app.dialog_purpose(), Some("Jobs"));
    assert_eq!(rows(&s), ["Finished, 1 job", "Done: trash f1.txt"]);

    // Enter opens the job's actions, the first of them Show what failed.
    s.key("enter");
    assert_eq!(s.dialog_title().as_deref(), Some("Done: trash f1.txt"));
    assert_eq!(rows(&s)[0], "Show what failed");
    let said = s.key("enter");
    assert_eq!(s.app.dialog_purpose(), Some("JobItems"));
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
    assert_eq!(s.app.dialog_purpose(), Some("Jobs"));
    s.key("escape");
    assert!(s.app.dialog().is_none(), "{:?}", s.dialog_title());
    assert_eq!(s.app.dialog_purpose(), None);
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

/// F4 again on the file the editor was just started on starts nothing
/// for two seconds and says so (the owner's Notepad report of November
/// 6, 2026: repeated starts overlapped into Notepad's "cannot find the
/// path" dialog); another file, a moment later, or a start that failed
/// starts it again.
#[test]
fn a_second_f4_at_once_says_the_editor_is_already_started() {
    let starter = Arc::new(Starter::default());
    let mut s = started(starter.clone());
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    assert_eq!(s.key("f4"), ["Started the editor on f1.txt"]);
    let again = s.key("f4");
    assert_eq!(again, ["Editor already started on f1.txt"]);
    assert!(again[0].chars().count() <= 40, "{again:?}");
    s.clock.advance(1);
    assert_eq!(s.key("f4"), ["Editor already started on f1.txt"]);
    assert_eq!(starter.programs.lock().unwrap().len(), 1);
    // Another file starts at once.
    assert!(s.app.focus_name(Side::Left, "f2.txt"));
    assert_eq!(s.key("f4"), ["Started the editor on f2.txt"]);
    // Two seconds on, the same file starts again.
    s.clock.advance(2);
    assert_eq!(s.key("f4"), ["Started the editor on f2.txt"]);
    assert_eq!(starter.programs.lock().unwrap().len(), 3);

    // A start that failed is tried again on the next press.
    let refusing = Arc::new(Starter {
        refusal: Some("the program was not found".to_owned()),
        ..Starter::default()
    });
    let mut s = started(refusing.clone());
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    for _ in 0..2 {
        let said = s.key("f4");
        assert!(
            said[0].starts_with("Could not start the editor on f1.txt"),
            "{said:?}"
        );
    }
    assert_eq!(refusing.programs.lock().unwrap().len(), 2);
}

/// The terminal's editor has the screen until it ends, so F4 at once
/// after it ended starts it again.
#[test]
fn the_terminal_starts_the_editor_again_after_it_ended() {
    let (_, mut s) = world();
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    for _ in 0..2 {
        let said = s.key("f4");
        assert!(
            said.iter().any(|l| l.starts_with("Back in diroxide")),
            "{said:?}"
        );
    }
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

/// The focused row's index of the top dialog's list, and whether a
/// field's candidates have the focus.
fn dialog_focus(s: &Session) -> (Option<usize>, bool) {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => (list.focus_index(), true),
        Some(Dialog::Question { choices, .. }) => (choices.focus_index(), true),
        Some(Dialog::Edit {
            candidates: Some(list),
            in_list,
            ..
        }) => (list.focus_index(), *in_list),
        other => panic!("no list: {other:?}"),
    }
}

/// A click on a dialog's row focuses it by its place, without walking
/// Home and Down: a choice list's row, and a field's candidate, which the
/// focus then enters; a place past the end, or a dialog with no list,
/// changes nothing.
#[test]
fn a_dialogs_row_is_focused_by_its_place() {
    let (_, mut s) = world();
    assert!(!s.app.focus_dialog_row(0), "no dialog is open");

    // The menu bar, a choice list: Enter opens the menu the click chose.
    s.command(ids::MENU);
    let menus = rows(&s);
    assert!(menus.len() > 3, "{menus:?}");
    assert!(s.app.focus_dialog_row(2));
    assert_eq!(dialog_focus(&s), (Some(2), true));
    assert!(!s.app.focus_dialog_row(menus.len()));
    assert_eq!(dialog_focus(&s), (Some(2), true));
    assert!(s.app.focus_dialog_row(0));
    assert_eq!(dialog_focus(&s), (Some(0), true));
    assert!(s.app.focus_dialog_row(2));
    s.key("enter");
    assert_eq!(s.app.dialog_purpose(), Some("Menu"));
    assert_eq!(s.dialog_title(), Some(format!("{} menu", menus[2])));
    s.key("escape");
    s.key("escape");
    assert!(s.app.dialog().is_none());

    // The palette, a field with its candidates open: the click enters them.
    s.command(ids::PALETTE);
    assert!(!dialog_focus(&s).1, "the field has the focus");
    assert!(s.app.focus_dialog_row(3));
    assert_eq!(dialog_focus(&s), (Some(3), true));
    s.key("escape");
    s.key("escape");
    assert!(s.app.dialog().is_none());

    // A confirmation has no list to focus.
    assert!(s.app.focus_name(Side::Left, "f1.txt"));
    s.key("f8");
    assert!(matches!(s.dialog(), Some(Dialog::Confirm { .. })));
    assert!(!s.app.focus_dialog_row(0));
    s.key("escape");
}

/// The quick view's time until it is done, in real time: it reads on its
/// own thread.
fn settle_quick(s: &mut Session) -> Vec<String> {
    let mut said = Vec::new();
    for _ in 0..400 {
        if s.app.quick_view_wants_tick().is_none() {
            break;
        }
        let effects = s.app.quick_view_tick();
        said.extend(s.run(effects));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    said
}

/// A click on a row of the preview focuses it by its id, with nothing
/// said; an id the region has not, or the quick view off, changes
/// nothing.
#[test]
fn a_preview_row_is_focused_by_its_id() {
    let mut quick = crate::support::settings();
    quick.quick_view.wait_ms = 0;
    let setup = Setup {
        settings: quick,
        ..Setup::default()
    };
    let (_, mut s) = world_with(setup, Keymap::defaults());
    let registry = s.runtime.registry().clone();
    s.app.set_quick_view_registry(registry);
    assert!(!s.app.focus_preview_row("0"), "the quick view is off");

    assert!(s.app.focus_name(Side::Left, "notes.md"));
    let mut said = s.key("shift+f3");
    said.extend(settle_quick(&mut s));
    assert_eq!(said, ["Quick view on", "Preview: notes.md, 1 heading"]);
    s.key("tab");
    assert!(s.app.in_preview());
    let region = s.app.quick_view().unwrap().region();
    let ids: Vec<String> = region.rows().iter().map(|r| r.id.clone()).collect();
    assert!(ids.len() >= 3, "{ids:?}");
    assert_eq!(region.focus_index(), Some(0));

    let before = s.said.len();
    let last = ids.last().unwrap().clone();
    assert!(s.app.focus_preview_row(&last));
    let region = s.app.quick_view().unwrap().region();
    assert_eq!(region.focused().map(|r| r.id.as_str()), Some(last.as_str()));
    assert!(!s.app.focus_preview_row("99"));
    let region = s.app.quick_view().unwrap().region();
    assert_eq!(region.focused().map(|r| r.id.as_str()), Some(last.as_str()));
    assert_eq!(s.said.len(), before, "nothing is said");
    // The keys go on from the clicked row.
    let said = s.key("up");
    assert_eq!(said.len(), 1, "{said:?}");
    let region = s.app.quick_view().unwrap().region();
    assert_eq!(region.focus_index(), Some(ids.len() - 2));
}
