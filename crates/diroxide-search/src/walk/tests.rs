//! The walkers on one local fixture: hidden entries, ignore files, a
//! snapshot folder, and links (inside, outside, and back to the start)
//! where this machine can make them.

use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diroxide_core::{Attributes, Place};
use diroxide_vfs::{AttributeChange, LocalProvider, OpContext, Provider, Registry};
#[cfg(windows)]
use diroxide_vfs::{LinkKind, LinkRequest};

use super::*;
use crate::query::SNAPSHOT_FOLDERS;
use crate::sink::SearchControl;
use crate::test_support::scratch;

/// The fixture, and whether its links could be made.
pub(crate) struct Fixture {
    _dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) links: bool,
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// Marks an entry hidden the Windows way (on Unix the leading dot does).
fn hide(path: &Path) {
    if cfg!(windows) {
        let local = LocalProvider::new();
        let place = Place::parse(&path.to_string_lossy()).unwrap();
        let ctx = OpContext::new();
        let bits = match local.stat(&place, &ctx).unwrap().attributes {
            Some(Attributes::Windows(bits)) => bits,
            _ => 0,
        };
        let change = AttributeChange {
            attributes: Some(Attributes::Windows(bits | 0x2)),
            ..AttributeChange::default()
        };
        local.set_attributes(&place, &change, &ctx).unwrap();
    }
}

/// A link to a folder: a symbolic link, or on Windows without the right
/// to make one, a junction.
fn link(at: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, at).is_ok()
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(target, at).is_ok() {
            return true;
        }
        let place = Place::parse(&at.to_string_lossy()).unwrap();
        let request = LinkRequest {
            kind: LinkKind::Junction,
            target: target.to_string_lossy().into_owned(),
            target_is_folder: true,
        };
        LocalProvider::new()
            .create_link(&place, &request, &OpContext::new())
            .is_ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (at, target);
        false
    }
}

/// root/
///   a.txt, .hidden.txt, notes.log (ignored), .gitignore (*.log, build/)
///   .hiddendir/x.txt, build/out.o (ignored), .snapshots/s.txt
///   sub/b.txt, sub/.ignore (skip.me), sub/skip.me, sub/deeper/c.txt
///   link-sub -> sub, link-out -> ../outside, sub/deeper/loop -> root
pub(crate) fn fixture() -> Fixture {
    let dir = scratch();
    let root = dir.path().join("root");
    write(&root.join("a.txt"), "alpha\n");
    write(&root.join(".hidden.txt"), "hidden\n");
    hide(&root.join(".hidden.txt"));
    write(&root.join(".hiddendir").join("x.txt"), "x\n");
    hide(&root.join(".hiddendir"));
    write(&root.join("notes.log"), "log\n");
    write(&root.join(".gitignore"), "*.log\nbuild/\n");
    write(&root.join("build").join("out.o"), "o\n");
    write(&root.join(".snapshots").join("s.txt"), "s\n");
    write(&root.join("sub").join("b.txt"), "beta\n");
    write(&root.join("sub").join(".ignore"), "skip.me\n");
    write(&root.join("sub").join("skip.me"), "skip\n");
    write(&root.join("sub").join("deeper").join("c.txt"), "gamma\n");
    write(&dir.path().join("outside").join("far.txt"), "far\n");
    let links = link(&root.join("link-sub"), &root.join("sub"))
        && link(&root.join("link-out"), &dir.path().join("outside"))
        && link(&root.join("sub").join("deeper").join("loop"), &root);
    if !links {
        eprintln!("Links could not be made here: the link cases are skipped.");
    }
    Fixture {
        _dir: dir,
        root,
        links,
    }
}

/// A visitor that collects the paths it is given, below the start, with
/// forward slashes.
#[derive(Clone)]
struct Collect {
    root: String,
    seen: Arc<Mutex<BTreeSet<String>>>,
}

impl Visitor for Collect {
    fn visit(&mut self, candidate: Candidate) -> ControlFlow<()> {
        let text = candidate.path_text().replace('\\', "/");
        let below = text
            .strip_prefix(&self.root)
            .unwrap_or(&text)
            .trim_start_matches('/')
            .to_owned();
        self.seen.lock().unwrap().insert(below);
        ControlFlow::Continue(())
    }
}

struct Options {
    hidden: bool,
    ignore_files: bool,
    follow_links: bool,
    max_depth: Option<u32>,
    provider_walk: bool,
}

fn walk_with(fixture: &Fixture, options: &Options) -> (BTreeSet<String>, WalkStats) {
    let place = Place::parse(&fixture.root.to_string_lossy()).unwrap();
    let root = Root::new(place);
    let control = SearchControl::new();
    let skip: Vec<String> = SNAPSHOT_FOLDERS.iter().map(|s| (*s).to_owned()).collect();
    let plan = WalkPlan {
        root: &root,
        hidden: options.hidden,
        ignore_files: options.ignore_files,
        follow_links: options.follow_links,
        one_file_system: true,
        archives: false,
        max_depth: options.max_depth,
        threads: if options.provider_walk { 3 } else { 2 },
        skip_names: &skip,
        skip_places: &[],
        metadata: true,
        control: &control,
        live: &Live::default(),
    };
    let seen = Arc::new(Mutex::new(BTreeSet::new()));
    let visitor = Collect {
        root: fixture.root.to_string_lossy().replace('\\', "/"),
        seen: Arc::clone(&seen),
    };
    let make = || visitor.clone();
    let stats = if options.provider_walk {
        provider::walk(
            &plan,
            Arc::new(LocalProvider::new()),
            &Registry::with_local(),
            &make,
        )
    } else {
        assert!(uses_local_walker(&plan));
        run(&plan, &Registry::with_local(), &make).unwrap()
    };
    let seen = seen.lock().unwrap().clone();
    (seen, stats)
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn both_walkers_give_the_same_entries_and_counts() {
    let fixture = fixture();
    for hidden in [false, true] {
        for ignore_files in [false, true] {
            let options = |provider_walk| Options {
                hidden,
                ignore_files,
                follow_links: false,
                max_depth: None,
                provider_walk,
            };
            let (local, local_stats) = walk_with(&fixture, &options(false));
            let (walked, walked_stats) = walk_with(&fixture, &options(true));
            assert_eq!(
                local, walked,
                "hidden {hidden}, ignore files {ignore_files}"
            );
            assert_eq!(local_stats.entries, walked_stats.entries);
            assert_eq!(local_stats.folders, walked_stats.folders);
            assert_eq!(local_stats.skips, walked_stats.skips);
            assert_eq!(local_stats.entries, local.len() as u64);
        }
    }
}

#[test]
fn each_option_changes_what_is_walked() {
    let fixture = fixture();
    let walk = |hidden, ignore_files| {
        walk_with(
            &fixture,
            &Options {
                hidden,
                ignore_files,
                follow_links: false,
                max_depth: None,
                provider_walk: false,
            },
        )
    };
    let mut plain = vec![
        "a.txt",
        "build",
        "build/out.o",
        "notes.log",
        "sub",
        "sub/b.txt",
        "sub/deeper",
        "sub/deeper/c.txt",
        "sub/skip.me",
    ];
    // The dot files are hidden only on Unix; on Windows hidden is the
    // attribute, which the fixture sets on .hidden.txt and .hiddendir.
    if cfg!(windows) {
        plain.extend([".gitignore", "sub/.ignore"]);
    }
    if fixture.links {
        plain.extend(["link-out", "link-sub", "sub/deeper/loop"]);
    }
    let (seen, stats) = walk(false, false);
    assert_eq!(seen, set(&plain));
    assert_eq!(stats.skips.hidden, if cfg!(windows) { 2 } else { 5 });
    assert_eq!(
        stats.skips.snapshots,
        if cfg!(windows) { 1 } else { 0 },
        "{stats:?}"
    );
    if fixture.links {
        assert_eq!(stats.skips.links_not_followed, 3);
    }
    let (with_hidden, stats) = walk(true, false);
    for hidden in [
        ".hidden.txt",
        ".hiddendir",
        ".hiddendir/x.txt",
        ".gitignore",
    ] {
        assert!(with_hidden.contains(hidden), "{hidden}");
    }
    assert_eq!(stats.skips.hidden, 0);
    assert_eq!(stats.skips.snapshots, 1);
    assert!(!with_hidden.iter().any(|p| p.starts_with(".snapshots")));
    let (ignored, stats) = walk(true, true);
    for gone in ["notes.log", "build", "build/out.o", "sub/skip.me"] {
        assert!(!ignored.contains(gone), "{gone}");
    }
    assert!(ignored.contains("sub/b.txt") && ignored.contains("sub/.ignore"));
    assert!(stats.skips.ignore_files);
}

#[test]
fn followed_links_stay_inside_the_start_and_loops_are_caught() {
    let fixture = fixture();
    if !fixture.links {
        eprintln!("Links could not be made here: test skipped.");
        return;
    }
    let (seen, stats) = walk_with(
        &fixture,
        &Options {
            hidden: false,
            ignore_files: false,
            follow_links: true,
            max_depth: None,
            provider_walk: true,
        },
    );
    assert!(seen.contains("link-sub/b.txt"), "{seen:?}");
    assert!(seen.contains("link-sub/deeper/c.txt"));
    assert!(!seen.iter().any(|p| p.starts_with("link-out/")), "{seen:?}");
    assert!(!seen.iter().any(|p| p.contains("loop/")), "{seen:?}");
    assert_eq!(stats.skips.links_outside, 1);
    assert_eq!(stats.skips.loops, 2, "{stats:?}");
    assert_eq!(stats.skips.links_not_followed, 0);
}

#[test]
fn the_depth_limit_holds_in_both_walkers() {
    let fixture = fixture();
    for provider_walk in [false, true] {
        let (seen, _) = walk_with(
            &fixture,
            &Options {
                hidden: true,
                ignore_files: false,
                follow_links: false,
                max_depth: Some(1),
                provider_walk,
            },
        );
        assert!(seen.iter().all(|p| !p.contains('/')), "{seen:?}");
        assert!(seen.contains("sub") && seen.contains("a.txt"));
    }
}

#[test]
fn a_visitor_can_stop_the_walk() {
    struct First(Arc<Mutex<u32>>);
    impl Visitor for First {
        fn visit(&mut self, _: Candidate) -> ControlFlow<()> {
            *self.0.lock().unwrap() += 1;
            ControlFlow::Break(())
        }
    }
    let fixture = fixture();
    let place = Place::parse(&fixture.root.to_string_lossy()).unwrap();
    let root = Root::new(place);
    let control = SearchControl::new();
    for provider_walk in [false, true] {
        let plan = WalkPlan {
            root: &root,
            hidden: true,
            ignore_files: false,
            follow_links: provider_walk,
            one_file_system: true,
            archives: false,
            max_depth: None,
            threads: 1,
            skip_names: &[],
            skip_places: &[],
            metadata: false,
            control: &control,
            live: &Live::default(),
        };
        let count = Arc::new(Mutex::new(0));
        let stats = run(&plan, &Registry::with_local(), &|| {
            First(Arc::clone(&count))
        })
        .unwrap();
        assert!(stats.stopped);
        assert_eq!(*count.lock().unwrap(), 1);
    }
}
