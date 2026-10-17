//! The unpack plan (plan commit 29): the members, the folder rule, paths
//! kept or flattened, the room it needs, the destination's name rules,
//! and every row of the odd-names table of research/archives.md section
//! 7, against the fake provider. Nothing is written by a plan.

use diroxide_archive::{
    Allowances, FolderRule, RecordKind, UnpackPlan, UnpackRequest, plan_unpack,
};
use diroxide_core::format::{human_size, within_cells};
use diroxide_core::{JobId, JobKind, Place};
use diroxide_vfs::OpContext;

use crate::build::{tar_header, tar_of, tar_pad, zip_of};
use crate::hostile::{hostile_names, links_out_of_root};
use crate::world::World;

/// The destination folder on the fake disk: `out` beside the archives.
fn out(world: &World) -> Place {
    let out = world.root.parent().unwrap().join("out").unwrap();
    world.fake.add_folder(&out);
    out
}

/// The demo set's course.zip: two folders of three files, and two files
/// at the top; ten entries.
pub fn course() -> Vec<u8> {
    zip_of(&[
        ("week1/", b""),
        ("week1/a.txt", b"week one, a"),
        ("week1/b.txt", b"week one, b"),
        ("week1/c.txt", b"week one, c"),
        ("week2/", b""),
        ("week2/a.txt", b"week two, a"),
        ("week2/b.txt", b"week two, b"),
        ("week2/c.txt", b"week two, c"),
        ("notes.txt", b"the notes"),
        ("README.md", b"# Course"),
    ])
}

fn plan(world: &World, request: &UnpackRequest) -> UnpackPlan {
    plan_unpack(
        &world.provider,
        &world.registry,
        request,
        JobId::new(41),
        &OpContext::new(),
        Allowances::default(),
    )
    .unwrap()
}

fn targets(plan: &UnpackPlan, root: &Place) -> Vec<String> {
    let root_text = root.to_string();
    plan.members
        .iter()
        .map(|m| {
            m.target.to_string()[root_text.len()..]
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn course_zip_plans_ten_members_into_its_own_folder() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    let before = world.fake.snapshot();
    let plan = plan(&world, &UnpackRequest::new(zip, out.clone()));
    assert_eq!(plan.made_folder.as_deref(), Some("course"));
    assert_eq!(plan.destination, out.join("course").unwrap());
    assert_eq!(plan.count(), 10);
    assert_eq!((plan.facts.files, plan.facts.folders), (8, 2));
    let rows = plan.rows();
    assert_eq!(rows[0], "Unpack 10 members into course");
    assert_eq!(rows[1], format!("Where: {}", plan.destination));
    assert_eq!(rows[2], "Folder: course, 4 items at the top");
    assert_eq!(rows[3], "Paths: kept");
    assert!(rows[4].starts_with("Writes 8 files, "), "{}", rows[4]);
    assert_eq!(rows[5], "When a name is taken: ask");
    for row in rows.iter().filter(|r| !r.starts_with("Where: ")) {
        assert!(row.chars().count() <= 40, "{row}");
    }
    assert_eq!(
        plan.question().text,
        "Unpack 10 members into course? y or n"
    );
    assert!(plan.refusal().is_none());
    let request = plan.job_request();
    assert_eq!(request.kind, JobKind::Unpack);
    assert_eq!(request.destination.as_ref(), Some(&out));
    assert_eq!(world.fake.snapshot(), before, "a plan writes nothing");
    // In archive order, folders and files as the index has them.
    let order = targets(&plan, &plan.destination);
    assert_eq!(order[0], "week1");
    assert!(order.contains(&"week2/c.txt".to_owned()));
}

#[test]
fn the_folder_rule_and_flattening() {
    let world = World::plain();
    let one = world.put(
        "one.zip",
        &zip_of(&[("docs/", b""), ("docs/a.txt", b"a"), ("docs/b.txt", b"b")]),
    );
    let out = out(&world);
    let single = plan(&world, &UnpackRequest::new(one.clone(), out.clone()));
    assert_eq!(single.made_folder, None, "one item at the top: no folder");
    assert_eq!(single.destination, out);
    assert!(
        single
            .rows()
            .contains(&"Folder: none, one item at the top".to_owned())
    );
    let always = plan(
        &world,
        &UnpackRequest {
            folder_rule: FolderRule::Always,
            ..UnpackRequest::new(one.clone(), out.clone())
        },
    );
    assert_eq!(always.made_folder.as_deref(), Some("one"));
    assert!(
        always
            .rows()
            .contains(&"Folder: one, always made".to_owned())
    );
    let course = world.put("course.zip", &course());
    let never = plan(
        &world,
        &UnpackRequest {
            folder_rule: FolderRule::Never,
            ..UnpackRequest::new(course.clone(), out.clone())
        },
    );
    assert_eq!(never.made_folder, None);
    let flat = plan(
        &world,
        &UnpackRequest {
            flatten: true,
            folder_rule: FolderRule::Never,
            ..UnpackRequest::new(course, out.clone())
        },
    );
    assert_eq!(flat.facts.folders, 0);
    assert_eq!(flat.count(), 8);
    let names = targets(&flat, &out);
    assert!(names.iter().all(|n| !n.contains('/')), "{names:?}");
    assert!(
        flat.rows()
            .contains(&"Paths: names only, folders left out".to_owned())
    );
}

#[test]
fn chosen_members_land_relative_to_their_folder() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    let folder = plan(
        &world,
        &UnpackRequest {
            members: vec!["week1".to_owned()],
            ..UnpackRequest::new(zip.clone(), out.clone())
        },
    );
    assert_eq!(folder.made_folder, None);
    assert_eq!(
        targets(&folder, &out),
        ["week1", "week1/a.txt", "week1/b.txt", "week1/c.txt"]
    );
    let file = plan(
        &world,
        &UnpackRequest {
            members: vec!["week2/b.txt".to_owned(), "missing.txt".to_owned()],
            ..UnpackRequest::new(zip, out.clone())
        },
    );
    assert_eq!(targets(&file, &out), ["b.txt"]);
    assert_eq!(
        file.left_out[0].line(),
        "Skipped missing.txt: not in the archive."
    );
}

#[test]
fn too_little_room_refuses_before_writing() {
    let world = World::plain();
    let zip = world.put("course.zip", &course());
    let out = out(&world);
    world.fake.switches_mut().free_space = Some(40);
    let plan = plan(&world, &UnpackRequest::new(zip, out));
    let refusal = plan.refusal().expect("the room is short");
    let start = format!(
        "Needs {}, only {} free",
        human_size(plan.facts.bytes),
        human_size(40)
    );
    assert!(refusal.text.starts_with(&start), "{}", refusal.text);
    assert!(refusal.text.ends_with('.'));
    assert!(within_cells(&refusal.text, &start));
    assert_eq!(plan.rows()[4], refusal.text);
}

/// The odd names of the table, one archive each: the hostile name list,
/// the links out of the root, a file and a folder with one name, a tar
/// name that is not text, a backslash name made on Unix, and a device.
fn odd_world() -> (World, Vec<Place>, Place) {
    let world = World::plain();
    world.fake.switches_mut().reserved_names = true;
    let mut raw_tar = Vec::new();
    raw_tar.extend_from_slice(&tar_header(b"caf\xE9.txt", 4, b'0'));
    raw_tar.extend_from_slice(b"text");
    tar_pad(&mut raw_tar);
    raw_tar.extend_from_slice(&tar_header(br"win\style.txt", 4, b'0'));
    raw_tar.extend_from_slice(b"back");
    tar_pad(&mut raw_tar);
    raw_tar.extend_from_slice(&tar_header(b"dev/null", 0, b'3'));
    raw_tar.extend_from_slice(&[0u8; 1024]);
    let archives = vec![
        world.put("hostile.zip", &hostile_names()),
        world.put("links.zip", &links_out_of_root()),
        world.put(
            "clash.tar",
            &tar_of(&[("same", b"file"), ("same/inner.txt", b"in")]),
        ),
        world.put("raw.tar", &raw_tar),
    ];
    let out = out(&world);
    (world, archives, out)
}

#[test]
fn every_row_of_the_odd_names_table() {
    let (world, archives, out) = odd_world();
    let plans: Vec<UnpackPlan> = archives
        .iter()
        .map(|archive| {
            plan(
                &world,
                &UnpackRequest {
                    folder_rule: FolderRule::Never,
                    ..UnpackRequest::new(archive.clone(), out.clone())
                },
            )
        })
        .collect();
    let hostile = &plans[0];
    let names = targets(hostile, &out);
    // A name that climbs out: not planned, said with its reason.
    assert!(
        hostile
            .rows()
            .contains(&"Skipped ../x: name leaves the archive.".to_owned()),
        "{:?}",
        hostile.rows()
    );
    assert!(!names.iter().any(|n| n.contains("..")));
    // Absolute and drive-qualified: under the destination, renamed.
    assert!(names.contains(&"abs.txt".to_owned()), "{names:?}");
    assert!(names.contains(&"x".to_owned()), "{names:?}");
    // NUL or control: renamed by the rule.
    assert!(names.contains(&"a_b.txt".to_owned()), "{names:?}");
    // A Windows device and a reserved character: renamed on a place with
    // Windows's rules.
    assert!(names.contains(&"CON_".to_owned()), "{names:?}");
    assert!(names.contains(&"a.txt_b".to_owned()), "{names:?}");
    // A trailing space and a direction override: renamed.
    assert!(names.contains(&"trailing_".to_owned()), "{names:?}");
    assert!(names.contains(&"photo_gpj.exe".to_owned()), "{names:?}");
    // An archive marker: extracted by its record, renamed.
    assert!(names.contains(&"inner.zip_x.txt".to_owned()), "{names:?}");
    // A duplicate: the last copy only.
    assert_eq!(names.iter().filter(|n| *n == "dup.txt").count(), 1);
    // A case clash, and composed and decomposed twins: both planned; the
    // destination's conflict policy decides at the run.
    assert!(names.contains(&"README".to_owned()) && names.contains(&"readme".to_owned()));
    assert!(names.contains(&"caf\u{e9}.txt".to_owned()));
    assert!(names.contains(&"cafe\u{301}.txt".to_owned()));
    // Junk is unpacked too: hidden in the listing, never silently lost.
    assert!(names.contains(&"__MACOSX/._junk".to_owned()), "{names:?}");
    assert!(names.contains(&"ok.txt".to_owned()));
    assert!(hostile.renamed >= 8, "{} renamed", hostile.renamed);
    assert!(
        hostile
            .rows()
            .iter()
            .any(|r| r.starts_with("Renamed ") && r.ends_with(" names: unsafe characters."))
    );
    // Links: not followed; the link that a later member writes through is
    // a folder in the index, and its member lands inside.
    let links = &plans[1];
    let lines: Vec<String> = links.left_out.iter().map(|l| l.line()).collect();
    assert!(
        lines.contains(&"Skipped up: link not followed.".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("link and folder share the name"))
    );
    assert!(targets(links, &out).contains(&"link/passwd".to_owned()));
    // A file and a folder with one name: the folder, the file said.
    let clash = &plans[2];
    assert!(
        clash.left_out[0]
            .line()
            .contains("file and folder share the name")
    );
    assert_eq!(targets(clash, &out), ["same", "same/inner.txt"]);
    // Not valid text, a backslash made on Unix, a device: renamed, read
    // as folders, left out.
    let raw = &plans[3];
    let raw_names = targets(raw, &out);
    assert!(raw_names.contains(&"caf_.txt".to_owned()), "{raw_names:?}");
    assert!(
        raw_names.contains(&"win/style.txt".to_owned()),
        "{raw_names:?}"
    );
    assert!(
        raw.left_out
            .iter()
            .any(|l| l.line() == "Skipped dev/null: device or special entry, not unpacked.")
    );
    // Every target, in every plan, is inside the destination.
    for plan in &plans {
        for member in &plan.members {
            assert!(member.target.is_within(&out), "{}", member.target);
            assert_ne!(member.kind, RecordKind::Special);
        }
    }
}
