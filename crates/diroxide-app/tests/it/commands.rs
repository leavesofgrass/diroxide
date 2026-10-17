//! One test per wired command family, driven by keys and commands through
//! the app and its runtime, on a fixture folder made at test time with the
//! test trash; no terminal, no real trash, no real config or state folder.

use std::path::PathBuf;
use std::sync::Arc;

use diroxide_app::commands::ids;
use diroxide_app::{Answer, AppEvent, Dialog, SpawnKind};
use diroxide_core::{ArgValue, Command, Side};

use crate::support::{
    Recorder, Session, Setup, TestTrash, fixture, place, registry_with_trash, temp,
};

struct Fixture {
    _dir: tempfile::TempDir,
    work: PathBuf,
    backup: PathBuf,
    s: Session,
    trash: Arc<TestTrash>,
    recorder: Arc<Recorder>,
}

fn open() -> Fixture {
    let dir = temp();
    let work = fixture(dir.path());
    let backup = work.join("Backup");
    let (registry, trash) = registry_with_trash(&dir.path().join("bin"));
    let recorder = Arc::new(Recorder::default());
    let s = Session::open(
        place(&work),
        place(&backup),
        registry,
        Setup {
            home: Some(place(&work)),
            recorder: recorder.clone(),
            ..Setup::default()
        },
    );
    Fixture {
        _dir: dir,
        work,
        backup,
        s,
        trash,
        recorder,
    }
}

fn last(said: &[String]) -> &str {
    said.last().map(String::as_str).unwrap_or_default()
}

#[test]
fn the_panels_switch_swap_and_show_each_other() {
    let mut f = open();
    let said = f.s.key("tab");
    assert_eq!(said, ["Right panel: Backup, 1 item"]);
    assert_eq!(f.s.app.active_side(), Side::Right);
    f.s.key("tab");
    let said = f.s.key("ctrl+u");
    assert_eq!(said[0], "Panels swapped: left Backup, right work");
    assert_eq!(f.s.app.panel(Side::Left).place(), &place(&f.backup));
    f.s.key("ctrl+u");
    f.s.focus("Photos");
    let said = f.s.key("ctrl+right");
    assert!(said.contains(&"Other panel: Photos".to_owned()), "{said:?}");
    assert_eq!(f.s.names(Side::Right), ["crow.jpg"]);
    let said = f.s.key("ctrl+left");
    assert!(said.contains(&"Other panel: work".to_owned()), "{said:?}");
    let said = f.s.key("alt+o");
    assert!(
        said[0].starts_with("Other panel: work, 8 items, on Photos, folder"),
        "{said:?}"
    );
}

#[test]
fn enter_right_left_back_forward_and_home() {
    let mut f = open();
    f.s.focus("Photos");
    f.s.key("right");
    assert_eq!(f.s.names(Side::Left), ["crow.jpg"]);
    let said = f.s.key("right");
    assert_eq!(said, ["Not a folder: crow.jpg, JPEG"]);
    f.s.key("left");
    assert_eq!(
        f.s.focused(),
        "Photos",
        "Left lands on the folder just left"
    );
    f.s.key("alt+left");
    assert_eq!(f.s.names(Side::Left), ["crow.jpg"]);
    f.s.key("alt+right");
    assert_eq!(f.s.focused(), "Photos", "forward keeps the folder's cursor");
    f.s.key("enter");
    f.s.key("alt+home");
    assert_eq!(f.s.app.active_panel().place(), &place(&f.work));
    let said = f.s.key("ctrl+r");
    assert_eq!(said, ["Reloaded: 8 items"]);
    f.s.focus("notes.txt");
    let said = f.s.key("enter");
    assert_eq!(said, ["Opening notes.txt"]);
    assert_eq!(f.recorder.opened.lock().unwrap().len(), 1);
}

#[test]
fn up_at_the_top_says_so() {
    let mut f = open();
    for _ in 0..64 {
        if f.s.app.active_panel().place().parent().is_none() {
            break;
        }
        f.s.key("backspace");
    }
    assert_eq!(f.s.key("alt+up"), ["At the top already"]);
}

#[test]
fn a_program_asks_first_and_starts_on_no() {
    let mut f = open();
    let name = if cfg!(windows) {
        "setup.bat"
    } else {
        "setup.sh"
    };
    let path = f.work.join(name);
    std::fs::write(&path, b"echo hi").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    f.s.key("ctrl+r");
    f.s.focus(name);
    let said = f.s.key("enter");
    assert_eq!(said, [format!("Run {name}? No is the default. y or n")]);
    f.s.key("enter");
    assert!(f.s.dialog().is_none());
    assert!(f.recorder.opened.lock().unwrap().is_empty(), "Enter is No");
    f.s.key("enter");
    let said = f.s.key("y");
    assert_eq!(said, [format!("Running {name}")]);
    assert_eq!(f.recorder.opened.lock().unwrap().len(), 1);
}

#[test]
fn go_to_a_path_completes_and_keeps_its_history() {
    let mut f = open();
    let said = f.s.key("ctrl+l");
    assert!(said[0].starts_with("Go to a path: "), "{said:?}");
    // Clear the field, then type the folder and a stem, and complete.
    for _ in 0..400 {
        f.s.key("backspace");
    }
    let sep = if cfg!(windows) { '\\' } else { '/' };
    f.s.type_text(&format!("{}{sep}Pho", f.work.display()));
    let said = f.s.key("tab");
    assert_eq!(said, ["completed to Photos"]);
    f.s.key("enter");
    assert_eq!(f.s.names(Side::Left), ["crow.jpg"]);
    f.s.key("ctrl+l");
    f.s.key("up");
    if let Some(Dialog::Edit { field, .. }) = f.s.dialog() {
        assert!(field.text().ends_with("Photos"), "{}", field.text());
    } else {
        panic!("no field");
    }
    f.s.key("escape");
    assert!(f.s.dialog().is_none());
    let quoted = format!("\"{}\"", f.work.display());
    f.s.run_command(Command::new(ids::GO_PATH));
    let id = f.s.app.dialog().unwrap().0;
    let effects = f.s.app.handle(AppEvent::Answer {
        dialog: id,
        answer: Answer::Text(quoted),
    });
    f.s.run(effects);
    assert_eq!(f.s.app.active_panel().place(), &place(&f.work));
}

#[test]
fn the_folder_history_and_the_places_list() {
    let mut f = open();
    f.s.focus("Photos");
    f.s.key("enter");
    f.s.command(ids::HISTORY);
    assert_eq!(f.s.dialog_title().as_deref(), Some("Folder history"));
    f.s.key("enter");
    assert_eq!(f.s.app.active_panel().place(), &place(&f.work));
    let said = f.s.key("ctrl+g");
    assert!(said[0].starts_with("Go to a place"), "{said:?}");
    f.s.type_text("work");
    f.s.key("down");
    f.s.key("enter");
    assert!(f.s.dialog().is_none());
}

#[test]
fn selection_by_every_command() {
    let mut f = open();
    assert_eq!(f.s.key("ctrl+a"), ["8 items selected"]);
    assert_eq!(f.s.command(ids::SELECT_NONE), ["Nothing selected"]);
    assert_eq!(f.s.key("*"), ["8 items selected"]);
    f.s.key("*");
    let said = f.s.key("plus");
    assert_eq!(said, ["Select by pattern: *"]);
    for _ in 0..3 {
        f.s.key("backspace");
    }
    f.s.type_text("*.txt");
    // The count comes first, as a question that starts on No (W3a).
    assert_eq!(
        f.s.key("enter"),
        ["4 entries will be selected. No is the default. y or n"]
    );
    assert_eq!(f.s.key("y"), ["4 selected"]);
    f.s.key("-");
    f.s.key("backspace");
    f.s.type_text("notes*");
    assert_eq!(
        f.s.key("enter"),
        ["1 entry will be unselected. No is the default. y or n"]
    );
    assert_eq!(f.s.key("y"), ["3 selected"]);
    f.s.key("home");
    f.s.key("ctrl+down");
    assert_eq!(f.s.focused(), "raccoon \u{1f99d}.txt");
    f.s.key("ctrl+up");
    assert_eq!(f.s.key("ctrl+up"), ["No selected row above"]);
    let said = f.s.key("ctrl+d");
    assert!(
        said[0].starts_with("3 selected, 11 bytes. work, 8 items, on "),
        "{said:?}"
    );
    let said = f.s.key("plus");
    assert_eq!(said, ["Select by pattern: *"]);
    f.s.type_text("[bad");
    let said = f.s.key("enter");
    assert!(said[0].starts_with("Filter problem: "), "{said:?}");
}

#[test]
fn the_previous_selection_comes_back() {
    let mut f = open();
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["No earlier selection in this folder to restore"]
    );
    f.s.focus("budget.xlsx");
    f.s.key("space");
    f.s.key("space");
    // A stray select none does not cost the marks.
    assert_eq!(f.s.command(ids::SELECT_NONE), ["Nothing selected"]);
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 2 selected."]
    );
    // Select all replaces them; Restore swaps back, and again.
    f.s.key("ctrl+a");
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 2 selected."]
    );
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 8 selected."]
    );
    f.s.command(ids::SELECT_RESTORE);
    // A finished copy clears the marks, and they come back.
    f.s.key("f5");
    let said = f.s.key("y");
    assert!(
        said.contains(&"Copy finished: 2 done.".to_owned()),
        "{said:?}"
    );
    assert_eq!(f.s.app.active_panel().list().selected_count(), 0);
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 2 selected."]
    );
    // Leaving the folder and coming up again.
    f.s.focus("Photos");
    f.s.key("enter");
    f.s.key("backspace");
    assert_eq!(f.s.app.active_panel().list().selected_count(), 0);
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 2 selected."]
    );
    // A row that went away is counted, never guessed.
    f.s.command(ids::SELECT_NONE);
    f.s.focus("notes.txt");
    f.s.key("f8");
    f.s.key("y");
    assert!(!f.work.join("notes.txt").exists());
    assert_eq!(
        f.s.command(ids::SELECT_RESTORE),
        ["Selection restored: 1 selected. 1 is not in the list."]
    );
    assert_eq!(f.trash.count(), 1);
}

#[test]
fn type_ahead_jumps_and_counts_matches() {
    let mut f = open();
    assert_eq!(f.s.key("r"), ["raccoon \u{1f99d}.txt, 1 of 3 matches"]);
    assert_eq!(f.s.key("e"), ["report.pdf, 1 of 1 matches"]);
    f.s.clock.advance(5);
    assert_eq!(f.s.key("r"), ["R\u{e9}sum\u{e9}.txt, 3 of 3 matches"]);
    f.s.clock.advance(5);
    assert_eq!(f.s.key("z"), ["No name starts with z"]);
}

#[test]
fn sorting_by_the_keys_the_list_and_the_palette() {
    let mut f = open();
    f.s.focus("report.pdf");
    assert_eq!(f.s.key("ctrl+f6"), ["Sorted by size, largest first"]);
    assert_eq!(f.s.focused(), "report.pdf", "the cursor stays on its entry");
    assert_eq!(f.s.key("ctrl+f6"), ["Sorted by size, smallest first"]);
    assert_eq!(f.s.key("ctrl+f3"), ["Sorted by name, A to Z"]);
    assert_eq!(
        f.s.key("ctrl+f5"),
        ["Sorted by modified time, newest first"]
    );
    assert_eq!(f.s.key("ctrl+f4"), ["Sorted by extension, A to Z"]);
    assert_eq!(f.s.key("ctrl+f7"), ["Sorted in the folder's own order"]);
    let said = f.s.command(ids::SORT_LABEL);
    assert_eq!(
        said,
        ["Sorted by label, A to Z. No entry here has a label; Alt+L sets one."]
    );
    let said = f.s.command(ids::SORT_LIST);
    assert!(
        said[0].starts_with("Sort by, Label, current, A to Z"),
        "{said:?}"
    );
    f.s.key("home");
    assert_eq!(f.s.key("enter"), ["Sorted by name, A to Z"]);
    f.s.key("ctrl+p");
    f.s.type_text("sort by size");
    assert_eq!(f.s.key("enter"), ["Sorted by size, largest first"]);
}

#[test]
fn the_filter_narrows_as_you_type_and_clears() {
    let mut f = open();
    let said = f.s.key("ctrl+s");
    assert_eq!(said, ["Filter, empty"]);
    f.s.type_text("*.txt");
    assert_eq!(f.s.app.active_panel().list().len(), 4);
    assert_eq!(f.s.key("enter"), ["Filter on, 4 of 8 shown"]);
    assert_eq!(f.s.command(ids::FILTER_CLEAR), ["Filter off, 8 shown"]);
    f.s.key("ctrl+s");
    f.s.type_text("pdf");
    assert_eq!(f.s.key("escape"), ["No filter, 8 shown"]);
    assert_eq!(f.s.app.active_panel().list().len(), 8, "Escape restores");
    assert_eq!(f.s.key("alt+."), ["Hidden files shown"]);
}

#[test]
fn make_folder_lands_on_it_and_a_bad_name_is_refused() {
    let mut f = open();
    let said = f.s.key("f7");
    assert_eq!(said, ["New folder in work, empty"]);
    f.s.type_text("Reports");
    let said = f.s.key("enter");
    assert!(
        said.contains(&"Created folder Reports".to_owned()),
        "{said:?}"
    );
    assert_eq!(f.s.focused(), "Reports");
    assert!(f.work.join("Reports").is_dir());
    f.s.key("f7");
    f.s.type_text("memo ");
    let said = f.s.key("enter");
    assert_eq!(
        said,
        ["Name needs care: ends in a dot or space. Nothing was made."]
    );
    assert!(!f.work.join("memo ").exists());
}

#[test]
fn rename_steps_through_several_marked_entries() {
    let mut f = open();
    f.s.focus("budget.xlsx");
    f.s.key("space");
    f.s.key("space");
    f.s.key("f2");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Rename budget.xlsx to"));
    f.s.type_text("-2026");
    f.s.key("enter");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Rename notes.txt to"));
    f.s.type_text("-old");
    f.s.key("enter");
    assert!(f.work.join("budget-2026.xlsx").exists());
    assert!(f.work.join("notes-old.txt").exists());
    let said = f.s.key("ctrl+z");
    assert!(
        said.contains(&"Undid rename: 2 items are back".to_owned()),
        "{said:?}"
    );
    assert!(f.work.join("budget.xlsx").exists());
}

#[test]
fn duplicate_fills_in_a_free_name() {
    let mut f = open();
    f.s.focus("notes.txt");
    let said = f.s.key("shift+f5");
    assert_eq!(said, ["Duplicate notes.txt as: notes (2).txt"]);
    f.s.key("enter");
    assert!(f.work.join("notes (2).txt").exists());
}

#[test]
fn move_asks_and_moves() {
    let mut f = open();
    f.s.focus("notes.txt");
    let said = f.s.key("f6");
    assert!(last(&said).starts_with("Move 1 item to "), "{said:?}");
    let said = f.s.key("y");
    assert!(
        said.contains(&"Move finished: 1 done.".to_owned()),
        "{said:?}"
    );
    assert!(f.backup.join("notes.txt").exists());
    assert!(!f.work.join("notes.txt").exists());
}

#[test]
fn a_conflict_asks_one_item_at_a_time_and_escape_skips() {
    let mut f = open();
    f.s.focus("report.pdf");
    f.s.key("f5");
    let said = f.s.key("y");
    assert!(
        said.iter().any(|l| l == "Name taken: report.pdf"),
        "{said:?}"
    );
    let Some(Dialog::Question { lines, choices }) = f.s.dialog() else {
        panic!("no question");
    };
    assert!(lines[1].starts_with("New is"), "{lines:?}");
    assert_eq!(choices.announce_focus(), "Skip this one, 1 of 9");
    let said = f.s.key("escape");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Copy finished: 0 done, 1 skipped")),
        "{said:?}"
    );
    assert_eq!(std::fs::read(f.backup.join("report.pdf")).unwrap(), b"old");
    f.s.key("f5");
    f.s.key("y");
    for _ in 0..4 {
        f.s.key("down");
    }
    let Some(Dialog::Question { choices, .. }) = f.s.dialog() else {
        panic!("no question");
    };
    assert!(
        choices
            .announce_focus()
            .starts_with("Keep both: report (2).pdf")
    );
    f.s.key("enter");
    assert!(f.backup.join("report (2).pdf").exists());
}

#[test]
fn quitting_while_a_job_waits_asks_first() {
    let mut f = open();
    f.s.focus("report.pdf");
    f.s.key("f5");
    f.s.key("y");
    let said = f.s.key("ctrl+q");
    assert_eq!(said, ["Quit? 1 job is running. No is the default. y or n"]);
    f.s.key("n");
    assert!(!f.s.app.wants_quit());
    f.s.key("escape");
    assert!(f.s.app.jobs().is_empty());
}

#[test]
fn delete_for_good_lists_and_says_the_honest_sentence() {
    let mut f = open();
    f.s.focus("notes.txt");
    let said = f.s.key("shift+delete");
    assert_eq!(
        last(&said),
        "Delete 1 item for good? No is the default. y or n"
    );
    let Some(Dialog::Confirm { listing, .. }) = f.s.dialog() else {
        panic!("no question");
    };
    assert!(
        listing.iter().any(|l| l.starts_with("delete ")),
        "{listing:?}"
    );
    assert_eq!(
        listing.last().map(String::as_str),
        Some("Deleted data may stay on the disk. diroxide cannot erase it for good.")
    );
    f.s.key("y");
    assert!(!f.work.join("notes.txt").exists());
    assert_eq!(f.trash.count(), 0, "for good, not the trash");
}

#[test]
fn the_file_clipboard_cuts_once_and_copies_twice() {
    let mut f = open();
    f.s.focus("notes.txt");
    assert_eq!(f.s.key("ctrl+c"), ["Copied 1 item. Paste with Ctrl+V."]);
    f.s.key("tab");
    f.s.key("ctrl+v");
    f.s.key("y");
    assert!(f.backup.join("notes.txt").exists());
    assert!(f.s.app.clipboard().is_some(), "a copy stays");
    f.s.key("tab");
    f.s.focus("budget.xlsx");
    assert_eq!(f.s.key("ctrl+x"), ["Cut 1 item. Paste with Ctrl+V."]);
    f.s.key("tab");
    f.s.key("ctrl+v");
    f.s.key("y");
    assert!(f.backup.join("budget.xlsx").exists());
    assert!(!f.work.join("budget.xlsx").exists());
    assert!(f.s.app.clipboard().is_none(), "a cut is spent");
    assert_eq!(f.s.key("ctrl+v"), ["The file clipboard is empty"]);
}

#[test]
fn copy_to_a_place_lists_recent_destinations_first() {
    let mut f = open();
    f.s.focus("notes.txt");
    f.s.command(ids::COPY_TO);
    assert_eq!(f.s.dialog_title().as_deref(), Some("Copy 1 item to"));
    f.s.type_text(&f.backup.display().to_string());
    f.s.key("enter");
    f.s.key("y");
    assert!(f.backup.join("notes.txt").exists());
    f.s.focus("budget.xlsx");
    f.s.command(ids::MOVE_TO);
    let Some(Dialog::Edit {
        candidates: Some(list),
        ..
    }) = f.s.dialog()
    else {
        panic!("no list");
    };
    assert_eq!(list.rows()[0].id, f.backup.display().to_string());
    assert_eq!(list.rows()[0].details[0], "recent destination");
    f.s.key("escape");
}

#[test]
fn details_context_menu_help_and_the_key_bar() {
    let mut f = open();
    f.s.focus("report.pdf");
    let said = f.s.key("alt+enter");
    assert!(
        said[0].starts_with("Details of report.pdf, Type: PDF document"),
        "{said:?}"
    );
    f.s.key("escape");
    let said = f.s.key("shift+f10");
    assert!(
        said[0].starts_with("Actions for report.pdf, Open, Enter"),
        "{said:?}"
    );
    f.s.key("escape");
    let said = f.s.key("shift+f1");
    assert!(
        said[0].starts_with("Keyboard, ") && said[0].contains(" commands, Up and Down move"),
        "{said:?}"
    );
    // The keyboard page as a list: headings, then each command with its
    // keys; a letter jumps by name.
    let said = f.s.key("down");
    assert!(said[0].starts_with("Everywhere, 2 of "), "{said:?}");
    let said = f.s.key("c");
    assert!(
        said[0].starts_with("Command palette, Ctrl+P, Finds"),
        "{said:?}"
    );
    let said = f.s.key("enter");
    assert!(said[0].starts_with("Command palette"), "{said:?}");
    f.s.key("escape");
    let said = f.s.command(ids::SAY_KEY_BAR);
    assert!(
        said[0].starts_with("Key bar: F2 Rename, F3 View, F4 Edit, F5 Copy"),
        "{said:?}"
    );
    assert!(said[0].ends_with("Control plus Q Quit."), "{said:?}");
    assert_eq!(
        f.s.app.key_bar_within(40),
        "F2 Rename  F3 View  F4 Edit  F5 Copy"
    );
    assert_eq!(
        f.s.command(ids::SWITCH_BANK),
        ["Only one bank: Function keys."]
    );
    let said = f.s.key("f3");
    assert!(said[0].starts_with("report.pdf, in "), "{said:?}");
    assert_eq!(f.s.key("escape"), ["Viewer closed: report.pdf"]);
    assert_eq!(f.s.command(ids::PLUGINS), ["Plugins come with alpha.4."]);
}

#[test]
fn the_menus_open_by_bar_letter_and_access_key() {
    let mut f = open();
    let said = f.s.key("f9");
    assert_eq!(said, ["Menu bar, File, Alt+F, 1 of 8"]);
    f.s.key("enter");
    assert_eq!(f.s.dialog_title().as_deref(), Some("File menu"));
    f.s.key("right");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Edit menu"));
    f.s.key("escape");
    f.s.key("alt+v");
    f.s.key("s");
    assert_eq!(f.s.dialog_title().as_deref(), Some("Sort menu"));
    f.s.key("escape");
    f.s.key("escape");
    f.s.key("alt+g");
    let said = f.s.key("h");
    assert!(f.s.dialog().is_none(), "{said:?}");
}

#[test]
fn settings_set_and_reset_with_their_words() {
    let mut f = open();
    let said = f.s.command(ids::SETTINGS);
    assert!(
        said[0].starts_with("Settings, Layout, side by side, 1 of"),
        "{said:?}"
    );
    f.s.key("enter");
    f.s.key("down");
    assert_eq!(f.s.key("enter"), ["Layout: stacked"]);
    assert_eq!(f.s.app.settings().layout, diroxide_config::Layout::Stacked);
    f.s.key("escape");
    f.s.command(ids::SETTINGS);
    f.s.key("enter");
    f.s.key("end");
    assert_eq!(f.s.key("enter"), ["Layout back to side by side"]);
    f.s.key("escape");
    assert_eq!(f.s.command(ids::LAYOUT), ["Layout: stacked"]);
    let said = f.s.command(ids::NEXT_THEME);
    assert!(said.last().unwrap().starts_with("Theme "), "{said:?}");
    assert_eq!(f.s.command(ids::PROBLEMS), ["No config problems."]);
}

#[test]
fn the_key_test_says_each_key_until_cancel() {
    let mut f = open();
    let said = f.s.command(ids::KEY_TEST);
    assert_eq!(
        said,
        ["Key test: press keys to hear what each sends and runs. Escape ends it."]
    );
    assert_eq!(f.s.key("f5"), ["Got F5. Runs Copy to other panel."]);
    assert_eq!(
        f.s.key("escape"),
        ["Got Escape. Runs Cancel. In the viewer: Close viewer. Key test ended."]
    );
    assert!(!f.s.app.key_test_active());
}

#[test]
fn run_a_command_shell_here_and_the_editor() {
    let mut f = open();
    f.s.command(ids::RUN_COMMAND);
    assert_eq!(f.s.dialog_title().as_deref(), Some("Run in work"));
    f.s.type_text("echo hi");
    let said = f.s.key("enter");
    assert_eq!(said, ["Back in diroxide, work"]);
    f.s.command(ids::SHELL_HERE);
    f.s.focus("notes.txt");
    f.s.key("f4");
    let programs = f.recorder.programs.lock().unwrap();
    assert_eq!(programs.len(), 3);
    assert_eq!(programs[0].args.last().unwrap(), "echo hi");
    assert_eq!(programs[1].program, "testshell");
    assert_eq!(programs[2].program, "testeditor");
    let _ = SpawnKind::Shell;
}

#[test]
fn undo_with_nothing_and_the_job_history() {
    let mut f = open();
    assert_eq!(f.s.key("ctrl+z"), ["Nothing to undo"]);
    assert_eq!(f.s.command(ids::JOB_HISTORY), ["No jobs have run yet."]);
    f.s.focus("notes.txt");
    f.s.key("f8");
    f.s.key("y");
    let said = f.s.command(ids::JOB_HISTORY);
    assert!(
        said[0].starts_with("Job history, Move to the trash finished"),
        "{said:?}"
    );
    f.s.key("enter");
    assert!(
        f.s.dialog_title()
            .unwrap()
            .starts_with("Move to the trash finished")
    );
}

#[test]
fn commands_with_arguments_act_at_once() {
    let mut f = open();
    let said = f.s.run_command(
        Command::new(ids::SELECT_PATTERN).with_arg("pattern", ArgValue::Text("*.pdf".into())),
    );
    assert_eq!(said, ["Selected 1 matching *.pdf. 1 selected."]);
    f.s.run_command(Command::new(ids::MAKE_FOLDER).with_arg("name", ArgValue::Text("New".into())));
    assert!(f.work.join("New").is_dir());
    f.s.run_command(
        Command::new(ids::GO_PATH).with_arg("place", ArgValue::Place(place(&f.backup))),
    );
    assert_eq!(f.s.app.active_panel().place(), &place(&f.backup));
}

/// Answer 49: every palette row carries its key beside its name, or the
/// menu that holds a command with no key, and typing says the first match
/// with its key.
#[test]
fn the_palette_says_each_commands_key_beside_its_name() {
    let mut f = open();
    f.s.key("ctrl+p");
    let said = f.s.type_text("sort by s");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Sort by size, Ctrl+F6. 1 match")
    );
    for _ in 0.."sort by s".len() {
        f.s.key("backspace");
    }
    let said = f.s.type_text("switch b");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Switch bank, View menu. 1 match")
    );
    f.s.key("escape");
}
