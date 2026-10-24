//! The Volumes list (lane A3, R25 commit 88), over local volumes in the
//! test's own folder (never the owner's drives) whose labels and free
//! space a fake local provider answers, and the fake plugin's storage:
//! rows that say their state first, each row's free space said as it
//! comes and never zero, the actions with the reason of one that needs
//! the Volume actions plugin, Open, and Hide with the ignore list.

use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diroxide_app::commands::ids;
use diroxide_app::{Dialog, Runtime};
use diroxide_core::{Entry, Place, Side};
use diroxide_plugins::testing::memfs::MemFs;
use diroxide_plugins::testing::{FakeLauncher, Mode};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, LocalPlace, LocalProvider, OpContext, PlaceRole,
    Provider, Registry, SpaceInfo, VfsError, VolumeInfo, VolumeKind, WriteMode,
};

use crate::plugins::{MANIFEST, enable, world};
use crate::support::{Session, Setup, place, settings};

/// The local disk, with each volume's label and free space made up: a
/// folder named `stick` is "Backup" with 412 GB free of 931 GB; one named
/// `slow` takes a minute to say its space.
#[derive(Debug)]
struct Volumes {
    local: LocalProvider,
}

impl Provider for Volumes {
    fn scheme(&self) -> &str {
        "file"
    }
    fn capabilities(&self, place: &Place) -> Capabilities {
        self.local.capabilities(place)
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.local.list(place, ctx)
    }
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.local.stat(place, ctx)
    }
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.local.read(place, ctx)
    }
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.local.write(place, mode, ctx)
    }
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.create_dir(place, ctx)
    }
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.rename(from, to, ctx)
    }
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.remove(place, ctx)
    }
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.local.set_attributes(place, change, ctx)
    }
    fn volume(&self, place: &Place, _ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        let mut info = VolumeInfo::unknown(place.clone()).with_kind(VolumeKind::Removable);
        if place.name() == Some("stick") {
            info.label = Some("Backup".into());
        }
        Ok(info)
    }
    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        if place.name() == Some("slow") {
            for _ in 0..600 {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Ok(SpaceInfo {
            free_to_user: Some(412_000_000_000),
            total: Some(931_000_000_000),
            max_file_size: None,
        })
    }
}

/// A session with two local volumes in the test's folder, `stick` and
/// `slow`, and the fake plugin's toy storage connected at `memfs://box/`.
fn session() -> (crate::plugins::World, Session) {
    let manifest = format!(
        "{MANIFEST}\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\nwritable = true\nmethods = [\"connect\", \"list\", \"stat\", \"read\", \"volumes\", \"places\"]\n"
    );
    let w = world(&manifest);
    enable(&w.paths);
    for folder in ["stick", "slow"] {
        std::fs::create_dir_all(w.work.join(folder)).unwrap();
    }
    let toy = Arc::new(Mutex::new(MemFs::default()));
    let launcher = FakeLauncher::new(Mode::Echo);
    launcher.with_memfs(Arc::clone(&toy));
    let mut registry = Registry::new();
    registry.set_local(Arc::new(Volumes {
        local: LocalProvider::new(),
    }));
    let here = place(&w.work);
    let mut s = Session::open(
        here.clone(),
        here,
        registry,
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    );
    let mount = |name: &str| {
        LocalPlace::new(
            place(&w.work.join(name)),
            PlaceRole::Mount,
            VolumeKind::Removable,
        )
    };
    let runtime = std::mem::replace(&mut s.runtime, Runtime::with_local());
    s.runtime = runtime.with_local_volumes(vec![mount("stick"), mount("slow")]);
    (w, s)
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        _ => Vec::new(),
    }
}

fn focus_row(s: &mut Session, starts: &str) {
    for _ in 0..10 {
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

fn starts(rows: &[String], start: &str) -> bool {
    rows.iter().any(|r| r.starts_with(start))
}

#[test]
fn rows_say_their_state_first_and_a_slow_volume_is_not_known() {
    let (w, mut s) = session();
    let said = s.command(ids::VOLUMES);
    assert_eq!(said[0], "Reading the volumes");
    assert_eq!(s.dialog_title().as_deref(), Some("Volumes, 4 volumes"));
    let shown = rows(&s);
    // The label answered: the mount is named by it, its space said.
    assert!(
        starts(&shown, "Mounted: Backup, removable, 412 GB free"),
        "{shown:#?}"
    );
    assert!(
        starts(&shown, "Mounted: slow, removable, free space not known"),
        "never zero, and the list did not wait for it: {shown:#?}"
    );
    assert!(
        starts(&shown, "Connected: Memory box, memfs, "),
        "the plugin's storages: {shown:#?}"
    );
    assert!(
        starts(&shown, "Connected: Spare shelf, memfs, "),
        "{shown:#?}"
    );
    for row in &shown {
        let head = row.split(',').next().unwrap();
        assert!(diroxide_core::format::within_cells(row, head), "{row}");
    }
    let _ = w;
}

#[test]
fn eject_without_the_plugin_says_why_and_open_goes_there() {
    let (w, mut s) = session();
    s.command(ids::VOLUMES);
    focus_row(&mut s, "Mounted: Backup");
    let said = s.key("enter");
    // Windows mounts and unmounts removable drives itself: Eject only.
    let actions = if cfg!(windows) { 3 } else { 4 };
    assert_eq!(said[0], format!("Backup, Open, 1 of {actions}"), "{said:?}");
    let actions = rows(&s);
    assert_eq!(actions[0], "Open");
    let unmount = if cfg!(windows) {
        None
    } else {
        Some("Unmount, needs the Volume actions plugin, unavailable")
    };
    if let Some(unmount) = unmount {
        assert!(actions.contains(&unmount.to_owned()), "{actions:#?}");
    }
    assert!(
        actions.contains(&"Eject, needs the Volume actions plugin, unavailable".to_owned()),
        "{actions:#?}"
    );
    assert_eq!(actions.last().map(String::as_str), Some("Hide"));
    // Enter on Eject says the reason, then asks first for the examples.
    focus_row(&mut s, "Eject");
    let said = s.key("enter");
    assert_eq!(
        said[0], "Eject needs the Volume actions plugin.",
        "{said:?}"
    );
    s.key("escape");
    assert!(s.dialog().is_none());
    // Open goes to the volume in the active panel.
    s.command(ids::VOLUMES);
    focus_row(&mut s, "Mounted: Backup");
    s.key("enter");
    s.key("enter");
    assert!(s.dialog().is_none());
    assert_eq!(
        s.app.panel(Side::Left).place(),
        &place(&w.work.join("stick"))
    );
}

#[test]
fn hide_leaves_a_volume_out_until_the_setting_has_it_no_more() {
    let (w, mut s) = session();
    s.command(ids::VOLUMES);
    focus_row(&mut s, "Mounted: slow");
    s.key("enter");
    focus_row(&mut s, "Hide");
    let said = s.key("enter");
    assert!(
        said.contains(&"Hidden: slow. Settings, Hidden volumes has it.".to_owned()),
        "{said:?}"
    );
    assert!(!starts(&rows(&s), "Mounted: slow"), "{:?}", rows(&s));
    let key = place(&w.work.join("slow")).to_string();
    assert_eq!(s.app.settings().hidden_volumes, std::slice::from_ref(&key));
    let saved = diroxide_config::load(&w.paths).settings;
    assert_eq!(saved.hidden_volumes, [key]);
    // Read again: still left out.
    s.key("escape");
    s.command(ids::VOLUMES);
    assert!(!starts(&rows(&s), "Mounted: slow"), "{:?}", rows(&s));
    assert!(starts(&rows(&s), "Mounted: Backup"));
}

#[test]
fn the_volumes_command_is_in_the_go_menu() {
    let (_w, mut s) = session();
    s.command(ids::MENU_GO);
    assert!(
        rows(&s).iter().any(|r| r.starts_with("Volumes")),
        "{:?}",
        rows(&s)
    );
    assert!(
        rows(&s).iter().any(|r| r.starts_with("Trash")),
        "{:?}",
        rows(&s)
    );
    let _ = Path::new(".");
}

/// The Volume actions plugin as a test plugin: its commands answer at
/// once, Eject saying the row it was given.
const ACTIONS: &str = r#"
[plugin]
id = "volume-actions"
name = "Volume actions"
version = "0.1.0"
protocol = 1

[run]
command = ["test-plugin"]
startup_timeout = "2s"

[timeouts]
command = "1s"

[permissions]
fs = "read"

[[command]]
id = "devices"
name = "List devices"
description = "Lists the devices."
selection = "none"

[[command]]
id = "eject"
name = "Eject"
description = "Ejects the volume on this row."
selection = "one"

[[command]]
id = "eject-windows"
name = "Eject"
description = "Ejects the drive on this row."
selection = "one"

[[command]]
id = "unmount"
name = "Unmount"
description = "Unmounts the volume on this row."
selection = "one"

[[command]]
id = "mount"
name = "Mount"
description = "Mounts the device on this row."
selection = "one"
"#;

#[test]
fn eject_runs_the_plugins_command_with_the_row_and_the_list_follows() {
    use crate::plugins::{Act, TestLauncher};
    use diroxide_plugins::manifest::Manifest;
    use diroxide_plugins::script::consent::{ConsentStore, Kind, plugin_fingerprint};

    let w = world(MANIFEST);
    let folder = w.paths.plugins_dir().join("volume-actions");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(Manifest::FILE), ACTIONS).unwrap();
    let manifest = Manifest::from_toml(ACTIONS).unwrap();
    let fingerprint =
        plugin_fingerprint(&manifest, &folder, diroxide_components::sha256_hex).unwrap();
    ConsentStore::open(&w.paths.state_dir.join("plugins"))
        .enable("volume-actions", Kind::Plugin, fingerprint)
        .unwrap();
    std::fs::create_dir_all(w.work.join("stick")).unwrap();
    let mut registry = Registry::new();
    registry.set_local(Arc::new(Volumes {
        local: LocalProvider::new(),
    }));
    let here = place(&w.work);
    let mut s = Session::open(
        here.clone(),
        here,
        registry,
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(TestLauncher::with(&[
                ("eject", Act::Rows),
                ("eject-windows", Act::Rows),
            ])),
            ..Setup::default()
        },
    );
    let stick = LocalPlace::new(
        place(&w.work.join("stick")),
        PlaceRole::Mount,
        VolumeKind::Removable,
    );
    let runtime = std::mem::replace(&mut s.runtime, Runtime::with_local());
    s.runtime = runtime.with_local_volumes(vec![stick]);
    // List devices ran first, quietly: nothing of it is said.
    let said = s.command(ids::VOLUMES);
    assert!(!said.iter().any(|l| l.contains("List devices")), "{said:?}");
    assert_eq!(rows(&s)[0].split(", ").next(), Some("Mounted: Backup"));
    s.key("enter");
    focus_row(&mut s, "Eject");
    assert_eq!(
        match s.dialog() {
            Some(Dialog::Choose { list, .. }) => list.focused().map(|r| r.spoken()),
            _ => None,
        }
        .as_deref(),
        Some("Eject")
    );
    let said = s.key("enter");
    assert!(
        said.iter().any(|l| l == "Rows: stick. By Volume actions."),
        "the row's identity is the selection: {said:?}"
    );
    // Back in the list, which read the volumes again.
    assert_eq!(s.dialog_title().as_deref(), Some("Volumes, 1 volume"));
}

#[test]
fn the_place_list_has_volumes_and_the_trash_each_its_kind_first() {
    let (w, mut s) = session();
    s.command(ids::PLACES);
    let candidates: Vec<String> = match s.dialog() {
        Some(Dialog::Edit {
            candidates: Some(list),
            ..
        }) => list.rows().iter().map(|r| r.spoken()).collect(),
        other => panic!("{other:?}"),
    };
    // The volumes as the list knows them, no drive asked for its space.
    assert!(
        candidates.contains(&"Volume, stick, mounted, removable".to_owned()),
        "{candidates:#?}"
    );
    assert!(
        candidates
            .iter()
            .any(|c| c.starts_with("Volume, Memory box, connected, memfs, ")),
        "{candidates:#?}"
    );
    assert!(
        candidates.contains(&"Trash, what is in the trash".to_owned()),
        "{candidates:#?}"
    );
    // A volume's row opens its place.
    s.type_text("stick");
    s.key("enter");
    assert!(s.dialog().is_none());
    assert_eq!(
        s.app.panel(Side::Left).place(),
        &place(&w.work.join("stick"))
    );
}
