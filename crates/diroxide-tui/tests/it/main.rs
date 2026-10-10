//! Integration tests of the terminal frontend's screen.

use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::Place;
use diroxide_tui::{TuiOptions, render};

#[test]
fn a_narrow_screen_still_starts_with_the_title() {
    let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
    let mut app = App::new(AppOptions::new(here.clone(), here));
    let mut runtime = Runtime::with_local();
    let start = app.start();
    runtime.run_until_idle(&mut app, start);
    let options = TuiOptions {
        width: 40,
        height: 6,
        ..TuiOptions::default()
    };
    let screen = render(&app, &options);
    assert!(screen.lines.iter().all(|l| l.chars().count() <= 40));
    assert!(screen.lines[0].starts_with("diroxide-tui"));
    assert_eq!(screen.lines.len(), 6);
}
