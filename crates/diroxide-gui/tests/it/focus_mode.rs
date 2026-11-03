//! Focus mode and full screen (R27's commit 28): after three seconds of
//! rest the bars around the body fade under a veil, and any key brings
//! them back; F11 runs the app's Full screen (view.full-screen, lane
//! A6), which says "Full screen on." and "Full screen off.". The fade
//! says nothing, and the tree is the same before and after it, the
//! viewer's included. Drawn with Vello's CPU renderer in the harness.

#![allow(clippy::unwrap_used)]

use std::time::Duration;

use diroxide_core::Side;
use diroxide_gui::frontend::FADE_AFTER;
use diroxide_theme::Rgb;

use crate::rig::{HostOf, Rig, SIZE, app_with, fixture};

/// The window with focus mode on (or off) over a small folder.
fn window(focus_mode: bool) -> (tempfile::TempDir, Rig) {
    let dir = fixture(&["notes.txt", "report.pdf"], &["Backup"]);
    let (app, runtime) = app_with(dir.path(), dir.path(), |options| {
        options.settings.focus_mode = focus_mode;
    });
    let rig = Rig::new(app, runtime);
    (dir, rig)
}

/// The window rests `rest` on the test's clock, and its timers run.
fn rest(rig: &mut Rig, rest: Duration) {
    rig.now += rest;
    let now = rig.now;
    rig.f.tick(&mut HostOf(&mut rig.h), now);
    rig.redraw();
}

/// The pixel at `(x, y)` as drawn now.
fn pixel(rig: &mut Rig, x: u32, y: u32) -> Rgb {
    let image = rig.h.render();
    let px = image.get_pixel(x, y).0;
    Rgb::new(px[0], px[1], px[2])
}

/// Whether two colors are within a few steps of each other.
fn near(a: Rgb, b: Rgb) -> bool {
    let d = |x: u8, y: u8| x.abs_diff(y);
    d(a.r, b.r) <= 24 && d(a.g, b.g) <= 24 && d(a.b, b.b) <= 24
}

#[test]
fn the_bars_fade_after_three_seconds_and_the_tree_stays_the_same() {
    let (_dir, mut rig) = window(true);
    let tree = rig.walk();
    let spoken = rig.spoken();
    // The key bar's row, near the window's foot, as drawn before.
    let foot = (40, SIZE.1 - 12);
    let page = rig.f.palette().page;
    let bar_before = pixel(&mut rig, foot.0, foot.1);
    let body = (SIZE.0 / 4, SIZE.1 / 2);
    let body_before = pixel(&mut rig, body.0, body.1);
    // Not yet: two seconds of rest.
    rest(&mut rig, Duration::from_secs(2));
    assert!(!rig.f.veiled());
    // Three seconds: the veil is down over the bars, not over the body.
    rest(&mut rig, FADE_AFTER);
    assert!(rig.f.veiled());
    let bar_after = pixel(&mut rig, foot.0, foot.1);
    assert!(
        near(bar_after, page),
        "{bar_after:?} is not near the page {page:?}"
    );
    assert_eq!(pixel(&mut rig, body.0, body.1), body_before);
    // The same tree, and nothing said.
    assert_eq!(rig.walk(), tree);
    assert_eq!(rig.spoken(), spoken);
    // Any key brings the bars back, and says nothing of them.
    rig.press("down");
    assert!(!rig.f.veiled());
    assert_eq!(pixel(&mut rig, foot.0, foot.1), bar_before);
    assert!(
        !rig.spoken()[spoken.len()..]
            .iter()
            .any(|l| l.to_lowercase().contains("bar")),
        "{:?}",
        rig.spoken()
    );
}

#[test]
fn without_focus_mode_the_bars_stay() {
    let (_dir, mut rig) = window(false);
    rest(&mut rig, FADE_AFTER * 2);
    assert!(!rig.f.veiled());
}

/// The last thing the app said, as text.
fn last_said(rig: &Rig) -> Option<String> {
    rig.said().last().map(|m| m.text.clone())
}

#[test]
fn f11_runs_full_screen_and_the_bars_fade_there_too() {
    let (_dir, mut rig) = window(false);
    rig.press("f11");
    assert!(rig.f.app().full_screen(), "the app's command ran");
    assert!(rig.f.full_screen());
    assert_eq!(rig.f.take_full_screen(), Some(true));
    assert_eq!(rig.f.take_full_screen(), None);
    assert_eq!(last_said(&rig).as_deref(), Some("Full screen on."));
    // Full screen fades the bars as focus mode does, saying nothing, the
    // tree unchanged.
    let tree = rig.walk();
    let said = rig.said().len();
    rest(&mut rig, FADE_AFTER);
    assert!(rig.f.veiled());
    assert_eq!(rig.walk(), tree);
    assert_eq!(rig.said().len(), said, "the fade says nothing");
    // F11 again: back, the bars too.
    rig.press("f11");
    assert!(!rig.f.full_screen());
    assert_eq!(rig.f.take_full_screen(), Some(false));
    assert!(!rig.f.veiled());
    assert_eq!(last_said(&rig).as_deref(), Some("Full screen off."));
    // The View menu and the palette run the same command.
    rig.run("view.full-screen");
    assert!(rig.f.full_screen());
    assert_eq!(rig.f.take_full_screen(), Some(true));
    // A window that runs in the background stays as it is, and the app
    // starts the next Full screen from off.
    rig.f.refuse_full_screen();
    assert!(!rig.f.full_screen() && !rig.f.app().full_screen());
    rig.press("f11");
    assert_eq!(rig.f.take_full_screen(), Some(true));
}

#[test]
fn the_viewers_tree_is_the_same_under_the_veil() {
    let (dir, mut rig) = window(true);
    std::fs::write(dir.path().join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    rig.press("f3");
    assert!(
        rig.settle_until(|rig| rig
            .focus_list()
            .is_some_and(|(name, _)| name.starts_with("Viewer"))),
        "{:?}",
        rig.focus_list()
    );
    let tree = rig.walk();
    rest(&mut rig, FADE_AFTER);
    assert!(rig.f.veiled());
    assert_eq!(rig.walk(), tree);
    rig.press("down");
    assert!(!rig.f.veiled());
}
