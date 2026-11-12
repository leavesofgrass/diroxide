//! Tests of the duplicate finder on made trees in the fake provider.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use diroxide_core::Timestamp;
use diroxide_core::format::{BRAILLE_CELLS, first_cells};
use diroxide_vfs::{CancelToken, Registry, VfsError};

use super::*;
use crate::engine::JobControl;
use crate::fake::FakeProvider;
use crate::pace::FakePacer;
use crate::testing::p;

/// A hook run on each progress report.
type Hook = Box<dyn Fn(&DupProgress)>;

/// A sink that records what it hears and answers the question from a
/// switch; a hook may act on each report.
#[derive(Default)]
struct Ears {
    heard: Mutex<Vec<DupProgress>>,
    asked: Mutex<Vec<DupEstimate>>,
    yes: bool,
    hook: Option<Hook>,
}

impl DupSink for Ears {
    fn progress(&self, progress: &DupProgress) {
        self.heard.lock().unwrap().push(progress.clone());
        if let Some(hook) = &self.hook {
            hook(progress);
        }
    }

    fn confirm(&self, estimate: &DupEstimate) -> bool {
        self.asked.lock().unwrap().push(*estimate);
        self.yes
    }
}

/// A hash that calls everything equal: only a byte comparison can tell
/// its groups apart.
struct Collider;

impl ContentHasher for Collider {
    fn name(&self) -> &'static str {
        "collider"
    }

    fn hash(
        &self,
        reader: &mut dyn Read,
        _cancel: &CancelToken,
        on_step: &mut dyn FnMut(u64),
    ) -> Result<Vec<u8>, VfsError> {
        let mut all = Vec::new();
        reader.read_to_end(&mut all)?;
        on_step(all.len() as u64);
        Ok(vec![7])
    }
}

fn registry(fake: &FakeProvider) -> Registry {
    let mut registry = Registry::new();
    registry.set_local(Arc::new(fake.clone()));
    registry
}

fn run_with(
    fake: &FakeProvider,
    roots: &[&str],
    options: &DupOptions,
    sink: &dyn DupSink,
    hasher: &dyn ContentHasher,
) -> Result<DupResult, DupError> {
    let registry = registry(fake);
    let pacer = FakePacer::new();
    let env = DupEnv {
        registry: &registry,
        sink,
        hasher,
        pacer: &pacer,
    };
    let roots: Vec<_> = roots.iter().map(|r| p(r)).collect();
    find_duplicates(&roots, options, &JobControl::new(), &env)
}

fn run(fake: &FakeProvider, roots: &[&str], options: &DupOptions) -> DupResult {
    run_with(fake, roots, options, &QuietSink, &Sha256Hasher).unwrap()
}

fn names(group: &DupGroup) -> Vec<String> {
    group
        .files
        .iter()
        .map(|f| f.place.name().unwrap().to_owned())
        .collect()
}

fn fits(line: &str) -> bool {
    line.chars().count() <= BRAILLE_CELLS
}

#[test]
fn three_copies_across_two_folders_are_one_group() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/report.pdf"), b"same bytes here");
    fake.add_file(&p("/a/sub/report-copy.pdf"), b"same bytes here");
    fake.add_file(&p("/b/report.pdf"), b"same bytes here");
    fake.add_file(&p("/b/other.txt"), b"different bytes");
    let result = run(&fake, &["/a", "/b"], &DupOptions::default());
    assert_eq!(result.groups.len(), 1);
    let group = &result.groups[0];
    assert_eq!(group.files.len(), 3);
    assert_eq!(group.size, 15);
    assert_eq!(group.waste(), 30);
    assert_eq!(result.files_seen, 4);
    assert_eq!(result.hasher, "SHA-256");
    // The count line first, inside 40 cells: 30 cells.
    let line = result.count_line();
    assert_eq!(line, "1 group, 3 files, 30 bytes to free");
    assert!(fits(&line), "{} cells", line.chars().count());
    // The group's row: the number and the copies in the first 40 cells.
    let row = group.row(1, 1);
    assert_eq!(
        row,
        "Group 1 of 1, 3 copies, 15 bytes each, 30 bytes to free"
    );
    assert!(first_cells(&row, BRAILLE_CELLS).contains("3 copies"));
    // Each file's row: the name first, its folder after.
    let first = group.files[0].row();
    assert!(first.starts_with("report.pdf, in "), "{first}");
    let flat = result.flat_rows();
    assert_eq!(flat.len(), 3);
    assert!(
        flat[0].starts_with("report.pdf, copy 1 of 3, group 1, in "),
        "{}",
        flat[0]
    );
}

#[test]
fn equal_size_and_ends_but_different_inside_is_not_a_duplicate() {
    // The collision test: the same size, the same first and last blocks,
    // so the quick check matches; one byte differs in the middle.
    let fake = FakeProvider::new();
    let mut one = vec![b'a'; 64];
    let mut two = one.clone();
    one[32] = b'x';
    two[32] = b'y';
    fake.add_file(&p("/t/one.bin"), &one);
    fake.add_file(&p("/t/two.bin"), &two);
    let options = DupOptions {
        partial_block: 8,
        ..DupOptions::default()
    };
    let ears = Ears::default();
    let result = run_with(&fake, &["/t"], &options, &ears, &Sha256Hasher).unwrap();
    assert!(result.groups.is_empty());
    assert_eq!(result.count_line(), "No duplicates in 2 files");
    // Both passed the quick check, so both were hashed whole.
    let heard = ears.heard.lock().unwrap();
    let hashing = heard
        .iter()
        .rfind(|p| p.stage == DupStage::Hashing)
        .unwrap();
    assert_eq!((hashing.files_total, hashing.bytes_total), (2, 128));
}

#[test]
fn the_quick_check_alone_never_makes_a_group() {
    // The same ends with a big middle that differs: a partial match only.
    let fake = FakeProvider::new();
    let mut one = vec![0u8; 4096];
    let two = one.clone();
    one[2000] = 1;
    fake.add_file(&p("/t/one.bin"), &one);
    fake.add_file(&p("/t/two.bin"), &two);
    let options = DupOptions {
        partial_block: 16,
        ..DupOptions::default()
    };
    assert!(run(&fake, &["/t"], &options).groups.is_empty());
}

#[test]
fn empty_files_are_said_and_left_out_unless_asked() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/e/a.txt"), b"");
    fake.add_file(&p("/e/b.txt"), b"");
    fake.add_file(&p("/e/c.txt"), b"");
    let result = run(&fake, &["/e"], &DupOptions::default());
    assert!(result.groups.is_empty());
    assert_eq!(result.empty_left_out, 3);
    assert_eq!(result.notes(), vec!["3 empty files left out".to_owned()]);
    let options = DupOptions {
        include_empty: true,
        ..DupOptions::default()
    };
    let result = run(&fake, &["/e"], &options);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].row(1, 1),
        "Group 1 of 1, 3 copies, empty files"
    );
}

#[test]
fn one_file_under_two_names_without_a_link_count_is_reached_twice() {
    // The fake gives no link count, so a second name of one identity is
    // the same file reached again, as through a junction: counted once,
    // even when hard links are kept, so no helper can mark it.
    let fake = FakeProvider::new();
    fake.add_file(&p("/h/a.txt"), b"one file");
    fake.add_hard_link(&p("/h/b.txt"), &p("/h/a.txt"));
    for include_hard_links in [false, true] {
        let options = DupOptions {
            include_hard_links,
            ..DupOptions::default()
        };
        let result = run(&fake, &["/h"], &options);
        assert!(result.groups.is_empty());
        assert_eq!(result.reached_twice, 1);
        assert_eq!(result.notes(), ["1 file reached twice, counted once"]);
        let row = result.left_out[0].row();
        assert!(row.starts_with("b.txt, in "), "{row}");
        assert!(row.contains(": reached twice, counted once as "), "{row}");
    }
}

#[test]
fn a_byte_comparison_splits_what_the_hash_joined() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/c/a.bin"), b"alpha");
    fake.add_file(&p("/c/b.bin"), b"bravo");
    fake.add_file(&p("/c/c.bin"), b"alpha");
    let joined = run_with(
        &fake,
        &["/c"],
        &DupOptions::default(),
        &QuietSink,
        &Collider,
    )
    .unwrap();
    assert_eq!(joined.groups.len(), 1, "the hash alone decides");
    let options = DupOptions {
        byte_compare: true,
        ..DupOptions::default()
    };
    let split = run_with(&fake, &["/c"], &options, &QuietSink, &Collider).unwrap();
    assert_eq!(split.groups.len(), 1);
    assert_eq!(names(&split.groups[0]), ["a.bin", "c.bin"]);
    assert!(split.groups[0].bytes_compared);
}

#[test]
fn groups_come_largest_waste_first() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/w/s1"), b"ab");
    fake.add_file(&p("/w/s2"), b"ab");
    fake.add_file(&p("/w/s3"), b"ab");
    fake.add_file(&p("/w/b1"), b"abcdef");
    fake.add_file(&p("/w/b2"), b"abcdef");
    let result = run(&fake, &["/w"], &DupOptions::default());
    let wastes: Vec<u64> = result.groups.iter().map(DupGroup::waste).collect();
    assert_eq!(wastes, [6, 4]);
    assert_eq!(
        result.group_rows(),
        [
            "Group 1 of 2, 2 copies, 6 bytes each, 6 bytes to free",
            "Group 2 of 2, 3 copies, 2 bytes each, 4 bytes to free",
        ]
    );
    assert_eq!(result.count_line(), "2 groups, 5 files, 10 bytes to free");
}

#[test]
fn a_root_given_twice_counts_each_file_once() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/r/a"), b"one");
    fake.add_file(&p("/r/sub/b"), b"two");
    let result = run(&fake, &["/r", "/r/sub"], &DupOptions::default());
    assert_eq!(result.files_seen, 2);
    assert!(result.groups.is_empty());
}

#[test]
fn small_files_under_the_minimum_are_counted() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/m/a"), b"x");
    fake.add_file(&p("/m/b"), b"x");
    let options = DupOptions {
        min_size: 2,
        ..DupOptions::default()
    };
    let result = run(&fake, &["/m"], &options);
    assert!(result.groups.is_empty());
    assert_eq!(result.notes(), ["2 files left out: too small"]);
}

#[test]
fn an_unreadable_file_is_said_and_left_out() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/u/a"), b"same");
    fake.add_file(&p("/u/b"), b"same");
    fake.add_file(&p("/u/c"), b"same");
    fake.switches_mut().locked.push(p("/u/c"));
    let result = run(&fake, &["/u"], &DupOptions::default());
    assert_eq!(names(&result.groups[0]), ["a", "b"]);
    assert_eq!(result.unreadable, 1);
    assert_eq!(result.notes(), ["1 file could not be read"]);
    assert!(result.left_out[0].row().starts_with("c, in "));
}

#[test]
fn no_roots_is_an_error_in_words() {
    let fake = FakeProvider::new();
    let error = run_with(
        &fake,
        &[],
        &DupOptions::default(),
        &QuietSink,
        &Sha256Hasher,
    );
    assert_eq!(error, Err(DupError::NoRoots));
    assert_eq!(
        DupError::NoRoots.to_string(),
        "Nothing to search: no folders were given"
    );
}

#[test]
fn a_large_read_asks_first_and_no_stops_it() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/q/a"), &[1u8; 300]);
    fake.add_file(&p("/q/b"), &[1u8; 300]);
    let options = DupOptions {
        partial_block: 16,
        ask_above: Some(100),
        ..DupOptions::default()
    };
    let no = Ears::default();
    let stopped = run_with(&fake, &["/q"], &options, &no, &Sha256Hasher);
    let estimate = DupEstimate {
        files: 2,
        bytes: 600,
    };
    assert_eq!(stopped, Err(DupError::Declined(estimate)));
    assert_eq!(no.asked.lock().unwrap().as_slice(), &[estimate]);
    let question = estimate.question();
    assert_eq!(
        question,
        "Read 600 bytes to find duplicates? 2 files. No is the default. y or n"
    );
    assert!(first_cells(&question, BRAILLE_CELLS).contains("600 bytes"));
    assert!(question.ends_with("No is the default. y or n"));
    assert_eq!(
        DupError::Declined(estimate).to_string(),
        "Stopped before reading 600 bytes: not confirmed"
    );
    let yes = Ears {
        yes: true,
        ..Ears::default()
    };
    let result = run_with(&fake, &["/q"], &options, &yes, &Sha256Hasher).unwrap();
    assert_eq!(result.groups.len(), 1);
    // Small enough for the quick check to cover it whole: nothing asked.
    let small = DupOptions {
        ask_above: Some(100),
        ..DupOptions::default()
    };
    let quiet = Ears::default();
    run_with(&fake, &["/q"], &small, &quiet, &Sha256Hasher).unwrap();
    assert!(quiet.asked.lock().unwrap().is_empty());
}

#[test]
fn cancel_stops_the_search() {
    let fake = FakeProvider::new();
    for n in 0..20 {
        fake.add_file(&p(&format!("/k/f{n}")), b"same");
    }
    let control = JobControl::new();
    let stop = control.clone();
    let ears = Ears {
        hook: Some(Box::new(move |progress: &DupProgress| {
            if progress.stage == DupStage::QuickCheck {
                stop.cancel.cancel();
            }
        })),
        ..Ears::default()
    };
    let registry = registry(&fake);
    let pacer = FakePacer::new();
    let env = DupEnv {
        registry: &registry,
        sink: &ears,
        hasher: &Sha256Hasher,
        pacer: &pacer,
    };
    let result = find_duplicates(&[p("/k")], &DupOptions::default(), &control, &env);
    assert_eq!(result, Err(DupError::Cancelled));
    assert_eq!(
        DupError::Cancelled.to_string(),
        "Finding duplicates was cancelled"
    );
}

#[test]
fn pause_is_said_once_each_way_and_the_search_goes_on() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/z/a"), b"same");
    fake.add_file(&p("/z/b"), b"same");
    let control = JobControl::new();
    control.pause();
    let resume = control.clone();
    let ears = Ears {
        hook: Some(Box::new(move |progress: &DupProgress| {
            if progress.paused {
                resume.resume();
            }
        })),
        ..Ears::default()
    };
    let registry = registry(&fake);
    let pacer = FakePacer::new();
    let env = DupEnv {
        registry: &registry,
        sink: &ears,
        hasher: &Sha256Hasher,
        pacer: &pacer,
    };
    let result = find_duplicates(&[p("/z")], &DupOptions::default(), &control, &env).unwrap();
    assert_eq!(result.groups.len(), 1);
    let heard = ears.heard.lock().unwrap();
    let paused: Vec<_> = heard.iter().filter(|p| p.paused).collect();
    assert_eq!(paused.len(), 1);
    assert_eq!(paused[0].describe(), "Duplicates paused at 0 percent");
}

#[test]
fn progress_comes_at_the_interval_and_each_step_at_once() {
    let fake = FakeProvider::new();
    for n in 0..10 {
        fake.add_file(&p(&format!("/i/f{n}")), b"same");
    }
    let ears = Ears::default();
    let options = DupOptions {
        progress_interval: Duration::from_secs(60),
        ..DupOptions::default()
    };
    // The fake clock never moves, so only the steps' starts are said.
    run_with(&fake, &["/i"], &options, &ears, &Sha256Hasher).unwrap();
    let stages: Vec<DupStage> = ears.heard.lock().unwrap().iter().map(|p| p.stage).collect();
    // Four-byte files are read whole by the quick check: no hashing step.
    assert_eq!(stages, [DupStage::Listing, DupStage::QuickCheck]);
    let every = Ears::default();
    let options = DupOptions {
        progress_interval: Duration::ZERO,
        ..DupOptions::default()
    };
    run_with(&fake, &["/i"], &options, &every, &Sha256Hasher).unwrap();
    assert!(every.heard.lock().unwrap().len() > 10);
}

#[test]
fn the_progress_lines_fit_and_say_the_step_first() {
    let listing = DupProgress {
        files_done: 120_000,
        ..DupProgress::start(DupStage::Listing, 0, 0)
    };
    // 34 cells.
    assert_eq!(listing.describe(), "Duplicates: listed 120,000 files");
    let mut hashing = DupProgress::start(DupStage::Hashing, 45_000, 8_000_000_000);
    hashing.files_done = 12_000;
    hashing.bytes_done = 3_100_000_000;
    // 31 cells.
    assert_eq!(hashing.describe(), "Duplicates: hashing, 38 percent");
    assert_eq!(
        hashing.detail(),
        "Hashing 12,000 of 45,000 files, 3.1 of 8.0 GB"
    );
    let mut every = vec![listing.describe(), hashing.describe()];
    for stage in [
        DupStage::QuickCheck,
        DupStage::Comparing,
        DupStage::Rechecking,
    ] {
        let mut progress = DupProgress::start(stage, 10, 0);
        progress.files_done = 10;
        every.push(progress.describe());
    }
    // "Duplicates: checking again, 100 percent": 39 cells, the longest.
    assert!(every.contains(&"Duplicates: checking again, 100 percent".to_owned()));
    for line in every {
        assert!(fits(&line), "{line}: {} cells", line.chars().count());
    }
}

fn three_dated(fake: &FakeProvider) -> DupResult {
    fake.add_file(&p("/s/longer-name/old.txt"), b"same bytes");
    fake.add_file(&p("/s/new.txt"), b"same bytes");
    fake.add_file(&p("/t/mid.txt"), b"same bytes");
    fake.set_modified(&p("/s/longer-name/old.txt"), Timestamp::from_unix(100, 0));
    fake.set_modified(&p("/s/new.txt"), Timestamp::from_unix(300, 0));
    fake.set_modified(&p("/t/mid.txt"), Timestamp::from_unix(200, 0));
    run(&fake.clone(), &["/s", "/t"], &DupOptions::default())
}

fn kept(result: &DupResult, selection: &Selection) -> Vec<String> {
    selection
        .choices
        .iter()
        .flat_map(|c| {
            c.keep
                .iter()
                .map(|&at| {
                    result.groups[c.group].files[at]
                        .place
                        .name()
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_helpers_keep_one_and_say_what_they_chose() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let newest = select(&result, &KeepRule::Newest);
    assert_eq!(kept(&result, &newest), ["new.txt"]);
    assert_eq!(newest.marked.len(), 2);
    assert_eq!(newest.bytes, 20);
    // 54 cells; the count and the bytes in the first 40.
    assert_eq!(
        newest.summary(),
        "Marked 2 files, 20 bytes. Kept the newest in 1 group."
    );
    assert_eq!(
        kept(&result, &select(&result, &KeepRule::Oldest)),
        ["old.txt"]
    );
    assert_eq!(
        kept(&result, &select(&result, &KeepRule::FirstFound)),
        ["old.txt"]
    );
    // new.txt and mid.txt tie on length; the first found of the tie stays.
    let shortest = select(&result, &KeepRule::ShortestPath);
    assert_eq!(kept(&result, &shortest), ["new.txt"]);
    let here = select(&result, &KeepRule::InFolder(p("/t")));
    assert_eq!(kept(&result, &here), ["mid.txt"]);
}

#[test]
fn no_helper_ever_marks_a_whole_group() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let rules = [
        KeepRule::Newest,
        KeepRule::Oldest,
        KeepRule::FirstFound,
        KeepRule::ShortestPath,
        KeepRule::InFolder(p("/t")),
        KeepRule::InFolder(p("/s")),
        KeepRule::InFolder(p("/nowhere")),
    ];
    for rule in rules {
        let selection = select(&result, &rule);
        for choice in &selection.choices {
            let size = result.groups[choice.group].files.len();
            assert!(choice.mark.len() < size, "{rule:?} marked a whole group");
            assert!(choice.left_alone.is_some() || !choice.keep.is_empty());
        }
    }
}

#[test]
fn a_group_the_helper_cannot_decide_is_left_alone_in_words() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let none = select(&result, &KeepRule::InFolder(p("/nowhere")));
    assert!(none.marked.is_empty());
    assert_eq!(none.summary(), "Marked nothing. 1 group left alone.");
    let line = &none.left_alone_lines()[0];
    assert!(
        line.starts_with("Group 1 left alone: no copy in "),
        "{line}"
    );
    // Every copy of a two-copy group in one folder: nothing to mark.
    let fake = FakeProvider::new();
    fake.add_file(&p("/one/a"), b"same");
    fake.add_file(&p("/one/b"), b"same");
    let both = run(&fake, &["/one"], &DupOptions::default());
    let selection = select(&both, &KeepRule::InFolder(p("/one")));
    assert!(selection.marked.is_empty());
    assert!(selection.left_alone_lines()[0].contains("every copy is in"));
    // A time not known leaves a group alone for Newest.
    let fake = FakeProvider::new();
    fake.add_file(&p("/n/a"), b"same");
    fake.add_file(&p("/n/b"), b"same");
    let mut untimed = run(&fake, &["/n"], &DupOptions::default());
    untimed.groups[0].files[0].modified = None;
    let selection = select(&untimed, &KeepRule::Newest);
    assert_eq!(
        selection.left_alone_lines(),
        ["Group 1 left alone: a time is not known"]
    );
}

fn recheck(fake: &FakeProvider, result: &DupResult, selection: &Selection) -> Rechecked {
    let registry = registry(fake);
    let pacer = FakePacer::new();
    let env = DupEnv {
        registry: &registry,
        sink: &QuietSink,
        hasher: &Sha256Hasher,
        pacer: &pacer,
    };
    recheck_selection(result, selection, &JobControl::new(), &env).unwrap()
}

#[test]
fn an_intact_selection_passes_the_check_again() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let selection = select(&result, &KeepRule::Newest);
    let checked = recheck(&fake, &result, &selection);
    assert_eq!(checked.safe, selection.marked);
    assert!(checked.refused.is_empty());
    assert_eq!(
        checked.summary(),
        "Checked again: 2 files can go to the trash"
    );
}

#[test]
fn a_keeper_that_changed_refuses_its_whole_group() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let selection = select(&result, &KeepRule::Newest);
    // The kept copy is rewritten after the search, same size.
    fake.add_file(&p("/s/new.txt"), b"SAME BYTES");
    let checked = recheck(&fake, &result, &selection);
    assert!(checked.safe.is_empty(), "nothing of the group may go");
    assert_eq!(checked.refused.len(), 1);
    assert_eq!(
        checked.refused[0].line(),
        "Group 1 refused: the kept copy, new.txt, changed"
    );
    assert_eq!(
        checked.summary(),
        "Checked again: no files can go to the trash, 1 held back"
    );
    // The others are untouched.
    assert!(fake.exists(&p("/s/longer-name/old.txt")));
    assert!(fake.exists(&p("/t/mid.txt")));
}

#[test]
fn a_keeper_that_is_gone_refuses_its_whole_group() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let selection = select(&result, &KeepRule::Newest);
    fake.vanish(&p("/s/new.txt"));
    let checked = recheck(&fake, &result, &selection);
    assert!(checked.safe.is_empty());
    assert_eq!(
        checked.refused[0].line(),
        "Group 1 refused: the kept copy, new.txt, is gone"
    );
}

#[test]
fn a_marked_file_that_changed_is_dropped_and_said() {
    let fake = FakeProvider::new();
    let result = three_dated(&fake);
    let selection = select(&result, &KeepRule::Newest);
    fake.add_file(&p("/t/mid.txt"), b"other byte");
    let checked = recheck(&fake, &result, &selection);
    assert_eq!(checked.safe, vec![p("/s/longer-name/old.txt")]);
    assert_eq!(
        checked.refused[0].line(),
        "Not trashed: mid.txt changed, group 1"
    );
}

#[test]
fn the_check_compares_bytes_when_the_search_did() {
    let fake = FakeProvider::new();
    fake.add_file(&p("/c/a.bin"), b"alpha");
    fake.add_file(&p("/c/b.bin"), b"alpha");
    let options = DupOptions {
        byte_compare: true,
        ..DupOptions::default()
    };
    let result = run_with(&fake, &["/c"], &options, &QuietSink, &Collider).unwrap();
    let selection = select(&result, &KeepRule::FirstFound);
    // b.bin changes; the colliding hash cannot see it, the bytes can.
    fake.add_file(&p("/c/b.bin"), b"bravo");
    let registry = registry(&fake);
    let pacer = FakePacer::new();
    let env = DupEnv {
        registry: &registry,
        sink: &QuietSink,
        hasher: &Collider,
        pacer: &pacer,
    };
    let checked = recheck_selection(&result, &selection, &JobControl::new(), &env).unwrap();
    assert!(checked.safe.is_empty());
    assert_eq!(
        checked.refused[0].line(),
        "Not trashed: b.bin changed, group 1"
    );
}

#[test]
fn sha256_hashes_in_steps_and_hears_cancel() {
    let data = vec![5u8; STEP + 10];
    let mut steps = Vec::new();
    let digest = Sha256Hasher
        .hash(
            &mut std::io::Cursor::new(data.clone()),
            &CancelToken::new(),
            &mut |done| steps.push(done),
        )
        .unwrap();
    assert_eq!(steps, [STEP as u64, STEP as u64 + 10]);
    assert_eq!(digest.len(), 32);
    let cancel = CancelToken::new();
    cancel.cancel();
    let stopped = Sha256Hasher.hash(&mut std::io::Cursor::new(data), &cancel, &mut |_| {});
    assert_eq!(stopped, Err(VfsError::Cancelled));
}

#[test]
fn the_default_hasher_is_blake3_and_finds_the_same_groups() {
    // BLAKE3 of nothing, as the reference implementation gives it.
    let empty = Blake3Hasher
        .hash(
            &mut std::io::Cursor::new(Vec::new()),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
    let hex: String = empty.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
    let fake = FakeProvider::new();
    fake.add_file(&p("/a/report.pdf"), b"same bytes here");
    fake.add_file(&p("/b/report.pdf"), b"same bytes here");
    fake.add_file(&p("/b/other.txt"), b"different bytes");
    let result = run_with(
        &fake,
        &["/a", "/b"],
        &DupOptions::default(),
        &QuietSink,
        &Blake3Hasher,
    )
    .unwrap();
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.hasher, "BLAKE3");
}
