//! Sites in the go-to-place list and the connect flow (lane A2, R25 commit
//! 78), with the fake plugin's toy provider standing in for the SFTP
//! plugin: its places are the list's Sites group, each its kind and state
//! first and the plugin last; a place typed in the path edit with a
//! scheme, a user and a port opens through the plugin; a refused host key
//! is said with the plugin's next step. Nothing touches the network.

use std::sync::{Arc, Mutex};

use diroxide_app::commands::ids;
use diroxide_app::{Answer, AppEvent, Dialog};
use diroxide_core::{Place, Side};
use diroxide_plugins::testing::memfs::MemFs;
use diroxide_plugins::testing::{FakeLauncher, Mode};

use crate::plugins::{MANIFEST, enable, world};
use crate::support::{Session, Setup, place, settings};

fn session() -> (crate::plugins::World, Session) {
    let manifest = format!(
        "{MANIFEST}\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\nwritable = true\nmethods = [\"connect\", \"list\", \"stat\", \"read\", \"write_begin\", \"write_chunk\", \"write_end\", \"create_dir\", \"rename\", \"remove\", \"places\"]\n"
    );
    let w = world(&manifest);
    enable(&w.paths);
    let toy = Arc::new(Mutex::new(MemFs::default()));
    toy.lock().unwrap().add_file("/docs/a.txt", b"alpha");
    let launcher = FakeLauncher::new(Mode::Echo);
    launcher.with_memfs(Arc::clone(&toy));
    let here = place(&w.work);
    let s = Session::open(
        here.clone(),
        here,
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    );
    (w, s)
}

#[test]
fn the_go_to_place_list_has_the_plugins_sites_kind_and_state_first() {
    let (_w, mut s) = session();
    s.command(ids::PLACES);
    let candidates: Vec<String> = match s.dialog() {
        Some(Dialog::Edit {
            candidates: Some(list),
            ..
        }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("{other:?}"),
    };
    assert!(
        candidates
            .iter()
            .any(|c| c == "Site, box, connected, Memory box, by Line counter"),
        "{candidates:#?}"
    );
    assert!(
        candidates
            .iter()
            .any(|c| c == "Site, spare, saved, by Line counter"),
        "{candidates:#?}"
    );
    // The rows a plugin may not offer are left out.
    assert!(!candidates.iter().any(|c| c.contains("Site, other")));
    assert!(!candidates.iter().any(|c| c.contains("Site, drive")));
}

#[test]
fn a_place_with_a_scheme_a_user_and_a_port_opens_through_the_plugin() {
    let (_w, mut s) = session();
    s.command(ids::GO_PATH);
    let dialog = s.app.dialog().unwrap().0;
    let effects = s.app.handle(AppEvent::Answer {
        dialog,
        answer: Answer::Text("memfs://ada@box:2222/docs".into()),
    });
    let said = s.run(effects);
    assert_eq!(
        s.app.panel(Side::Left).place(),
        &Place::parse("memfs://ada@box:2222/docs").unwrap(),
        "{said:?}"
    );
    assert_eq!(s.names(Side::Left), ["a.txt"]);
    assert!(said_start(&said, "docs, 1 item"), "{said:?}");
}

#[test]
fn a_refused_host_key_says_the_plugins_next_step() {
    let (_w, mut s) = session();
    let effects = s
        .app
        .open_place(Side::Left, Place::parse("memfs://changed/").unwrap());
    let said = s.run(effects);
    let line = said
        .iter()
        .find(|l| l.contains("Host key changed"))
        .unwrap_or_else(|| panic!("{said:?}"));
    assert!(line.contains("ssh-keygen -R changed"), "{line}");
    assert_ne!(
        s.app.panel(Side::Left).place(),
        &Place::parse("memfs://changed/").unwrap()
    );
}

fn said_start(said: &[String], start: &str) -> bool {
    said.iter().any(|l| l.starts_with(start))
}
