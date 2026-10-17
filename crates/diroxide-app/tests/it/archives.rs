//! Archives in the panels (plan commits 59 to 62): entering and leaving
//! them with the titles and the opening line, the read-only words, copying
//! out through the unpack runner, the pack, unpack and test dialogs, the
//! password flow and the limit questions.

use std::fs;
use std::path::{Path, PathBuf};

use diroxide_app::commands::ids;
use diroxide_archive::{ArchiveFormat, PackRequest};
use diroxide_core::{Secret, Side};
use diroxide_vfs::{OpContext, Registry};

use crate::support::{Session, Setup, place, temp};

/// Packs `items` of `folder` into `archive` there, with diroxide's own
/// runner, and a password when given.
pub fn pack_into(folder: &Path, archive: &str, items: &[&str], password: Option<&str>) {
    let destination = place(&folder.join(archive));
    let format = ArchiveFormat::from_name(archive).unwrap();
    let sources = items.iter().map(|i| place(&folder.join(i))).collect();
    let mut request = PackRequest::new(sources, destination, format);
    request.password = password.map(|p| Secret::new(p.to_owned()));
    diroxide_archive::pack(&request, &Registry::with_local(), &OpContext::new()).unwrap();
}

/// A small demo set, as `cargo xtask demo` makes the owner's: `course.zip`
/// (week1 and week2 with three text files each, notes.txt and README.md:
/// ten entries), and an empty `out` folder.
pub fn demo(root: &Path) -> PathBuf {
    let demo = root.join("demo");
    let build = root.join("build");
    for week in ["week1", "week2"] {
        fs::create_dir_all(build.join(week)).unwrap();
    }
    for (week, names) in [("week1", ["a", "b", "c"]), ("week2", ["d", "e", "f"])] {
        for name in names {
            fs::write(
                build.join(week).join(format!("{name}.txt")),
                format!("{name} line one\n{name} line two\n"),
            )
            .unwrap();
        }
    }
    fs::write(build.join("notes.txt"), "notes\nmore notes\n").unwrap();
    fs::write(build.join("README.md"), "# Course\n\nRead me.\n").unwrap();
    fs::create_dir_all(demo.join("out")).unwrap();
    pack_into(
        &build,
        "course.zip",
        &["week1", "week2", "notes.txt", "README.md"],
        None,
    );
    fs::rename(build.join("course.zip"), demo.join("course.zip")).unwrap();
    demo
}

fn session(demo: &Path) -> Session {
    Session::open(
        place(demo),
        place(&demo.join("out")),
        Registry::with_local(),
        Setup::default(),
    )
}

#[test]
fn entering_and_leaving_an_archive_says_where_and_what_opened() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = session(&demo);
    s.focus("course.zip");
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Opened course.zip: 10 items, ")),
        "{said:?}"
    );
    let title = s.app.title_line();
    assert!(title.starts_with("course.zip, archive, in "), "{title}");
    assert!(title.ends_with(", left panel"), "{title}");
    // Rows say the name first, then the kind and the size.
    s.focus("notes.txt");
    let row = s.app.active_panel().list().announce_focus();
    assert!(row.starts_with("notes.txt, "), "{row}");
    s.focus("week1");
    let said = s.key("enter");
    assert!(
        said.iter().any(|l| l == "week1, 3 items, in course.zip"),
        "{said:?}"
    );
    assert!(
        s.app
            .title_line()
            .starts_with("week1, in course.zip, left panel"),
        "{}",
        s.app.title_line()
    );
    // A change inside an archive is refused in words.
    assert_eq!(
        s.key("f7"),
        ["Archives are read only here. Use Pack to make a new one."]
    );
    assert_eq!(
        s.key("f8"),
        ["Archives are read only here. Use Pack to make a new one."]
    );
    // Back up to the archive's root, then out of it.
    let said = s.key("left");
    assert!(
        said.iter()
            .any(|l| l.starts_with("course.zip, 4 items, in ")),
        "{said:?}"
    );
    let said = s.key("left");
    assert!(said.iter().any(|l| l.starts_with("demo, ")), "{said:?}");
    assert_eq!(s.focused(), "course.zip");
    assert!(s.app.title_line().starts_with("demo, left panel"));
}

#[test]
fn open_as_archive_reads_a_file_by_its_bytes() {
    let dir = temp();
    let demo = demo(dir.path());
    fs::copy(demo.join("course.zip"), demo.join("course.bin")).unwrap();
    let mut s = session(&demo);
    s.focus("course.bin");
    let said = s.command(ids::OPEN_AS_ARCHIVE);
    assert!(
        said.iter()
            .any(|l| l.starts_with("Opened course.bin: 10 items")),
        "{said:?}"
    );
    assert_eq!(s.app.panel(Side::Left).list().len(), 4);
}

/// A session over a provider whose trash is a test folder.
fn trash_session(dir: &Path, demo: &Path) -> Session {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let (registry, _trash) = crate::support::registry_with_trash(&bin);
    Session::open(
        place(demo),
        place(&demo.join("out")),
        registry,
        Setup::default(),
    )
}

#[test]
fn a_copy_out_of_an_archive_is_one_unpack_and_undo_trashes_it() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = trash_session(dir.path(), &demo);
    s.focus("course.zip");
    s.key("enter");
    s.focus("week1");
    let said = s.key("f5");
    let question = said.last().cloned().unwrap_or_default();
    assert!(
        question.starts_with("Unpack 4 members into out? No is the default. y or n"),
        "{said:?}"
    );
    let listing = match s.dialog() {
        Some(diroxide_app::Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        listing.iter().any(|l| l.starts_with("Where: ")),
        "{listing:?}"
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Unpack finished: 4 done")),
        "{said:?}"
    );
    assert!(demo.join("out").join("week1").join("a.txt").exists());
    let said = s.key("ctrl+z");
    assert!(
        said.iter().any(|l| l.starts_with("Undid unpack: ")),
        "{said:?}"
    );
    assert!(!demo.join("out").join("week1").exists());
}

#[test]
fn a_copy_into_an_archive_says_it_is_read_only() {
    let dir = temp();
    let demo = demo(dir.path());
    fs::write(demo.join("out").join("new.txt"), "new").unwrap();
    let mut s = Session::open(
        place(&demo.join("out")),
        place(&demo.join("course.zip")).into_archive(),
        Registry::with_local(),
        Setup::default(),
    );
    s.focus("new.txt");
    assert_eq!(
        s.key("f5"),
        ["Archives are read only here. Use Pack to make a new one."]
    );
}

#[test]
fn the_pack_dialog_is_a_list_of_settings_and_its_question_starts_on_no() {
    let dir = temp();
    let demo = demo(dir.path());
    fs::write(demo.join("main.rs"), "fn main() {}\n").unwrap();
    fs::write(demo.join("report.md"), "# Report\n").unwrap();
    let mut s = trash_session(dir.path(), &demo);
    s.focus("main.rs");
    s.key("space");
    s.focus("report.md");
    s.key("space");
    let said = s.command(ids::PACK);
    assert_eq!(said, ["Pack, Pack 2 items into demo.zip, 1 of 9"]);
    let said = s.key("down");
    assert_eq!(said, ["Name: demo.zip, 2 of 9"]);
    let said = s.key("down");
    assert_eq!(said, ["Format: zip, 3 of 9"]);
    s.key("enter");
    assert_eq!(s.dialog_title().as_deref(), Some("Format"));
    s.key("down");
    let said = s.key("enter");
    assert!(
        said.iter().any(|l| l.starts_with("Pack, Format: 7z")),
        "{said:?}"
    );
    let said = s.key("up");
    assert_eq!(said, ["Name: demo.7z, 2 of 9"]);
    s.key("home");
    let said = s.key("enter");
    let question = said.last().cloned().unwrap_or_default();
    assert_eq!(
        question,
        "Pack 2 items into demo.7z? No is the default. y or n"
    );
    let listing = match s.dialog() {
        Some(diroxide_app::Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        listing
            .iter()
            .any(|l| l.starts_with("Format: 7z, level normal")),
        "{listing:?}"
    );
    assert!(
        listing
            .iter()
            .any(|l| l.starts_with("Where: the other panel, ")),
        "{listing:?}"
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Pack started: 2 items into demo.7z")),
        "{said:?}"
    );
    assert!(said.iter().any(|l| l.contains("all good")), "{said:?}");
    assert!(demo.join("out").join("demo.7z").exists());
}

#[test]
fn unpack_and_test_archive_say_the_runners_lines() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = trash_session(dir.path(), &demo);
    s.focus("course.zip");
    let said = s.command(ids::TEST_ARCHIVE);
    assert!(
        said.iter()
            .any(|l| l.starts_with("Archive is sound: 10 members")),
        "{said:?}"
    );
    assert_eq!(s.key("ctrl+z"), ["Nothing to undo"]);
    let said = s.command(ids::UNPACK);
    assert_eq!(said, ["Unpack, Unpack course.zip into out, 1 of 6"]);
    let said = s.key("down");
    assert!(said[0].starts_with("Where: the other panel, "), "{said:?}");
    let said = s.key("down");
    assert_eq!(
        said,
        ["Folder: made when several items are at the top, 3 of 6"]
    );
    let said = s.key("end");
    assert_eq!(said, ["Links: left out, each one said, 6 of 6"]);
    s.key("home");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Unpack 10 members into course? No is the default. y or n")
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Unpack finished: 10 done")),
        "{said:?}"
    );
    assert!(
        demo.join("out")
            .join("course")
            .join("week1")
            .join("a.txt")
            .exists()
    );
}

const CANARY: &str = "canary-7f3a-not-a-password";

/// The demo set with `locked.zip` (one AES member) and `locked.7z` (its
/// names hidden too), both under the canary password.
fn locked_demo(root: &Path) -> PathBuf {
    let demo = demo(root);
    let build = root.join("build");
    fs::write(build.join("secret.txt"), "the secret\n").unwrap();
    pack_into(&build, "locked.zip", &["secret.txt"], Some(CANARY));
    let destination = place(&build.join("locked.7z"));
    let mut request = PackRequest::new(
        vec![place(&build.join("secret.txt"))],
        destination,
        ArchiveFormat::SevenZ,
    );
    request.password = Some(Secret::new(CANARY.to_owned()));
    request.encrypt_names = true;
    diroxide_archive::pack(&request, &Registry::with_local(), &OpContext::new()).unwrap();
    for name in ["locked.zip", "locked.7z"] {
        fs::rename(build.join(name), demo.join(name)).unwrap();
    }
    demo
}

fn assert_no_canary(s: &Session) {
    for message in &s.said {
        assert!(!message.text.contains(CANARY), "{}", message.text);
    }
    assert!(!format!("{:?}", s.app.dialogs()).contains(CANARY));
}

#[test]
fn a_wrong_password_says_so_first_then_the_prompt_comes_back() {
    let dir = temp();
    let demo = locked_demo(dir.path());
    let mut s = trash_session(dir.path(), &demo);
    s.focus("locked.zip");
    s.key("enter");
    s.focus("secret.txt");
    s.key("f5");
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l == "Waiting for the password for locked.zip."),
        "{said:?}"
    );
    assert_eq!(
        said.last().map(String::as_str),
        Some("Password for locked.zip (hidden):")
    );
    s.type_text("not it");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Wrong password for locked.zip. Try again? No is the default. y or n")
    );
    let said = s.key("y");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Password for locked.zip (hidden):")
    );
    s.type_text(CANARY);
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Unpack finished: 1 done")),
        "{said:?}"
    );
    assert_eq!(
        fs::read_to_string(demo.join("out").join("secret.txt")).unwrap(),
        "the secret\n"
    );
    assert_no_canary(&s);
}

#[test]
fn escape_at_the_prompt_gives_no_password_and_skips() {
    let dir = temp();
    let demo = locked_demo(dir.path());
    let mut s = trash_session(dir.path(), &demo);
    s.focus("locked.zip");
    s.key("enter");
    s.focus("secret.txt");
    s.key("f5");
    s.key("y");
    let said = s.key("escape");
    assert!(said.iter().any(|l| l == "No password given."), "{said:?}");
    assert!(!demo.join("out").join("secret.txt").exists());
}

#[test]
fn a_locked_7z_asks_on_entering_and_again_after_leaving() {
    let dir = temp();
    let demo = locked_demo(dir.path());
    let mut s = trash_session(dir.path(), &demo);
    s.focus("locked.7z");
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Password for locked.7z (hidden):")
    );
    s.type_text(CANARY);
    let said = s.key("enter");
    assert!(
        said.iter()
            .any(|l| l.starts_with("Opened locked.7z: 1 item")),
        "{said:?}"
    );
    s.key("left");
    assert_eq!(s.focused(), "locked.7z");
    // Leaving made the provider forget it: entering asks again.
    let said = s.key("enter");
    assert_eq!(
        said.last().map(String::as_str),
        Some("Password for locked.7z (hidden):")
    );
    s.key("escape");
    assert_no_canary(&s);
}

/// A limit from the `[archives]` settings asks its question once, starting
/// on No, and yes lets the job go on.
#[test]
fn a_limit_from_the_settings_asks_once_and_starts_on_no() {
    let dir = temp();
    let demo = demo(dir.path());
    let build = dir.path().join("build");
    fs::write(build.join("zeros.bin"), vec![0u8; 256 * 1024]).unwrap();
    fs::write(build.join("more.bin"), vec![0u8; 256 * 1024]).unwrap();
    pack_into(&build, "zeros.zip", &["zeros.bin", "more.bin"], None);
    fs::rename(build.join("zeros.zip"), demo.join("zeros.zip")).unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let (registry, _trash) = crate::support::registry_with_trash(&bin);
    let mut settings = crate::support::settings();
    settings.archives.max_ratio = 4;
    settings.archives.ratio_floor_mib = 0;
    let mut s = Session::open(
        place(&demo),
        place(&demo.join("out")),
        registry,
        Setup {
            settings,
            ..Setup::default()
        },
    );
    s.focus("zeros.zip");
    s.key("enter");
    s.key("ctrl+a");
    s.key("f5");
    let said = s.key("y");
    let question = said.last().cloned().unwrap_or_default();
    assert!(question.starts_with("Expands to "), "{said:?}");
    assert!(question.ends_with("No is the default. y or n"), "{said:?}");
    let said = s.key("y");
    // Asked once for its kind: the second member goes on without asking.
    assert!(
        said.iter()
            .any(|l| l.starts_with("Unpack finished: 2 done")),
        "{said:?}"
    );
}
