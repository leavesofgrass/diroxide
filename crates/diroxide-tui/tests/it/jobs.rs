//! The Jobs list in the terminal (lane A2, R25 commit 79): the group
//! headings with their counts, every row the state first, the hardware
//! cursor on the focused row's first cell in the list and in a job's
//! actions, and Escape back to the panel with the cursor where it was.

use diroxide_app::AppEvent;
use diroxide_app::commands::ids;
use diroxide_app::config::Layout;
use diroxide_core::Command;

use crate::support::{
    Clock, app, cursor_at, fixture, focus, line_of, place, press, session, settings, settle, start,
    temp,
};

#[test]
fn the_jobs_list_says_each_group_and_keeps_the_cursor_on_its_focus() {
    let dir = temp();
    let work = fixture(dir.path());
    let (mut a, mut r) = app(
        place(&work),
        place(&work.join("Backup")),
        settings(Layout::Stacked),
    );
    let mut s = session(&mut a, &mut r, 100, 30, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    focus(&mut s, &clock, "notes.txt");
    let row = s.screen().cursor_line.unwrap();
    press(&mut s, &clock, "f5");
    press(&mut s, &clock, "y");
    assert!(s.app().jobs().is_empty(), "{}", s.app().jobs().line());

    s.event(AppEvent::Command(Command::new(ids::JOBS_LIST)), clock.now());
    settle(&mut s, &clock);
    let screen = s.screen();
    line_of(&screen, "Finished, 1 job");
    let done = line_of(&screen, "Done: copy notes.txt to Backup");
    cursor_at(&s, 0, done);

    // Enter opens the job's actions, named by its row; the cursor on the
    // first.
    press(&mut s, &clock, "enter");
    let screen = s.screen();
    let first = line_of(&screen, "Show what failed");
    cursor_at(&s, 0, first);
    line_of(&screen, "Retry the failed items, unavailable");
    line_of(&screen, "Pause the queue");

    press(&mut s, &clock, "escape");
    cursor_at(
        &s,
        0,
        line_of(&s.screen(), "Done: copy notes.txt to Backup"),
    );
    press(&mut s, &clock, "escape");
    assert!(s.app().dialog().is_none());
    cursor_at(&s, 0, row);
}
