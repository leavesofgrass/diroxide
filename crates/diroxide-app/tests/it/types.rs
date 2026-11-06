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
