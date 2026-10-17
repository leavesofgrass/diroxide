//! Split and join (plan commit 65): the presets as a list, the plan's rows
//! before anything is written, the join from any part with its verdict and
//! the offer to trash the parts, undo, a missing part, and the presets in
//! the Settings list.

use std::fs;

use diroxide_app::commands::ids;
use diroxide_app::{Answer, AppEvent};

use crate::archives::demo;
use crate::support::{Session, Setup, place, registry_with_trash, temp};

fn confirm_listing(s: &Session) -> Vec<String> {
    match s.dialog() {
        Some(diroxide_app::Dialog::Confirm { listing, .. }) => listing.clone(),
        other => panic!("no confirmation: {other:?}"),
    }
}

#[test]
fn split_then_join_then_undo_and_a_missing_part() {
    let dir = temp();
    let demo = demo(dir.path());
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(demo.join("big.bin"), &data).unwrap();
    fs::create_dir_all(demo.join("joined")).unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let (registry, _trash) = registry_with_trash(&bin);
    let mut s = Session::open(
        place(&demo),
        place(&demo.join("out")),
        registry,
        Setup::default(),
    );
    s.focus("big.bin");
    let said = s.command(ids::SPLIT);
    assert_eq!(
        said,
        ["Split into, Email attachment, 18 MB, Parts of 18,000,000 bytes, 1 of 9"]
    );
    let said = s.key("end");
    assert_eq!(said, ["Custom size, with its unit, 9 of 9"]);
    s.key("enter");
    s.type_text("40KB");
    let said = s.key("enter");
    let question = said.last().cloned().unwrap_or_default();
    assert!(question.ends_with("No is the default. y or n"), "{said:?}");
    let listing = confirm_listing(&s);
    assert!(
        listing
            .iter()
            .any(|l| l.starts_with("3 parts of 40 KB, the last 20 KB")),
        "{listing:?}"
    );
    let said = s.key("y");
    assert!(said.iter().any(|l| l.contains("finished")), "{said:?}");
    for name in ["big.bin.001", "big.bin.002", "big.bin.003", "big.bin.crc"] {
        assert!(demo.join("out").join(name).exists(), "{name}");
    }
    // Join from any part, into an empty folder in the other panel.
    s.focus("joined");
    s.key("enter");
    s.key("tab");
    s.focus("big.bin.002");
    let said = s.command(ids::JOIN);
    assert!(
        said.last()
            .is_some_and(|q| q.starts_with("Join ") && q.ends_with("No is the default. y or n")),
        "{said:?}"
    );
    let said = s.key("y");
    assert!(
        said.iter()
            .any(|l| l == "Joined big.bin: checksum matches."),
        "{said:?}"
    );
    assert_eq!(
        said.last().map(String::as_str),
        Some("Move the 3 parts to the trash? No is the default. y or n")
    );
    let said = s.key("n");
    assert_eq!(said, ["The parts stay where they are."]);
    assert_eq!(fs::read(demo.join("joined").join("big.bin")).unwrap(), data);
    let said = s.key("ctrl+z");
    assert!(
        said.iter().any(|l| l.starts_with("Undid join: ")),
        "{said:?}"
    );
    assert!(!demo.join("joined").join("big.bin").exists());
    // A missing part is said first, and nothing is joined.
    fs::rename(
        demo.join("out").join("big.bin.002"),
        demo.join("out").join("kept.bin"),
    )
    .unwrap();
    s.key("ctrl+r");
    s.focus("big.bin.001");
    let said = s.command(ids::JOIN);
    assert!(
        said.iter()
            .any(|l| l.starts_with("Part 002 is missing. Nothing was joined.")),
        "{said:?}"
    );
}

#[test]
fn the_split_presets_row_of_the_settings_list_is_a_list() {
    let dir = temp();
    let demo = demo(dir.path());
    let mut s = Session::open(
        place(&demo),
        place(&demo.join("out")),
        diroxide_vfs::Registry::with_local(),
        Setup::default(),
    );
    s.command(ids::SETTINGS);
    let dialog = s.app.dialog().map(|(id, _)| id).unwrap();
    let effects = s.app.handle(AppEvent::Answer {
        dialog,
        answer: Answer::Chosen("split_presets".to_owned()),
    });
    let said = s.run(effects);
    assert!(
        said[0].starts_with("Split presets, 7 presets, Email attachment, 18 MB"),
        "{said:?}"
    );
}

/// No on an odd part cancels the join: a part is never left out.
#[test]
fn no_on_an_odd_part_cancels_the_join() {
    let dir = temp();
    let demo = demo(dir.path());
    let out = demo.join("out");
    fs::write(out.join("big.bin.001"), vec![1u8; 40_000]).unwrap();
    fs::write(out.join("big.bin.002"), vec![2u8; 10_000]).unwrap();
    fs::write(out.join("big.bin.003"), vec![3u8; 30_000]).unwrap();
    fs::create_dir_all(demo.join("joined")).unwrap();
    let mut s = Session::open(
        place(&out),
        place(&demo.join("joined")),
        diroxide_vfs::Registry::with_local(),
        Setup::default(),
    );
    s.focus("big.bin.001");
    let said = s.command(ids::JOIN);
    let question = said.last().cloned().unwrap_or_default();
    assert!(
        question.starts_with("Join anyway? No is the default"),
        "{said:?}"
    );
    let listing = confirm_listing(&s);
    assert!(listing[0].starts_with("Part 002 is "), "{listing:?}");
    let said = s.key("n");
    assert_eq!(said, ["Nothing was joined."]);
    assert!(fs::read_dir(demo.join("joined")).unwrap().next().is_none());
}
