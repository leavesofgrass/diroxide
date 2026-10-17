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
