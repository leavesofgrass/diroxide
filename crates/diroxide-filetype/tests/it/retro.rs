//! The retro pack (R30's commit 26): embedded, off by default, on through
//! `filetypes.retro`; the owner's own retro types wait aside while off.

use std::collections::HashSet;

use diroxide_config::{FileTypesConfig, Settings};
use diroxide_core::{Entry, EntryKind, FixedClock, Timestamp, UtcOffset};
use diroxide_filetype::{Probe, RETRO_TABLE, TypeRegistry, retro_line};

fn clock() -> FixedClock {
    let now = Timestamp::from_civil(2026, 11, 6, 4, 0, 0, UtcOffset::UTC).unwrap();
    FixedClock::new(now, UtcOffset::UTC)
}

fn type_of(types: &TypeRegistry, name: &str, head: Option<&[u8]>) -> String {
    let entry = Entry::new(name, EntryKind::File).with_size(4_096);
    match types.check(&Probe::new(&entry, head), &clock()) {
        Ok(found) => found.type_id,
        Err(special) => special.words().to_owned(),
    }
}

#[test]
fn the_pack_reads_whole_and_names_no_shipped_type() {
    let config = FileTypesConfig::from_toml(RETRO_TABLE).unwrap();
    let shipped: HashSet<String> = TypeRegistry::builtin()
        .types()
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert!(config.types.len() >= 30);
    for t in &config.types {
        assert!(t.retro, "{} is not marked retro", t.id);
        assert!(!shipped.contains(&t.id), "{} is shipped too", t.id);
        assert!(
            t.mime.as_deref().is_some_and(|m| m.contains('/')),
            "{}",
            t.id
        );
    }
    let on = TypeRegistry::builtin().with_retro(true);
    assert!(on.problems().is_empty(), "{:?}", on.problems());
    assert_eq!(on.types().len(), shipped.len() + config.types.len());
}

/// Off by default, as the setting is; on, the old formats read.
#[test]
fn off_by_default_and_on_by_the_setting() {
    assert!(!Settings::default().filetypes.retro);
    assert_eq!(retro_line(false), "Retro types: off");
    assert_eq!(retro_line(false).chars().count(), 16);
    assert_eq!(retro_line(true), "Retro types: on");
    let off = TypeRegistry::builtin();
    assert!(!off.retro());
    let ilbm: &[u8] = b"FORM\x00\x00\x10\x00ILBMBMHD";
    let module: &[u8] = b"Extended Module: tune";
    assert_eq!(type_of(&off, "song.mod", None), "type pending");
    assert_eq!(type_of(&off, "picture", Some(ilbm)), "unknown");
    assert_eq!(type_of(&off, "tune", Some(module)), "unknown");
    let on = off.with_retro(true);
    assert!(on.retro());
    assert_eq!(type_of(&on, "song.mod", None), "mod");
    assert_eq!(type_of(&on, "picture", Some(ilbm)), "ilbm");
    assert_eq!(type_of(&on, "tune", Some(module)), "xm");
    assert_eq!(type_of(&on, "letter.sdw", None), "sdw");
    // An AIFF is still an AIFF beside the IFF family's heading.
    assert_eq!(
        type_of(&on, "take", Some(b"FORM\x00\x00\x10\x00AIFFCOMM")),
        "aiff"
    );
    let off_again = on.with_retro(false);
    assert_eq!(
        off_again.types().len(),
        TypeRegistry::builtin().types().len()
    );
}

#[test]
fn the_owners_retro_types_wait_while_the_pack_is_off() {
    let config = FileTypesConfig::from_toml(
        "[[type]]\nid = \"amos\"\nname = \"AMOS program\"\nshort = \"AMOS\"\nmime = \"application/x-amos\"\nextensions = [\"amos\"]\nretro = true\n",
    )
    .unwrap();
    let off = TypeRegistry::builtin().with_config(&config);
    assert!(off.get("amos").is_none());
    assert_eq!(
        off.with_retro(true).get("amos").unwrap().name,
        "AMOS program"
    );
    let on_first = TypeRegistry::builtin()
        .with_retro(true)
        .with_config(&config);
    assert!(on_first.get("amos").unwrap().yours);
    let back_off = on_first.with_retro(false);
    assert!(back_off.get("amos").is_none());
    assert!(back_off.with_retro(true).get("amos").is_some());
}
