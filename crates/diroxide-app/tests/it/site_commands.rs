//! The Sites list's commands and dialogs in the app (lane A3, wiring S-C's
//! commit 73): a lister action that takes no row runs in an empty list
//! (New site, Alt+N); a lister opened again by its plugin replaces the open
//! one; after one of the lister's own commands, or a lost connection, the
//! rows are brought up to date quietly; Connect's `panel/set` closes the
//! lister so the owner lands in the start folder; Enter on a Sites row of
//! the go-to-place list runs the site's plugin's own Connect. The test
//! plugin and the fake plugin stand in for the SFTP plugin; nothing touches
//! the network.

use std::sync::{Arc, Mutex};

use diroxide_app::commands::ids;
use diroxide_app::plugins::PluginEvent;
use diroxide_app::{AppEvent, Dialog};
use diroxide_core::{Command, CommandId, Place, Side};
use diroxide_plugins::testing::memfs::MemFs;
use diroxide_plugins::testing::{FakeLauncher, Mode};
use serde_json::json;

use crate::plugins::{Act, MANIFEST, TestLauncher, enable, session, world};
use crate::support::{Session, Setup, place, settings};

/// The test plugin with a Sites list as the SFTP plugin declares it:
/// Enter connects, Alt+N makes a new site with no row needed, Alt+D
/// deletes the row's site.
fn manifest() -> String {
    format!(
        "{MANIFEST}
[[command]]
id = \"sites\"
name = \"Test sites\"
description = \"Lists the sites.\"
selection = \"none\"

[[command]]
id = \"connect\"
name = \"Connect to the site\"
description = \"Connects and opens the start folder.\"

[[command]]
id = \"new_site\"
name = \"New test site\"
description = \"Saves a new site.\"
selection = \"none\"

[[command]]
id = \"delete_site\"
name = \"Delete the site\"
description = \"Deletes the site after asking.\"
selection = \"one\"

[[lister]]
id = \"sites\"
name = \"Test sites\"
activate = \"connect\"

[[lister.columns]]
id = \"state\"
header = \"State\"
kind = \"text\"
sorts = true

[[lister.action]]
key = \"alt+n\"
command = \"new_site\"
selection = \"none\"

[[lister.action]]
key = \"alt+d\"
command = \"delete_site\"
selection = \"one\"
"
    )
}

fn id(text: &str) -> CommandId {
    CommandId::new(text).unwrap()
}

fn lister_rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        _ => Vec::new(),
    }
}

/// How many times a line was said that starts so.
fn said_count(said: &[String], start: &str) -> usize {
    said.iter().filter(|l| l.starts_with(start)).count()
}

#[test]
fn new_site_runs_in_an_empty_list_and_the_rows_follow_quietly() {
    let w = world(&manifest());
    enable(&w.paths);
    let open = json!({"lister": "sites", "title": "Sites, 0 saved", "rows": []});
    let mut s = session(
        &w,
        TestLauncher::with(&[
            ("sites", Act::Call("lister/open", open)),
            ("new_site", Act::Done),
        ]),
    );
    let said = s.run_command(Command::new(id("line-counter.sites")));
    assert!(
        said.iter()
            .any(|l| l.starts_with("Sites, 0 saved, no rows")),
        "{said:?}"
    );
    assert_eq!(s.app.dialogs().len(), 1);
    // Alt+N needs no row; the old rule refused it in an empty list.
    let said = s.key("alt+n");
    assert!(!said.iter().any(|l| l == "No row is chosen"), "{said:?}");
    assert_eq!(
        said_count(&said, "New test site done"),
        1,
        "the command ran: {said:?}"
    );
    // The list was opened again by its own command, quietly, in place:
    // one dialog, no start or end line of that run.
    assert_eq!(said_count(&said, "Host said"), 1, "{said:?}");
    assert_eq!(said_count(&said, "Test sites"), 0, "{said:?}");
    assert_eq!(s.app.dialogs().len(), 1, "replaced, not opened twice");
    // The quiet run leaves no row in the Jobs list.
    s.key("escape");
    s.command(ids::JOBS_LIST);
    let jobs = lister_rows(&s);
    assert!(!jobs.iter().any(|r| r.contains("Test sites")), "{jobs:#?}");
}

#[test]
fn a_row_command_still_needs_its_row() {
    let w = world(&manifest());
    enable(&w.paths);
    let open = json!({"lister": "sites", "title": "Sites, 0 saved", "rows": []});
    let mut s = session(
        &w,
        TestLauncher::with(&[("sites", Act::Call("lister/open", open))]),
    );
    s.run_command(Command::new(id("line-counter.sites")));
    assert_eq!(s.key("alt+d"), ["No row is chosen"]);
}

#[test]
fn connect_from_the_list_closes_it_and_lands_in_the_start_folder() {
    let w = world(&manifest());
    enable(&w.paths);
    let start = w.work.join("docs");
    std::fs::create_dir_all(&start).unwrap();
    let open = json!({"lister": "sites", "title": "Sites, 1 saved", "rows": [
        {"name": "build, saved, ada@build.example", "kind": "item",
            "extra": {"id": "site:k3f9", "state": "saved"}}
    ]});
    let set = json!({"panel": "active", "place": start.to_str().unwrap()});
    let mut s = session(
        &w,
        TestLauncher::with(&[
            ("sites", Act::Call("lister/open", open)),
            ("connect", Act::Call("panel/set", set)),
        ]),
    );
    s.run_command(Command::new(id("line-counter.sites")));
    assert_eq!(
        lister_rows(&s),
        ["build, saved, ada@build.example, state saved"]
    );
    let said = s.key("enter");
    assert!(s.dialog().is_none(), "the list closed: {said:?}");
    assert_eq!(s.app.panel(Side::Left).place(), &place(&start), "{said:?}");
}

#[test]
fn a_lost_connection_brings_the_open_list_up_to_date() {
    let w = world(&manifest());
    enable(&w.paths);
    let open = json!({"lister": "sites", "title": "Sites, 1 saved", "rows": [
        {"name": "build, lost, ada@build.example", "kind": "item",
            "extra": {"id": "site:k3f9", "state": "lost"}}
    ]});
    let mut s = session(
        &w,
        TestLauncher::with(&[("sites", Act::Call("lister/open", open))]),
    );
    s.run_command(Command::new(id("line-counter.sites")));
    let lost = diroxide_plugins::host::HostEvent::ProviderLost {
        plugin: "line-counter".into(),
        authority: "sftp://ada@build.example".into(),
        message: diroxide_core::Message::warning("Connection to build lost (Line counter)"),
        for_good: false,
    };
    let effects = s
        .app
        .handle(AppEvent::Plugin(Box::new(PluginEvent::Host(lost))));
    let said = s.run(effects);
    assert_eq!(said[0], "Connection to build lost (Line counter)");
    // The list's own command ran again, quietly, and its rows came.
    assert_eq!(said_count(&said, "Host said"), 1, "{said:?}");
    assert_eq!(s.app.dialogs().len(), 1);
    assert_eq!(
        lister_rows(&s),
        ["build, lost, ada@build.example, state lost"]
    );
}

#[test]
fn enter_on_a_sites_row_runs_the_plugins_own_connect() {
    let manifest = format!(
        "{MANIFEST}
[[command]]
id = \"connect\"
name = \"Connect to the site\"
description = \"Connects and opens the start folder.\"

[[provider]]
scheme = \"memfs\"
name = \"Memory files\"
writable = true
methods = [\"connect\", \"list\", \"stat\", \"read\", \"places\"]
"
    );
    let w = world(&manifest);
    enable(&w.paths);
    let toy = Arc::new(Mutex::new(MemFs::default()));
    let launcher = FakeLauncher::new(Mode::Echo);
    launcher.with_memfs(Arc::clone(&toy));
    let here = place(&w.work);
    let mut s = Session::open(
        here.clone(),
        here.clone(),
        diroxide_vfs::Registry::with_local(),
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            // The home folder is the test's own, never the owner's.
            home: Some(here.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    );
    s.command(ids::PLACES);
    s.type_text("memfs://box/");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Connect to the site") && l.ends_with("(Line counter)")),
        "the plugin's connect ran: {said:?}"
    );
    // The fake's connect opens no folder; the place was not opened by the
    // app behind the plugin's back.
    assert_eq!(s.app.panel(Side::Left).place(), &here);
    // A place that is not a site still simply opens.
    s.command(ids::PLACES);
    s.type_text("memfs://elsewhere/");
    s.key("enter");
    assert_eq!(
        s.app.panel(Side::Left).place(),
        &Place::parse("memfs://elsewhere/").unwrap()
    );
}
