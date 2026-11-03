//! Every dialog purpose (R27's commit 25): the app's dialogs are 77
//! purposes (`Purpose` in `crates/diroxide-app/src/app.rs`) drawn as
//! seven shapes, and this test opens each one the app can open from a
//! test through the window, checking that the dialog has a name and that
//! the keyboard focus is on a named control inside it. The purposes it
//! cannot open are listed apart, each with the reason; the two lists
//! together are held against the enum's variants as the source declares
//! them, so a purpose added later fails here until it is opened or said.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use diroxide_app::config::Paths;
use diroxide_app::{Answer, AppEvent, AppOptions, AskId, Question};
use diroxide_core::{SecretKind, SecretRequest, Side};
use masonry::accesskit::Role;

use crate::rig::{HostOf, Rig, app_with, name_of};

/// The purposes no test can open, each with the reason, in words.
const UNREACHABLE: &[(&str, &str)] = &[
    (
        "JobItems",
        "it opens from a finished job's row in the Jobs list, a scenario the next GUI lane adds (lane A5 removed JobHistory, whose dialog the Jobs list had replaced)",
    ),
    (
        "JunctionInstead",
        "it needs a link the system refuses for want of a privilege, which needs a provider double from diroxide-vfs, not a dependency of the GUI crate",
    ),
    (
        "Trash",
        "the trash browser would list the owner's real trash; a test trash needs diroxide-vfs, not a dependency of the GUI crate",
    ),
];

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
    let work = world(root);
    let (app, runtime) = app_with(&work, &work.join("Backup"), change);
    let paths = Paths::new(root.join("config"), root.join("state"));
    let runtime = runtime.with_paths(paths).with_components_offline();
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
    let names: Vec<String> = rig
        .dialog_walk()
        .into_iter()
        .filter(|(role, _)| *role == Role::ListBoxOption)
        .map(|(_, name)| name)
        .collect();
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

/// A search for text files run from the search dialog, its results up.
fn saved_search(rig: &mut Rig) {
    rig.run("search.find");
    rig.press("enter");
    rig.type_text("*.txt");
    rig.press("enter");
    answer(rig, "go");
    wait(rig);
}

/// An item that cannot be read during a copy: on Windows a file held
/// open with no sharing; on Unix a file with no read permission (not
/// possible as root, which reads it anyway).
fn item_error(root: &Path) -> Option<Rig> {
    let mut rig = window(root, |_| {});
    let file = root.join("work").join("notes.txt");
    #[cfg(windows)]
    let _held = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&file).is_ok() {
            return None;
        }
    }
    focus(&mut rig, "notes.txt");
    rig.run("file.copy");
    rig.press("y");
    wait(&mut rig);
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
fn every_dialog_purpose_opens_with_a_name_and_a_focus() {
    let variants = purpose_variants();
    assert_eq!(variants.len(), 77, "{variants:?}");
    let scenarios = scenarios();
    let mut listed: BTreeSet<String> = scenarios.iter().map(|(p, _, _)| (*p).to_owned()).collect();
    for (purpose, _) in UNREACHABLE {
        assert!(listed.insert((*purpose).to_owned()), "{purpose} twice");
    }
    assert_eq!(listed, variants, "every purpose is opened or said why not");
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
        match checked(&rig) {
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
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(opened.len() >= 70, "{} opened", opened.len());
}
