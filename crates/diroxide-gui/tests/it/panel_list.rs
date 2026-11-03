//! The panel's list box (R27's commit 9, ADR-0007): options named by the
//! row's label and details, the states as states, absolute positions, the
//! real set size, the focused row as the active descendant, the clip flag
//! cleared, marks that follow ids, and only a window of rows as nodes
//! (spike S3: 100,000 rows).

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use accesskit_consumer::{FilterResult, common_filter};
use diroxide_app::AppEvent;
use diroxide_core::{Entry, EntryKind, Place, Side};
use masonry::accesskit::Role;

use crate::rig::{HostOf, Rig, app_on, fixture, name_of};

/// Lists `n` files named "file-00001.txt" and on into the left panel at a
/// place under `root`, answering the app's listing request directly, so
/// no folder of `n` files is ever written.
pub fn list_synthetic(rig: &mut Rig, root: &std::path::Path, n: usize) -> Place {
    let place = Place::parse(root.join("synthetic").to_str().unwrap()).unwrap();
    let _ = rig.f.app_mut().open_place(Side::Left, place.clone());
    // The request is answered here, never by the disk.
    let _ = rig.f.app_mut().take_requests();
    let request = rig.f.app().panel(Side::Left).loading_request().unwrap();
    let entries: Vec<Entry> = (1..=n)
        .map(|i| Entry::new(format!("file-{i:06}.txt"), EntryKind::File).with_size(i as u64))
        .collect();
    let _ = rig.f.app_mut().handle(AppEvent::Listed {
        request,
        place: place.clone(),
        result: Ok(entries),
    });
    rig.f.refresh(&mut HostOf(&mut rig.h));
    rig.redraw();
    place
}

/// An option as the tree has it: its name, its position (zero-based in
/// AccessKit), and its selection state.
type OptionNode = (String, Option<usize>, Option<bool>);

/// The left list box's size and its options as the tree has them.
fn list_options(rig: &Rig) -> (usize, Vec<OptionNode>) {
    let id = rig.panel_id(Side::Left);
    let list = rig.h.access_node(id).unwrap();
    assert_eq!(list.role(), Role::ListBox);
    let size = list.size_of_set().unwrap();
    let options = list
        .children()
        .filter(|o| o.role() == Role::ListBoxOption)
        .map(|o| (name_of(&o), o.position_in_set(), o.is_selected()))
        .collect();
    (size, options)
}

#[test]
fn every_windowed_option_is_a_node_with_its_place_and_the_real_count() {
    let dir = fixture(&[], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    list_synthetic(&mut rig, dir.path(), 40);
    let (size, options) = list_options(&rig);
    assert_eq!(size, 40);
    assert_eq!(options.len(), 40, "a list shorter than the window is whole");
    // The name is the label and the details, never the position.
    assert!(options[0].0.starts_with("file-000001.txt, "), "{options:?}");
    assert!(!options[0].0.contains("of 40"), "{options:?}");
    for (i, (_, position, _)) in options.iter().enumerate() {
        assert_eq!(*position, Some(i), "zero-based in AccessKit");
    }
    // The clip flag is cleared, so AccessKit's filter (the adapters')
    // keeps every option, not only those in view.
    let list = rig.h.access_node(rig.panel_id(Side::Left)).unwrap();
    assert!(!list.clips_children());
    let kept = list
        .children()
        .filter(|o| common_filter(o) == FilterResult::Include)
        .count();
    assert_eq!(kept, 40);
    assert!(list.is_multiselectable());
    // The focused row is the active descendant, and so the screen
    // reader's focus.
    let (role, name) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(name.starts_with("file-000001.txt"), "{name}");
    rig.press("down");
    rig.press("down");
    let (_, name) = rig.focus_node();
    assert!(name.starts_with("file-000003.txt"), "{name}");
}

/// How a marked row reads on this system: on Windows and Linux the
/// option is selected and the word is not in its name; on macOS, where
/// VoiceOver reads a list's options as static text, the word is in the
/// name and there is no state (`model::states_in_name`). An unmarked row
/// has neither, so nothing says "not selected" on every row.
fn assert_mark(option: &OptionNode, marked: bool) {
    let (name, _, state) = option;
    let word = name.contains(", selected");
    if diroxide_gui::model::states_in_name() {
        assert_eq!(*state, None, "no state on macOS: {option:?}");
        assert_eq!(word, marked, "the word in the name on macOS: {option:?}");
    } else {
        assert_eq!(*state, marked.then_some(true), "the state: {option:?}");
        assert!(!word, "the state is not in the name: {option:?}");
    }
}

/// Both forms of a marked row, whatever the system the tests run on:
/// the word in the name with no state (macOS), the state with no word
/// (Windows and Linux).
#[test]
fn a_marked_row_has_its_state_or_its_word_never_both() {
    let mut row = diroxide_core::ListRow::new("b.txt", "b.txt").with_details(["text", "8 bytes"]);
    row.selected = true;
    let mac = diroxide_gui::model::RowView::of(&row, true);
    assert!(mac.name.contains(", selected"), "{mac:?}");
    let other = diroxide_gui::model::RowView::of(&row, false);
    assert!(!other.name.contains("selected"), "{other:?}");
    assert!(mac.selected && other.selected);
    // The drawn row says it in words on every system.
    assert!(other.text.starts_with("* "), "{other:?}");
}

#[test]
fn a_mark_is_a_state_said_once_and_follows_its_row_across_a_refresh() {
    let dir = fixture(&["a.txt", "b.txt", "c.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "b.txt"));
    rig.f.refresh(&mut HostOf(&mut rig.h));
    rig.press("space");
    let (_, options) = list_options(&rig);
    let b = options.iter().find(|o| o.0.starts_with("b.txt")).unwrap();
    assert_mark(b, true);
    let a = options.iter().find(|o| o.0.starts_with("a.txt")).unwrap();
    assert_mark(a, false);
    // A new file arrives before it: the mark stays on b.txt's node.
    let list = rig.h.access_node(rig.panel_id(Side::Left)).unwrap();
    let b_node = list
        .children()
        .find(|o| name_of(o).starts_with("b.txt"))
        .unwrap()
        .id();
    std::fs::write(dir.path().join("aa.txt"), b"new").unwrap();
    rig.press("ctrl+r");
    // The listing comes back from a worker: wait for the new row, not
    // for a time.
    assert!(
        rig.settle_until(|rig| list_options(rig)
            .1
            .iter()
            .any(|o| o.0.starts_with("aa.txt"))),
        "aa.txt never arrived: {:?}",
        list_options(&rig).1
    );
    let list = rig.h.access_node(rig.panel_id(Side::Left)).unwrap();
    let b_again = list
        .children()
        .find(|o| name_of(o).starts_with("b.txt"))
        .unwrap();
    assert_eq!(b_again.id(), b_node, "the row keeps its node");
    assert_mark(
        &(
            name_of(&b_again),
            b_again.position_in_set(),
            b_again.is_selected(),
        ),
        true,
    );
    assert_eq!(b_again.position_in_set(), Some(2));
}

/// The median of some durations.
fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
fn spike_s3_a_hundred_thousand_rows_are_a_window_of_nodes() {
    let dir = fixture(&[], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    let started = Instant::now();
    list_synthetic(&mut rig, dir.path(), 100_000);
    let listed = started.elapsed();
    let (size, options) = list_options(&rig);
    assert_eq!(size, 100_000, "the set size is the real count");
    let page = rig.f.app().panel(Side::Left).list().page_size();
    let nodes = options.len();
    assert!(
        nodes <= 5 * page + 1 && nodes < 400,
        "only a window of rows is in the tree: {nodes} nodes, {page} rows a page"
    );
    let mut presses = Vec::new();
    for _ in 0..20 {
        let at = Instant::now();
        rig.press("down");
        presses.push(at.elapsed());
    }
    let at = Instant::now();
    rig.press("end");
    let end = at.elapsed();
    let (_, options) = list_options(&rig);
    let last = options.last().unwrap();
    assert!(last.0.starts_with("file-100000.txt"), "{last:?}");
    assert_eq!(last.1, Some(99_999), "absolute positions at the end");
    let (_, focused) = rig.focus_node();
    assert!(focused.starts_with("file-100000.txt"), "{focused}");
    // A refresh: the whole list again, the focus kept.
    let at = Instant::now();
    rig.f.refresh(&mut HostOf(&mut rig.h));
    rig.redraw();
    let refresh = at.elapsed();
    let press = median(presses);
    eprintln!(
        "S3: {size} rows, {nodes} option nodes ({page} rows a page); listing and first tree {listed:?}; key press median {press:?}; End {end:?}; refresh {refresh:?}"
    );
    // The goal is under 16 ms for a key press in a release build; a debug
    // test build on a loaded machine is held to a looser line.
    assert!(
        press < Duration::from_millis(250),
        "a key press took {press:?}"
    );
}
