//! The tabs, driven through the runtime on this crate's own folders: the
//! left panel opens on the crate, the right on its `src` folder.

use super::*;
use crate::app::{AppEvent, AppOptions};
use crate::runtime::Runtime;
use diroxide_config::PanelState;
use diroxide_core::{FixedClock, Key, Timestamp, UtcOffset};
use diroxide_vfs::WatchEvent;

fn key(text: &str) -> Key {
    Key::parse(text).unwrap()
}

fn here() -> Place {
    Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap()
}

fn src() -> Place {
    here().join("src").unwrap()
}

/// What the app said, without the watcher's notice: where a folder
/// cannot be watched (the development container), a tab that lists a
/// folder also says "Folder not watched. Reload with Ctrl+R.", which is
/// the machine's line and not the tabs'.
fn settle(app: &mut App, runtime: &mut Runtime, effects: Vec<Effect>) -> Vec<String> {
    runtime
        .run_until_idle(app, effects)
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Announce(message) => Some(message.text),
            _ => None,
        })
        .filter(|line| !line.starts_with("Folder not watched."))
        .collect()
}

fn options() -> AppOptions {
    let mut options = AppOptions::new(here(), src());
    options.clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 11, 12, 10, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    // The clock stands still, so a question takes its answer at once.
    options.settings.dialog_settle_ms = 0;
    options
}

fn started(options: AppOptions) -> (App, Runtime) {
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local();
    let start = app.start();
    settle(&mut app, &mut runtime, start);
    (app, runtime)
}

fn run(app: &mut App, runtime: &mut Runtime, id: CommandId) -> Vec<String> {
    let effects = app.dispatch(&Command::new(id));
    settle(app, runtime, effects)
}

fn press(app: &mut App, runtime: &mut Runtime, text: &str) -> Vec<String> {
    let effects = app.handle(AppEvent::Key(key(text)));
    settle(app, runtime, effects)
}

/// Goes into the `src` folder in the left panel's current tab.
fn into_src(app: &mut App, runtime: &mut Runtime) -> Vec<String> {
    let effects = app.open(Side::Left, src(), None);
    settle(app, runtime, effects)
}

fn fits(line: &str) {
    assert!(line.chars().count() <= 40, "{line}: over 40 cells");
}

#[test]
fn a_new_tab_is_a_copy_and_every_tab_keeps_its_own_place() {
    let (mut app, mut runtime) = started(options());
    assert_eq!(app.tab_count(Side::Left), 1);
    assert_eq!(app.tab_title_words(Side::Left), None);
    let said = press(&mut app, &mut runtime, "ctrl+t");
    assert_eq!(said, ["New tab: diroxide-app, tab 2 of 2"]);
    fits(&said[0]);
    assert_eq!(app.tab_count(Side::Left), 2);
    assert_eq!(app.tab_index(Side::Left), 1);
    let title = app.title_line();
    assert!(
        title.starts_with("diroxide-app, tab 2 of 2, left panel, in "),
        "{title}"
    );
    // The new tab goes elsewhere; the first keeps its place.
    let said = into_src(&mut app, &mut runtime);
    assert!(said[0].starts_with("src, tab 2 of 2, "), "{said:?}");
    assert!(said[0].contains(" items, in "), "{said:?}");
    let said = run(&mut app, &mut runtime, ids::TAB_PREVIOUS);
    // Where a folder cannot be watched (the development container), a
    // second line says so; the tab line is the first.
    assert_eq!(said[0], "diroxide-app, tab 1 of 2", "{said:?}");
    assert_eq!(app.panel(Side::Left).place(), &here());
    assert!(!app.panel(Side::Left).is_loading());
    let said = run(&mut app, &mut runtime, ids::TAB_NEXT);
    assert_eq!(said, ["src, tab 2 of 2"]);
    assert_eq!(app.panel(Side::Left).place(), &src());
    // Each tab its own history: back in this tab goes to the crate.
    assert_eq!(app.panel(Side::Left).history(), [here()]);
    assert_eq!(
        app.tab_rows(Side::Left),
        ["diroxide-app, tab 1 of 2", "src, tab 2 of 2"]
    );
    // Round the end.
    let said = run(&mut app, &mut runtime, ids::TAB_NEXT);
    // Where a folder cannot be watched (the development container), a
    // second line says so; the tab line is the first.
    assert_eq!(said[0], "diroxide-app, tab 1 of 2", "{said:?}");
}

#[test]
fn the_gui_keys_are_the_decided_ones_and_a_terminal_has_the_menu() {
    let info = |id| crate::commands::info(&id).unwrap();
    assert_eq!(info(ids::TAB_NEW).gui, ["ctrl+t"]);
    assert_eq!(info(ids::TAB_CLOSE).gui, ["ctrl+w"]);
    assert_eq!(info(ids::TAB_NEXT).gui, ["ctrl+tab"]);
    assert_eq!(info(ids::TAB_PREVIOUS).gui, ["ctrl+shift+tab"]);
    assert_eq!(info(ids::TAB_GO_3).gui, ["alt+3"]);
    assert!(info(ids::TAB_NEXT).terminal.is_empty());
    assert!(info(ids::TAB_LIST).palette_only);
}

#[test]
fn each_tab_keeps_its_own_cursor_and_selection() {
    let (mut app, mut runtime) = started(options());
    press(&mut app, &mut runtime, "c");
    press(&mut app, &mut runtime, "space");
    assert_eq!(app.panel(Side::Left).targets().len(), 1);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    press(&mut app, &mut runtime, "space");
    assert_eq!(app.panel(Side::Left).list().selected_count(), 0);
    run(&mut app, &mut runtime, ids::TAB_GO_1);
    let panel = app.panel(Side::Left);
    assert_eq!(panel.list().selected_count(), 1);
    assert_eq!(panel.list().focused().unwrap().id, "Cargo.toml");
}

#[test]
fn a_parked_tabs_late_listing_changes_nothing_and_it_fills_on_return() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_NEW);
    // Tab 2 starts a listing of src and is parked before it answers.
    let slow = app.open(Side::Left, src(), None);
    let switch = app.dispatch(&Command::new(ids::TAB_PREVIOUS));
    let said = settle(&mut app, &mut runtime, switch);
    // Where a folder cannot be watched (the development container), a
    // second line says so; the tab line is the first.
    assert_eq!(said[0], "diroxide-app, tab 1 of 2", "{said:?}");
    // The late answer for tab 2 lands while tab 1 is shown.
    settle(&mut app, &mut runtime, slow);
    assert_eq!(app.panel(Side::Left).place(), &here());
    assert!(
        app.panel(Side::Left)
            .list()
            .rows()
            .iter()
            .any(|r| r.id == "src")
    );
    // Shown again, tab 2 lists src after all.
    let said = run(&mut app, &mut runtime, ids::TAB_NEXT);
    assert!(
        said.iter().any(|s| s.starts_with("src, tab 2 of 2")),
        "{said:?}"
    );
    assert_eq!(app.panel(Side::Left).place(), &src());
    assert!(
        app.panel(Side::Left)
            .list()
            .rows()
            .iter()
            .any(|r| r.id == "app.rs")
    );
}

#[test]
fn a_watch_event_for_a_parked_tabs_folder_leaves_the_shown_tab_alone() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_NEW);
    into_src(&mut app, &mut runtime);
    let before = app.panel(Side::Left).entries_generation();
    let effects = app.handle(AppEvent::Watched {
        side: Side::Left,
        place: here(),
        event: WatchEvent::Changed(here().join("Cargo.toml").unwrap()),
    });
    let said = settle(&mut app, &mut runtime, effects);
    assert!(said.is_empty(), "{said:?}");
    assert_eq!(app.panel(Side::Left).place(), &src());
    assert_eq!(app.panel(Side::Left).entries_generation(), before);
}

#[test]
fn closing_says_where_it_lands_the_last_tab_stays_and_undo_brings_it_back() {
    let (mut app, mut runtime) = started(options());
    let said = run(&mut app, &mut runtime, ids::TAB_CLOSE);
    assert_eq!(said, ["Last tab stays open: diroxide-app."]);
    fits(&said[0]);
    assert_eq!(app.tab_count(Side::Left), 1);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    into_src(&mut app, &mut runtime);
    let said = press(&mut app, &mut runtime, "ctrl+w");
    assert_eq!(said, ["Closed src. diroxide-app."]);
    assert_eq!(app.tab_count(Side::Left), 1);
    assert_eq!(app.panel(Side::Left).place(), &here());
    let said = run(&mut app, &mut runtime, ids::TAB_UNDO_CLOSE);
    assert_eq!(said, ["Reopened src, tab 2 of 2"]);
    fits(&said[0]);
    assert_eq!(app.panel(Side::Left).place(), &src());
    let said = run(&mut app, &mut runtime, ids::TAB_UNDO_CLOSE);
    assert_eq!(said, ["No closed tab to bring back."]);
}

#[test]
fn a_locked_tab_asks_before_closing_and_opens_a_new_tab_to_move() {
    let (mut app, mut runtime) = started(options());
    let said = run(&mut app, &mut runtime, ids::TAB_LOCK);
    assert_eq!(said, ["diroxide-app, locked"]);
    let said = into_src(&mut app, &mut runtime);
    assert_eq!(said[0], "Tab locked: opening in tab 2.");
    fits(&said[0]);
    assert!(said[1].starts_with("src, tab 2 of 2, "), "{said:?}");
    assert_eq!(app.tab_lock(Side::Left), TabLock::Free);
    assert_eq!(
        app.tab_rows(Side::Left),
        ["diroxide-app, tab 1 of 2, locked", "src, tab 2 of 2"]
    );
    run(&mut app, &mut runtime, ids::TAB_PREVIOUS);
    assert_eq!(app.panel(Side::Left).place(), &here());
    let title = app.title_line();
    assert!(
        title.starts_with("diroxide-app, tab 1 of 2, locked, left panel"),
        "{title}"
    );
    // Close asks, starting on No; No keeps the tab.
    let said = run(&mut app, &mut runtime, ids::TAB_CLOSE);
    assert_eq!(app.dialog_purpose(), Some("TabCloseLocked"));
    assert!(
        said[0].starts_with("Close locked tab diroxide-app? No is the default."),
        "{said:?}"
    );
    let said = press(&mut app, &mut runtime, "n");
    assert_eq!(said, ["Nothing closed; the tab stays."]);
    assert_eq!(app.tab_count(Side::Left), 2);
    run(&mut app, &mut runtime, ids::TAB_CLOSE);
    let said = press(&mut app, &mut runtime, "y");
    assert_eq!(said[0], "Closed diroxide-app. src.");
    assert_eq!(app.tab_count(Side::Left), 1);
}

#[test]
fn close_other_tabs_keeps_the_locked_ones() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_LOCK);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    run(&mut app, &mut runtime, ids::TAB_LOCK);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    assert_eq!(app.tab_count(Side::Left), 4);
    let said = run(&mut app, &mut runtime, ids::TAB_CLOSE_OTHERS);
    assert_eq!(said, ["Closed 1 tab. Kept 2 locked tabs."]);
    fits(&said[0]);
    assert_eq!(app.tab_count(Side::Left), 3);
    assert_eq!(app.tab_index(Side::Left), 2);
}

#[test]
fn a_pinned_tab_remembers_its_base_folder() {
    let (mut app, mut runtime) = started(options());
    let said = run(&mut app, &mut runtime, ids::TAB_BASE);
    assert_eq!(said, ["Not a pinned tab: it has no base folder."]);
    fits(&said[0]);
    let said = run(&mut app, &mut runtime, ids::TAB_PIN);
    assert_eq!(said, ["diroxide-app, pinned"]);
    assert_eq!(app.tab_base(Side::Left), Some(&here()));
    // Pinned, it changes folder freely, in the same tab.
    into_src(&mut app, &mut runtime);
    assert_eq!(app.tab_count(Side::Left), 1);
    let said = run(&mut app, &mut runtime, ids::TAB_BASE);
    assert!(said[0].starts_with("diroxide-app, "), "{said:?}");
    assert_eq!(app.panel(Side::Left).place(), &here());
    let said = run(&mut app, &mut runtime, ids::TAB_BASE);
    assert_eq!(said, ["Already in the base folder, diroxide-app."]);
    let said = run(&mut app, &mut runtime, ids::TAB_PIN);
    assert_eq!(said, ["diroxide-app, free"]);
    assert_eq!(app.tab_base(Side::Left), None);
}

#[test]
fn tabs_move_left_and_right_and_go_by_number() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_NEW);
    into_src(&mut app, &mut runtime);
    let said = run(&mut app, &mut runtime, ids::TAB_MOVE_RIGHT);
    assert_eq!(said, ["Already the last tab."]);
    let said = run(&mut app, &mut runtime, ids::TAB_MOVE_LEFT);
    assert_eq!(said, ["src, tab 1 of 2"]);
    assert_eq!(
        app.tab_rows(Side::Left),
        ["src, tab 1 of 2", "diroxide-app, tab 2 of 2"]
    );
    let said = run(&mut app, &mut runtime, ids::TAB_GO_9);
    assert_eq!(said, ["No tab 9: this panel has 2 tabs."]);
    fits(&said[0]);
    let said = run(&mut app, &mut runtime, ids::TAB_GO_2);
    assert_eq!(said, ["diroxide-app, tab 2 of 2"]);
}

#[test]
fn the_tab_list_is_a_chooser_of_rows_name_first() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_NEW);
    into_src(&mut app, &mut runtime);
    let said = run(&mut app, &mut runtime, ids::TAB_LIST);
    assert_eq!(app.dialog_purpose(), Some("TabList"));
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no chooser");
    };
    assert_eq!(list.title(), "Tabs, left panel, 2 tabs");
    assert_eq!(list.focused().unwrap().label, "src, tab 2 of 2");
    assert!(
        said.iter().any(|s| s.contains("src, tab 2 of 2")),
        "{said:?}"
    );
    press(&mut app, &mut runtime, "up");
    let said = press(&mut app, &mut runtime, "enter");
    assert!(app.dialog().is_none());
    assert_eq!(said.last().unwrap(), "diroxide-app, tab 1 of 2");
    assert_eq!(app.panel(Side::Left).place(), &here());
}

#[test]
fn a_tab_moves_or_copies_to_the_other_panel_and_becomes_current_there() {
    let (mut app, mut runtime) = started(options());
    let said = run(&mut app, &mut runtime, ids::TAB_TO_OTHER);
    assert_eq!(said, ["Only one tab here: it cannot move."]);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    into_src(&mut app, &mut runtime);
    let said = run(&mut app, &mut runtime, ids::TAB_TO_OTHER);
    assert_eq!(said[0], "Moved src to the right panel, tab 2 of 2");
    assert_eq!(app.active_side(), Side::Right);
    assert_eq!(app.tab_count(Side::Left), 1);
    assert_eq!(app.tab_count(Side::Right), 2);
    assert_eq!(app.panel(Side::Right).side(), Side::Right);
    assert_eq!(app.panel(Side::Right).place(), &src());
    assert_eq!(app.panel(Side::Left).place(), &here());
    let said = run(&mut app, &mut runtime, ids::TAB_COPY_TO_OTHER);
    assert_eq!(said[0], "Copied src to the left panel, tab 2 of 2");
    assert_eq!(app.tab_count(Side::Right), 2);
    assert_eq!(app.tab_count(Side::Left), 2);
    assert_eq!(app.panel(Side::Left).side(), Side::Left);
    assert_eq!(app.panel(Side::Left).place(), &src());
}

#[test]
fn a_job_that_ends_after_a_tab_switch_only_speaks() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_NEW);
    let tab = app.panel(Side::Left).tab_id();
    app.tabs.job_tabs.insert(JobId::new(77), (Side::Left, tab));
    assert!(!app.job_left_its_tab(JobId::new(77), Side::Left));
    // Straight to the switch, checked before the runtime answers: the
    // app forgets a job with no flow after each event.
    let effects = app.switch_tab(Side::Left, 0);
    assert!(app.job_left_its_tab(JobId::new(77), Side::Left));
    assert!(!app.job_left_its_tab(JobId::new(78), Side::Left));
    settle(&mut app, &mut runtime, effects);
}

#[test]
fn the_session_keeps_every_tab_and_restores_them_at_start() {
    let (mut app, mut runtime) = started(options());
    run(&mut app, &mut runtime, ids::TAB_LOCK);
    into_src(&mut app, &mut runtime);
    run(&mut app, &mut runtime, ids::TAB_PREVIOUS);
    let session = app.session();
    assert_eq!(session.version, 2);
    let (tabs, current) = session.tab_list(Side::Left);
    assert_eq!(tabs.len(), 2);
    assert_eq!(current, 0);
    // The panel's own state is the current tab's, as version 1 wrote it.
    assert_eq!(session.panel(Side::Left).place.as_ref(), Some(&here()));
    assert_eq!(tabs[0].lock, TabLock::Locked);
    assert_eq!(tabs[1].place.as_ref(), Some(&src()));
    assert!(session.tabs[1].is_empty());
    // A session written as text and read back starts the same tabs.
    let text = session.to_toml().unwrap();
    let mut again = options();
    again.startup.session = Some(SessionState::from_toml(&text).unwrap());
    let (mut app, mut runtime) = started(again);
    assert_eq!(app.tab_count(Side::Left), 2);
    assert_eq!(app.tab_lock(Side::Left), TabLock::Locked);
    let said = run(&mut app, &mut runtime, ids::TAB_NEXT);
    // A restored tab is listed when it is first shown, and its arrival
    // says the tab.
    assert!(said[0].starts_with("src, tab 2 of 2, "), "{said:?}");
    assert_eq!(app.panel(Side::Left).place(), &src());
}

#[test]
fn a_session_opened_elsewhere_keeps_its_tabs_beside_the_new_place() {
    let tab = |place: Place| TabState {
        place: Some(place),
        ..TabState::default()
    };
    let mut session = SessionState::default();
    session.set_tabs(
        Side::Left,
        vec![tab(src()), tab(src().join("app").unwrap())],
        1,
    );
    let mut options = options();
    options.startup.session = Some(session);
    let (app, _) = started(options);
    assert_eq!(app.tab_count(Side::Left), 3);
    assert_eq!(app.tab_index(Side::Left), 2);
    assert_eq!(app.panel(Side::Left).place(), &here());
    assert_eq!(
        app.tab_rows(Side::Left),
        [
            "src, tab 1 of 3",
            "app, tab 2 of 3",
            "diroxide-app, tab 3 of 3"
        ]
    );
}

#[test]
fn a_version_one_session_opens_one_tab_a_panel() {
    let mut session = SessionState {
        version: 1,
        ..SessionState::default()
    };
    *session.panel_mut(Side::Left) = PanelState {
        place: Some(here()),
        ..PanelState::default()
    };
    let mut options = options();
    options.startup.session = Some(session);
    let (app, _) = started(options);
    assert_eq!(app.tab_count(Side::Left), 1);
    assert_eq!(app.tab_count(Side::Right), 1);
}

#[test]
fn a_tab_set_is_saved_by_name_and_loads_in_place_of_the_tabs() {
    let (mut app, mut runtime) = started(options());
    let said = run(&mut app, &mut runtime, ids::TAB_LOAD_SET);
    assert_eq!(said, ["No tab sets saved yet."]);
    fits(&said[0]);
    run(&mut app, &mut runtime, ids::TAB_NEW);
    into_src(&mut app, &mut runtime);
    run(&mut app, &mut runtime, ids::TAB_SAVE_SET);
    assert_eq!(app.dialog_purpose(), Some("TabSetName"));
    let effects = app.save_set_named("  ");
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, ["Type a name for the tab set."]);
    let effects = app.save_set_named("Work");
    let said = settle(&mut app, &mut runtime, effects);
    assert_eq!(said, ["Saved tab set Work, 3 tabs."]);
    fits(&said[0]);
    assert!(app.dialog().is_none());
    assert_eq!(app.tab_sets().get("work").unwrap().tab_count(), 3);
    // Close the second tab, then load the set back.
    run(&mut app, &mut runtime, ids::TAB_CLOSE);
    assert_eq!(app.tab_count(Side::Left), 1);
    run(&mut app, &mut runtime, ids::TAB_LOAD_SET);
    assert_eq!(app.dialog_purpose(), Some("TabSets"));
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no chooser");
    };
    assert_eq!(list.focused().unwrap().label, "Work, 3 tabs");
    let said = press(&mut app, &mut runtime, "enter");
    assert_eq!(said[0], "Tab set Work loaded, 3 tabs");
    fits(&said[0]);
    assert_eq!(app.tab_count(Side::Left), 2);
    assert_eq!(app.tab_index(Side::Left), 1);
    assert_eq!(app.panel(Side::Left).place(), &src());
    assert!(app.panel(Side::Left).is_listed());
    // The tabs it replaced wait on the closed stack.
    let said = run(&mut app, &mut runtime, ids::TAB_UNDO_CLOSE);
    assert!(said[0].starts_with("Reopened "), "{said:?}");
    assert_eq!(app.tab_count(app.active_side()), 3);
}
