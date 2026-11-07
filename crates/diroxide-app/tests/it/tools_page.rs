//! The Settings Tools page (alpha 4, lane VA, R30 commit 42): each helper
//! program in the tools registry's words, its name first; a missing one's
//! install command copied on Enter, or its component offered through the
//! ask-first question. The registry looks on an empty PATH, so the report
//! runs no program.

use std::fs;

use diroxide_app::AppEvent;
use diroxide_app::commands::ids;
use diroxide_app::components::{ComponentEvent, Listing, Row};
use diroxide_components::messages::AskFirst;
use diroxide_components::{Kind, Status};
use diroxide_core::Effect;
use diroxide_tools::{Platform, ToolRegistry};
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, place, temp};

fn session(root: &std::path::Path) -> Session {
    let work = root.join("work");
    let empty = root.join("no-programs");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&empty).unwrap();
    let mut s = Session::open(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup::default(),
    );
    s.app.set_tools(
        ToolRegistry::builtin()
            .with_path(Some(empty.into_os_string()))
            .with_platform(Platform::Windows, None),
    );
    s
}

fn list(s: &Session) -> (String, Vec<(String, String)>) {
    match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => (
            list.title().to_owned(),
            list.rows()
                .iter()
                .map(|r| (r.id.clone(), r.spoken()))
                .collect(),
        ),
        other => panic!("{other:?}"),
    }
}

/// Moves the list's focus down to the row with this id.
fn go_to(s: &mut Session, id: &str) {
    let (_, rows) = list(s);
    let at = rows.iter().position(|(i, _)| i == id).unwrap();
    s.key("home");
    for _ in 0..at {
        s.key("down");
    }
}

#[test]
fn the_tools_list_says_each_tool_and_enter_copies_its_install_command() {
    let dir = temp();
    let mut s = session(dir.path());
    let said = s.command(ids::TOOLS_LIST);
    let (title, rows) = list(&s);
    assert!(title.starts_with("Tools: 1 of "), "{title}");
    assert!(title.ends_with(" found"), "{title}");
    assert!(said.last().unwrap().starts_with("Tools: 1 of "), "{said:?}");
    let spoken: Vec<&str> = rows.iter().map(|(_, s)| s.as_str()).collect();
    assert!(spoken.contains(&"carta: 0.0.13, built in"), "{spoken:?}");
    assert!(
        spoken.contains(&"Tesseract: not found, Install: winget install UB-Mannheim.TesseractOCR"),
        "{spoken:?}"
    );
    for (_, line) in &rows {
        let name = line.split(':').next().unwrap();
        assert!(diroxide_core::format::within_cells(line, name), "{line}");
    }
    go_to(&mut s, "tesseract");
    let said = s.key("enter");
    assert_eq!(said, ["Copied: winget install UB-Mannheim.TesseractOCR"]);
    assert!(s.effects.iter().any(|e| matches!(
        e,
        Effect::SetClipboardText(t) if t == "winget install UB-Mannheim.TesseractOCR"
    )));
    go_to(&mut s, "carta");
    assert_eq!(s.key("enter"), ["carta: 0.0.13, built in"]);
    s.key("escape");
    assert!(s.dialog().is_none());
}

#[test]
fn settings_open_the_tools_list_and_a_component_is_offered_first() {
    let dir = temp();
    let mut s = session(dir.path());
    s.command(ids::SETTINGS);
    go_to(&mut s, diroxide_app::settings::TOOLS_ROW);
    s.key("enter");
    let (title, _) = list(&s);
    assert!(title.starts_with("Tools: "), "{title}");
    // A helper component that brings tesseract: the row says what it
    // downloads, and Enter asks first, starting on No.
    let listing = Listing {
        rows: vec![Row {
            id: "tesseract-ocr".to_owned(),
            title: "Tesseract OCR".to_owned(),
            line: "Tesseract OCR: not installed".to_owned(),
            status: Status::NotInstalled,
            kind: Kind::Helper,
            features: Vec::new(),
            ask: AskFirst {
                lines: vec!["Tesseract OCR reads text in pictures.".to_owned()],
                question: "Download Tesseract OCR, 48.2 MB? y or n".to_owned(),
            },
            ask_for_feature: None,
            no_build: Vec::new(),
            programs: vec!["tesseract".to_owned()],
            size: "48.2 MB".to_owned(),
        }],
        ..Listing::default()
    };
    let effects = s
        .app
        .handle(AppEvent::Component(Box::new(ComponentEvent::Listed(
            Box::new(listing),
        ))));
    s.run(effects);
    let (_, rows) = list(&s);
    let tesseract = rows.iter().find(|(id, _)| id == "tesseract").unwrap();
    assert_eq!(
        tesseract.1,
        "Tesseract: not found, 48.2 MB to download, Install: winget install UB-Mannheim.TesseractOCR"
    );
    go_to(&mut s, "tesseract");
    let said = s.key("enter");
    assert!(
        said[0]
            .starts_with("Tesseract OCR reads text in pictures. Download Tesseract OCR, 48.2 MB?"),
        "{said:?}"
    );
    let said = s.key("escape");
    assert_eq!(said.last().unwrap(), "Not downloaded.");
}
