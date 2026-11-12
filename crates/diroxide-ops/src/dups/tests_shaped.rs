//! Tests over a fake shaped like other places: a listing without file
//! identity (local folders on Windows), link counts, and a place with no
//! random read.

use std::sync::{Arc, Mutex};

use diroxide_core::{Entry, EntryKind, Place, Timestamp};
use diroxide_vfs::{
    AttributeChange, Capabilities, FileWriter, OpContext, Provider, ReadSeek, Registry, VfsError,
    WriteMode,
};

use super::*;
use crate::engine::JobControl;
use crate::fake::FakeProvider;
use crate::pace::FakePacer;
use crate::testing::p;

/// The fake, with its listing and capabilities reshaped.
#[derive(Clone, Debug, Default)]
struct Shaped {
    fake: FakeProvider,
    /// Places whose entries say two links.
    linked: Vec<Place>,
    /// The listing gives files no inode, as on Windows.
    no_ids: bool,
    /// The place has no random read.
    no_random: bool,
    /// How many times `stat` was called.
    stats: Arc<Mutex<u32>>,
}

impl Provider for Shaped {
    fn scheme(&self) -> &str {
        self.fake.scheme()
    }

    fn capabilities(&self, place: &Place) -> Capabilities {
        let mut capabilities = self.fake.capabilities(place);
        if self.no_random {
            capabilities.random_read = false;
        }
        capabilities
    }

    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        let mut entries = self.fake.list(place, ctx)?;
        for entry in &mut entries {
            if entry.kind != EntryKind::File {
                continue;
            }
            if self.no_ids {
                entry.inode = None;
            }
            if let Ok(child) = place.join(&entry.name)
                && self.linked.contains(&child)
            {
                entry.hard_links = Some(2);
            }
        }
        Ok(entries)
    }

    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        *self.stats.lock().unwrap() += 1;
        let mut entry = self.fake.stat(place, ctx)?;
        if self.linked.contains(place) {
            entry.hard_links = Some(2);
        }
        Ok(entry)
    }

    fn read(
        &self,
        place: &Place,
        ctx: &OpContext,
    ) -> Result<Box<dyn std::io::Read + Send>, VfsError> {
        self.fake.read(place, ctx)
    }

    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        if self.no_random {
            return Err(VfsError::unsupported("shaped", "read with random access"));
        }
        self.fake.open_seekable(place, ctx)
    }

    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.fake.write(place, mode, ctx)
    }

    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.create_dir(place, ctx)
    }

    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.rename(from, to, ctx)
    }

    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.fake.remove(place, ctx)
    }

    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.fake.set_attributes(place, change, ctx)
    }
}

fn run(shaped: &Shaped, roots: &[&str], options: &DupOptions) -> DupResult {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(shaped.clone()));
    let pacer = FakePacer::new();
    let env = DupEnv {
        registry: &registry,
        sink: &QuietSink,
        hasher: &Sha256Hasher,
        pacer: &pacer,
    };
    let roots: Vec<_> = roots.iter().map(|r| p(r)).collect();
    find_duplicates(&roots, options, &JobControl::new(), &env).unwrap()
}

fn names(group: &DupGroup) -> Vec<String> {
    group
        .files
        .iter()
        .map(|f| f.place.name().unwrap().to_owned())
        .collect()
}

fn linked_pair() -> Shaped {
    let fake = FakeProvider::new();
    fake.add_file(&p("/h/a.txt"), b"linked bytes");
    fake.add_hard_link(&p("/h/b.txt"), &p("/h/a.txt"));
    Shaped {
        fake,
        linked: vec![p("/h/a.txt"), p("/h/b.txt")],
        no_ids: true,
        ..Shaped::default()
    }
}

#[test]
fn hard_links_are_found_by_stat_when_the_listing_has_no_identity() {
    let shaped = linked_pair();
    let result = run(&shaped, &["/h"], &DupOptions::default());
    assert!(result.groups.is_empty(), "a file and its link are one file");
    assert!(
        *shaped.stats.lock().unwrap() >= 2,
        "the candidates were statted"
    );
    assert_eq!(result.hard_links_left_out, 1);
    let row = result.left_out[0].row();
    assert!(row.starts_with("b.txt, in "), "{row}");
    assert!(row.ends_with(": same file as a.txt, 2 links"), "{row}");
    assert_eq!(result.notes(), ["1 hard link left out: same file"]);
}

#[test]
fn hard_links_kept_beside_a_real_copy_free_nothing() {
    let shaped = linked_pair();
    shaped.fake.add_file(&p("/h/c.txt"), b"linked bytes");
    let options = DupOptions {
        include_hard_links: true,
        ..DupOptions::default()
    };
    let result = run(&shaped, &["/h"], &options);
    assert_eq!(result.groups.len(), 1);
    let group = &result.groups[0];
    assert_eq!(names(group), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(group.files[1].same_file_as, Some(0));
    assert_eq!(group.distinct(), 2);
    assert_eq!(group.waste(), 12);
    let row = group.file_row(1, 1).unwrap();
    assert!(
        row.starts_with("b.txt, copy 2 of 3, group 1, same file as copy 1"),
        "{row}"
    );
    // Keeping a.txt leaves its link b.txt unmarked: trashing it frees
    // nothing.
    let selection = select(&result, &KeepRule::FirstFound);
    assert_eq!(selection.marked, vec![p("/h/c.txt")]);
    assert_eq!(selection.bytes, 12);
}

#[test]
fn a_folder_reached_through_an_alias_counts_its_files_once() {
    // /alias is /real under a second name (a junction). Searching both
    // must not make /real/a.txt a copy of itself, or a helper could mark
    // its only copy.
    let fake = FakeProvider::new();
    fake.add_file(&p("/real/a.txt"), b"only copy");
    fake.add_folder_alias(&p("/alias"), &p("/real"));
    fake.add_file(&p("/other/z.txt"), b"different");
    let shaped = Shaped {
        fake,
        no_ids: true,
        ..Shaped::default()
    };
    let result = run(&shaped, &["/real", "/alias"], &DupOptions::default());
    assert!(result.groups.is_empty(), "{:?}", result.groups);
    assert_eq!(result.reached_twice, 1);
    assert_eq!(result.notes(), ["1 file reached twice, counted once"]);
    let selection = select(&result, &KeepRule::default());
    assert!(selection.marked.is_empty());
}

#[test]
fn a_real_copy_beside_an_alias_is_one_pair_not_three() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/real/a.txt"), b"same bytes");
    fake.add_folder_alias(&p("/alias"), &p("/real"));
    fake.add_file(&p("/other/a.txt"), b"same bytes");
    let shaped = Shaped {
        fake,
        ..Shaped::default()
    };
    let result = run(
        &shaped,
        &["/real", "/alias", "/other"],
        &DupOptions::default(),
    );
    assert_eq!(result.groups.len(), 1);
    let group = &result.groups[0];
    assert_eq!(group.files.len(), 2);
    assert_eq!(group.files[0].place, p("/real/a.txt"));
    assert_eq!(group.files[1].place, p("/other/a.txt"));
}

#[test]
fn without_random_read_the_quick_check_is_skipped_and_said() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/s/a.bin"), &[3u8; 4096]);
    fake.add_file(&p("/s/b.bin"), &[3u8; 4096]);
    let mut middle = vec![3u8; 4096];
    middle[2048] = 4;
    fake.add_file(&p("/s/c.bin"), &middle);
    let shaped = Shaped {
        fake,
        no_random: true,
        ..Shaped::default()
    };
    let options = DupOptions {
        partial_block: 16,
        ..DupOptions::default()
    };
    let result = run(&shaped, &["/s"], &options);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(names(&result.groups[0]), ["a.bin", "b.bin"]);
    assert_eq!(result.no_quick, 3);
    // 36 cells.
    assert_eq!(result.notes(), ["3 files read whole: no quick check there"]);
}

#[test]
fn roots_can_be_files_as_marked_entries_or_a_collection() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/x.txt"), b"same");
    fake.add_file(&p("/b/y.txt"), b"same");
    fake.add_file(&p("/b/z.txt"), b"same");
    let shaped = Shaped {
        fake,
        ..Shaped::default()
    };
    let result = run(&shaped, &["/a/x.txt", "/b/y.txt"], &DupOptions::default());
    assert_eq!(result.files_seen, 2);
    assert_eq!(names(&result.groups[0]), ["x.txt", "y.txt"]);
    // Members carry their size and time for the trash plan's check.
    assert!(result.groups[0].files.iter().all(|f| f.size == 4));
}

fn pair(first: &str, second: &str, times: [i64; 2]) -> DupResult {
    let fake = FakeProvider::new();
    fake.add_file(&p(first), b"same bytes");
    fake.add_file(&p(second), b"same bytes");
    fake.set_modified(&p(first), Timestamp::from_unix(times[0], 0));
    fake.set_modified(&p(second), Timestamp::from_unix(times[1], 0));
    let shaped = Shaped {
        fake,
        ..Shaped::default()
    };
    run(&shaped, &[first, second], &DupOptions::default())
}

fn kept(result: &DupResult, rule: &KeepRule) -> Vec<Place> {
    let selection = select(result, rule);
    let choice = &selection.choices[0];
    choice
        .keep
        .iter()
        .map(|&at| result.groups[0].files[at].place.clone())
        .collect()
}

#[test]
fn the_default_keeps_the_oldest_and_a_tie_goes_to_the_shortest_path() {
    assert_eq!(KeepRule::default(), KeepRule::Oldest);
    let dated = pair("/x/new.txt", "/y/old.txt", [200, 100]);
    assert_eq!(kept(&dated, &KeepRule::default()), [p("/y/old.txt")]);
    // The same time: the shorter path stays, though it was found second.
    let tie = pair("/longer-folder/a.txt", "/s/a.txt", [100, 100]);
    assert_eq!(kept(&tie, &KeepRule::Oldest), [p("/s/a.txt")]);
    assert_eq!(kept(&tie, &KeepRule::Newest), [p("/s/a.txt")]);
}

#[test]
fn first_by_name_ignores_case_and_settles_a_tie_by_path() {
    let named = pair("/a/b.txt", "/longer/A.txt", [1, 2]);
    assert_eq!(kept(&named, &KeepRule::FirstByName), [p("/longer/A.txt")]);
    let summary = select(&named, &KeepRule::FirstByName).summary();
    assert_eq!(
        summary,
        "Marked 1 file, 10 bytes. Kept the first by name in 1 group."
    );
    let same_name = pair("/longer/a.txt", "/s/a.txt", [1, 2]);
    assert_eq!(kept(&same_name, &KeepRule::FirstByName), [p("/s/a.txt")]);
}
