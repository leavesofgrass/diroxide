//! The braille walk: a session through the owner's ten-minute check
//! (research/alpha1-plan.md part six) with every line of part three's
//! table asserted to carry its key fact in the first 40 cells, and the
//! catalog walk: the alpha.1 sentences of the catalogs are reachable, and
//! a message that names a key takes it from the live keymap.

use diroxide_app::commands::ids;
use diroxide_app::{AppEvent, AskId, Dialog, Keymap, Question};
use diroxide_config::{KeyOverride, KeymapConfig, Paths};
use diroxide_core::format::{BRAILLE_CELLS, first_cells, within_cells};
use diroxide_core::{Key, KeyColumn, Layer, SecretKind, SecretRequest};

use crate::support::{Session, Setup, fixture, place, registry_with_trash, temp};

/// Asserts a line says `fact` within its first 40 cells.
fn fact(line: &str, fact: &str) {
    assert!(
        within_cells(line, fact),
        "\"{fact}\" is not in the first {BRAILLE_CELLS} cells of \"{line}\" (shown: \"{}\")",
        first_cells(line, BRAILLE_CELLS)
    );
}

/// The line said that starts with `start`.
fn line<'a>(said: &'a [String], start: &str) -> &'a str {
    said.iter()
        .find(|l| l.starts_with(start))
        .unwrap_or_else(|| panic!("nothing said starts with \"{start}\": {said:?}"))
}

#[test]
fn the_ten_minute_check_reads_on_a_40_cell_line() {
    let dir = temp();
    let work = fixture(dir.path());
    let backup = work.join("Backup");
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let (registry, _trash) = registry_with_trash(&dir.path().join("bin"));
    let mut s = Session::open(
        place(&work),
        place(&backup),
        registry,
        Setup {
            paths: Some(paths),
            ..Setup::default()
        },
    );
    let start: Vec<String> = s.said.iter().map(|m| m.text.clone()).collect();
    // Step one: start.
    fact(
        line(&start, "diroxide alpha 1."),
        "diroxide alpha 1. Shift+F1 for help.",
    );
    fact(
        line(&start, "Layout:"),
        "Layout: side by side. Ctrl+P, Layout.",
    );
    fact(
        line(&start, "Folder not watched"),
        "Folder not watched. Reload with Ctrl+R.",
    );
    // Step two: move; every row says its name first.
    for row in s.app.active_panel().list().rows() {
        fact(&row.spoken(), &row.label);
    }
    // Step three: switch panels.
    fact(&s.key("tab")[0], "Right panel: Backup, 1 item");
    fact(&s.key("tab")[0], "Left panel: work, 8 items");
    // Step four: mark, and Say Status.
    s.focus("budget.xlsx");
    fact(&s.key("space")[0], "Selected budget.xlsx. 1 selected.");
    s.key("space");
    fact(&s.key("space")[0], "Selected raccoon \u{1f99d}.txt.");
    fact(&s.key("ctrl+d")[0], "3 selected, 312 bytes. work, 8 items");
    fact(&s.key("alt+o")[0], "Other panel: Backup, 1 item");
    // Step five: sort.
    fact(&s.key("ctrl+f6")[0], "Sorted by size, largest first");
    fact(&s.key("ctrl+f6")[0], "Sorted by size, smallest first");
    fact(&s.key("ctrl+f3")[0], "Sorted by name, A to Z");
    // Step six: copy with a conflict.
    s.key("ctrl+a");
    s.key("*");
    s.focus("report.pdf");
    s.key("space");
    let said = s.key("f5");
    fact(said.last().unwrap(), "Copy 1 item to ");
    let said = s.key("y");
    fact(
        line(&said, "Copy started"),
        "Copy started: 1 item to Backup",
    );
    fact(line(&said, "Name taken"), "Name taken: report.pdf");
    if let Some(Dialog::Question { lines, choices }) = s.dialog() {
        for l in lines {
            let key = l.split([':', '.']).next().unwrap_or(l);
            fact(l, key);
        }
        for row in choices.rows() {
            fact(&row.spoken(), &row.label);
        }
        fact(&choices.announce_focus(), "Skip this one, 1 of 9");
    } else {
        panic!("no conflict question");
    }
    let said = s.key("escape");
    fact(
        line(&said, "Copy finished"),
        "Copy finished: 0 done, 1 skipped.",
    );
    // Step seven: rename, and a name that needs care.
    s.focus("report.pdf");
    fact(&s.key("f2")[0], "Rename report.pdf to: report.pdf");
    s.type_text("-2026");
    let said = s.key("enter");
    fact(
        line(&said, "Renamed"),
        "Renamed report.pdf to report-2026.pdf",
    );
    s.key("f7");
    s.type_text("memo ");
    fact(
        &s.key("enter")[0],
        "Name needs care: ends in a dot or space",
    );
    // Step eight: trash and undo.
    s.focus("report-2026.pdf");
    fact(s.key("f8").last().unwrap(), "Move 1 item to the trash");
    let said = s.key("y");
    fact(line(&said, "Moved"), "Moved 1 item to the trash.");
    let said = s.key("ctrl+z");
    fact(line(&said, "Undid"), "Undid trash: 1 item is back");
    let said = s.key("ctrl+z");
    fact(line(&said, "Undid"), "Undid rename: 1 item is back");
    // Step ten: layout.
    s.key("ctrl+p");
    s.type_text("layout");
    fact(&s.key("enter")[0], "Layout: stacked");
    // The file clipboard and the prompts.
    s.focus("notes.txt");
    fact(&s.key("ctrl+x")[0], "Cut 1 item. Paste with Ctrl+V.");
    fact(&s.key("f7")[0], "New folder in work");
    s.type_text("Reports");
    let said = s.key("enter");
    fact(line(&said, "Created"), "Created folder Reports");
    fact(
        &s.key("shift+delete").last().unwrap().clone(),
        "Delete 1 item for good",
    );
    s.key("n");
    fact(&s.key("ctrl+d")[0], "work, 9 items, no selection");
    // Step twelve: quit while a job waits.
    s.focus("budget.xlsx");
    s.key("tab");
    s.key("tab");
    s.focus("report.pdf");
    s.key("f5");
    s.key("y");
    fact(&s.key("ctrl+q")[0], "Quit? 1 job is running.");
    s.key("n");
    s.key("escape");
    // Every message said in the walk: its first words are in the first
    // 40 cells, and none holds a control character.
    for message in &s.said {
        let words: Vec<&str> = message.text.split_whitespace().take(3).collect();
        fact(&message.text, &words.join(" "));
        assert!(
            !message.text.chars().any(char::is_control),
            "{}",
            message.text
        );
    }
}

#[test]
fn keys_in_messages_come_from_the_live_keymap() {
    let dir = temp();
    let work = fixture(dir.path());
    let (registry, _trash) = registry_with_trash(&dir.path().join("bin"));
    let mut overrides = KeymapConfig::default();
    for (command, key) in [(ids::CLIPBOARD_PASTE, "ctrl+y"), (ids::RELOAD, "ctrl+e")] {
        overrides.overrides.push(KeyOverride {
            column: KeyColumn::Terminal,
            layer: Layer::Panel,
            command,
            keys: vec![Key::parse(key).unwrap()],
            line: 0,
        });
    }
    let keymap = Keymap::defaults().with_overrides(&overrides);
    let mut s = Session::open_with_keymap(
        place(&work),
        place(&work.join("Backup")),
        registry,
        Setup::default(),
        keymap,
    );
    let said: Vec<String> = s.said.iter().map(|m| m.text.clone()).collect();
    assert!(
        said.contains(&"Folder not watched. Reload with Ctrl+E.".to_owned()),
        "{said:?}"
    );
    s.focus("notes.txt");
    assert_eq!(s.key("ctrl+c"), ["Copied 1 item. Paste with Ctrl+Y."]);
}

fn fake_place(text: &str) -> diroxide_core::Place {
    if cfg!(windows) {
        diroxide_core::Place::parse(&format!("D:{}", text.replace('/', "\\"))).unwrap()
    } else {
        diroxide_core::Place::parse(text).unwrap()
    }
}

/// The catalogs' alpha.1 sentences are reachable from the app, each said
/// with its meaning first: the no-trash question with its listing and the
/// honest sentence, the folder gone, the masked prompt's lines, and the
/// startup problems line.
#[test]
fn the_catalog_sentences_of_alpha_1_are_reachable() {
    use diroxide_core::catalog;
    use diroxide_ops::{FakeProvider, FakeSwitches};
    use diroxide_vfs::WatchEvent;

    let fake = FakeProvider::with_switches(FakeSwitches {
        no_trash: true,
        ..FakeSwitches::default()
    });
    fake.add_file(&fake_place("/stick/a.txt"), b"a");
    fake.add_folder(&fake_place("/stick/sub"));
    let mut registry = diroxide_vfs::Registry::new();
    registry.set_local(std::sync::Arc::new(fake.clone()));
    let mut s = Session::open(
        fake_place("/stick"),
        fake_place("/stick"),
        registry,
        Setup::default(),
    );
    s.focus("a.txt");
    let said = s.key("f8");
    let question = said.last().unwrap();
    assert_eq!(
        question,
        "This drive has no trash. Delete 1 item for good? No is the default. y or n"
    );
    fact(question, "This drive has no trash.");
    let Some(Dialog::Confirm { listing, .. }) = s.dialog() else {
        panic!("no question");
    };
    assert_eq!(
        listing.last().map(String::as_str),
        Some(catalog::PERMANENT_DELETE_NOTE)
    );
    s.key("y");
    assert!(!fake.exists(&fake_place("/stick/a.txt")));
    assert_eq!(fake.trashed(), 0, "never the trash, never a fall-through");

    s.focus("sub");
    s.key("enter");
    let effects = s.app.handle(AppEvent::Watched {
        side: diroxide_core::Side::Left,
        place: fake_place("/stick/sub"),
        event: WatchEvent::Gone,
    });
    let said = s.run(effects);
    assert_eq!(said[0], catalog::FOLDER_GONE);
    assert_eq!(s.app.active_panel().place(), &fake_place("/stick"));

    let effects = s.app.handle(AppEvent::Asks {
        ask: AskId::new(2),
        question: Question::Secret {
            job: None,
            request: SecretRequest::new(SecretKind::ArchivePassword, "a.zip"),
        },
    });
    s.run(effects);
    assert_eq!(s.key("enter"), [catalog::SECRET_EMPTY]);
    assert_eq!(s.key("escape"), ["No password given."]);

    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(
        paths.config_file(Paths::KEYMAP),
        "[terminal.panel]\n\"file.copy\" = \"hyper+c\"\n\"no.such\" = \"f5\"\n",
    )
    .unwrap();
    let work = fixture(dir.path());
    let (registry, _trash) = registry_with_trash(&dir.path().join("bin"));
    let s = Session::open(
        place(&work),
        place(&work),
        registry,
        Setup {
            paths: Some(paths),
            ..Setup::default()
        },
    );
    let first = &s.said[0].text;
    fact(first, "keymap.toml: 2 problems");
    assert!(first.ends_with("Config problems in the Help menu lists them."));
}

#[test]
fn the_secret_prompt_reads_on_a_40_cell_line() {
    let dir = temp();
    let work = fixture(dir.path());
    let (registry, _trash) = registry_with_trash(&dir.path().join("bin"));
    let mut s = Session::open(place(&work), place(&work), registry, Setup::default());
    let long = format!("{}.zip", "quarterly-reports-".repeat(5));
    let effects = s.app.handle(AppEvent::Asks {
        ask: AskId::new(1),
        question: Question::Secret {
            job: Some(diroxide_core::JobId::new(5)),
            request: SecretRequest::new(SecretKind::ArchivePassword, long),
        },
    });
    let said = s.run(effects);
    fact(&said[0], "Waiting for the password for");
    fact(&said[1], "Password for quarter");
    fact(&said[1], ".zip (hidden):");
}
