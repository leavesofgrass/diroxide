//! The live region (R27's commits 10 and 11): Tab says the panel once,
//! importance decides the politeness, the same words twice are two
//! announcements, and the start's lines wait for the tree.

#![allow(clippy::unwrap_used)]

use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::{Message, Place};
use diroxide_gui::frontend::{ANNOUNCER, Frontend};
use diroxide_gui::widgets::announcer::Announcer;
use masonry::accesskit::{Live, Role, TreeUpdate};
use masonry_testing::TestHarness;

use crate::rig::{HostOf, Rig, app_on, fixture, name_of};

/// How many live nodes say `text`, with their politeness.
fn heard(rig: &Rig, text: &str) -> Vec<Live> {
    rig.live()
        .into_iter()
        .filter(|(said, _)| said == text)
        .map(|(_, live)| live)
        .collect()
}

#[test]
fn tab_says_the_other_panel_once_and_the_status_line_is_not_live() {
    let dir = fixture(&["notes.txt", "report.pdf"], &["Backup"]);
    let backup = dir.path().join("Backup");
    std::fs::write(backup.join("old.txt"), b"old").unwrap();
    let (app, runtime) = app_on(dir.path(), &backup);
    let mut rig = Rig::new(app, runtime);
    rig.press("tab");
    assert_eq!(heard(&rig, "Right panel: Backup, 1 item"), [Live::Polite]);
    // The status line shows it too, in a node that is not live, so it is
    // not said a second time.
    let status = rig
        .walk()
        .into_iter()
        .filter(|(role, name)| *role == Role::Label && name == "Right panel: Backup, 1 item")
        .count();
    assert_eq!(status, 2, "the live node and the status line's");
    let live_labels = rig.live().len();
    assert!(live_labels <= diroxide_gui::widgets::announcer::KEEP);
    // The focus is the right panel's first row.
    let (role, name) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(name.starts_with("old.txt"), "{name}");
}

#[test]
fn normal_is_polite_high_is_assertive_and_low_is_only_shown() {
    let dir = fixture(&["a.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    rig.h.edit_widget(ANNOUNCER, |mut a| {
        Announcer::say(
            &mut a,
            [
                Message::status("Reading 40 percent"),
                Message::info("Copy finished: 3 done"),
                Message::error("Could not copy report.pdf: the disk is full. Free some space."),
            ],
        );
    });
    rig.redraw();
    assert!(heard(&rig, "Reading 40 percent").is_empty());
    assert_eq!(heard(&rig, "Copy finished: 3 done"), [Live::Polite]);
    assert_eq!(
        heard(
            &rig,
            "Could not copy report.pdf: the disk is full. Free some space."
        ),
        [Live::Assertive]
    );
}

#[test]
fn the_same_words_twice_are_two_new_nodes() {
    let dir = fixture(&["a.txt", "b.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    // Right on a file says what it is, each time.
    rig.press("right");
    rig.press("right");
    let said: Vec<&str> = rig
        .said()
        .iter()
        .map(|m| m.text.as_str())
        .filter(|t| t.starts_with("Not a folder: a.txt"))
        .collect();
    assert_eq!(said.len(), 2, "{said:?}");
    let text = said[0].to_owned();
    let state = rig.h.access_tree().state();
    let mut ids = Vec::new();
    let mut stack = vec![state.root()];
    while let Some(node) = stack.pop() {
        if node.live() != Live::Off && name_of(&node) == text {
            ids.push(node.id());
        }
        stack.extend(node.children());
    }
    assert_eq!(ids.len(), 2, "two nodes, so two announcements");
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn the_starts_lines_wait_for_the_tree_and_are_said_once() {
    let dir = fixture(&["a.txt"], &[]);
    let place = Place::parse(dir.path().to_str().unwrap()).unwrap();
    let mut options = AppOptions::new(place.clone(), place);
    options.startup.first_run = true;
    let app = App::new(options);
    let mut f = Frontend::new(app, Runtime::with_local(), "diroxide");
    f.start();
    let welcome = f
        .said()
        .iter()
        .find(|m| m.text.starts_with("diroxide "))
        .map(|m| m.text.clone())
        .unwrap();
    let start_lines = f.said().len();
    assert!(start_lines >= 3, "{:?}", f.said());
    // The harness's first tree is the first a screen reader gets: the
    // start's lines are held out of it.
    let mut h = TestHarness::create_with_size(
        masonry::theme::default_property_set(),
        f.root_widget(),
        crate::rig::SIZE,
    );
    let first: Vec<String> = live_values(&h.redraw().1);
    assert!(first.iter().all(|t| !t.contains(&welcome)), "{first:?}");
    // The driver's next turn releases them as one new live node, the
    // welcome first and the lines after it in their order.
    f.refresh(&mut HostOf(&mut h));
    let (_, update) = h.redraw();
    let released = live_values(&update);
    assert_eq!(released.len(), 1, "{released:?}");
    assert!(released[0].starts_with(&welcome), "{released:?}");
    for message in f.said() {
        assert!(released[0].contains(&message.text), "{released:?}");
    }
    // Said once: a later update does not send them as new.
    f.refresh(&mut HostOf(&mut h));
    let _ = h.redraw();
    assert_eq!(h.get_widget(ANNOUNCER).inner().announced(), 1);
}

/// The texts of the live nodes an update sends.
fn live_values(update: &TreeUpdate) -> Vec<String> {
    update
        .nodes
        .iter()
        .filter(|(_, node)| node.live().is_some_and(|l| l != Live::Off))
        .filter_map(|(_, node)| node.value().map(str::to_owned))
        .collect()
}
