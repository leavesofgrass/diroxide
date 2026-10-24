//! Free space in the plan and in Say Status (lane A3, R25 commit 90): a
//! remote destination is named by its host ("Not enough room on box:"), a
//! place that cannot say its free space is "not known", never zero, and
//! the copy goes on; Say Status gives the active place's free space as it
//! answered off the input thread, and a slow place never holds it up. The
//! fake plugin's toy provider stands in for an SFTP server; nothing
//! touches the network.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_app::commands::ids;
use diroxide_app::{Dialog, Runtime};
use diroxide_core::{Entry, Place, Side};
use diroxide_plugins::testing::memfs::MemFs;
use diroxide_plugins::testing::{FakeLauncher, Mode};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, Registry, SpaceInfo, VfsError,
    WriteMode,
};

use crate::plugins::{MANIFEST, enable, world};
use crate::support::{Session, Setup, place, settings};

/// A session with the toy provider in the right panel at `memfs://box/`
/// and a work folder holding `big.bin` of `size` bytes in the left.
fn session(toy: MemFs, size: usize) -> (crate::plugins::World, Session, Arc<Mutex<MemFs>>) {
    let manifest = format!(
        "{MANIFEST}\n[[provider]]\nscheme = \"memfs\"\nname = \"Memory files\"\nwritable = true\nmethods = [\"connect\", \"list\", \"stat\", \"read\", \"write_begin\", \"write_chunk\", \"write_end\", \"create_dir\", \"rename\", \"remove\", \"volumes\", \"places\"]\n"
    );
    let w = world(&manifest);
    enable(&w.paths);
    std::fs::write(w.work.join("big.bin"), vec![7u8; size]).unwrap();
    let toy = Arc::new(Mutex::new(toy));
    toy.lock().unwrap().add_folder("/docs");
    let launcher = FakeLauncher::new(Mode::Echo);
    launcher.with_memfs(Arc::clone(&toy));
    let mut s = Session::open(
        place(&w.work),
        Place::parse("memfs://box/docs").unwrap(),
        Registry::with_local(),
        Setup {
            settings: settings(),
            paths: Some(w.paths.clone()),
            plugin_launcher: Some(launcher),
            ..Setup::default()
        },
    );
    // The plugin's provider is there once its catalog came: list again.
    let effects = s
        .app
        .open_place(Side::Right, Place::parse("memfs://box/docs").unwrap());
    s.run(effects);
    s.focus("big.bin");
    (w, s, toy)
}

#[test]
fn a_place_that_cannot_say_its_free_space_is_not_known_and_the_copy_goes_on() {
    let (_w, mut s, toy) = session(MemFs::default().without_free_space(), 1200);
    let said = s.key("f5");
    let note = said
        .iter()
        .position(|l| l == "Free space on box is not known.")
        .unwrap_or_else(|| panic!("{said:?}"));
    // Said before the question, which starts on No as every copy's does.
    let question = said
        .iter()
        .position(|l| l.ends_with("y or n"))
        .unwrap_or_else(|| panic!("{said:?}"));
    assert!(note < question, "{said:?}");
    assert!(
        !said.iter().any(|l| l.starts_with("Not enough room")),
        "{said:?}"
    );
    let said = s.key("y");
    assert!(
        said.iter().any(|l| l.starts_with("Copy finished")),
        "{said:?}"
    );
    assert!(toy.lock().unwrap().file("/docs/big.bin").is_some());
}

#[test]
fn not_enough_room_on_a_remote_place_names_its_host_and_starts_on_no() {
    let size = 1_200_000;
    let (_w, mut s, toy) = session(MemFs::default(), size);
    let said = s.key("f5");
    assert!(!said.iter().any(|l| l.contains("is not known")), "{said:?}");
    let Some(Dialog::Confirm {
        question, listing, ..
    }) = s.dialog()
    else {
        panic!("{:?}", s.dialog());
    };
    assert!(
        question.text.contains("Not enough room. Go on anyway"),
        "{}",
        question.text
    );
    assert!(
        question.text.contains("No is the default"),
        "{}",
        question.text
    );
    let at = listing
        .iter()
        .position(|l| l == "Not enough room on box:")
        .unwrap_or_else(|| panic!("{listing:#?}"));
    assert!(
        listing[at + 1].starts_with("Needs 1.2 MB, "),
        "{listing:#?}"
    );
    assert!(listing[at + 1].ends_with(" free."), "{listing:#?}");
    for line in &listing[at..at + 2] {
        assert!(line.chars().count() <= 40, "{line}");
    }
    let said = s.key("n");
    assert!(said.iter().any(|l| l == "Nothing changed."), "{said:?}");
    assert!(toy.lock().unwrap().file("/docs/big.bin").is_none());
}

#[test]
fn say_status_gives_the_places_free_space_once_it_answered() {
    let (_w, mut s, _toy) = session(MemFs::default(), 10);
    let said = s.command(ids::SAY_STATUS);
    assert!(said[0].contains(" free of "), "{said:?}");
    s.command(ids::SWITCH_PANEL);
    let said = s.command(ids::SAY_STATUS);
    // The toy's million bytes, less what it holds.
    assert!(said[0].contains("free of 1.0 MB"), "{said:?}");
}

/// A remote place that lists at once and takes five seconds to say its
/// free space.
#[derive(Debug)]
struct Slow;

impl Provider for Slow {
    fn scheme(&self) -> &str {
        "slow"
    }
    fn capabilities(&self, _place: &Place) -> Capabilities {
        Capabilities::read_only()
    }
    fn list(&self, _place: &Place, _ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        Ok(Vec::new())
    }
    fn stat(&self, _place: &Place, _ctx: &OpContext) -> Result<Entry, VfsError> {
        Err(VfsError::NotFound)
    }
    fn read(&self, _place: &Place, _ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        Err(VfsError::NotFound)
    }
    fn write(
        &self,
        _place: &Place,
        _mode: WriteMode,
        _ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        Err(VfsError::NotFound)
    }
    fn create_dir(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::NotFound)
    }
    fn rename(&self, _from: &Place, _to: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::NotFound)
    }
    fn remove(&self, _place: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        Err(VfsError::NotFound)
    }
    fn set_attributes(
        &self,
        _place: &Place,
        _change: &AttributeChange,
        _ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        Err(VfsError::NotFound)
    }
    fn space(&self, _place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            ctx.check()?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(SpaceInfo {
            free_to_user: Some(1),
            total: Some(2),
            max_file_size: None,
        })
    }
}

#[test]
fn a_slow_place_never_holds_up_say_status() {
    let here = place(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
    let mut registry = Registry::with_local();
    registry.register("slow", Arc::new(Slow));
    let mut s = Session::start(here.clone(), here, registry, None, settings());
    let runtime = std::mem::replace(&mut s.runtime, Runtime::with_local());
    s.runtime = runtime.with_space_limit(Duration::from_millis(100));
    let started = Instant::now();
    let effects = s
        .app
        .open_place(Side::Left, Place::parse("slow://far/").unwrap());
    s.run(effects);
    let said = s.command(ids::SAY_STATUS);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(said[0].starts_with("far, "), "{said:?}");
    assert!(said[0].ends_with("free space not known"), "{said:?}");
}
