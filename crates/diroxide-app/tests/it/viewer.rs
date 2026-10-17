//! The viewer wired into the app (plan commit 63): F3 opens the focused
//! file, inside an archive too; the viewer layer's keys; Escape goes back
//! to the row.

use diroxide_app::commands::ids;
use diroxide_vfs::Registry;

use crate::archives::demo;
use crate::support::{Session, Setup, place, temp};

fn settle_viewer(s: &mut Session) -> Vec<String> {
    let mut said = Vec::new();
    for _ in 0..1_000 {
        if s.app.viewer_wants_tick().is_none() {
            break;
        }
        let effects = s.app.viewer_tick();
        said.extend(s.run(effects));
    }
    said
}

#[test]
fn f3_views_a_member_of_an_archive_and_escape_goes_back_to_its_row() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = Session::open(
        place(&demo),
        place(&demo.join("out")),
        Registry::with_local(),
        Setup::default(),
    );
    s.focus("course.zip");
    s.key("enter");
    s.focus("week1");
    s.key("enter");
    s.focus("a.txt");
    let said = s.key("f3");
    assert_eq!(
        said.first().map(String::as_str),
        Some("a.txt, in course.zip!week1")
    );
    settle_viewer(&mut s);
    assert!(s.app.viewer_open());
    assert_eq!(s.app.title_line(), "a.txt, in course.zip!week1");
    assert_eq!(s.key("ctrl+d"), ["Line 1 of 2, 50 percent"]);
    assert!(
        s.app
            .key_bar()
            .starts_with("Escape Close  / Search  N Next"),
        "{}",
        s.app.key_bar()
    );
    // Search: the prompt, then the count; next; a word not there.
    let said = s.key("/");
    assert_eq!(said, ["Search for, empty"]);
    s.type_text("line");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Match 1 of 2, line 1")
    );
    let said = s.key("n");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Match 2 of 2, line 2")
    );
    s.key("/");
    s.type_text("zebra");
    let said = s.key("enter");
    assert_eq!(said.last().map(String::as_str), Some("No match for zebra."));
    // Hex mode and Say byte.
    let said = s.key("h");
    assert!(said.iter().any(|l| l.starts_with("Hex mode")), "{said:?}");
    let said = s.key("b");
    assert!(said[0].starts_with("Byte "), "{said:?}");
    s.key("h");
    // Next file, in the same viewer.
    let said = s.key("alt+n");
    assert_eq!(said.first().map(String::as_str), Some("b.txt, 2 of 3"));
    settle_viewer(&mut s);
    assert_eq!(
        s.app.viewer().map(|v| v.name().to_owned()).as_deref(),
        Some("b.txt")
    );
    // A panel's command waits for the viewer to close.
    assert_eq!(s.key("f5"), ["Close the viewer first. Escape closes it."]);
    let said = s.key("escape");
    assert_eq!(said, ["Viewer closed: b.txt"]);
    assert!(!s.app.viewer_open());
    assert_eq!(s.focused(), "b.txt");
    // With the viewer closed, its commands say how to open it.
    assert_eq!(
        s.command(ids::VIEWER_HEX),
        ["The viewer is not open. F3 opens it."]
    );
}

#[test]
fn single_letters_off_leaves_the_viewer_to_its_menu_and_the_palette() {
    let dir = temp();
    let demo = demo(dir.path());
    std::fs::write(demo.join("notes.txt"), "one\ntwo two\nthree\n").unwrap();
    let mut settings = crate::support::settings();
    settings.single_letters = diroxide_config::SingleLetters::Off;
    let keymap = diroxide_app::Keymap::defaults().with_single_letters(settings.single_letters);
    let mut s = Session::open_with_keymap(
        place(&demo),
        place(&demo.join("out")),
        Registry::with_local(),
        Setup {
            settings,
            ..Setup::default()
        },
        keymap,
    );
    s.focus("notes.txt");
    s.key("f3");
    settle_viewer(&mut s);
    assert!(s.key("h").is_empty(), "letters are off");
    s.key("ctrl+p");
    s.type_text("search in file");
    let said = s.key("enter");
    assert_eq!(said.last().map(String::as_str), Some("Search for, empty"));
    s.type_text("two");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Match 1 of 2, line 2")
    );
}
