//! Every dialog purpose (R27's commit 25, R30's commit 114): the app's
//! dialogs are 112 purposes drawn as seven shapes: the 77 variants of
//! `Purpose` in `crates/diroxide-app/src/app.rs`, and the 35 lists and
//! questions the alpha 4 lanes host under `Organize` and `RunArchive`,
//! each with its own name (`Wants::purpose` in `app/organize.rs`). This
//! test opens each one the app can open from a test through the window,
//! checking that the dialog has a name and that the keyboard focus is on
//! a named control inside it. The purposes it cannot open are listed
//! apart, each with the reason; the two lists together are held against
//! the names as the source declares them, so a purpose added later fails
//! here until it is opened or said. Each opened dialog's purpose is
//! checked by the app's own test hook (lane A5's `App::dialog_purpose`),
//! not only its title. The trash browser, a junction offered for a
//! refused link and an item that cannot be read are made with the file
//! system crate's doubles (`tests/it/doubles.rs`), so they open on every
//! system; every window runs its helper programs through a tools double
//! that starts none, and Open with lists two made-up programs.
//!
//! The purposes are opened in six tests of about nineteen each, so a
//! loaded machine stretches six short tests that run side by side rather
//! than one long one (the run of over eight minutes AE saw while another
//! lane built, Saturday, November 7, 2026).

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use diroxide_app::config::Paths;
use diroxide_app::{Answer, AppEvent, AppOptions, AskId, Question};
use diroxide_core::{SecretKind, SecretRequest, Side};
use masonry::accesskit::Role;

use crate::doubles::{Doubles, OneItemBin, TwoPrograms, WindowTools};
use crate::rig::{HostOf, Rig, app_with, name_of};

/// The purposes no test can open, each with the reason, in words: none
/// since the file system crate's doubles (lane A5 removed JobHistory,
/// whose dialog the Jobs list had replaced).
const UNREACHABLE: &[(&str, &str)] = &[];

/// How many purposes the app declares: 77 variants of `Purpose` and the
/// 35 hosted lists and questions of alpha 4.
const PURPOSES: usize = 112;

/// The purposes whose scenario may say "not made on this system", each
/// for its reason: a symbolic link needs Developer Mode on Windows; the
/// outline needs the syntax set; the preview's three need the quick view
/// to take the focus; Change owner's three are Unix's (Windows hands off
/// to its Security dialog); the internet mark needs a disk that keeps
/// extended attributes or streams.
const MAY_BE_ABSENT: &[&str] = &[
    "Relink",
    "ViewerOutline",
    "PreviewFind",
    "PreviewLinks",
    "PreviewLink",
    "OwnerForm",
    "OwnerName",
    "GroupName",
    "MarkQuestion",
];

/// The names `dialog_purpose` can give, as the source declares them: the
/// string arms of `Purpose::name` in `app.rs` and of `Wants::purpose` in
/// `app/organize.rs`.
fn purpose_names() -> BTreeSet<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../diroxide-app/src");
    let arms = |file: &str, start: &str| -> Vec<String> {
        let source = std::fs::read_to_string(src.join(file))
            .unwrap()
            .replace("\r\n", "\n");
        let body = source
            .split(start)
            .nth(1)
            .unwrap_or_else(|| panic!("{start} not in {file}"))
            .split("\n    }\n")
            .next()
            .unwrap()
            .to_owned();
        body.split("=> \"")
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap().to_owned())
            .collect()
    };
    let mut names: BTreeSet<String> = BTreeSet::new();
    for name in arms("app.rs", "impl Purpose {").into_iter().chain(arms(
        "app/organize.rs",
        "fn purpose(&self) -> &'static str {",
    )) {
        assert!(names.insert(name.clone()), "{name} twice");
    }
    names
}

/// The variants of `Purpose` as `app.rs` declares them.
fn purpose_variants() -> BTreeSet<String> {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../diroxide-app/src/app.rs"),
    )
    .unwrap();
    let body = source
        .split("enum Purpose {")
        .nth(1)
        .unwrap()
        .split("\n}")
        .next()
        .unwrap();
    body.lines()
        .filter_map(|line| {
            let line = line.strip_prefix("    ")?;
            let first = line.chars().next()?;
            if !first.is_ascii_uppercase() {
                return None;
            }
            let name: String = line
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            Some(name)
        })
        .collect()
}

/// A stored (uncompressed) zip archive of `files`, written by hand, so
/// the test needs no archive crate.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let head = |sig: u32, out: &mut Vec<u8>, central: bool| {
            out.extend(sig.to_le_bytes());
            if central {
                out.extend(20u16.to_le_bytes());
            }
            out.extend(20u16.to_le_bytes());
            out.extend(0u16.to_le_bytes()); // flags
            out.extend(0u16.to_le_bytes()); // stored
            out.extend(0u16.to_le_bytes()); // time
            out.extend(0x21u16.to_le_bytes()); // January 1, 1980
            out.extend(crc.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes()); // extra
        };
        head(0x0403_4b50, &mut out, false);
        out.extend(name.as_bytes());
        out.extend(*data);
        head(0x0201_4b50, &mut central, true);
        central.extend(0u16.to_le_bytes()); // comment
        central.extend(0u16.to_le_bytes()); // disk
        central.extend(0u16.to_le_bytes()); // internal
        central.extend(0u32.to_le_bytes()); // external
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let at = out.len() as u32;
    let size = central.len() as u32;
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

const NOTES_MD: &str = "# Crows\n\nCrows remember faces.\n\n## Jimothy\n\nSee [the wiki](https://example.org/jimothy).\n\n## Portland\n\nKeep it weird.\n";

/// The folder every scenario starts in: `work`, with `Backup` beside
/// its files as the right panel.
fn world(root: &Path) -> PathBuf {
    let work = root.join("work");
    std::fs::create_dir_all(work.join("Backup")).unwrap();
    std::fs::create_dir_all(work.join("Photos")).unwrap();
    std::fs::write(work.join("Photos").join("crow.jpg"), b"caw").unwrap();
    std::fs::write(work.join("report.pdf"), vec![b'r'; 2100]).unwrap();
    std::fs::write(work.join("notes.txt"), b"notes of the crows\n").unwrap();
    std::fs::write(work.join("a.txt"), b"one\ntwo\nthree\n").unwrap();
    std::fs::write(work.join("b.txt"), b"one\nTWO\nthree\n").unwrap();
    std::fs::write(work.join("notes.md"), NOTES_MD).unwrap();
    std::fs::write(
        work.join("course.zip"),
        stored_zip(&[("lesson.txt", b"lesson one\n"), ("quiz.txt", b"quiz\n")]),
    )
    .unwrap();
    std::fs::write(work.join("Backup").join("report.pdf"), b"old").unwrap();
    work
}

/// A window over `work` (left) and its Backup (right), with the
/// settings changed by `change`, its own config and state folders under
/// `root`, and no components fetched.
fn window(root: &Path, change: impl FnOnce(&mut AppOptions)) -> Rig {
    window_finding(root, change, &[])
}

/// The same window, its tools double finding `tools` (by registry id)
/// and starting none; Open with lists two made-up programs.
fn window_finding(
    root: &Path,
    change: impl FnOnce(&mut AppOptions),
    tools: &[&'static str],
) -> Rig {
    let work = world(root);
    let (mut app, runtime) = app_with(&work, &work.join("Backup"), change);
    let tools = Arc::new(WindowTools::finding(tools));
    app.set_tool_runner(tools.clone());
    let paths = Paths::new(root.join("config"), root.join("state"));
    let runtime = runtime
        .with_paths(paths)
        .with_components_offline()
        .with_tool_runner(tools)
        .with_open_with(Arc::new(TwoPrograms));
    Rig::new(app, runtime)
}

/// The same window over the file system crate's doubles: the local disk
/// with `doubles`' refusals, and the trash `bin` in place of the
/// owner's.
fn window_over(root: &Path, doubles: Doubles, bin: Option<OneItemBin>) -> Rig {
    let work = world(root);
    let (app, _) = app_with(&work, &work.join("Backup"), |_| {});
    let mut registry = diroxide_vfs::Registry::new();
    registry.set_local(Arc::new(doubles));
    let paths = Paths::new(root.join("config"), root.join("state"));
    let mut runtime = diroxide_app::Runtime::new(Arc::new(registry))
        .with_paths(paths)
        .with_components_offline();
    if let Some(bin) = bin {
        runtime = runtime.with_trash_bin(Arc::new(bin));
    }
    Rig::new(app, runtime)
}

fn focus(rig: &mut Rig, name: &str) {
    assert!(rig.f.app_mut().focus_name(Side::Left, name), "{name}");
    rig.refresh();
}

fn mark(rig: &mut Rig, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
    rig.f.app_mut().mark_names(Side::Left, &names, true);
    rig.refresh();
}

/// Chooses the open dialog's row with `id`, as Enter on it does.
fn answer(rig: &mut Rig, id: &str) {
    let (dialog, _) = rig.f.app().dialog().unwrap();
    let now = rig.now;
    rig.f.event(
        &mut HostOf(&mut rig.h),
        AppEvent::Answer {
            dialog,
            answer: Answer::Chosen(id.to_owned()),
        },
        now,
    );
    rig.settle();
}

/// Sends the app a question as a worker would.
fn ask(rig: &mut Rig, question: Question) {
    let now = rig.now;
    rig.f.event(
        &mut HostOf(&mut rig.h),
        AppEvent::Asks {
            ask: AskId::new(1),
            question,
        },
        now,
    );
    rig.settle();
}

/// Waits for a dialog to open (one a worker's result opens).
fn wait(rig: &mut Rig) {
    rig.settle_until(|rig| rig.dialog_name().is_some());
}

/// Chooses the open list's row whose name starts with `start`, by keys.
fn choose(rig: &mut Rig, start: &str) {
    // The choices' options, not the lines of a question's message, which
    // is a list to read before them ("Message, 2 lines").
    let mut in_message = false;
    let mut names: Vec<String> = Vec::new();
    for (role, name) in rig.dialog_walk() {
        if role == Role::ListBox {
            in_message = name.starts_with("Message, ");
        } else if role == Role::ListBoxOption && !in_message {
            names.push(name);
        }
    }
    let at = names
        .iter()
        .position(|n| n.starts_with(start))
        .unwrap_or_else(|| panic!("no row {start} in {names:?}"));
    rig.press("home");
    for _ in 0..at {
        rig.press("down");
    }
    rig.press("enter");
}

/// A scenario: the purpose, the start of the dialog's name ("" for any),
/// and what opens it.
type Open = fn(&Path) -> Option<Rig>;

/// Each purpose a test can open, with what opens it. A scenario gives
/// `None` where this system cannot make the situation (said in the
/// report); the test then names it.
fn scenarios() -> Vec<(&'static str, &'static str, Open)> {
    let mut all = first_scenarios();
    all.extend(hosted_scenarios());
    all
}

/// The purposes of alpha 3 and before: the variants of `Purpose`.
fn first_scenarios() -> Vec<(&'static str, &'static str, Open)> {
    vec![
        ("RunPlan", "Copy 1 item to Backup", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.copy");
            Some(rig)
        }),
        ("TypedPlan", "", |root| {
            let mut rig = window(root, |_| {});
            let many = root.join("work").join("many");
            std::fs::create_dir_all(&many).unwrap();
            for n in 0..60 {
                std::fs::write(many.join(format!("f{n:02}.txt")), b"x").unwrap();
            }
            rig.run("panel.reload");
            focus(&mut rig, "many");
            rig.run("file.delete-permanently");
            wait(&mut rig);
            Some(rig)
        }),
        ("Quit", "Quit", |root| {
            let mut rig = window(root, |_| {});
            rig.run("go.path");
            rig.run("app.quit");
            Some(rig)
        }),
        ("RunProgram", "Run ", |root| {
            let mut rig = window(root, |_| {});
            let work = root.join("work");
            let name = if cfg!(windows) { "tool.bat" } else { "tool.sh" };
            std::fs::write(work.join(name), b"echo crow\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(work.join(name), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            rig.run("panel.reload");
            focus(&mut rig, name);
            rig.run("panel.enter");
            Some(rig)
        }),
        ("Conflict", "Name taken: report.pdf", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "report.pdf");
            rig.run("file.copy");
            rig.press("y");
            wait(&mut rig);
            Some(rig)
        }),
        ("ConflictName", "Rename the copy", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "report.pdf");
            rig.run("file.copy");
            rig.press("y");
            wait(&mut rig);
            choose(&mut rig, "Rename this copy");
            Some(rig)
        }),
        ("ItemError", "Could not", |root| item_error(root)),
        ("JunctionInstead", "Make a junction instead", |root| {
            junction(root)
        }),
        ("Trash", "Trash", |root| trash(root)),
        ("JobItems", "Copy finished", |root| job_items(root)),
        ("Secret", "Password for course.zip", |root| {
            let mut rig = window(root, |_| {});
            ask(
                &mut rig,
                Question::Secret {
                    job: None,
                    request: SecretRequest::new(SecretKind::ArchivePassword, "course.zip"),
                },
            );
            Some(rig)
        }),
        ("WrongPassword", "Wrong password", |root| {
            let mut rig = window(root, |_| {});
            ask(
                &mut rig,
                Question::Secret {
                    job: None,
                    request: SecretRequest::new(SecretKind::ArchivePassword, "course.zip")
                        .next_attempt(),
                },
            );
            Some(rig)
        }),
        ("Rename", "Rename notes.txt to", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.rename");
            Some(rig)
        }),
        ("Duplicate", "Duplicate notes.txt", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.duplicate");
            Some(rig)
        }),
        ("MakeFolder", "New folder", |root| {
            let mut rig = window(root, |_| {});
            rig.run("folder.make");
            Some(rig)
        }),
        ("GoPath", "Go to a path", |root| {
            let mut rig = window(root, |_| {});
            rig.run("go.path");
            Some(rig)
        }),
        ("Places", "Go to a place", |root| {
            let mut rig = window(root, |_| {});
            rig.run("go.places");
            Some(rig)
        }),
        ("History", "", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "Photos");
            rig.run("panel.enter");
            rig.run("go.history");
            Some(rig)
        }),
        ("Filter", "Filter", |root| {
            let mut rig = window(root, |_| {});
            rig.run("view.filter");
            Some(rig)
        }),
        ("FilterContents", "Read the contents", |root| {
            let mut rig = window(root, |_| {});
            rig.run("view.filter");
            rig.type_text("content:crows");
            rig.press("enter");
            wait(&mut rig);
            Some(rig)
        }),
        ("Pattern", "Select by pattern", |root| {
            let mut rig = window(root, |_| {});
            rig.run("select.pattern");
            Some(rig)
        }),
        ("SelectCount", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("select.pattern");
            rig.type_text("*.txt");
            rig.press("enter");
            Some(rig)
        }),
        ("SelectKind", "Select by kind", |root| {
            let mut rig = window(root, |_| {});
            rig.run("select.by-kind");
            Some(rig)
        }),
        ("NextRows", "Select the next rows", |root| {
            let mut rig = window(root, |_| {});
            rig.run("select.next-rows");
            Some(rig)
        }),
        ("SearchForm", "Search", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.find");
            Some(rig)
        }),
        ("SearchText", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.find");
            rig.press("enter");
            Some(rig)
        }),
        ("Results", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.largest");
            wait(&mut rig);
            Some(rig)
        }),
        ("Refine", "Refine", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.largest");
            wait(&mut rig);
            rig.press("ctrl+s");
            Some(rig)
        }),
        ("ResultsMenu", "Actions on the results", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.largest");
            wait(&mut rig);
            rig.press("shift+f10");
            Some(rig)
        }),
        ("SaveSearchName", "Save the search", |root| {
            let mut rig = window(root, |_| {});
            saved_search(&mut rig);
            rig.run("search.save");
            Some(rig)
        }),
        ("SavedList", "Saved searches", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.saved");
            wait(&mut rig);
            Some(rig)
        }),
        ("DeleteSaved", "Delete saved search", |root| {
            let mut rig = window(root, |_| {});
            saved_search(&mut rig);
            rig.run("search.save");
            rig.type_text("texts");
            rig.press("enter");
            while rig.dialog_name().is_some() {
                rig.press("escape");
            }
            rig.run("search.saved");
            wait(&mut rig);
            rig.press("delete");
            Some(rig)
        }),
        ("Space", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("search.space");
            wait(&mut rig);
            Some(rig)
        }),
        ("OfferSizeSort", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("dir.size-every-folder");
            wait(&mut rig);
            Some(rig)
        }),
        ("Compare", "", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["a.txt", "b.txt"]);
            rig.run("compare.files");
            wait(&mut rig);
            Some(rig)
        }),
        ("Unified", "Unified view", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["a.txt", "b.txt"]);
            rig.run("compare.files");
            wait(&mut rig);
            rig.press("u");
            Some(rig)
        }),
        ("CompareLarge", "Too large", |root| {
            let mut rig = window(root, |options| options.settings.compare.text_limit_mib = 0);
            mark(&mut rig, &["a.txt", "b.txt"]);
            rig.run("compare.files");
            wait(&mut rig);
            Some(rig)
        }),
        ("CompareMenu", "Actions on the compare", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["a.txt", "b.txt"]);
            rig.run("compare.files");
            wait(&mut rig);
            rig.press("shift+f10");
            Some(rig)
        }),
        ("PickDestination", "Copy 1 item to", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.copy-to");
            Some(rig)
        }),
        ("RunCommand", "Run in work", |root| {
            let mut rig = window(root, |_| {});
            rig.run("file.run-command");
            Some(rig)
        }),
        ("Palette", "Command palette", |root| {
            let mut rig = window(root, |_| {});
            rig.run("app.palette");
            Some(rig)
        }),
        ("MenuBar", "Menu bar", |root| {
            let mut rig = window(root, |_| {});
            rig.run("app.menu");
            Some(rig)
        }),
        ("Menu", "File", |root| {
            let mut rig = window(root, |_| {});
            rig.run("menu.file");
            Some(rig)
        }),
        ("ContextMenu", "Actions for notes.txt", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.context-menu");
            Some(rig)
        }),
        ("SortList", "Sort by", |root| {
            let mut rig = window(root, |_| {});
            rig.run("view.sort-list");
            Some(rig)
        }),
        ("Limit", "", |root| {
            let mut rig = window(root, |_| {});
            ask(
                &mut rig,
                Question::OnlineOnly {
                    name: "a.txt".to_owned(),
                    size: Some(1),
                },
            );
            Some(rig)
        }),
        ("ArchiveQuestion", "Expands to", |root| {
            let mut rig = window(root, |options| options.settings.archives.ask_total_gib = 0);
            focus(&mut rig, "course.zip");
            rig.run("archive.unpack");
            answer(&mut rig, "go");
            wait(&mut rig);
            Some(rig)
        }),
        ("RunArchive", "Unpack", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "course.zip");
            rig.run("archive.unpack");
            answer(&mut rig, "go");
            wait(&mut rig);
            Some(rig)
        }),
        ("PlanQuestion", "", |root| {
            let mut rig = window(root, |_| {});
            let work = root.join("work");
            std::fs::write(work.join("x.001"), vec![b'1'; 40_000]).unwrap();
            std::fs::write(work.join("x.002"), vec![b'2'; 10_000]).unwrap();
            std::fs::write(work.join("x.003"), vec![b'3'; 30_000]).unwrap();
            rig.run("panel.reload");
            focus(&mut rig, "x.001");
            rig.run("file.join");
            wait(&mut rig);
            Some(rig)
        }),
        ("PackForm", "Pack", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["notes.txt"]);
            rig.run("archive.pack");
            Some(rig)
        }),
        ("PackName", "Archive name", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["notes.txt"]);
            rig.run("archive.pack");
            answer(&mut rig, "name");
            Some(rig)
        }),
        ("PackChoice", "Format", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["notes.txt"]);
            rig.run("archive.pack");
            answer(&mut rig, "format");
            Some(rig)
        }),
        ("NewPassword", "New password", |root| {
            let mut rig = window(root, |_| {});
            mark(&mut rig, &["notes.txt"]);
            rig.run("archive.pack");
            answer(&mut rig, "password");
            Some(rig)
        }),
        ("UnpackForm", "Unpack", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "course.zip");
            rig.run("archive.unpack");
            Some(rig)
        }),
        ("SplitSize", "Split into", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "report.pdf");
            rig.run("file.split");
            Some(rig)
        }),
        ("SplitTyped", "Number of parts", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "report.pdf");
            rig.run("file.split");
            answer(&mut rig, "equal");
            Some(rig)
        }),
        ("TrashParts", "Move the 3 parts to the trash", |root| {
            trash_parts(root)
        }),
        ("LinkForm", "Make link", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.link");
            Some(rig)
        }),
        ("Relink", "Point ", |root| relink(root)),
        ("AccessForm", "Permissions", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.permissions");
            Some(rig)
        }),
        ("Details", "Details of notes.txt", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.details");
            wait(&mut rig);
            Some(rig)
        }),
        ("ViewerJump", "Go to line", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.view");
            rig.run("viewer.jump");
            Some(rig)
        }),
        ("ViewerSearch", "Search for", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.view");
            rig.run("viewer.search");
            Some(rig)
        }),
        ("ViewerOutline", "Outline", |root| viewer_outline(root)),
        ("PreviewFind", "Find in the preview", |root| {
            let mut rig = in_preview(root)?;
            rig.run("preview.find");
            Some(rig)
        }),
        ("PreviewLinks", "Links", |root| {
            let mut rig = in_preview(root)?;
            rig.run("preview.links");
            Some(rig)
        }),
        ("PreviewLink", "Open https://example.org/jimothy", |root| {
            let mut rig = in_preview(root)?;
            rig.run("preview.links");
            rig.press("enter");
            rig.run("preview.open-link");
            Some(rig)
        }),
        ("Settings", "Settings", |root| {
            let mut rig = window(root, |_| {});
            rig.run("app.settings");
            Some(rig)
        }),
        ("SettingValue", "Layout", |root| {
            let mut rig = window(root, |_| {});
            rig.run("app.settings");
            answer(&mut rig, "layout");
            Some(rig)
        }),
        ("SettingText", "Editor", |root| {
            let mut rig = window(root, |_| {});
            rig.run("app.settings");
            answer(&mut rig, "editor");
            Some(rig)
        }),
        ("Read", "Keyboard", |root| {
            let mut rig = window(root, |_| {});
            rig.run("app.help");
            Some(rig)
        }),
        ("Organize", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("label.list");
            Some(rig)
        }),
        ("Keys", "Keys", |root| {
            let mut rig = window(root, |_| {});
            rig.run("keys.list");
            Some(rig)
        }),
        ("Plugin", "Manage plugins", |root| {
            let mut rig = window(root, |_| {});
            rig.run("plugins.list");
            wait(&mut rig);
            Some(rig)
        }),
        ("Component", "", |root| {
            let mut rig = window(root, |_| {});
            rig.run("components.manage");
            wait(&mut rig);
            Some(rig)
        }),
        ("Jobs", "Jobs", |root| {
            let mut rig = window(root, |_| {});
            rig.run("jobs.pause-queue");
            focus(&mut rig, "notes.txt");
            rig.run("file.copy");
            rig.press("y");
            rig.run("jobs.list");
            Some(rig)
        }),
        ("Volumes", "Volumes", |root| {
            let mut rig = window(root, |_| {});
            rig.run("go.volumes");
            wait(&mut rig);
            Some(rig)
        }),
    ]
}

/// The lists and questions of alpha 4 (lanes TA, AW, MA, AE and AP),
/// each its own purpose since the fold lane (R30's commit 114).
fn hosted_scenarios() -> Vec<(&'static str, &'static str, Open)> {
    vec![
        // File types (lane TA).
        ("TypeActions", "Actions for notes.txt", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.actions");
            Some(rig)
        }),
        ("OpenWith", "Open report.pdf with", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "report.pdf");
            rig.run("file.open-with");
            wait(&mut rig);
            Some(rig)
        }),
        ("Encoding", "", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.view");
            rig.run("viewer.encoding");
            Some(rig)
        }),
        // Convert and the conversion reports (lane AW).
        ("ConvertRecipes", "Convert notes.md", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.md");
            rig.run("file.convert");
            wait(&mut rig);
            Some(rig)
        }),
        ("ConvertPlan", "Convert 1 file to ", |root| {
            convert_plan(root)
        }),
        ("ConvertReports", "Conversion reports", |root| {
            let mut rig = convert_plan(root)?;
            rig.press("y");
            rig.settle_until(|rig| rig.f.app().jobs().front().is_none());
            rig.run("file.convert-reports");
            wait(&mut rig);
            Some(rig)
        }),
        // The Watched folders list (lane AW).
        ("WatchedFolders", "Watched folders: 1 task", |root| {
            watched(root, None)
        }),
        ("WatchedActions", "Notes actions", |root| {
            let mut rig = watched(root, None)?;
            rig.press("enter");
            Some(rig)
        }),
        ("WatchedApprove", "Turn on Notes?", |root| {
            watched(root, Some("Turn on"))
        }),
        ("WatchedCatchUp", "Process the 2 files in Notes?", |root| {
            let mut rig = watched(root, Some("Turn on"))?;
            rig.press("y");
            wait(&mut rig);
            Some(rig)
        }),
        ("WatchedPutBack", "Put back 1 file from failed?", |root| {
            watched(root, Some("Put failed back"))
        }),
        // Notes, fields and the columns' question (lane MA).
        ("Note", "", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("label.note");
            Some(rig)
        }),
        ("Field", "", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("label.field");
            Some(rig)
        }),
        ("ReadColumns", "Read ", |root| {
            let mut rig = window(root, |options| {
                options.settings.columns = vec!["text.lines".to_owned()];
            });
            let work = root.join("work");
            for n in 0..230 {
                std::fs::write(work.join(format!("n{n:03}.txt")), b"a\n").unwrap();
            }
            rig.run("panel.reload");
            rig.run("view.sort-list");
            rig.press("end");
            rig.press("enter");
            Some(rig)
        }),
        // The metadata commands (lane MA).
        ("Checksums", "SHA-256 of report.pdf", |root| checksums(root)),
        ("ChecksumCompare", "Compare with the SHA-256", |root| {
            let mut rig = checksums(root)?;
            choose(&mut rig, "Compare");
            Some(rig)
        }),
        ("VerifyLists", "Checksum lists beside report.pdf", |root| {
            let mut rig = window(root, |_| {});
            let work = root.join("work");
            std::fs::write(work.join("report.pdf.sha256"), format!("{REPORT}\n")).unwrap();
            std::fs::write(work.join("SHA256SUMS"), format!("{REPORT}  report.pdf\n")).unwrap();
            rig.run("panel.reload");
            focus(&mut rig, "report.pdf");
            rig.run("file.verify-checksum");
            wait(&mut rig);
            Some(rig)
        }),
        ("VerifyReport", "1 differs, 1 missing, 1 match", |root| {
            verify_report(root)
        }),
        ("StripQuestion", "Strip ", |root| {
            let mut rig = window(root, |_| {});
            std::fs::write(root.join("work").join("beach.jpg"), beach_jpeg()).unwrap();
            rig.run("panel.reload");
            focus(&mut rig, "beach.jpg");
            rig.run("file.strip-metadata");
            Some(rig)
        }),
        (
            "MarkQuestion",
            "Remove the internet mark from setup.exe",
            |root| marked(root),
        ),
        ("MetaLines", "", |root| {
            let mut rig = window(root, |_| {});
            std::fs::write(root.join("work").join("beach.jpg"), beach_jpeg()).unwrap();
            rig.run("panel.reload");
            focus(&mut rig, "beach.jpg");
            rig.run("file.check-before-sharing");
            wait(&mut rig);
            Some(rig)
        }),
        // The Encryption submenu (lane AE), gpg found by the tools double.
        ("GpgRecipients", "Encrypt for gpg keys", |root| {
            let mut rig = window_finding(root, |_| {}, &["gpg"]);
            focus(&mut rig, "notes.txt");
            rig.run("file.gpg-encrypt");
            Some(rig)
        }),
        ("GpgSigner", "Sign with gpg key", |root| {
            let mut rig = window_finding(root, |_| {}, &["gpg"]);
            focus(&mut rig, "notes.txt");
            rig.run("file.sign");
            Some(rig)
        }),
        ("GpgPlan", "Encrypt 1 file for 1 recipient", |root| {
            gpg_plan(root)
        }),
        ("TrashOriginals", "Move the original to the trash", |root| {
            let mut rig = gpg_plan(root)?;
            rig.press("y");
            rig.settle_until(|rig| rig.f.app().dialog_purpose() == Some("TrashOriginals"));
            Some(rig)
        }),
        ("MinisignKey", "minisign public key", |root| {
            let mut rig = window(root, |_| {});
            std::fs::write(
                root.join("work").join("notes.txt.minisig"),
                "untrusted comment: x\n",
            )
            .unwrap();
            rig.run("panel.reload");
            focus(&mut rig, "notes.txt");
            rig.run("file.verify-signature");
            wait(&mut rig);
            Some(rig)
        }),
        // Change time and Change owner (lane AP).
        ("TimeForm", "Change time", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.change-time");
            wait(&mut rig);
            Some(rig)
        }),
        ("TypedTime", "Set to, as ", |root| {
            let mut rig = window(root, |_| {});
            focus(&mut rig, "notes.txt");
            rig.run("file.change-time");
            wait(&mut rig);
            choose(&mut rig, "Type a date and time");
            Some(rig)
        }),
        ("OwnerForm", "Change owner of notes.txt", |root| {
            owner(root, None)
        }),
        ("OwnerName", "New owner of notes.txt", |root| {
            owner(root, Some("Owner"))
        }),
        ("GroupName", "New group of notes.txt", |root| {
            owner(root, Some("Group"))
        }),
        // The photo helpers (lane AP).
        ("Review", "Review: 1 of 2", |root| review(root)),
        ("ReviewMove", "", |root| {
            let mut rig = review(root)?;
            rig.press("m");
            Some(rig)
        }),
        ("MonthFolders", "Move 2 pictures into ", |root| {
            let mut rig = window(root, |_| {});
            pictures(root);
            focus(&mut rig, "Photos");
            rig.run("panel.enter");
            rig.run("photo.month-folders");
            wait(&mut rig);
            Some(rig)
        }),
        // Editing inside a zip (lane AW): its question before the rewrite.
        (
            "RewriteArchive",
            "Remove lesson.txt from course.zip",
            |root| {
                let mut rig = window(root, |_| {});
                focus(&mut rig, "course.zip");
                rig.run("panel.enter");
                wait_listed(&mut rig, "lesson.txt");
                focus(&mut rig, "lesson.txt");
                rig.press("delete");
                wait(&mut rig);
                Some(rig)
            },
        ),
    ]
}

/// The SHA-256 of `report.pdf` as `world` writes it: 2,100 bytes of "r".
const REPORT: &str = "9a12407951304313f4f909d1efe5c01f6d54393b311fb3a148f53c26625535af";

/// Waits for the active panel to list `name`.
fn wait_listed(rig: &mut Rig, name: &str) {
    rig.settle_until(|rig| {
        rig.f
            .app()
            .active_panel()
            .list()
            .rows()
            .iter()
            .any(|r| r.id == name)
    });
}

/// Convert on notes.md, then its plain-text recipe: the plan's question.
fn convert_plan(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    focus(&mut rig, "notes.md");
    rig.run("file.convert");
    wait(&mut rig);
    choose(&mut rig, "Markdown to plain text");
    rig.settle_until(|rig| rig.f.app().dialog_purpose() == Some("ConvertPlan"));
    Some(rig)
}

/// The task `notes` in the config folder, two Markdown files waiting in
/// its `in` and one in `failed`; the Watched folders list open, and
/// `action` chosen from the task's actions when one is given. The task
/// stays off: no watcher runs.
fn watched(root: &Path, action: Option<&str>) -> Option<Rig> {
    let home = root.join("notes");
    for folder in ["in", "out", "processed", "failed"] {
        std::fs::create_dir_all(home.join(folder)).unwrap();
    }
    std::fs::write(home.join("in").join("a.md"), "# A\n\nOne.\n").unwrap();
    std::fs::write(home.join("in").join("b.md"), "# B\n\nTwo.\n").unwrap();
    std::fs::write(home.join("failed").join("c.md"), "# C\n").unwrap();
    let tasks = root.join("config").join(Paths::TASKS);
    std::fs::create_dir_all(&tasks).unwrap();
    let task = format!(
        "name = \"notes\"\nkind = \"convert\"\nrecipe = \"markdown-to-org\"\n\n[trigger]\nkind = \"folder\"\nhome = '{}'\nmatch = [\"*.md\"]\nsettle = \"2s\"\n",
        home.display()
    );
    std::fs::write(tasks.join("notes.toml"), task).unwrap();
    let mut rig = window(root, |_| {});
    rig.run("app.hot-folders");
    wait(&mut rig);
    if let Some(action) = action {
        rig.press("enter");
        choose(&mut rig, action);
        wait(&mut rig);
    }
    Some(rig)
}

/// The Checksum list of report.pdf, its value computed.
fn checksums(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    focus(&mut rig, "report.pdf");
    rig.run("file.checksum");
    rig.settle_until(|rig| rig.f.app().dialog_purpose() == Some("Checksums"));
    Some(rig)
}

/// Verify checksum on a list naming three files: one matches, one
/// differs, one is missing.
fn verify_report(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    let work = root.join("work");
    std::fs::write(
        work.join("SHA256SUMS"),
        format!(
            "{REPORT}  report.pdf\n{}  notes.txt\n{REPORT}  gone.txt\n",
            "0".repeat(64)
        ),
    )
    .unwrap();
    rig.run("panel.reload");
    focus(&mut rig, "SHA256SUMS");
    rig.run("file.verify-checksum");
    rig.settle_until(|rig| rig.f.app().dialog_purpose() == Some("VerifyReport"));
    Some(rig)
}

/// A small JPEG made in memory with a comment and XMP data, the app
/// test's beach picture.
fn beach_jpeg() -> Vec<u8> {
    let segments: [(u8, &[u8]); 2] = [
        (0xE1, b"http://ns.adobe.com/xap/1.0/\0<x/>"),
        (0xFE, b"from the beach"),
    ];
    let mut out = vec![0xFF, 0xD8];
    for (marker, body) in segments {
        out.extend_from_slice(&[0xFF, marker]);
        out.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(body);
    }
    out.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
    out.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// setup.exe with an internet mark, written through the file system
/// crate: a Zone.Identifier stream on Windows, the quarantine attribute
/// on macOS, the origin attribute elsewhere. `None` where the disk keeps
/// no such thing.
fn marked(root: &Path) -> Option<Rig> {
    use diroxide_vfs::Provider;
    let work = world(root);
    let file = work.join("setup.exe");
    std::fs::write(&file, b"not really a program").unwrap();
    let place = diroxide_core::Place::parse(file.to_str().unwrap()).unwrap();
    let provider = diroxide_vfs::LocalProvider::new();
    if !provider.capabilities(&place).extended_attributes {
        return None;
    }
    // The names diroxide-meta's origin module reads (ZONE_STREAM,
    // QUARANTINE, XDG_ORIGIN), written out: the GUI does not depend on it.
    let (name, bytes): (&str, &[u8]) = if cfg!(windows) {
        (
            "Zone.Identifier",
            b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.org/setup.exe\r\n",
        )
    } else if cfg!(target_vendor = "apple") {
        ("com.apple.quarantine", b"0083;6542a1b2;Safari;0B7B4E1D")
    } else {
        ("user.xdg.origin.url", b"https://example.org/setup.exe")
    };
    let extended = provider.as_extended()?;
    extended
        .write_extended(&place, name, bytes, &diroxide_vfs::OpContext::new())
        .ok()?;
    let mut rig = window(root, |_| {});
    focus(&mut rig, "setup.exe");
    rig.run("file.remove-internet-mark");
    wait(&mut rig);
    Some(rig)
}

/// Encrypt with gpg on notes.txt for the key typed: the plan's question.
fn gpg_plan(root: &Path) -> Option<Rig> {
    let mut rig = window_finding(root, |_| {}, &["gpg"]);
    focus(&mut rig, "notes.txt");
    rig.run("file.gpg-encrypt");
    rig.type_text("4E7B");
    rig.press("enter");
    rig.settle_until(|rig| rig.f.app().dialog_purpose() == Some("GpgPlan"));
    Some(rig)
}

/// Change owner of notes.txt, then `row` chosen; Unix only (Windows
/// hands the file to its Security dialog, which no test opens).
fn owner(root: &Path, row: Option<&str>) -> Option<Rig> {
    if !cfg!(unix) {
        return None;
    }
    let mut rig = window(root, |_| {});
    focus(&mut rig, "notes.txt");
    rig.run("file.change-owner");
    wait(&mut rig);
    if let Some(row) = row {
        choose(&mut rig, row);
    }
    Some(rig)
}

/// Two pictures in Photos, made in memory (one-pixel PNGs).
fn pictures(root: &Path) {
    let photos = root.join("work").join("Photos");
    std::fs::remove_file(photos.join("crow.jpg")).unwrap();
    std::fs::write(photos.join("crow.png"), one_pixel_png()).unwrap();
    std::fs::write(photos.join("raccoon.png"), one_pixel_png()).unwrap();
}

/// Review pictures from the first of two in Photos.
fn review(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    pictures(root);
    focus(&mut rig, "Photos");
    rig.run("panel.enter");
    wait_listed(&mut rig, "crow.png");
    focus(&mut rig, "crow.png");
    rig.run("photo.review");
    wait(&mut rig);
    Some(rig)
}

/// A PNG of one gray pixel, written by hand.
fn one_pixel_png() -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn adler32(bytes: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for byte in bytes {
            a = (a + u32::from(*byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        (b << 16) | a
    }
    let chunk = |kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend(1u32.to_be_bytes());
    header.extend(1u32.to_be_bytes());
    header.extend([8, 0, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut out);
    // One row: the filter byte, then one gray byte, stored uncompressed.
    let raw = [0u8, 0x80];
    let mut data = vec![0x78, 0x01, 1, raw.len() as u8, 0, !(raw.len() as u8), 0xFF];
    data.extend(raw);
    data.extend(adler32(&raw).to_be_bytes());
    chunk(b"IDAT", &data, &mut out);
    chunk(b"IEND", &[], &mut out);
    out
}

/// A search for text files run from the search dialog, its results up.
fn saved_search(rig: &mut Rig) {
    rig.run("search.find");
    rig.press("enter");
    rig.type_text("*.txt");
    rig.press("enter");
    answer(rig, "go");
    wait(rig);
}

/// An item that cannot be read during a copy, on every system: the
/// provider double refuses notes.txt's reading, as a file another program
/// holds (root reads any file on Unix, so permissions cannot make it).
fn item_error(root: &Path) -> Option<Rig> {
    let file = root.join("work").join("notes.txt");
    let doubles = Doubles {
        unreadable: Some(file),
        ..Doubles::default()
    };
    let mut rig = window_over(root, doubles, None);
    focus(&mut rig, "notes.txt");
    rig.run("file.copy");
    rig.press("y");
    wait(&mut rig);
    Some(rig)
}

/// A folder link the system refuses for the privilege (Windows without
/// Developer Mode, made by the provider double on every system): a
/// junction is offered instead.
fn junction(root: &Path) -> Option<Rig> {
    let doubles = Doubles {
        refuse_links: true,
        ..Doubles::default()
    };
    let mut rig = window_over(root, doubles, None);
    focus(&mut rig, "Photos");
    rig.run("file.link");
    answer(&mut rig, "go");
    wait(&mut rig);
    rig.press("y");
    rig.settle_until(|rig| {
        rig.dialog_name()
            .is_some_and(|name| name.starts_with("Make a junction instead"))
    });
    Some(rig)
}

/// The trash browser over a trash of one item, never the owner's.
fn trash(root: &Path) -> Option<Rig> {
    let gone =
        diroxide_core::Place::parse(root.join("work").join("old.txt").to_str().unwrap()).unwrap();
    let mut rig = window_over(root, Doubles::default(), Some(OneItemBin::holding(gone)));
    rig.run("go.trash");
    rig.settle_until(|rig| rig.f.app().dialog_purpose() == Some("Trash"));
    Some(rig)
}

/// A finished job's Show what failed (lane A5): a copy, then the job
/// history, the job's actions, and the first of them.
fn job_items(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    focus(&mut rig, "notes.txt");
    rig.run("file.copy");
    rig.press("y");
    rig.settle_until(|rig| rig.f.app().jobs().front().is_none());
    rig.run("jobs.history");
    rig.press("enter");
    choose(&mut rig, "Show what failed");
    Some(rig)
}

/// A file split into three parts in the right panel, joined back into an
/// empty folder on the left: the offer to trash the parts.
fn trash_parts(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    let parts = root.join("work").join("Backup");
    std::fs::remove_file(parts.join("report.pdf")).unwrap();
    rig.run("panel.reload");
    focus(&mut rig, "report.pdf");
    rig.run("file.split");
    answer(&mut rig, "equal");
    rig.type_text("3");
    rig.press("enter");
    wait(&mut rig);
    rig.press("y");
    rig.settle_until(|_| parts.join("report.pdf.003").exists());
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    // The left panel goes to the empty folder, as Go to a path takes it.
    let place = diroxide_core::Place::parse(out.to_str().unwrap()).unwrap();
    let go =
        diroxide_core::Command::new(diroxide_core::CommandId::new("go.path".to_owned()).unwrap())
            .with_arg("place", diroxide_core::ArgValue::Place(place));
    let now = rig.now;
    rig.f.command(&mut HostOf(&mut rig.h), go, now);
    rig.settle();
    rig.run("panel.switch");
    rig.run("panel.reload");
    assert!(rig.f.app_mut().focus_name(Side::Right, "report.pdf.001"));
    rig.refresh();
    rig.run("file.join");
    wait(&mut rig);
    rig.press("y");
    rig.settle_until(|rig| {
        rig.dialog_name()
            .is_some_and(|name| name.starts_with("Move the 3 parts"))
    });
    Some(rig)
}

/// A link to a folder, then Change link target on it. On Windows a
/// symbolic link needs Developer Mode or an administrator; without it
/// the scenario cannot be made here.
fn relink(root: &Path) -> Option<Rig> {
    let work = world(root);
    let link = work.join("to-photos");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(work.join("Photos"), &link).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(work.join("Photos"), &link).is_ok();
    if !made {
        return None;
    }
    let mut rig = window(root, |_| {});
    focus(&mut rig, "to-photos");
    rig.run("file.relink");
    Some(rig)
}

/// The outline of a Markdown file in the viewer, with the syntax set in
/// the build.
fn viewer_outline(root: &Path) -> Option<Rig> {
    if !cfg!(feature = "syntax-set") {
        return None;
    }
    let mut rig = window(root, |_| {});
    focus(&mut rig, "notes.md");
    rig.run("file.view");
    rig.settle_until(|rig| rig.f.app().viewer().is_some_and(|v| !v.is_highlighting()));
    rig.run("viewer.outline");
    Some(rig)
}

/// The quick view on notes.md, the focus in its region.
fn in_preview(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |options| options.settings.quick_view.wait_ms = 0);
    focus(&mut rig, "notes.md");
    rig.run("view.quick-view");
    rig.settle_until(|rig| {
        rig.f
            .app()
            .quick_view()
            .is_some_and(|q| q.region().len() > 3)
    });
    rig.press("tab");
    rig.f.app().in_preview().then_some(rig)
}

/// The open dialog's name and the focused control's role and name,
/// checked: a named dialog, the focus inside it on a named control.
fn checked(rig: &Rig) -> Result<(String, Role, String), String> {
    let state = rig.h.access_tree().state();
    let mut stack = vec![state.root()];
    let mut dialog = None;
    while let Some(node) = stack.pop() {
        if node.role() == Role::Dialog && !node.is_hidden() {
            dialog = Some(node);
            break;
        }
        stack.extend(node.children());
    }
    let dialog = dialog.ok_or("no dialog is open")?;
    let name = name_of(&dialog);
    if name.trim().is_empty() {
        return Err("the dialog has no name".to_owned());
    }
    let focused = state.focus_in_tree();
    let node = focused.active_descendant().unwrap_or(focused);
    let mut inside = false;
    let mut at = node.parent();
    while let Some(parent) = at {
        if parent.id() == dialog.id() {
            inside = true;
            break;
        }
        at = parent.parent();
    }
    if !inside {
        return Err(format!(
            "the focus is outside the dialog, on {:?} {:?}",
            node.role(),
            name_of(&node)
        ));
    }
    let control = name_of(&node);
    if control.trim().is_empty() {
        return Err(format!("the focused {:?} has no name", node.role()));
    }
    Ok((name, node.role(), control))
}

#[test]
fn every_purpose_is_opened_or_said_why_not() {
    let variants = purpose_variants();
    assert_eq!(variants.len(), 77, "{variants:?}");
    let names = purpose_names();
    assert_eq!(names.len(), PURPOSES, "{names:?}");
    // Every variant has its name among them (Organize's from the hosted
    // lists' function, the others from `Purpose::name`).
    let missing: Vec<&String> = variants.difference(&names).collect();
    assert!(missing.is_empty(), "variants with no name: {missing:?}");
    let scenarios = scenarios();
    let mut listed = BTreeSet::new();
    for (purpose, _, _) in &scenarios {
        assert!(listed.insert((*purpose).to_owned()), "{purpose} twice");
    }
    for (purpose, _) in UNREACHABLE {
        assert!(listed.insert((*purpose).to_owned()), "{purpose} twice");
    }
    assert_eq!(listed, names, "every purpose is opened or said why not");
    for absent in MAY_BE_ABSENT {
        assert!(names.contains(*absent), "{absent} is no purpose");
    }
}

/// The parts the purposes are opened in, each its own test.
const PARTS: usize = 6;

#[test]
fn every_dialog_purpose_opens_with_a_name_and_a_focus() {
    open_part(0);
}

#[test]
fn every_dialog_purpose_opens_part_two() {
    open_part(1);
}

#[test]
fn every_dialog_purpose_opens_part_three() {
    open_part(2);
}

#[test]
fn every_dialog_purpose_opens_part_four() {
    open_part(3);
}

#[test]
fn every_dialog_purpose_opens_part_five() {
    open_part(4);
}

#[test]
fn every_dialog_purpose_opens_part_six() {
    open_part(5);
}

/// Opens the purposes of part `part` (every sixth, starting at it), each
/// in a window of its own: a named dialog, the focus on a named control
/// inside it, the app's own purpose the one meant. A scenario may say
/// "not made on this system" only for a purpose in [`MAY_BE_ABSENT`].
fn open_part(part: usize) {
    let scenarios: Vec<_> = scenarios()
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % PARTS == part)
        .map(|(_, s)| s)
        .collect();
    let mut failures = Vec::new();
    let mut opened = Vec::new();
    let mut not_here = Vec::new();
    for (purpose, title, open) in scenarios {
        let root = tempfile::Builder::new()
            .prefix("gui-it-purpose-")
            .tempdir()
            .unwrap();
        let Some(rig) = open(root.path()) else {
            not_here.push(purpose);
            continue;
        };
        let shown = rig.f.app().dialog_purpose();
        match checked(&rig) {
            Ok(_) if shown != Some(purpose) => {
                failures.push(format!("{purpose}: the app's purpose is {shown:?}"))
            }
            Ok((name, role, control)) if name.starts_with(title) => {
                opened.push(format!("{purpose}: {name} | {role:?} {control}"));
            }
            Ok((name, _, _)) => failures.push(format!("{purpose}: named {name:?}, not {title:?}")),
            Err(why) => failures.push(format!(
                "{purpose}: {why}; last said {:?}",
                rig.said()
                    .iter()
                    .rev()
                    .take(3)
                    .map(|m| m.text.clone())
                    .collect::<Vec<_>>()
            )),
        }
    }
    for line in &opened {
        eprintln!("opened {line}");
    }
    for purpose in &not_here {
        eprintln!("not made on this system: {purpose}");
        if !MAY_BE_ABSENT.contains(purpose) {
            failures.push(format!("{purpose}: not made, and it always can be"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
