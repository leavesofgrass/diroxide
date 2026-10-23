//! Optional components in the app (W4d, R22 commit 41): Manage optional
//! components, the ask-first question starting on No, a download as a job
//! from a folder source (never the network), Verify, Remove with its
//! listing, Get example plugins, and the first-run list built and not
//! shown.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_app::Dialog;
use diroxide_app::commands::ids;
use diroxide_components::fake::FakeFetcher;
use diroxide_config::Paths;

use crate::support::{Session, Setup, fixture, place, settings, temp};

/// A folder source with its own list: one small data pack, "Test pack".
fn source(root: &Path) -> PathBuf {
    let folder = root.join("source");
    fs::create_dir_all(folder.join("test-pack-0.1.0")).unwrap();
    fs::write(folder.join("test-pack-0.1.0").join("pack.txt"), b"abc").unwrap();
    fs::write(
        folder.join("components.toml"),
        r#"format = 1

[[component]]
id = "test-pack"
title = "Test pack"
kind = "data"
version = "0.1.0"
license = "MIT"
action = "place"

[[component.file]]
name = "pack.txt"
size = 3
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
target = "any"
"#,
    )
    .unwrap();
    folder
}

struct World {
    _dir: tempfile::TempDir,
    paths: Paths,
    source: PathBuf,
    work: PathBuf,
}

fn world() -> World {
    let dir = temp();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    let source = source(dir.path());
    let work = fixture(dir.path());
    World {
        _dir: dir,
        paths,
        source,
        work,
    }
}

fn session(w: &World, fetcher: Option<Arc<FakeFetcher>>) -> Session {
    let mut settings = settings();
    settings.components_source = w.source.to_string_lossy().into_owned();
    let here = place(&w.work);
    Session::open(
        here.clone(),
        here,
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings,
            paths: Some(w.paths.clone()),
            component_fetcher: fetcher.map(|f| f as Arc<dyn diroxide_components::Fetcher>),
            ..Setup::default()
        },
    )
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        _ => Vec::new(),
    }
}

fn focus_row(s: &mut Session, starts: &str) {
    for _ in 0..20 {
        let focused = match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.label.clone()),
            _ => None,
        };
        if focused.is_some_and(|f| f.starts_with(starts)) {
            return;
        }
        s.key("down");
    }
    panic!("no row starts with {starts}: {:?}", rows(s));
}

#[test]
fn manage_downloads_after_a_yes_then_verifies_and_removes() {
    let w = world();
    let mut s = session(&w, None);
    let said = s.command(ids::COMPONENTS_MANAGE);
    assert_eq!(said[0], "Reading the optional components");
    assert_eq!(s.dialog_title().as_deref(), Some("Optional components"));
    let list = rows(&s);
    assert!(
        list[0].starts_with("Example plugins: not installed, "),
        "{list:?}"
    );
    assert!(
        list.contains(&"Test pack: not installed, 3 bytes, license MIT".to_owned()),
        "{list:?}"
    );
    focus_row(&mut s, "Test pack");
    s.key("enter");
    assert_eq!(
        rows(&s),
        [
            "Download",
            "Install from a zip file",
            "Install from a folder"
        ]
    );
    let said = s.key("enter");
    assert_eq!(
        said[0],
        "Test pack 0.1.0, test-pack. Download it now? No is the default. y or n Size 3 bytes, license MIT. From your own components source."
    );
    let pack = w.paths.components_dir().join("test-pack");
    assert_eq!(s.key("n"), ["Not downloaded."]);
    assert!(!pack.exists(), "nothing fetched before a yes");
    s.key("enter");
    let said = s.key("y");
    assert!(
        said.contains(&"Downloading Test pack".to_owned()),
        "{said:?}"
    );
    assert!(
        said.contains(&"Ready: test-pack 0.1.0.".to_owned()),
        "{said:?}"
    );
    assert_eq!(fs::read(pack.join("pack.txt")).unwrap(), b"abc");
    assert!(s.dialog().is_none());
    let history = s.app.jobs().history();
    assert_eq!(
        history.last().unwrap().kind,
        diroxide_core::JobKind::Download
    );

    s.command(ids::COMPONENTS_MANAGE);
    focus_row(&mut s, "Test pack");
    assert!(
        rows(&s).contains(&"Test pack: installed, 3 bytes, license MIT".to_owned()),
        "{:?}",
        rows(&s)
    );
    s.key("enter");
    focus_row(&mut s, "Verify");
    assert_eq!(
        s.key("enter").last().unwrap(),
        "Files check out: Test pack."
    );
    focus_row(&mut s, "Remove");
    let said = s.key("enter");
    assert_eq!(said[0], "Remove Test pack? No is the default. y or n");
    let Some(Dialog::Confirm { listing, .. }) = s.dialog() else {
        panic!("the removal asks first");
    };
    assert!(
        listing.last().unwrap().starts_with("Removes 1 file."),
        "{listing:?}"
    );
    assert_eq!(s.key("n"), ["Nothing was removed."]);
    assert!(pack.join("pack.txt").exists());
    s.key("enter");
    let said = s.key("y");
    assert!(said.contains(&"Removed: Test pack.".to_owned()), "{said:?}");
    assert!(!pack.join("pack.txt").exists());
}

#[test]
fn get_example_plugins_asks_first_and_fetches_nothing_before_a_yes() {
    let w = world();
    let fetcher = Arc::new(FakeFetcher::new());
    let mut s = session(&w, Some(fetcher.clone()));
    s.command(ids::PLUGINS_EXAMPLES);
    let Some(Dialog::Confirm {
        question, listing, ..
    }) = s.dialog()
    else {
        panic!("the ask-first question");
    };
    assert!(
        question
            .text
            .starts_with("Getting example plugins needs Example plugins 0.1.0. Download it now? No is the default."),
        "{}",
        question.text
    );
    assert!(listing[0].starts_with("Size "), "{listing:?}");
    assert!(
        listing[0].contains("license Apache-2.0 OR MIT"),
        "{listing:?}"
    );
    assert_eq!(s.key("n"), ["Not downloaded."]);
    assert!(
        fetcher.requests().is_empty(),
        "nothing fetched before a yes"
    );
    assert!(s.dialog().is_none());
}

#[test]
fn the_first_run_list_is_built_and_not_shown_until_stage_three() {
    let w = world();
    let mut s = session(&w, None);
    assert!(s.dialog().is_none(), "a first start is not interrupted");
    s.command(ids::COMPONENTS_FIRST_RUN);
    assert_eq!(
        s.dialog_title().as_deref(),
        Some("Optional components to get now")
    );
    assert!(
        rows(&s).contains(&"Test pack: not installed, 3 bytes, license MIT".to_owned()),
        "the source's component is a first-run choice: {:?}",
        rows(&s)
    );
    assert!(
        !rows(&s).iter().any(|r| r.starts_with("Example plugins")),
        "the examples pack is not offered"
    );
    assert!(!s.app.settings().components_first_run_shown);
}
