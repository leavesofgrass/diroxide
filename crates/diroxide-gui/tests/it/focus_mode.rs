//! Focus mode and full screen (R27's commit 28): after three seconds of
//! rest the bars around the body fade under a veil, and any key brings
//! them back; F11 turns full screen on and off. Nothing is said by
//! either, and the tree is the same before and after the fade, the
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

#[test]
fn f11_is_full_screen_said_by_nothing_and_fading_there_too() {
    let (_dir, mut rig) = window(false);
    let tree = rig.walk();
    let spoken = rig.spoken();
    rig.press("f11");
    assert!(rig.f.full_screen());
    assert_eq!(rig.f.take_full_screen(), Some(true));
    assert_eq!(rig.f.take_full_screen(), None);
    assert_eq!(rig.spoken(), spoken, "nothing is said");
    assert_eq!(rig.walk(), tree);
    // Full screen fades the bars as focus mode does.
    rest(&mut rig, FADE_AFTER);
    assert!(rig.f.veiled());
    assert_eq!(rig.walk(), tree);
    // F11 again: back, the bars too, and still nothing said.
    rig.press("f11");
    assert!(!rig.f.full_screen());
    assert_eq!(rig.f.take_full_screen(), Some(false));
    assert!(!rig.f.veiled());
    assert_eq!(rig.spoken(), spoken);
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
