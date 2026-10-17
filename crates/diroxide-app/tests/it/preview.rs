//! The quick view wired into the app (plan commit 64): Shift+F3, Tab into
//! the preview region and back, its links opened only after a yes and
//! never a file, a panel's key in the region, the region's commands with
//! the quick view off, and "Open in textweaver" with no shell.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use diroxide_app::commands::ids;
use diroxide_tools::ToolRegistry;
use diroxide_vfs::Registry;

use crate::support::{Recorder, Session, Setup, place, settings, temp};

/// Gives the quick view its time until it is done, in real time: it reads
/// on its own thread.
fn settle_quick(s: &mut Session) -> Vec<String> {
    let mut said = Vec::new();
    for _ in 0..400 {
        if s.app.quick_view_wants_tick().is_none() {
            break;
        }
        let effects = s.app.quick_view_tick();
        said.extend(s.run(effects));
        std::thread::sleep(Duration::from_millis(5));
    }
    said
}

fn folder(root: &Path) -> std::path::PathBuf {
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("links.md"),
        "# Links\n\nSee [example](https://example.org), [docs](docs/index.md) and \
         [mail](mailto:someone@example.org).\n",
    )
    .unwrap();
    fs::write(work.join("notes.txt"), "one\ntwo\n").unwrap();
    work
}

fn session(work: &Path, recorder: &Arc<Recorder>) -> Session {
    let mut quick = settings();
    quick.quick_view.wait_ms = 0;
    Session::open(
        place(work),
        place(work),
        Registry::with_local(),
        Setup {
            settings: quick,
            recorder: Arc::clone(recorder),
            ..Setup::default()
        },
    )
}

#[test]
fn a_link_opens_only_after_yes_and_never_a_file() {
    let dir = temp();
    let work = folder(dir.path());
    let recorder = Arc::new(Recorder::default());
    let mut s = session(&work, &recorder);
    s.focus("links.md");
    let mut said = s.key("shift+f3");
    said.extend(settle_quick(&mut s));
    assert_eq!(said, ["Quick view on", "Preview: links.md, 1 heading"]);
    let said = s.key("tab");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("Preview, links.md, Links"), "{said:?}");
    assert!(s.app.in_preview());
    // The links as a list; Enter goes to one.
    let said = s.key("l");
    assert!(
        said[0].starts_with("Links: 3 in links.md, example, to https://example.org"),
        "{said:?}"
    );
    let said = s.key("enter");
    assert_eq!(said, ["Link 1 of 3: example, to https://example.org"]);
    // Enter on its line asks first, starting on No; No opens nothing.
    let said = s.key("enter");
    assert_eq!(
        said,
        ["Open https://example.org in the browser? No is the default. y or n"]
    );
    assert_eq!(s.key("n"), ["Not opened"]);
    assert!(recorder.addresses.lock().unwrap().is_empty());
    // Yes opens it, through the system opener and no shell.
    s.key("enter");
    s.key("y");
    assert_eq!(*recorder.addresses.lock().unwrap(), ["https://example.org"]);
    // A link to a file is refused in words, with no question.
    s.key("l");
    s.key("down");
    let said = s.key("enter");
    assert_eq!(said, ["Link 2 of 3: docs, to docs/index.md"]);
    let said = s.key("enter");
    assert_eq!(
        said,
        ["Could not open docs/index.md: only web and mail links open from the preview."]
    );
    assert!(recorder.opened.lock().unwrap().is_empty());
    // Escape: back on the panel's row.
    let said = s.key("escape");
    assert_eq!(said, ["Left panel: work, 2 items"]);
    assert!(!s.app.in_preview());
    assert_eq!(s.focused(), "links.md");
}

#[test]
fn a_panel_key_in_the_preview_says_how_to_leave_and_the_region_keys_need_it_on() {
    let dir = temp();
    let work = folder(dir.path());
    let recorder = Arc::new(Recorder::default());
    let mut s = session(&work, &recorder);
    assert_eq!(
        s.command(ids::PREVIEW_NEXT_HEADING),
        ["The quick view is off. Shift+F3 turns it on."]
    );
    s.focus("links.md");
    s.key("shift+f3");
    settle_quick(&mut s);
    s.key("tab");
    assert_eq!(
        s.key("f5"),
        ["Leave the preview first. Escape returns to the panel."]
    );
    // The key bar is the preview's.
    assert!(
        s.app
            .key_bar()
            .starts_with("Escape Back  H Heading  T Table  L Links"),
        "{}",
        s.app.key_bar()
    );
    assert_eq!(s.app.title_line(), "Preview, links.md");
    // Shift+F3 in the region turns it off and the focus is the panel's.
    assert_eq!(s.key("shift+f3"), ["Quick view off"]);
    assert!(!s.app.in_preview());
    assert!(s.app.title_line().starts_with("work, left panel"));
}

#[test]
fn open_in_textweaver_runs_tw_with_no_shell_or_says_how_to_install() {
    let dir = temp();
    let work = folder(dir.path());
    let recorder = Arc::new(Recorder::default());
    let mut s = session(&work, &recorder);
    s.focus("links.md");
    s.key("shift+f3");
    settle_quick(&mut s);
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    s.app
        .set_tools(ToolRegistry::builtin().with_path(Some(bin.as_os_str().to_owned())));
    let said = s.command(ids::OPEN_IN_TEXTWEAVER);
    assert_eq!(
        said,
        [
            "Could not open links.md in textweaver: textweaver is not installed. \
             Install textweaver from its releases page or with cargo."
        ]
    );
    let program = if cfg!(windows) { "tw.exe" } else { "tw" };
    fs::write(bin.join(program), b"").unwrap();
    s.app
        .set_tools(ToolRegistry::builtin().with_path(Some(bin.as_os_str().to_owned())));
    let said = s.command(ids::OPEN_IN_TEXTWEAVER);
    assert_eq!(
        said.first().map(String::as_str),
        Some("Opening links.md in textweaver")
    );
    let programs = recorder.programs.lock().unwrap();
    assert_eq!(programs.len(), 1);
    let run = &programs[0];
    assert!(
        Path::new(&run.program).starts_with(&bin),
        "{:?}",
        run.program
    );
    assert_eq!(run.args, [work.join("links.md").into_os_string()]);
}
