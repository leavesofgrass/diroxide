//! The connect flow and the first run in the window (R27's commit 30):
//! the SFTP plugin's questions as its fake session asks them (the
//! fixtures in `fixtures/sftp/dialogs`, row for row as the plugin host
//! cleans them): a new server key named with the plugin last, the focus
//! on "Do not connect", its message a list read line by line with the
//! fingerprint in hexadecimal groups of four, one part a line inside a
//! Braille line; the password prompt as a Secret whose text is in no
//! node; the Sites group in Go to a place and the Sites list as a plugin
//! list in the panel; the first-run components list once. No network,
//! no plugin runs.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use diroxide_app::AppEvent;
use diroxide_app::app::SiteRow;
use diroxide_app::plugins::{
    Call, CallId, ListerColumn, ListerSpec, PluginEvent, PluginLister, PluginPanel,
    PluginQuestionKind,
};
use diroxide_core::format::{BRAILLE_CELLS, within_cells};
use diroxide_core::{Entry, EntryKind, Place, Side};
use masonry::accesskit::Role;

use crate::plugins::{ask, question, send};
use crate::rig::{Rig, app_on, app_with, fixture};

/// A fixture of the SFTP plugin's dialogs: its rows, the title first.
fn rows(name: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/sftp/dialogs")
        .join(name);
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn window() -> (tempfile::TempDir, Rig) {
    let dir = fixture(&["notes.txt"], &[]);
    let (app, runtime) = app_on(dir.path(), dir.path());
    let mut rig = Rig::new(app, runtime);
    assert!(rig.f.app_mut().focus_name(Side::Left, "notes.txt"));
    rig.refresh();
    (dir, rig)
}

#[test]
fn a_new_server_key_is_read_line_by_line_and_starts_on_do_not_connect() {
    let (_dir, mut rig) = window();
    let golden = rows("new-host-key.rows");
    let mut new_key = question(
        "New server key",
        PluginQuestionKind::Confirm {
            destructive: true,
            yes_label: Some("Trust and connect".into()),
            no_label: Some("Do not connect".into()),
        },
    );
    assert_eq!(new_key.title, golden[0]);
    new_key.lines = golden[1..].to_vec();
    send(&mut rig, ask(1, new_key));
    assert_eq!(
        rig.dialog_name().as_deref(),
        Some("New server key (SFTP places)")
    );
    // The safe answer has the focus.
    assert_eq!(
        rig.focus_node(),
        (Role::ListBoxOption, "Do not connect".to_owned())
    );
    // Tab: the message, a list to read, its first line focused.
    rig.press("tab");
    let (list, size) = rig.focus_list().unwrap();
    assert_eq!(list, "Message, 11 lines");
    assert_eq!(size, 11);
    assert_eq!(
        rig.focus_node(),
        (
            Role::ListBoxOption,
            "New server key for build.example.".to_owned()
        )
    );
    // Down to the fingerprint: one part a line, each whole within a
    // Braille line, meaning first.
    for _ in 0..4 {
        rig.press("down");
    }
    let mut parts = Vec::new();
    for _ in 0..4 {
        let (_, part) = rig.focus_node();
        assert!(part.starts_with("Part "), "{part}");
        assert!(part.chars().count() <= BRAILLE_CELLS, "{part}");
        parts.push(part);
        rig.press("down");
    }
    assert_eq!(
        parts,
        [
            "Part 1 of 4: 2134 141b 8436 8a94",
            "Part 2 of 4: a6d8 d5c1 2155 8759",
            "Part 3 of 4: 8d34 a500 27c6 fb7d",
            "Part 4 of 4: c4f9 8430 2e65 fb1d",
        ]
    );
    // Shift+Tab: back on the choices, the safe answer still focused;
    // Enter answers it and the dialog closes onto the panel's row.
    rig.press("shift+tab");
    assert_eq!(rig.focus_node().1, "Do not connect");
    rig.press("enter");
    assert!(rig.dialog_name().is_none());
    assert!(rig.focus_node().1.starts_with("notes.txt"));
}

#[test]
fn the_password_prompt_is_a_secret_whose_text_is_in_no_node() {
    let (_dir, mut rig) = window();
    let golden = rows("password.rows");
    let mut password = question(
        "Password for alice at build.example",
        PluginQuestionKind::Secret,
    );
    assert_eq!(password.title, golden[0]);
    password.lines = golden[1..].to_vec();
    send(&mut rig, ask(2, password));
    let name = rig.dialog_name().unwrap();
    assert!(
        within_cells(&name, "Password for alice at build.example"),
        "{name}"
    );
    assert!(name.contains("(SFTP places)"), "{name}");
    rig.type_text("canary-secret");
    let tree = format!("{:?}", rig.walk());
    assert!(!tree.contains("canary"), "{tree}");
    let said = format!("{:?}", rig.said());
    assert!(!said.contains("canary"), "{said}");
    assert!(
        rig.dialog_buttons()
            .iter()
            .any(|(role, n)| *role == Role::CheckBox && n == "Show the password"),
        "{:?}",
        rig.dialog_buttons()
    );
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
}

#[test]
fn sites_are_in_go_to_a_place_kind_and_state_first_the_plugin_last() {
    let (_dir, mut rig) = window();
    // Go to a place asks the plugins for their places as it opens; they
    // arrive after it (here no plugin runs, so the fake session's answer
    // is sent as the runtime would send it).
    rig.run("go.places");
    send(
        &mut rig,
        AppEvent::Sites(vec![SiteRow {
            name: "build".into(),
            place: Place::parse("sftp://ada@build.example/").unwrap(),
            state: "saved".into(),
            detail: Some("ada@build.example".into()),
            plugin: "SFTP places".into(),
            plugin_id: "sftp".into(),
        }]),
    );
    let names: Vec<String> = rig
        .dialog_walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect();
    let site = names
        .iter()
        .find(|n| n.starts_with("Site, build"))
        .unwrap_or_else(|| panic!("no site in {names:?}"));
    assert!(within_cells(site, "Site, build, saved"), "{site}");
    assert!(site.ends_with("by SFTP places"), "{site}");
    rig.press("escape");
}

#[test]
fn the_sites_list_is_a_plugin_list_in_its_panel() {
    let (_dir, mut rig) = window();
    let site = |name: &str, state: &str| {
        let mut entry = Entry::new(name, EntryKind::Item);
        entry.extra.insert("id".into(), name.into());
        entry.extra.insert("state".into(), state.into());
        entry
    };
    let lister = PluginLister {
        plugin_id: "sftp".into(),
        plugin: "SFTP places".into(),
        spec: ListerSpec {
            id: "sites".into(),
            name: "Sites".into(),
            columns: vec![ListerColumn {
                id: "state".into(),
                header: "State".into(),
                kind: "text".into(),
                sorts: true,
            }],
            activate: Some("connect".into()),
            actions: Vec::new(),
        },
        title: "Sites (SFTP places)".into(),
        rows: vec![site("build", "saved"), site("backup", "connected")],
        panel: PluginPanel::Active,
        total: Some(2),
        truncated: false,
    };
    send(
        &mut rig,
        AppEvent::Plugin(Box::new(PluginEvent::Call {
            call: CallId::new(3),
            plugin: "sftp".into(),
            request: Call::OpenLister(Box::new(lister)),
        })),
    );
    assert!(rig.dialog_name().is_none(), "in the panel, not a dialog");
    let (list, size) = rig.focus_list().unwrap();
    assert!(list.starts_with("Left panel, Sites"), "{list}");
    assert_eq!(size, 2);
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    assert!(row.starts_with("build, state saved"), "{row}");
    rig.press("down");
    assert!(rig.focus_node().1.starts_with("backup, state connected"));
    // Backspace leaves the list: the folder, its row focused again.
    rig.press("backspace");
    assert!(rig.focus_node().1.starts_with("notes.txt"));
}

/// A folder source with one small pack, as the components tests use;
/// nothing is fetched from the network.
fn source(root: &Path) -> String {
    let folder = root.join("source");
    std::fs::create_dir_all(folder.join("test-pack-0.1.0")).unwrap();
    std::fs::write(folder.join("test-pack-0.1.0").join("pack.txt"), b"abc").unwrap();
    std::fs::write(
        folder.join("components.toml"),
        "format = 1\n\n[[component]]\nid = \"test-pack\"\ntitle = \"Test pack\"\nkind = \"data\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\naction = \"place\"\n\n[[component.file]]\nname = \"pack.txt\"\nsize = 3\nsha256 = \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\"\ntarget = \"any\"\n",
    )
    .unwrap();
    folder.to_string_lossy().into_owned()
}

/// The window at a first start, with the components list read from
/// `root`'s source folder and the state kept under `root`.
fn first_start(root: &Path, shown: bool) -> Rig {
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let source = source(root);
    let (app, _) = app_with(&work, &work, |options| {
        options.startup.first_run = true;
        options.settings.components_source = source;
        options.settings.components_first_run_shown = shown;
    });
    let paths = diroxide_app::config::Paths::new(root.join("config"), root.join("state"));
    let runtime = diroxide_app::Runtime::with_local()
        .with_components_offline()
        .with_paths(paths);
    let mut rig = Rig::new(app, runtime);
    if !shown {
        // The list is read on a worker and opens when it arrives.
        rig.settle_until(|rig| rig.dialog_name().is_some());
    }
    rig
}

#[test]
fn the_first_run_components_list_opens_once() {
    let dir = tempfile::Builder::new()
        .prefix("gui-it-")
        .tempdir()
        .unwrap();
    let mut rig = first_start(dir.path(), false);
    let name = rig.dialog_name().unwrap_or_default();
    assert!(name.starts_with("Optional components to get now"), "{name}");
    let (role, row) = rig.focus_node();
    assert_eq!(role, Role::ListBoxOption);
    // Nothing chosen: no row is marked.
    assert!(
        !rig.walk().iter().any(|(_, n)| n.contains(", selected")),
        "{row}"
    );
    rig.press("escape");
    assert!(rig.dialog_name().is_none());
    // Shown once: the app keeps that it was, and a start after it
    // (the setting as it was saved) opens no list.
    assert!(rig.f.app().settings().components_first_run_shown);
    let second = first_start(dir.path(), true);
    assert!(second.dialog_name().is_none(), "{:?}", second.dialog_name());
}

#[test]
fn a_changed_host_key_is_refused_with_the_line_to_forget_it_shown() {
    let (_dir, mut rig) = window();
    let before = rig.f.app().panel(Side::Left).place().clone();
    let place = Place::parse("sftp://build.example/").unwrap();
    let _ = rig.f.app_mut().open_place(Side::Left, place.clone());
    // The listing is answered here as the SFTP plugin's provider answers
    // a key ssh refused: never by the network.
    let _ = rig.f.app_mut().take_requests();
    let request = rig.f.app().panel(Side::Left).loading_request().unwrap();
    let from = rig.spoken().len();
    send(
        &mut rig,
        AppEvent::Listed {
            request,
            place,
            result: Err(diroxide_vfs::VfsError::Unsafe {
                why: "Host key changed. Not connected. To forget the old key, run: ssh-keygen -R build.example".to_owned(),
            }),
        },
    );
    let spoken = rig.spoken()[from..].join(" ");
    eprintln!("heard: {spoken}");
    assert!(spoken.contains("Host key changed"), "{spoken}");
    assert!(spoken.contains("ssh-keygen -R build.example"), "{spoken}");
    // Refused: no question to trust it, and the panel stays where it was.
    assert!(rig.dialog_name().is_none(), "{:?}", rig.dialog_name());
    assert_eq!(rig.f.app().panel(Side::Left).place(), &before);
    assert!(rig.focus_node().1.starts_with("notes.txt"));
}
