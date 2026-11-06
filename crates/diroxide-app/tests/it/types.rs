//! File types in the app (alpha 4, lane TA): the background type check,
//! driven by hand without a runtime, so every request and answer is seen.
//! Nothing here touches the disk: the entries are made in memory.

use diroxide_app::commands::ids;
use diroxide_app::request::Request;
use diroxide_app::typing::{TypeItem, check_item};
use diroxide_app::{App, AppEvent, AppOptions, Dialog};
use diroxide_core::{Command, Effect, Entry, EntryKind, Place, Side};
use diroxide_filetype::TypeRegistry;

fn local(path: &str) -> Place {
    if cfg!(windows) {
        Place::parse(&format!("D:{}", path.replace('/', "\\"))).unwrap()
    } else {
        Place::parse(path).unwrap()
    }
}

/// Sixty files whose names say nothing and a photo named PNG, listed by
/// hand in the left panel, ten rows a page.
fn listed() -> App {
    let mut app = App::new(AppOptions::new(local("/work"), local("/other")).in_utc());
    app.set_page_rows(Side::Left, 10);
    let effects = app.start();
    for effect in effects {
        if let Effect::List { request, place } = effect {
            let entries = if place == local("/work") {
                let mut entries: Vec<Entry> = (0..60)
                    .map(|i| Entry::new(format!("n{i:02}"), EntryKind::File).with_size(20))
                    .collect();
                entries.push(Entry::new("photo.png", EntryKind::File).with_size(20));
                entries
            } else {
                Vec::new()
            };
            app.handle(AppEvent::Listed {
                request,
                place,
                result: Ok(entries),
            });
        }
    }
    app
}

/// The type checks the app asked for: the listing, the urgent count and
/// the items' names in order.
fn checks(app: &mut App) -> Vec<(u64, usize, Vec<String>)> {
    app.take_requests()
        .into_iter()
        .filter_map(|request| match request {
            Request::CheckTypes {
                side: Side::Left,
                listing,
                urgent,
                items,
                ..
            } => Some((listing, urgent, items.into_iter().map(|i| i.name).collect())),
            _ => None,
        })
        .collect()
}

fn row(app: &App, name: &str) -> String {
    app.panel(Side::Left)
        .list()
        .rows()
        .iter()
        .find(|r| r.id == name)
        .unwrap()
        .spoken()
}

fn answer(app: &App, name: &str, head: &[u8]) -> diroxide_app::typing::TypeAnswer {
    let panel = app.panel(Side::Left);
    let index = panel
        .all_entries()
        .iter()
        .position(|e| e.name == name)
        .unwrap();
    let item = TypeItem::of(index, &panel.all_entries()[index]);
    check_item(
        &TypeRegistry::builtin(),
        &item,
        Some(head),
        &diroxide_core::SystemClock::utc(),
    )
}

/// R30 commit 29: the rows say the names' types at once and "type
/// pending" where a name says nothing; the check starts at the focus with
/// a page of urgent rows; scrolling a page away cancels the queue and asks
/// again from the new focus, while a small move does not.
#[test]
fn scrolling_cancels_the_queue_and_asks_again_from_the_focus() {
    let mut app = listed();
    assert_eq!(row(&app, "n00"), "n00, type pending, 20 bytes");
    assert_eq!(row(&app, "photo.png"), "photo.png, PNG, 20 bytes");
    let first = checks(&mut app);
    assert_eq!(first.len(), 1, "{first:?}");
    let (listing, urgent, names) = &first[0];
    assert_eq!(*urgent, 10);
    assert_eq!(names.len(), 61);
    assert_eq!(&names[..3], ["n00", "n01", "n02"]);

    app.dispatch(&Command::new(ids::LIST_DOWN));
    assert!(checks(&mut app).is_empty(), "a small move keeps the queue");

    app.dispatch(&Command::new(ids::LIST_END));
    let again = checks(&mut app);
    assert_eq!(again.len(), 1, "{again:?}");
    assert_eq!(again[0].0, *listing, "the same entries");
    assert_eq!(&again[0].2[..3], ["photo.png", "n59", "n58"]);
}

/// R30 commit 29: the answers change the rows quietly (a redraw, never an
/// announcement); a disagreement of name and contents is said on request,
/// in the details and when the file is opened.
#[test]
fn answers_change_the_rows_and_a_disagreement_is_said_on_request() {
    let mut app = listed();
    let (listing, _, _) = checks(&mut app).remove(0);
    let answers = vec![
        answer(&app, "photo.png", b"\xFF\xD8\xFF\xE0\0\x10JFIF\0"),
        answer(&app, "n00", b"#!/bin/sh\necho hello\n"),
    ];
    let effects = app.handle(AppEvent::TypesChecked {
        side: Side::Left,
        listing,
        answers,
    });
    assert!(
        effects.iter().all(|e| !matches!(e, Effect::Announce(_))),
        "{effects:?}"
    );
    assert!(effects.iter().any(|e| matches!(e, Effect::Redraw(_))));
    assert_eq!(row(&app, "photo.png"), "photo.png, JPEG, 20 bytes");
    assert_eq!(row(&app, "n00"), "n00, shell, 20 bytes");
    // An answer for entries replaced since is dropped.
    let stale = vec![answer(&app, "n01", b"%PDF-1.7\n")];
    app.handle(AppEvent::TypesChecked {
        side: Side::Left,
        listing: listing + 7,
        answers: stale,
    });
    assert_eq!(row(&app, "n01"), "n01, type pending, 20 bytes");

    app.dispatch(&Command::new(ids::LIST_END));
    app.dispatch(&Command::new(ids::DETAILS));
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no details");
    };
    let lines: Vec<String> = list.rows().iter().map(|r| r.label.clone()).collect();
    assert!(
        lines.contains(&"Name says PNG; contents say JPEG".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"File group: Images".to_owned()),
        "{lines:?}"
    );
    app.dispatch(&Command::new(ids::CANCEL));
    let effects = app.dispatch(&Command::new(ids::ENTER));
    let said: Vec<String> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Announce(m) => Some(m.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        said,
        ["Opening photo.png. Name says PNG; contents say JPEG."]
    );
    assert!(diroxide_core::format::within_cells(
        &said[0],
        "Name says PNG"
    ));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenExternally(_)))
    );
}

/// The owner's types with actions: PDF with an Enter action, a convert
/// tool that is here, a user action whose tool is missing and a named
/// one; PDF/A with none, so it takes PDF's. `here` is a file standing in
/// for pandoc's program, never run.
fn typed_app(here: &std::path::Path) -> App {
    let filetypes = diroxide_config::FileTypesConfig::from_toml(
        r#"
[[type]]
id = "pdf"
name = "PDF document"
short = "PDF"
group = "documents"
extensions = ["pdf"]
magic = [{ at = 0, hex = "25 50 44 46" }]

[[type.actions]]
name = "Open in reader"
slot = "enter"
command = "file.details"

[[type.actions]]
name = "Make thumbnail"
slot = "user3"
tool = "magick"
args = ["{file}", "thumb.png"]

[[type.actions]]
name = "To text"
slot = "convert"
tool = "pandoc"
args = ["{file}", "-o", "{name}.txt"]

[[type.actions]]
name = "Say hello"
plugin = "hello"
args = ["say"]

[[type]]
id = "pdfa"
name = "PDF/A document"
short = "PDF/A"
parent = "pdf"
extensions = ["pdfa"]
"#,
    )
    .unwrap();
    let mut tools = diroxide_config::ToolsConfig::default();
    tools.tools.insert(
        "magick".to_owned(),
        diroxide_config::ToolOverride {
            path: Some(here.join("not-here").display().to_string()),
            ..Default::default()
        },
    );
    tools.tools.insert(
        "pandoc".to_owned(),
        diroxide_config::ToolOverride {
            path: Some(here.join("pandoc-stand-in").display().to_string()),
            ..Default::default()
        },
    );
    std::fs::write(here.join("pandoc-stand-in"), b"not a program").unwrap();
    let mut options = AppOptions::new(local("/work"), local("/other")).in_utc();
    options.types = TypeRegistry::builtin().with_config(&filetypes);
    options.startup.type_slots = diroxide_app::typing::TypeSlots::from_config(&filetypes);
    options.startup.tools = tools;
    let mut app = App::new(options);
    let effects = app.start();
    for effect in effects {
        if let Effect::List { request, place } = effect {
            let entries = if place == local("/work") {
                ["report.pdf", "notes.txt", "old.pdfa"]
                    .into_iter()
                    .map(|n| Entry::new(n, EntryKind::File).with_size(10))
                    .collect()
            } else {
                Vec::new()
            };
            app.handle(AppEvent::Listed {
                request,
                place,
                result: Ok(entries),
            });
        }
    }
    app.take_requests();
    app
}

fn said(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Announce(m) => Some(m.text.clone()),
            _ => None,
        })
        .collect()
}

/// R30 commit 30: Actions for this file lists the type's actions, the
/// one Enter runs first, each with its slot in words; a tool action whose
/// tool is missing is unavailable and says why; Enter runs the type's
/// Enter action; a user action runs or says the file has none; an empty
/// list takes the parent type's.
#[test]
fn typed_actions_run_from_enter_the_user_slots_and_the_actions_list() {
    let temp = crate::support::temp();
    let mut app = typed_app(temp.path());
    assert!(app.focus_name(Side::Left, "report.pdf"));
    app.dispatch(&Command::new(ids::FILE_ACTIONS));
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no actions list");
    };
    assert_eq!(list.title(), "Actions for report.pdf");
    let rows: Vec<(String, bool)> = list
        .rows()
        .iter()
        .map(|r| (r.spoken(), r.enabled))
        .collect();
    assert_eq!(
        rows,
        [
            ("Open in reader, Enter".to_owned(), true),
            ("View, the built-in viewer".to_owned(), true),
            ("To text, Convert".to_owned(), true),
            (
                "Make thumbnail, unavailable, user action 3, Needs ImageMagick, not found"
                    .to_owned(),
                false
            ),
            ("Say hello".to_owned(), true),
        ]
    );
    for line in ["Actions for report.pdf", "Needs ImageMagick, not found."] {
        assert!(line.chars().count() <= 40, "{line}");
    }
    app.dispatch(&Command::new(ids::CANCEL));

    // Enter runs the type's Enter action: here, the details.
    app.dispatch(&Command::new(ids::ENTER));
    assert_eq!(app.dialog_purpose(), Some("Details"));
    app.dispatch(&Command::new(ids::CANCEL));

    let effects = app.dispatch(&Command::new(ids::TYPE_USER_3));
    assert_eq!(said(&effects), ["Needs ImageMagick, not found."]);
    app.focus_name(Side::Left, "notes.txt");
    let effects = app.dispatch(&Command::new(ids::TYPE_USER_3));
    assert_eq!(said(&effects), ["notes.txt has no user action 3."]);

    // PDF/A has no actions of its own: PDF's are its.
    app.focus_name(Side::Left, "old.pdfa");
    app.dispatch(&Command::new(ids::FILE_ACTIONS));
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no actions list");
    };
    assert_eq!(list.title(), "Actions for old.pdfa");
    assert_eq!(list.rows()[0].label, "Open in reader");
    app.dispatch(&Command::new(ids::CANCEL));

    // A tool action runs its program with its arguments filled one by
    // one, never through a shell.
    app.focus_name(Side::Left, "report.pdf");
    app.dispatch(&Command::new(ids::FILE_ACTIONS));
    app.dispatch(&Command::new(ids::LIST_DOWN));
    app.dispatch(&Command::new(ids::LIST_DOWN));
    let effects = app.dispatch(&Command::new(ids::ACCEPT));
    assert_eq!(said(&effects), ["Running To text on report.pdf"]);
    let spawned: Vec<_> = app
        .take_requests()
        .into_iter()
        .filter_map(|r| match r {
            Request::Spawn(spawn) => Some(spawn.kind),
            _ => None,
        })
        .collect();
    let [
        diroxide_app::request::SpawnKind::Program {
            name,
            program,
            args,
        },
    ] = spawned.as_slice()
    else {
        panic!("{spawned:?}");
    };
    assert_eq!(name, "To text");
    assert_eq!(program, &temp.path().join("pandoc-stand-in"));
    let file = local("/work").join("report.pdf").unwrap();
    assert_eq!(
        args,
        &[
            std::ffi::OsString::from(file.local_path().unwrap()),
            "-o".into(),
            "report.pdf.txt".into()
        ]
    );
}

/// The owner's group of receipts, by name, beside the built-in groups;
/// a folder of two receipts, a photo, a screenshot and a note.
fn grouped_app() -> App {
    let filetypes = diroxide_config::FileTypesConfig::from_toml(
        r#"
[[group]]
id = "receipts"
name = "Receipts"
about = "Receipts, by their names."
patterns = ["receipt-*"]
"#,
    )
    .unwrap();
    let mut options = AppOptions::new(local("/work"), local("/other")).in_utc();
    options.types = TypeRegistry::builtin().with_config(&filetypes);
    listed_with(
        App::new(options),
        &[
            "receipt-01.pdf",
            "receipt-02.pdf",
            "crow.png",
            "Screenshot (3).png",
            "notes.txt",
        ],
    )
}

/// The app started, its left panel listed with these files by hand.
fn listed_with(mut app: App, names: &[&str]) -> App {
    let effects = app.start();
    for effect in effects {
        if let Effect::List { request, place } = effect {
            let entries = if place == local("/work") {
                names
                    .iter()
                    .map(|n| Entry::new(*n, EntryKind::File).with_size(10))
                    .collect()
            } else {
                Vec::new()
            };
            app.handle(AppEvent::Listed {
                request,
                place,
                result: Ok(entries),
            });
        }
    }
    app.take_requests();
    app
}

fn dialog_rows(app: &App) -> (String, Vec<String>) {
    let Some((_, Dialog::Choose { list, .. })) = app.dialog() else {
        panic!("no list open");
    };
    (
        list.title().to_owned(),
        list.rows().iter().map(|r| r.label.clone()).collect(),
    )
}

/// R30 commit 31: a user group appears in Select by group; the Groups
/// list says each group's files here, the owner's own among them, and
/// Enter selects them; the last row turns the retro types on; `group:`
/// reads the owner's groups and matches a file in two groups by either.
#[test]
fn a_user_group_appears_in_select_by_group_and_the_groups_list() {
    let mut app = grouped_app();
    app.dispatch(&Command::new(ids::SELECT_BY_KIND));
    let (_, kinds) = dialog_rows(&app);
    assert!(kinds.contains(&"Receipts, group".to_owned()), "{kinds:?}");
    assert!(
        kinds.contains(&"Screenshots, group".to_owned()),
        "{kinds:?}"
    );
    app.dispatch(&Command::new(ids::CANCEL));

    app.dispatch(&Command::new(ids::GROUPS_LIST));
    let (title, rows) = dialog_rows(&app);
    assert_eq!(title, "Groups: 8 built in, 1 yours");
    assert_eq!(
        rows,
        [
            "Images: 2 files here",
            "Music: no files here",
            "Video: no files here",
            "Documents: 2 files here",
            "Archives: no files here",
            "Code: no files here",
            "Text: 1 file here",
            "Screenshots: 1 file here",
            "Receipts: 2 files here",
            "Retro types: off",
        ]
    );
    for line in &rows {
        assert!(line.chars().count() <= 40, "{line}");
    }
    app.dispatch(&Command::new(ids::LIST_END));
    app.dispatch(&Command::new(ids::LIST_UP));
    let effects = app.dispatch(&Command::new(ids::ACCEPT));
    assert_eq!(said(&effects), ["Selected 2 receipts. 2 selected."]);
    assert_eq!(app.panel(Side::Left).list().selected_count(), 2);

    app.dispatch(&Command::new(ids::GROUPS_LIST));
    app.dispatch(&Command::new(ids::LIST_END));
    let effects = app.dispatch(&Command::new(ids::ACCEPT));
    assert_eq!(said(&effects), ["Retro types: on"]);
    assert!(app.settings().filetypes.retro);

    let filter = |app: &mut App, text: &str| {
        app.dispatch(
            &Command::new(ids::FILTER)
                .with_arg("pattern", diroxide_core::ArgValue::Text(text.to_owned())),
        );
        let mut names: Vec<String> = app
            .panel(Side::Left)
            .entries()
            .iter()
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        filter(&mut app, "group:screenshots"),
        ["Screenshot (3).png"]
    );
    assert_eq!(
        filter(&mut app, "group:images"),
        ["Screenshot (3).png", "crow.png"]
    );
    assert_eq!(
        filter(&mut app, "group:receipt"),
        ["receipt-01.pdf", "receipt-02.pdf"]
    );
}

/// R30 commit 31: a theme may map a group to one of its existing color
/// roles; the row's words are unchanged, and the default maps none.
#[test]
fn a_theme_maps_a_group_to_an_existing_role() {
    let temp = crate::support::temp();
    std::fs::write(
        temp.path().join("crows.toml"),
        "[theme]\nname = \"crows\"\ndisplay_name = \"Crows\"\ninherits = \"galaxy\"\n\n[group-colors]\nimages = \"heading2\"\nnot-a-group = \"heading1\"\n",
    )
    .unwrap();
    let mut themes = diroxide_theme::Registry::builtin();
    let report = themes.load_dir(temp.path());
    assert_eq!(report.loaded, ["crows"]);
    let crows = themes.get("crows").unwrap().clone();
    let open = |theme: Option<&str>| {
        let mut options = AppOptions::new(local("/work"), local("/other")).in_utc();
        if let Some(theme) = theme {
            options.settings.theme = theme.to_owned();
        }
        options.startup.user_themes = vec![crows.clone()];
        listed_with(App::new(options), &["crow.png", "notes.txt"])
    };
    let app = open(None);
    assert_eq!(
        app.row_role(Side::Left, "crow.png"),
        None,
        "none by default"
    );
    let app = open(Some("crows"));
    assert_eq!(
        app.row_role(Side::Left, "crow.png"),
        Some(diroxide_theme::ColorRole::Heading2)
    );
    assert_eq!(app.row_role(Side::Left, "notes.txt"), None);
    let row = app.panel(Side::Left).list().rows()[0].spoken();
    assert_eq!(row, "crow.png, PNG, 10 bytes", "the words never change");
}

/// R30 commit 31: Open with lists the system's programs for the focused
/// file, read off the input thread: "Default program" first, each by
/// name, "Choose another program" last; Enter starts the chosen one
/// through the runtime, here a recorder, so no program is started.
#[test]
fn open_with_lists_the_programs_and_starts_one_through_the_runtime() {
    use crate::support::{Recorder, Session, Setup, fixture, place, temp};
    let dir = temp();
    let work = fixture(dir.path());
    let recorder = std::sync::Arc::new(Recorder::default());
    let mut s = Session::open(
        place(&work),
        place(&work.join("Backup")),
        diroxide_vfs::Registry::with_local(),
        Setup {
            recorder: recorder.clone(),
            ..Setup::default()
        },
    );
    s.focus("report.pdf");
    let said = s.command(ids::OPEN_WITH);
    assert_eq!(
        said,
        [
            "Finding programs for report.pdf",
            "Open report.pdf with: Default program, Reader, 1 of 4"
        ]
    );
    let (title, rows) = dialog_rows(&s.app);
    assert_eq!(title, "Open report.pdf with:");
    assert_eq!(
        rows,
        [
            "Default program",
            "Reader",
            "Writer",
            "Choose another program"
        ]
    );
    let spoken: Vec<String> = match s.app.dialog() {
        Some((_, Dialog::Choose { list, .. })) => list.rows().iter().map(|r| r.spoken()).collect(),
        _ => Vec::new(),
    };
    assert_eq!(spoken[0], "Default program, Reader");
    assert_eq!(spoken[1], "Reader, the default");
    s.key("down");
    s.key("down");
    let said = s.key("enter");
    assert_eq!(said, ["Opening report.pdf with Writer"]);
    let file = work.join("report.pdf");
    assert_eq!(
        recorder.started_with.lock().unwrap().as_slice(),
        [("writer".to_owned(), file.clone())]
    );
    s.command(ids::OPEN_WITH);
    s.key("end");
    let said = s.key("enter");
    assert_eq!(said, ["System's Open with list: report.pdf"]);
    assert_eq!(
        recorder.chosen_another.lock().unwrap().as_slice(),
        std::slice::from_ref(&file)
    );
    s.command(ids::OPEN_WITH);
    let said = s.key("enter");
    assert_eq!(said, ["Opening report.pdf"]);
    assert_eq!(recorder.opened.lock().unwrap().as_slice(), [file]);
}

/// A folder with a file in Windows-1251: "Privet" in Cyrillic letters,
/// which the automatic choice reads as Windows-1252.
fn cyrillic_folder(root: &std::path::Path) -> std::path::PathBuf {
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("legacy.txt"), b"\xcf\xf0\xe8\xe2\xe5\xf2\nok\n").unwrap();
    work
}

/// R30 commit 32: the encoding chooser in the viewer. "Automatic" first
/// with what it found, then the encodings by language; a choice reads
/// the file again in it and says so; the next time the list opens on
/// the choice; Automatic goes back.
#[test]
fn the_viewer_reads_a_file_in_the_encoding_chosen() {
    use crate::support::{Session, place, settings, temp};
    let dir = temp();
    let work = cyrillic_folder(dir.path());
    let mut s = Session::start(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        None,
        settings(),
    );
    s.focus("legacy.txt");
    s.command(ids::VIEW);
    let read = |s: &mut Session| s.app.viewer_mut().unwrap().line_text(0).unwrap();
    assert_eq!(read(&mut s), "\u{cf}\u{f0}\u{e8}\u{e2}\u{e5}\u{f2}");
    s.command(ids::ENCODING);
    let (title, rows) = dialog_rows(&s.app);
    assert_eq!(title, "Encoding of legacy.txt");
    assert_eq!(rows[0], "Automatic: Windows-1252 (not UTF-8)");
    assert_eq!(rows[4], "Western European (Windows-1252)");
    assert!(rows.contains(&"Cyrillic (Windows-1251)".to_owned()));
    for line in &rows {
        assert!(line.chars().count() <= 40, "{line}");
    }
    let to = |s: &mut Session, id: &str| {
        if let Some((_, Dialog::Choose { .. })) = s.app.dialog() {
            let at = dialog_rows(&s.app).1.len();
            for _ in 0..at {
                let focused = match s.app.dialog() {
                    Some((_, Dialog::Choose { list, .. })) => list.focused().unwrap().id.clone(),
                    _ => String::new(),
                };
                if focused == id {
                    break;
                }
                s.key("down");
            }
        }
    };
    to(&mut s, "windows-1251");
    let said = s.key("enter");
    assert_eq!(said, ["Encoding Windows-1251, chosen by you"]);
    assert_eq!(read(&mut s), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
    s.command(ids::ENCODING);
    let focused = match s.app.dialog() {
        Some((_, Dialog::Choose { list, .. })) => list.focused().unwrap().id.clone(),
        _ => String::new(),
    };
    assert_eq!(focused, "windows-1251", "the list opens on the choice");
    s.key("home");
    let said = s.key("enter");
    assert_eq!(said, ["Automatic: Windows-1252 (not UTF-8)"]);
    assert_eq!(read(&mut s), "\u{cf}\u{f0}\u{e8}\u{e2}\u{e5}\u{f2}");
}

/// R30 commit 32: the encoding chooser for the quick view's file; and
/// outside the viewer and the quick view it says where it works.
#[test]
fn the_quick_view_reads_its_file_in_the_encoding_chosen() {
    use crate::support::{Session, Setup, place, settings, temp};
    let dir = temp();
    let work = cyrillic_folder(dir.path());
    let mut quick = settings();
    quick.quick_view.wait_ms = 0;
    let mut s = Session::open(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings: quick,
            ..Setup::default()
        },
    );
    s.focus("legacy.txt");
    let said = s.command(ids::ENCODING);
    assert_eq!(said, ["Encoding needs the viewer or quick view."]);
    s.command(ids::QUICK_VIEW);
    settle(&mut s);
    s.command(ids::ENCODING);
    let (title, rows) = dialog_rows(&s.app);
    assert_eq!(title, "Encoding of legacy.txt");
    assert_eq!(rows[0], "Automatic: Windows-1252 (not UTF-8)");
    let at = rows
        .iter()
        .position(|r| r == "Cyrillic (Windows-1251)")
        .unwrap();
    for _ in 0..at {
        s.key("down");
    }
    let mut said = s.key("enter");
    said.extend(settle(&mut s));
    assert_eq!(said, ["Encoding Windows-1251, chosen by you"]);
    let shown = s.app.quick_view().unwrap();
    let first = shown.view().unwrap().rows()[0].shown();
    assert!(
        first.contains("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}"),
        "{first}"
    );
    assert!(shown.shown().unwrap().encoding.is_some());
}

fn settle(s: &mut crate::support::Session) -> Vec<String> {
    let mut said = Vec::new();
    for _ in 0..400 {
        if s.app.quick_view_wants_tick().is_none() {
            break;
        }
        let effects = s.app.quick_view_tick();
        said.extend(s.run(effects));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    said
}
