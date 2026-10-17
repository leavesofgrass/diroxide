//! The owner's check of alpha.2 (research/alpha2-plan.md, part six) as a
//! scripted test on ratatui's test backend, plan commit 68: each step
//! that is wired, with the words the owner is told to expect, and after
//! every frame the hardware cursor checked by spike Q2's rule: visible,
//! where the frame says, and on a row's first cell unless a text field
//! holds it.
//!
//! The demo set is made in the test's own folder the way `cargo xtask
//! demo` makes the owner's: course.zip, backup.7z, logs.tar.xz,
//! locked.zip, hostile.zip, big.bin (60 MB), main.rs, report.md,
//! notes.org, notes.txt, big.log, an empty out folder and a joined folder
//! for step 30. Steps that need the system's trash or a program (the
//! undos in 6 and 31, Enter on big.bin.001) run in the app's own tests,
//! over a test trash; this test never touches the system's trash and
//! never starts a program.
//!
//! Not here: step 8's "Reading" line (not wired; the listing comes
//! straight away). The quick view's steps, 20 to 25, run with the quick
//! view's wait at 100 milliseconds, so the test need not wait half a
//! second at each stop; spike Q2 has a test of its own below.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use diroxide_app::archives::{ArchiveFormat, PackRequest, pack_local};
use diroxide_app::commands::ids;
use diroxide_app::config::Layout;
use diroxide_app::{Answer, AppEvent, Dialog};
use diroxide_core::format::within_cells;
use diroxide_core::{Command, CommandId, Key, KeyCode, Modifiers, Secret};
use ratatui::backend::TestBackend;

use crate::support::{Clock, app, place, session, settings, temp};

type S<'a> = diroxide_tui::Session<'a, TestBackend>;

/// Not a real password: made up for this test, never kept anywhere else.
const PASSWORD: &str = "spruce-meadow-harbor-6146";

/// The demo's big file: 60 MB, so the email preset makes four parts.
const BIG: usize = 60_000_000;

/// Zip's CRC-32, for the hostile archive written by hand.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// One zip header, local or central, for a stored member.
fn zip_header(name: &str, data: &[u8], central_at: Option<u32>) -> Vec<u8> {
    let size = u32::try_from(data.len()).unwrap();
    let mut h = Vec::new();
    match central_at {
        Some(_) => {
            h.extend(0x0201_4b50u32.to_le_bytes());
            h.extend(20u16.to_le_bytes());
        }
        None => h.extend(0x0403_4b50u32.to_le_bytes()),
    }
    h.extend(20u16.to_le_bytes()); // version needed
    h.extend(0u16.to_le_bytes()); // flags
    h.extend(0u16.to_le_bytes()); // stored
    h.extend(0u32.to_le_bytes()); // time and date
    h.extend(crc32(data).to_le_bytes());
    h.extend(size.to_le_bytes());
    h.extend(size.to_le_bytes());
    h.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
    h.extend(0u16.to_le_bytes()); // extra
    if let Some(offset) = central_at {
        h.extend(0u16.to_le_bytes()); // comment
        h.extend(0u16.to_le_bytes()); // disk
        h.extend(0u16.to_le_bytes()); // internal attributes
        h.extend(0u32.to_le_bytes()); // external attributes
        h.extend(offset.to_le_bytes());
    }
    h.extend(name.as_bytes());
    h
}

/// A stored zip with members named as given, `../escape.txt` among them:
/// written by hand, since no packer writes such a name.
fn hostile_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let offset = u32::try_from(out.len()).unwrap();
        out.extend(zip_header(name, data, None));
        out.extend(*data);
        central.extend(zip_header(name, data, Some(offset)));
    }
    let at = u32::try_from(out.len()).unwrap();
    let count = u16::try_from(members.len()).unwrap();
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

fn pack(folder: &Path, archive: &str, items: &[&str], password: Option<&str>) {
    let format = ArchiveFormat::from_name(archive).unwrap();
    let sources = items.iter().map(|i| place(&folder.join(i))).collect();
    let mut request = PackRequest::new(sources, place(&folder.join(archive)), format);
    request.password = password.map(|p| Secret::new(p.to_owned()));
    pack_local(&request).unwrap();
}

const NOTES: &str = "Notes on the course.\nThe course meets on Mondays.\nBring a pen.\n";

/// The demo set, as part six names it.
fn demo(root: &Path) -> PathBuf {
    let demo = root.join("demo");
    let build = root.join("build");
    for week in ["week1", "week2"] {
        fs::create_dir_all(build.join(week)).unwrap();
    }
    for (week, names) in [("week1", ["a", "b", "c"]), ("week2", ["d", "e", "f"])] {
        for name in names {
            let text = format!("{name}: first line\n{name}: second line\n");
            fs::write(build.join(week).join(format!("{name}.txt")), text).unwrap();
        }
    }
    fs::write(build.join("notes.txt"), NOTES).unwrap();
    fs::write(build.join("README.md"), "# Course\n\nRead me first.\n").unwrap();
    pack(
        &build,
        "course.zip",
        &["week1", "week2", "notes.txt", "README.md"],
        None,
    );
    let mut six = Vec::new();
    for n in 1..=6 {
        let name = format!("file{n}.txt");
        fs::write(build.join(&name), format!("backup file {n}\n")).unwrap();
        six.push(name);
    }
    let six: Vec<&str> = six.iter().map(String::as_str).collect();
    pack(&build, "backup.7z", &six, None);
    for n in 1..=3 {
        fs::write(build.join(format!("app{n}.log")), format!("log line {n}\n")).unwrap();
    }
    pack(
        &build,
        "logs.tar.xz",
        &["app1.log", "app2.log", "app3.log"],
        None,
    );
    fs::write(build.join("secret.txt"), "the secret\n").unwrap();
    pack(&build, "locked.zip", &["secret.txt"], Some(PASSWORD));
    fs::create_dir_all(demo.join("out")).unwrap();
    fs::create_dir_all(demo.join("joined")).unwrap();
    for name in ["course.zip", "backup.7z", "logs.tar.xz", "locked.zip"] {
        fs::rename(build.join(name), demo.join(name)).unwrap();
    }
    let hostile = hostile_zip(&[
        ("../escape.txt", b"out!"),
        ("safe.txt", b"safe"),
        ("readme.txt", b"read me"),
    ]);
    fs::write(demo.join("hostile.zip"), hostile).unwrap();
    let block: Vec<u8> = (0..1_000_000u32).map(|i| (i % 253) as u8).collect();
    let mut big = Vec::with_capacity(BIG);
    while big.len() < BIG {
        big.extend(&block);
    }
    fs::write(demo.join("big.bin"), big).unwrap();
    fs::write(
        demo.join("main.rs"),
        "use std::env;\n\n/// The entry.\nfn main() {\n    let args: Vec<String> = env::args().collect();\n    parse_args(&args);\n}\n\nfn parse_args(args: &[String]) -> usize {\n    args.len()\n}\n",
    )
    .unwrap();
    let mut report = String::from("# Report\n\n");
    for n in 1..=11 {
        report.push_str(&format!("## Part {n}\n\nText of part {n}.\n\n"));
    }
    report.push_str("| Name | Age |\n|---|---|\n| Ada | 36 |\n");
    fs::write(demo.join("report.md"), report).unwrap();
    fs::write(demo.join("notes.org"), "* Notes\n** First\nText.\n").unwrap();
    fs::write(demo.join("notes.txt"), NOTES).unwrap();
    let mut log = String::new();
    while log.len() < 700_000 {
        log.push_str("2026-10-16 INFO a line of the big log\n");
    }
    fs::write(demo.join("big.log"), log).unwrap();
    demo
}

/// The owner's script: the session, the test's clock, and how much has
/// been said so far.
struct Check<'a> {
    s: S<'a>,
    clock: Clock,
    mark: usize,
}

impl Check<'_> {
    /// Spike Q2's rule after every frame: the cursor visible and where
    /// the frame says; on a row's first cell (of either panel) unless a
    /// text field holds it.
    fn cursor(&self) {
        let screen = self.s.screen();
        let backend = self.s.terminal().backend();
        assert!(backend.cursor_visible(), "the cursor is hidden");
        let at = backend.cursor_position();
        let line = screen.cursor_line.expect("the cursor has a line");
        assert_eq!(
            (usize::from(at.x), usize::from(at.y)),
            (screen.cursor_column, line),
            "{:#?}",
            screen.lines
        );
        let typing = self.s.app().dialog().is_some_and(|(_, d)| {
            matches!(
                d,
                Dialog::Edit { in_list: false, .. } | Dialog::Secret { .. } | Dialog::Prompt { .. }
            )
        });
        if !typing {
            assert!(
                screen.cursor_column == 0 || screen.cursor_column == 40,
                "the cursor is not on a row's first cell: column {} {:#?}",
                screen.cursor_column,
                screen.lines
            );
            // A file's blank line is a row too; a panel's row never is.
            let blank_allowed = (self.s.app().viewer_open() || self.s.app().in_preview())
                && self.s.app().dialog().is_none();
            assert!(
                blank_allowed || !screen.lines[line].trim().is_empty(),
                "{:#?}",
                screen.lines
            );
        }
    }

    /// The workers settled, the viewer given its time (a scan, the
    /// highlighter), a frame drawn and the cursor checked; returns what
    /// was said since the last step.
    fn settle(&mut self) -> Vec<String> {
        self.clock.advance(Duration::from_millis(200));
        for _ in 0..400 {
            self.s.settle(self.clock.now(), Duration::from_secs(20));
            self.s.tick(self.clock.now());
            if self.s.app().viewer_wants_tick().is_none()
                && self.s.app().quick_view_wants_tick().is_none()
            {
                break;
            }
            // The highlighter and the quick view work on their own
            // threads, in real time.
            std::thread::sleep(Duration::from_millis(5));
            self.clock.advance(Duration::from_millis(50));
        }
        self.s.settle(self.clock.now(), Duration::from_secs(20));
        self.s.draw(self.clock.now()).unwrap();
        self.cursor();
        let said = self.s.said()[self.mark..]
            .iter()
            .map(|m| m.text.clone())
            .collect();
        self.mark = self.s.said().len();
        said
    }

    fn key(&mut self, key: &str) -> Vec<String> {
        self.s.key(Key::parse(key).unwrap(), self.clock.now());
        self.settle()
    }

    fn command(&mut self, id: CommandId) -> Vec<String> {
        self.s.command(Command::new(id), self.clock.now());
        self.settle()
    }

    fn answer(&mut self, chosen: &str) -> Vec<String> {
        let dialog = self.s.app().dialog().map(|(id, _)| id).unwrap();
        let answer = Answer::Chosen(chosen.to_owned());
        self.s
            .event(AppEvent::Answer { dialog, answer }, self.clock.now());
        self.settle()
    }

    fn type_text(&mut self, text: &str) -> Vec<String> {
        for c in text.chars() {
            let key = Key::new(KeyCode::Char(c), Modifiers::NONE);
            self.s.key(key, self.clock.now());
        }
        self.settle()
    }

    fn focus(&mut self, name: &str) -> Vec<String> {
        let side = self.s.app().active_side();
        assert!(
            self.s.app_mut().focus_name(side, name),
            "{name} is not listed: {:?}",
            self.names()
        );
        self.settle()
    }

    /// Waits, in real time, for a line that starts so: the folder watch
    /// and the quick view's render after it take their own time.
    fn wait_for(&mut self, start: &str) -> Vec<String> {
        let mut said = Vec::new();
        for _ in 0..200 {
            said.extend(self.settle());
            if any_starts(&said, start) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        said
    }

    fn unmark(&mut self, names: &[&str]) {
        let side = self.s.app().active_side();
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        self.s.app_mut().mark_names(side, &names, false);
        self.settle();
    }

    fn line(&self, at: usize) -> String {
        self.s.screen().lines[at].clone()
    }

    fn focused_line(&self) -> String {
        let screen = self.s.screen();
        screen.lines[screen.cursor_line.unwrap()].clone()
    }

    fn names(&self) -> Vec<String> {
        let list = self.s.app().active_panel().list();
        list.rows().iter().map(|r| r.id.clone()).collect()
    }

    /// The open dialog's rows: a list's labels, a question's listing.
    fn rows(&self) -> Vec<String> {
        match self.s.app().dialog() {
            Some((_, Dialog::Choose { list, .. })) => {
                list.rows().iter().map(|r| r.label.clone()).collect()
            }
            Some((_, Dialog::Confirm { listing, .. })) => listing.clone(),
            _ => Vec::new(),
        }
    }
}

fn any_starts(said: &[String], start: &str) -> bool {
    said.iter().any(|l| l.starts_with(start))
}

fn any_contains(said: &[String], part: &str) -> bool {
    said.iter().any(|l| l.contains(part))
}

fn last_is(said: &[String], line: &str) -> bool {
    said.last().is_some_and(|l| l == line)
}

#[test]
fn the_owners_check_of_alpha_2_runs_with_its_words() {
    let dir = temp();
    let demo = demo(dir.path());
    let out = demo.join("out");
    let mut quick = settings(Layout::SideBySide);
    quick.quick_view.wait_ms = 100;
    let (mut one, mut runtime) = app(place(&demo), place(&out), quick);
    let mut c = Check {
        s: session(&mut one, &mut runtime, 80, 24, true),
        clock: Clock::at_start(),
        mark: 0,
    };
    c.s.start(c.clock.now());
    c.settle();

    // Archives. 1. Enter on course.zip: the title, then the opening line.
    c.focus("course.zip");
    let said = c.key("enter");
    assert!(
        c.line(0).starts_with("course.zip, archive, in "),
        "{}",
        c.line(0)
    );
    assert!(any_starts(&said, "Opened course.zip: 10 items"), "{said:?}");
    // 2. Each row: the name, then folder or text file, then a size.
    let said = c.key("down");
    assert!(any_starts(&said, "week2, folder"), "{said:?}");
    assert!(
        c.focused_line().starts_with("week2"),
        "{}",
        c.focused_line()
    );
    c.focus("notes.txt");
    assert!(
        c.focused_line().starts_with("notes.txt"),
        "{}",
        c.focused_line()
    );
    // 3. Left names the demo folder; Right on course.zip enters again.
    c.key("left");
    assert!(c.line(0).starts_with("demo, left panel"), "{}", c.line(0));
    c.key("right");
    assert!(
        c.line(0).starts_with("course.zip, archive, in "),
        "{}",
        c.line(0)
    );
    // 4. Enter on week1.
    c.focus("week1");
    let said = c.key("enter");
    assert!(
        any_starts(&said, "week1, 3 items, in course.zip"),
        "{said:?}"
    );
    assert!(
        c.line(0).starts_with("week1, in course.zip"),
        "{}",
        c.line(0)
    );
    // 5. F3 on a.txt: the viewer's title; Escape, back on the row.
    c.focus("a.txt");
    let said = c.key("f3");
    assert!(any_starts(&said, "a.txt, in course.zip!week1"), "{said:?}");
    assert_eq!(c.line(0), "a.txt, in course.zip!week1");
    c.key("escape");
    assert!(
        c.focused_line().starts_with("a.txt"),
        "{}",
        c.focused_line()
    );
    c.key("left");
    // 6. week1 marked, F5: the plan's question, then the finish.
    c.focus("week1");
    c.key("space");
    let said = c.key("f5");
    assert!(
        said.last()
            .is_some_and(|q| q.starts_with("Unpack 4 members into out?")),
        "{said:?}"
    );
    let said = c.key("y");
    assert!(any_starts(&said, "Unpack finished: 4 done"), "{said:?}");
    // The end is said once: the result, not also "Unpacking finished".
    let ends = said.iter().filter(|l| l.contains("finished")).count();
    assert_eq!(ends, 1, "{said:?}");
    assert!(out.join("week1").join("a.txt").exists());
    // 7. backup.7z, one file out: the same words.
    c.key("left");
    c.focus("backup.7z");
    let said = c.key("enter");
    assert!(any_starts(&said, "Opened backup.7z: 6 items"), "{said:?}");
    c.focus("file3.txt");
    let said = c.key("f5");
    assert!(
        said.last()
            .is_some_and(|q| q.starts_with("Unpack 1 member into out?")),
        "{said:?}"
    );
    let said = c.key("y");
    assert!(any_starts(&said, "Unpack finished: 1 done"), "{said:?}");
    assert!(out.join("file3.txt").exists());
    // 8. logs.tar.xz lists (the "Reading" line is not wired).
    c.key("left");
    c.focus("logs.tar.xz");
    let said = c.key("enter");
    assert!(any_starts(&said, "Opened logs.tar.xz: 3 items"), "{said:?}");
    // 9. locked.zip: the masked prompt, a wrong try, the right one;
    // nothing typed is ever said or shown.
    c.key("left");
    c.focus("locked.zip");
    c.key("enter");
    c.focus("secret.txt");
    let said = c.key("f3");
    assert!(
        last_is(&said, "Password for locked.zip (hidden):"),
        "{said:?}"
    );
    c.type_text("wrong");
    let said = c.key("enter");
    assert!(
        last_is(
            &said,
            "Wrong password for locked.zip. Try again? No is the default. y or n"
        ),
        "{said:?}"
    );
    let said = c.key("y");
    assert!(
        last_is(&said, "Password for locked.zip (hidden):"),
        "{said:?}"
    );
    c.type_text(PASSWORD);
    let said = c.key("enter");
    assert!(any_starts(&said, "secret.txt, in locked.zip"), "{said:?}");
    assert_eq!(c.line(1), "the secret");
    for message in c.s.said() {
        assert!(!message.text.contains(PASSWORD), "{}", message.text);
        assert!(!message.text.contains("wrong"), "{}", message.text);
    }
    assert!(c.s.screen().lines.iter().all(|l| !l.contains(PASSWORD)));
    c.key("escape");
    c.key("left");
    // 10. hostile.zip lists without the bad name; unpacking says it.
    c.focus("hostile.zip");
    c.key("enter");
    let names = c.names();
    assert!(names.iter().all(|n| !n.contains("escape")), "{names:?}");
    assert!(names.iter().any(|n| n == "safe.txt"), "{names:?}");
    c.key("left");
    c.focus("hostile.zip");
    let said = c.command(ids::UNPACK);
    assert!(
        any_starts(&said, "Unpack, Unpack hostile.zip into out"),
        "{said:?}"
    );
    let said = c.key("enter");
    assert!(
        said.last()
            .is_some_and(|q| q.ends_with("No is the default. y or n")),
        "{said:?}"
    );
    let said = c.key("y");
    assert!(any_contains(&said, "Skipped ../"), "{said:?}");
    assert!(any_contains(&said, "name leaves the archive"), "{said:?}");
    assert!(!dir.path().join("escape.txt").exists());
    assert!(!demo.join("escape.txt").exists());
    // 11. Two files packed from the palette: the rows, the question on
    // No, the finish and the check.
    c.focus("main.rs");
    c.key("space");
    c.focus("report.md");
    c.key("space");
    c.key("ctrl+p");
    c.type_text("pack");
    let said = c.key("enter");
    assert!(
        last_is(&said, "Pack, Pack 2 items into demo.zip, 1 of 9"),
        "{said:?}"
    );
    let rows = c.rows();
    assert!(
        rows.iter().any(|r| r.starts_with("Format: zip")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.starts_with("Level: ")), "{rows:?}");
    assert!(rows.iter().any(|r| r.starts_with("Where: ")), "{rows:?}");
    let said = c.key("enter");
    assert!(
        last_is(
            &said,
            "Pack 2 items into demo.zip? No is the default. y or n"
        ),
        "{said:?}"
    );
    let listing = c.rows();
    assert!(
        listing
            .iter()
            .any(|r| r.starts_with("Format: zip, level normal")),
        "{listing:?}"
    );
    assert!(
        listing
            .iter()
            .any(|r| r.starts_with("Where: the other panel, ")),
        "{listing:?}"
    );
    let said = c.key("y");
    assert!(
        any_starts(&said, "Pack started: 2 items into demo.zip"),
        "{said:?}"
    );
    assert!(
        last_is(
            &said,
            "Pack finished: 2 done. Checked 2 of 2 entries, all good."
        ),
        "{said:?}"
    );
    assert!(out.join("demo.zip").exists());
    c.unmark(&["main.rs", "report.md"]);
    // 12. Test archive on the new zip.
    c.key("tab");
    c.focus("demo.zip");
    let said = c.command(ids::TEST_ARCHIVE);
    assert!(any_starts(&said, "Archive is sound: 2 members"), "{said:?}");
    // 13. The right panel inside course.zip; F5 into it is refused.
    c.key("left");
    c.focus("course.zip");
    c.key("enter");
    c.key("tab");
    c.focus("notes.txt");
    c.key("space");
    let said = c.key("f5");
    assert_eq!(
        said,
        ["Archives are read only here. Use Pack to make a new one."]
    );
    c.unmark(&["notes.txt"]);

    // The viewer. 14. notes.txt: the title, the lines, the status.
    c.focus("notes.txt");
    let said = c.key("f3");
    assert!(any_starts(&said, "notes.txt, in "), "{said:?}");
    assert_eq!(c.line(1), "Notes on the course.");
    let said = c.key("ctrl+d");
    assert!(any_starts(&said, "Line 1 of 3"), "{said:?}");
    // 15. A word twice, next, a word not there.
    let said = c.key("/");
    assert_eq!(said, ["Search for, empty"]);
    c.type_text("course");
    let said = c.key("enter");
    assert!(last_is(&said, "Match 1 of 2, line 1"), "{said:?}");
    let said = c.key("n");
    assert!(last_is(&said, "Match 2 of 2, line 2"), "{said:?}");
    c.key("/");
    c.type_text("zebra");
    let said = c.key("enter");
    assert!(last_is(&said, "No match for zebra."), "{said:?}");
    let said = c.key("escape");
    assert_eq!(said, ["Viewer closed: notes.txt"]);
    // 16. big.log is under the cap: the normal title.
    c.focus("big.log");
    let said = c.key("f3");
    assert!(any_starts(&said, "big.log, in "), "{said:?}");
    // 17. Hex mode: an offset, eight byte pairs and the text, inside 40
    // cells; a byte said.
    let said = c.key("h");
    assert!(any_starts(&said, "Hex mode"), "{said:?}");
    let hex = c.line(1);
    assert!(hex.starts_with('0'), "{hex}");
    assert!(hex.chars().count() <= 40, "{hex}");
    let said = c.key("b");
    assert!(any_starts(&said, "Byte "), "{said:?}");
    c.key("escape");
    // 18. main.rs: the token kind at a function name, the line by kind,
    // the outline.
    c.focus("main.rs");
    let said = c.key("f3");
    assert!(any_starts(&said, "main.rs, in "), "{said:?}");
    for _ in 0..3 {
        c.key("down");
    }
    let said = c.key("right");
    assert_eq!(said, ["main"]);
    let said = c.key("k");
    assert!(any_starts(&said, "Function name: main"), "{said:?}");
    let said = c.key("shift+k");
    assert!(any_starts(&said, "Line 4: "), "{said:?}");
    let said = c.key("o");
    assert!(
        any_starts(&said, "Outline: 2 symbols in main.rs"),
        "{said:?}"
    );
    let rows = c.rows();
    assert!(rows.iter().all(|r| r.starts_with("function ")), "{rows:?}");
    c.key("escape");
    c.key("escape");
    // 19. big.bin looks binary.
    c.focus("big.bin");
    let said = c.key("f3");
    assert!(any_contains(&said, "looks binary; hex mode"), "{said:?}");
    c.key("escape");

    // The quick view, in the right panel's place. 20. Shift+F3 on
    // report.md: on, then its one line.
    c.focus("report.md");
    let said = c.key("shift+f3");
    assert_eq!(said, ["Quick view on", "Preview: report.md, 12 headings"]);
    // In screen-reader mode the region waits for Tab: its words, not the
    // document, so a new preview is never read in full.
    assert_eq!(from_column(&c.line(1), 40), "Preview");
    assert_eq!(from_column(&c.line(2), 40), "Tab goes into the preview");
    // 21. Up through five files, quickly: one line when the cursor stops.
    for _ in 0..5 {
        c.s.key(Key::parse("up").unwrap(), c.clock.now());
    }
    let said = c.settle();
    let lines: Vec<&String> = said
        .iter()
        .filter(|l| l.starts_with("Preview: ") || l.starts_with("No preview: "))
        .collect();
    assert_eq!(lines.len(), 1, "{said:?}");
    let said = c.focus("report.md");
    assert_eq!(said, ["Preview: report.md, 12 headings"]);
    // 22. Tab: the title and the row; by heading; the table; a cell.
    let said = c.key("tab");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("Preview, report.md, "), "{said:?}");
    assert_eq!(c.line(0), "Preview, report.md");
    let said = c.key("h");
    assert_eq!(said, ["Heading 2: Part 1, 2 of 12"]);
    let said = c.key("t");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("Table 1 of 1, "), "{said:?}");
    let said = c.key("right");
    assert_eq!(said, ["Age, header, row 1 of 2, column 2"]);
    let said = c.key("shift+down");
    assert_eq!(said, ["Age: 36, row 2 of 2, column 2"]);
    // 23. Find: the count; Escape: the panel, the cursor on its row.
    let said = c.key("/");
    assert!(any_starts(&said, "Find in the preview"), "{said:?}");
    c.type_text("part");
    let said = c.key("enter");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("Match 1 of 22 in the preview"),
        "{said:?}"
    );
    let said = c.key("n");
    assert!(
        any_starts(&said, "Match 2 of 22 in the preview"),
        "{said:?}"
    );
    let said = c.key("escape");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("Left panel: demo, "), "{said:?}");
    assert!(
        c.focused_line().starts_with("report.md"),
        "{}",
        c.focused_line()
    );
    // 24. notes.org; report.md changed on disk, said once.
    let said = c.focus("notes.org");
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("Preview: notes.org, Org"), "{said:?}");
    let said = c.focus("report.md");
    assert_eq!(said, ["Preview: report.md, 12 headings"]);
    let mut changed = fs::read_to_string(demo.join("report.md")).unwrap();
    changed.push_str("\n## Part 12\n\nText of part 12.\n");
    fs::write(demo.join("report.md"), changed).unwrap();
    let said = c.wait_for("Preview updated: report.md changed");
    let updated = said
        .iter()
        .filter(|l| l.starts_with("Preview updated: report.md changed"))
        .count();
    assert_eq!(updated, 1, "{said:?}");
    // 25. Shift+F3 again: off.
    let said = c.key("shift+f3");
    assert_eq!(said, ["Quick view off"]);

    // Split and join. The right panel back on out first.
    c.key("tab");
    c.key("left");
    c.focus("out");
    c.key("enter");
    c.key("tab");
    // 26. Split file: the presets, use first; the plan, the question.
    c.focus("big.bin");
    let said = c.command(ids::SPLIT);
    assert!(
        last_is(
            &said,
            "Split into, Email attachment, 18 MB, Parts of 18,000,000 bytes, 1 of 9"
        ),
        "{said:?}"
    );
    let said = c.key("enter");
    assert!(
        said.last()
            .is_some_and(|q| q.ends_with("No is the default. y or n")),
        "{said:?}"
    );
    let listing = c.rows();
    assert!(
        listing
            .iter()
            .any(|r| r == "4 parts of 18 MB, the last 6 MB"),
        "{listing:?}"
    );
    assert!(any_starts(&listing, "Enough space: "), "{listing:?}");
    // 27. Yes: the finish; the parts and big.bin.crc after them.
    let said = c.key("y");
    assert!(
        any_starts(&said, "Split started: 1 item to out"),
        "{said:?}"
    );
    assert!(any_starts(&said, "Split finished: 5 done."), "{said:?}");
    c.key("tab");
    let names = c.names();
    let at = |name: &str| names.iter().position(|n| n == name);
    for name in ["big.bin.001", "big.bin.002", "big.bin.003", "big.bin.004"] {
        assert!(at(name).is_some(), "{names:?}");
        assert!(at(name) < at("big.bin.crc"), "{names:?}");
    }
    // 28. big.bin.crc in the viewer: its three lines.
    c.focus("big.bin.crc");
    c.key("f3");
    assert_eq!(c.line(1), "filename=big.bin");
    assert_eq!(c.line(2), format!("size={BIG}"));
    assert!(c.line(3).starts_with("crc32="), "{}", c.line(3));
    c.key("escape");
    // 29. Part 003 renamed away: said first, nothing joined.
    fs::rename(out.join("big.bin.003"), out.join("kept.bin")).unwrap();
    c.key("ctrl+r");
    c.focus("big.bin.001");
    let said = c.command(ids::JOIN);
    assert!(
        any_starts(&said, "Part 003 is missing. Nothing was joined."),
        "{said:?}"
    );
    fs::rename(out.join("kept.bin"), out.join("big.bin.003")).unwrap();
    c.key("ctrl+r");
    // 30. Join from part 002 into the joined folder: the question, the
    // verdict, the offer answered no.
    c.key("tab");
    c.focus("joined");
    c.key("enter");
    c.key("tab");
    c.focus("big.bin.002");
    let said = c.command(ids::JOIN);
    assert!(
        said.last()
            .is_some_and(|q| q.starts_with("Join ") && q.ends_with("No is the default. y or n")),
        "{said:?}"
    );
    let said = c.key("y");
    assert!(
        any_starts(&said, "Join started: 4 items to joined"),
        "{said:?}"
    );
    assert!(
        any_starts(&said, "Joined big.bin: checksum matches."),
        "{said:?}"
    );
    assert!(
        last_is(
            &said,
            "Move the 4 parts to the trash? No is the default. y or n"
        ),
        "{said:?}"
    );
    let said = c.key("n");
    assert_eq!(said, ["The parts stay where they are."]);
    let joined = fs::metadata(demo.join("joined").join("big.bin")).unwrap();
    assert_eq!(joined.len(), BIG as u64);
    assert!(out.join("big.bin.001").exists());

    // Links, permissions and information. 32. Alt+Enter on course.zip.
    c.key("tab");
    c.key("left");
    c.focus("course.zip");
    let said = c.key("alt+enter");
    assert!(
        any_starts(&said, "Details of course.zip, Type: "),
        "{said:?}"
    );
    let rows = c.rows();
    assert!(rows[0].starts_with("Type: "), "{rows:?}");
    assert!(rows[1].starts_with("Size: "), "{rows:?}");
    assert!(rows.iter().any(|r| r == "Entries: 10 items"), "{rows:?}");
    c.key("escape");
    // 33. Change permissions on main.rs: its words first.
    c.focus("main.rs");
    let said = c.command(ids::PERMISSIONS);
    assert!(
        any_starts(&said, "Permissions, Change permissions of main.rs"),
        "{said:?}"
    );
    c.key("escape");
    // 34. Make symbolic link: made, or developer mode said.
    let said = c.command(ids::MAKE_LINK);
    assert!(
        last_is(&said, "Make link, Make a link to main.rs in out, 1 of 4"),
        "{said:?}"
    );
    let said = c.key("enter");
    assert!(
        said.last()
            .is_some_and(|q| q.ends_with("No is the default. y or n")),
        "{said:?}"
    );
    let said = c.key("y");
    assert!(any_starts(&said, "Link started: 1 item to out"), "{said:?}");
    let made = fs::symlink_metadata(out.join("main.rs")).is_ok_and(|m| m.file_type().is_symlink());
    assert!(
        made || any_contains(&said, "Cannot link: needs developer mode"),
        "{said:?}"
    );
    // 35. Shift+F5 proposes a copy name; Escape.
    c.focus("main.rs");
    let said = c.key("shift+f5");
    assert!(
        matches!(c.s.app().dialog(), Some((_, Dialog::Edit { .. }))),
        "{said:?}"
    );
    assert!(said.join(" ").contains("main"), "{said:?}");
    c.key("escape");

    // Keys and the display. 36. The pack key in a terminal is the File
    // menu: F9, File, the Archive submenu, Pack.
    c.key("f9");
    c.key("enter");
    c.key("a");
    let said = c.key("p");
    assert!(
        any_starts(&said, "Pack, Pack 1 item into main.zip"),
        "{said:?}"
    );
    // 37. Meaning inside the first 40 cells: the pack rows, the title
    // and the key bar (the hex row is checked in 17).
    for row in c.rows() {
        let meaning = row.split([':', ',']).next().unwrap_or(&row).to_owned();
        assert!(within_cells(&row, &meaning), "{row}");
    }
    c.key("escape");
    assert!(
        within_cells(&c.line(0), "demo, left panel"),
        "{}",
        c.line(0)
    );
    assert!(within_cells(&c.line(23), "F2 Rename"), "{}", c.line(23));
    // 38. Single-letter keys off in Settings; the viewer's search from
    // the palette.
    c.command(ids::SETTINGS);
    c.answer("single_letters");
    let said = c.answer("off");
    assert!(
        last_is(&said, "Single letters: single keys do nothing"),
        "{said:?}"
    );
    c.key("escape");
    c.focus("notes.txt");
    c.key("f3");
    assert!(c.key("h").is_empty(), "letters are off");
    c.key("ctrl+p");
    c.type_text("search in file");
    let said = c.key("enter");
    assert!(last_is(&said, "Search for, empty"), "{said:?}");
    c.type_text("Mondays");
    let said = c.key("enter");
    assert!(last_is(&said, "Match 1 of 1, line 2"), "{said:?}");
    c.key("escape");
    // No job's end was said twice, as "Splitting finished" before its
    // result.
    for message in c.s.said() {
        assert!(!message.text.ends_with("ing finished"), "{}", message.text);
    }
    // 39. Ctrl+Q quits.
    c.s.key(Key::parse("ctrl+q").unwrap(), c.clock.now());
    assert!(c.s.wants_quit());
}

/// The text of a screen line from a column on: the right panel's half.
fn from_column(line: &str, column: usize) -> String {
    line.chars().skip(column).collect()
}

/// Spike Q2 (research/alpha2-plan.md part 4.9) on the test backend: the
/// quick view turned on, Tab into its region, a move by heading, Escape
/// back; the cursor's cell after every frame; one line said for each
/// change; and in screen-reader mode the preview region frozen while the
/// focus is in the panel, as the inactive panel is, without the one-line
/// announcement being lost.
#[test]
fn spike_q2_the_preview_region_keeps_the_cursor_and_says_one_line_a_change() {
    let dir = temp();
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    let mut report = String::from("# Report\n\n");
    for n in 1..=11 {
        report.push_str(&format!("## Part {n}\n\nText of part {n}.\n\n"));
    }
    fs::write(work.join("report.md"), report).unwrap();
    fs::write(work.join("notes.txt"), NOTES).unwrap();
    for screen_reader in [true, false] {
        let mut quick = settings(Layout::SideBySide);
        quick.quick_view.wait_ms = 100;
        let (mut one, mut runtime) = app(place(&work), place(&work), quick);
        let mut c = Check {
            s: session(&mut one, &mut runtime, 80, 24, screen_reader),
            clock: Clock::at_start(),
            mark: 0,
        };
        c.s.start(c.clock.now());
        c.settle();
        c.focus("report.md");
        // On: its own frame says "Quick view on"; the render's line
        // comes on the next.
        c.s.key(Key::parse("shift+f3").unwrap(), c.clock.now());
        c.s.draw(c.clock.now()).unwrap();
        c.cursor();
        assert_eq!(c.line(21), "Quick view on");
        let said = c.settle();
        assert_eq!(said, ["Quick view on", "Preview: report.md, 12 headings"]);
        assert_eq!(c.line(21), "Preview: report.md, 12 headings");
        let title = if screen_reader {
            "Preview"
        } else {
            "Preview, report.md"
        };
        assert_eq!(from_column(&c.line(1), 40), title);
        let region: Vec<String> = (2..21).map(|n| from_column(&c.line(n), 40)).collect();
        // The cursor moves to notes.txt: one line, on the status line.
        let said = c.focus("notes.txt");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("Preview: notes.txt"), "{said:?}");
        assert_eq!(c.line(21), said[0]);
        let now: Vec<String> = (2..21).map(|n| from_column(&c.line(n), 40)).collect();
        if screen_reader {
            // The region waits for Tab; the line was said all the same.
            assert_eq!(now, region);
            assert_eq!(now[0], "Tab goes into the preview");
        } else {
            assert_ne!(now, region);
        }
        let said = c.focus("report.md");
        assert_eq!(said, ["Preview: report.md, 12 headings"]);
        // Tab: one line, the cursor on the region's focused row.
        let said = c.key("tab");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("Preview, report.md, Report"),
            "{said:?}"
        );
        let screen = c.s.screen();
        assert_eq!(screen.cursor_column, 40);
        assert!(
            from_column(&c.focused_line(), 40).starts_with("Report"),
            "{:#?}",
            screen.lines
        );
        // By heading: one line, the cursor on the heading's row.
        let said = c.key("h");
        assert_eq!(said, ["Heading 2: Part 1, 2 of 12"]);
        assert!(
            from_column(&c.focused_line(), 40).starts_with("Part 1"),
            "{}",
            c.focused_line()
        );
        // A move by line: the row is under the cursor; nothing on the
        // status line changes.
        let status = c.line(21);
        let said = c.key("down");
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(c.line(21), status);
        // Escape: back on the panel's row, column 0.
        let said = c.key("escape");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("Left panel: work, "), "{said:?}");
        assert_eq!(c.s.screen().cursor_column, 0);
        assert!(c.focused_line().trim_start().starts_with("report.md"));
        let said = c.key("shift+f3");
        assert_eq!(said, ["Quick view off"]);
    }
}
