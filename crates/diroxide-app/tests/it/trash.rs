//! The trash browser (lane A3, R25 commit 89; answer 98), over a
//! test-only bin in the test's own folder, so the owner's trash is never
//! listed or touched: the rows newest first, each its name first; Restore
//! after a question that starts on No, a taken name left out and said;
//! Delete for good with the listing first and No first; Empty the trash;
//! a trash that cannot be listed says so whole; an open browser follows a
//! job.

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diroxide_app::commands::ids;
use diroxide_app::{Dialog, Runtime};
use diroxide_core::{Timestamp, UtcOffset};
use diroxide_vfs::{OpContext, PurgeCheck, TrashBin, TrashId, TrashedItem, VfsError};

use crate::support::{Session, fixture, place, settings, temp};

/// A trash in a folder of the test's own: each item a file there, put
/// back with a rename, deleted for good by removing the test's file.
#[derive(Debug)]
struct TestBin {
    folder: PathBuf,
    items: Mutex<Vec<(TrashedItem, PathBuf)>>,
    listable: bool,
}

impl TestBin {
    fn new(folder: &Path) -> Arc<Self> {
        fs::create_dir_all(folder).unwrap();
        Arc::new(Self {
            folder: folder.to_owned(),
            items: Mutex::new(Vec::new()),
            listable: true,
        })
    }

    /// A bin of a system whose trash cannot be listed (macOS).
    fn unlisted(folder: &Path) -> Arc<Self> {
        Arc::new(Self {
            folder: folder.to_owned(),
            items: Mutex::new(Vec::new()),
            listable: false,
        })
    }

    /// Trashes a file of the test's: it moves into the bin, deleted on
    /// the day given (October 2026).
    fn trash(&self, file: &Path, day: u8) {
        let id = format!("item-{}-{day}", self.items.lock().unwrap().len());
        let stored = self.folder.join(&id);
        fs::rename(file, &stored).unwrap();
        let deleted = Timestamp::from_civil(2026, 10, day, 9, 0, 0, UtcOffset::UTC).unwrap();
        let item = TrashedItem::new(TrashId::new(id), place(file), Some(deleted));
        self.items.lock().unwrap().push((item, stored));
    }

    fn count(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

impl TrashBin for TestBin {
    fn list(
        &self,
        _ctx: &OpContext,
        batch: &mut dyn FnMut(Vec<TrashedItem>) -> ControlFlow<()>,
    ) -> Result<(), VfsError> {
        if !self.listable {
            return Err(VfsError::unsupported("file", "list the trash"));
        }
        let mut items: Vec<TrashedItem> = self
            .items
            .lock()
            .unwrap()
            .iter()
            .map(|(item, _)| item.clone())
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.deleted));
        let _ = batch(items);
        Ok(())
    }

    fn restore(&self, item: &TrashedItem, _ctx: &OpContext) -> Result<(), VfsError> {
        let mut items = self.items.lock().unwrap();
        let Some(at) = items.iter().position(|(held, _)| held.id == item.id) else {
            return Err(VfsError::NotFound);
        };
        let to = PathBuf::from(item.original.local_path().unwrap());
        if to.exists() {
            return Err(VfsError::AlreadyExists);
        }
        fs::rename(&items[at].1, &to).map_err(|_| VfsError::NotFound)?;
        items.remove(at);
        Ok(())
    }

    fn purge(&self, item: &TrashedItem, _ctx: &OpContext) -> Result<PurgeCheck, VfsError> {
        let mut items = self.items.lock().unwrap();
        let Some(at) = items.iter().position(|(held, _)| held.id == item.id) else {
            return Err(VfsError::NotFound);
        };
        let (_, stored) = items.remove(at);
        fs::remove_file(stored).map_err(|_| VfsError::NotFound)?;
        Ok(PurgeCheck::Gone)
    }
}

/// A session over the work folder with the test's bin, and the bin.
fn session(
    bin: impl FnOnce(&Path) -> Arc<TestBin>,
) -> (tempfile::TempDir, PathBuf, Arc<TestBin>, Session) {
    let dir = temp();
    let work = fixture(dir.path());
    let bin = bin(&dir.path().join("bin"));
    let mut s = Session::start(
        place(&work),
        place(&work),
        diroxide_vfs::Registry::with_local(),
        None,
        settings(),
    );
    let runtime = std::mem::replace(&mut s.runtime, Runtime::with_local());
    s.runtime = runtime.with_trash_bin(Arc::clone(&bin) as Arc<dyn TrashBin>);
    (dir, work, bin, s)
}

fn rows(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(Dialog::Choose { list, .. }) => list.rows().iter().map(|r| r.spoken()).collect(),
        _ => Vec::new(),
    }
}

fn question(s: &Session) -> (String, Vec<String>) {
    match s.dialog() {
        Some(Dialog::Confirm {
            question, listing, ..
        }) => (question.text.clone(), listing.clone()),
        other => panic!("{other:?}"),
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

#[test]
fn the_rows_say_the_name_first_newest_first_and_restore_asks_on_no() {
    let (_dir, work, bin, mut s) = session(TestBin::new);
    fs::write(work.join("old.txt"), b"old").unwrap();
    bin.trash(&work.join("old.txt"), 13);
    bin.trash(&work.join("notes.txt"), 16);
    let said = s.command(ids::TRASH_BROWSER);
    assert_eq!(said[0], "Reading the trash");
    assert_eq!(s.dialog_title().as_deref(), Some("Trash, 2 items"));
    let folder = work.to_string_lossy().into_owned();
    let shown = rows(&s);
    assert!(shown[0].starts_with("notes.txt, deleted "), "{shown:?}");
    assert!(shown[0].ends_with(&folder), "{shown:?}");
    assert!(shown[1].starts_with("old.txt, deleted "), "{shown:?}");
    for row in &shown {
        let name = row.split(',').next().unwrap();
        assert!(diroxide_core::format::within_cells(row, name));
    }
    // Enter offers the actions for the focused row.
    let said = s.key("enter");
    assert_eq!(said[0], "notes.txt, Restore 1 item, 1 of 3");
    assert_eq!(
        rows(&s),
        [
            "Restore 1 item",
            "Delete 1 item for good",
            "Empty the trash"
        ]
    );
    let said = s.key("enter");
    let (text, _) = question(&s);
    assert_eq!(
        text, "Restore 1 item? No is the default. y or n",
        "{said:?}"
    );
    assert_eq!(s.key("n"), ["Nothing changed."]);
    assert_eq!(bin.count(), 2);
    // Again, and yes: the file is back, and the open list follows.
    focus_row(&mut s, "notes.txt");
    s.key("enter");
    s.key("enter");
    let said = s.key("y");
    assert!(said.contains(&"Restored 1 item.".to_owned()), "{said:?}");
    assert!(work.join("notes.txt").is_file());
    assert_eq!(bin.count(), 1);
    assert_eq!(s.dialog_title().as_deref(), Some("Trash, 1 item"));
    assert!(rows(&s)[0].starts_with("old.txt"));
}

#[test]
fn a_taken_name_is_left_out_and_said() {
    let (_dir, work, bin, mut s) = session(TestBin::new);
    bin.trash(&work.join("notes.txt"), 16);
    fs::write(work.join("notes.txt"), b"a new one").unwrap();
    s.command(ids::TRASH_BROWSER);
    s.key("enter");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l == "Nothing to restore: notes.txt, its name is taken."),
        "{said:?}"
    );
    assert_eq!(fs::read(work.join("notes.txt")).unwrap(), b"a new one");
    assert_eq!(bin.count(), 1);
}

#[test]
fn delete_for_good_lists_the_marked_first_and_starts_on_no() {
    let (_dir, work, bin, mut s) = session(TestBin::new);
    bin.trash(&work.join("notes.txt"), 16);
    bin.trash(&work.join("budget.xlsx"), 15);
    s.command(ids::TRASH_BROWSER);
    assert_eq!(s.key("space"), ["Marked"]);
    assert_eq!(s.key("space"), ["Marked"]);
    s.key("enter");
    assert_eq!(
        rows(&s),
        [
            "Restore 2 items",
            "Delete 2 items for good",
            "Empty the trash"
        ]
    );
    s.key("down");
    s.key("enter");
    let (text, listing) = question(&s);
    assert_eq!(text, "Delete 2 items for good? No is the default. y or n");
    assert!(
        listing.iter().any(|l| l.contains("notes.txt"))
            && listing.iter().any(|l| l.contains("budget.xlsx")),
        "every item listed first: {listing:#?}"
    );
    assert_eq!(s.key("n"), ["Nothing changed."]);
    assert_eq!(bin.count(), 2);
    s.key("enter");
    s.key("down");
    s.key("enter");
    let said = s.key("y");
    assert!(
        said.contains(&"Deleted 2 items from the trash for good.".to_owned()),
        "{said:?}"
    );
    assert_eq!(bin.count(), 0);
}

#[test]
fn empty_the_trash_asks_on_no_and_an_empty_trash_says_so() {
    let (_dir, work, bin, mut s) = session(TestBin::new);
    bin.trash(&work.join("notes.txt"), 16);
    s.command(ids::TRASH_EMPTY);
    let (text, _) = question(&s);
    assert_eq!(
        text,
        "Empty the trash: delete 1 item for good? No is the default. y or n"
    );
    let said = s.key("y");
    assert!(
        said.contains(&"Deleted 1 item from the trash for good.".to_owned()),
        "{said:?}"
    );
    assert_eq!(bin.count(), 0);
    assert_eq!(s.command(ids::TRASH_EMPTY), ["The trash is empty."]);
    let said = s.command(ids::TRASH_BROWSER);
    assert_eq!(said, ["Reading the trash", "The trash is empty."]);
    assert!(s.dialog().is_none());
}

#[test]
fn a_trash_that_cannot_be_listed_says_so_whole() {
    let (_dir, _work, _bin, mut s) = session(TestBin::unlisted);
    let said = s.command(ids::TRASH_BROWSER);
    assert_eq!(
        said,
        [
            "Reading the trash",
            "The trash cannot be listed on this system."
        ]
    );
    assert!(s.dialog().is_none());
}

#[test]
fn over_1000_items_the_count_is_typed_to_delete_for_good() {
    let (_dir, work, bin, mut s) = session(TestBin::new);
    let many = work.join("many");
    fs::create_dir_all(&many).unwrap();
    for n in 0..1001 {
        let file = many.join(format!("f{n}.txt"));
        fs::write(&file, b"x").unwrap();
        bin.trash(&file, 16);
    }
    s.command(ids::TRASH_EMPTY);
    match s.dialog() {
        Some(Dialog::Edit { field, .. }) => {
            assert!(
                field.label().starts_with("Type 1001 to go on"),
                "{}",
                field.label()
            );
        }
        other => panic!("{other:?}"),
    }
    s.type_text("1000");
    let said = s.key("enter");
    assert!(said[0].starts_with("That is not 1001."), "{said:?}");
    assert_eq!(bin.count(), 1001);
    s.key("escape");
    assert!(s.dialog().is_none());
}
