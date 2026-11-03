//! The saved queue's interrupted jobs in the terminal (lane Q2): the
//! first start's one sentence, the Interrupted group first in the Jobs
//! list with the hardware cursor on its row's first cell, Retry and
//! Forget as the row's actions, and Retry running the job again. The
//! queue file is the test's own, in its own state folder.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::config::Layout;
use diroxide_app::{AppEvent, Runtime};
use diroxide_core::Command;

use crate::support::{
    Clock, app, cursor_at, fixture, line_of, place, press, session, settings, settle, start, temp,
};

/// A place as a JSON string: its text with quotes and backslashes
/// escaped.
fn json_place(path: &Path) -> String {
    let text = place(path).to_string();
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The queue a quit left: a copy of notes.txt to Backup that had begun,
/// and a trash saved with no request.
fn saved_queue(work: &Path) -> String {
    let source = json_place(&work.join("notes.txt"));
    let destination = json_place(&work.join("Backup"));
    format!(
        r#"{{
  "version": 1,
  "quit": true,
  "jobs": [
    {{
      "job": 4,
      "kind": "copy",
      "state": "running",
      "started": true,
      "what": "notes.txt to Backup",
      "request": {{
        "id": 4,
        "kind": "copy",
        "sources": [{source}],
        "destination": {destination},
        "new_name": null,
        "conflicts": "ask"
      }}
    }},
    {{
      "job": 5,
      "kind": "trash",
      "state": "waiting",
      "what": "budget.xlsx"
    }}
  ]
}}"#
    )
}

#[test]
fn interrupted_jobs_come_first_with_retry_and_forget_under_the_cursor() {
    let dir = temp();
    let work = fixture(dir.path());
    let state = dir.path().join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("queue.json"), saved_queue(&work)).unwrap();
    let (mut a, _) = app(
        place(&work),
        place(&work.join("Backup")),
        settings(Layout::Stacked),
    );
    let paths = diroxide_app::config::Paths::new(dir.path().join("config"), state.clone());
    let mut r = Runtime::with_local().with_paths(paths);
    let mut s = session(&mut a, &mut r, 100, 30, true);
    let clock = Clock::at_start();
    start(&mut s, &clock);
    let said: Vec<String> = s.said().iter().map(|m| m.text.clone()).collect();
    assert!(
        said.contains(
            &"2 jobs were interrupted by the last quit. The Jobs list has them.".to_owned()
        ),
        "{said:?}"
    );
    assert!(
        !work.join("Backup").join("notes.txt").exists(),
        "not started"
    );

    // The group first, the cursor on the first interrupted row.
    s.event(AppEvent::Command(Command::new(ids::JOBS_LIST)), clock.now());
    settle(&mut s, &clock);
    let screen = s.screen();
    let heading = line_of(&screen, "Interrupted, 2 jobs");
    let row = line_of(&screen, "Interrupted: copy notes.txt to Backup");
    assert_eq!(row, heading + 1);
    line_of(&screen, "Interrupted: trash budget.xlsx");
    cursor_at(&s, 0, row);

    // The trash was saved without its request: Retry is unavailable,
    // said with its reason; Forget takes it away.
    press(&mut s, &clock, "down");
    press(&mut s, &clock, "enter");
    let screen = s.screen();
    let retry = line_of(&screen, "Retry, unavailable");
    cursor_at(&s, 0, retry);
    line_of(&screen, "Forget");
    press(&mut s, &clock, "down");
    press(&mut s, &clock, "enter");
    let said: Vec<String> = s.said().iter().map(|m| m.text.clone()).collect();
    assert!(
        said.contains(&"Forgotten: trash budget.xlsx".to_owned()),
        "{said:?}"
    );
    let screen = s.screen();
    line_of(&screen, "Interrupted, 1 job");

    // Retry on the copy: the plan's question, then the copy runs.
    let row = line_of(&screen, "Interrupted: copy notes.txt to Backup");
    cursor_at(&s, 0, row);
    press(&mut s, &clock, "enter");
    let screen = s.screen();
    let first = line_of(&screen, "Retry");
    cursor_at(&s, 0, first);
    press(&mut s, &clock, "enter");
    press(&mut s, &clock, "y");
    assert!(work.join("Backup").join("notes.txt").exists());
    assert!(s.app().jobs().interrupted().is_empty());
    // Nothing is left to keep: the file goes. Its writer works off the
    // input thread, so the test gives it a moment.
    let file = state.join("queue.json");
    let deadline = Instant::now() + Duration::from_secs(30);
    while file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!file.exists());
}
