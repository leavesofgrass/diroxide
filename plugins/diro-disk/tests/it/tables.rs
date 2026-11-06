//! Disks with partition tables through the real host: the root lists one
//! folder per partition with its words, the volumes list names each, and
//! a partition with no files says so in the table's words.

use std::io::Cursor;

use diroxide_core::EntryKind;
use diroxide_core::format::within_cells;
use diroxide_plugins::provider::PluginProvider;
use diroxide_vfs::OpContext;

use crate::fixture::{Fixture, at, words};

/// A GPT disk of 6 MiB, written by `gptman`: an EFI System partition of
/// 1 MiB and a "home" Linux partition of 2 MiB, both empty.
pub fn gpt_disk() -> Vec<u8> {
    let sector = 512u64;
    let mut cur = Cursor::new(vec![0u8; 6 * 1024 * 1024]);
    let mut gpt = gptman::GPT::new_from(&mut cur, sector, [0x5a; 16]).unwrap();
    let first = gpt.header.first_usable_lba.max(2048);
    gpt[1] = gptman::GPTPartitionEntry {
        partition_type_guid: [
            0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
            0xc9, 0x3b,
        ],
        unique_partition_guid: [1; 16],
        starting_lba: first,
        ending_lba: first + 2047,
        attribute_bits: 0,
        partition_name: "".into(),
    };
    gpt[2] = gptman::GPTPartitionEntry {
        partition_type_guid: [
            0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47,
            0x7d, 0xe4,
        ],
        unique_partition_guid: [2; 16],
        starting_lba: first + 2048,
        ending_lba: first + 2048 + 4095,
        attribute_bits: 0,
        partition_name: "home".into(),
    };
    gptman::GPT::write_protective_mbr_into(&mut cur, sector).unwrap();
    gpt.write_into(&mut cur).unwrap();
    cur.into_inner()
}

#[test]
fn a_gpt_disk_lists_its_partitions_in_words() {
    let f = Fixture::new();
    let root = f.image("two-parts.img", &gpt_disk());
    let entries = f.list(&root).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["partition-1", "partition-2"]);
    assert!(entries.iter().all(|e| e.kind == EntryKind::Folder));
    let about: Vec<&str> = entries
        .iter()
        .map(|e| e.extra.get("disk.about").map(String::as_str).unwrap_or(""))
        .collect();
    assert_eq!(
        about,
        [
            "Partition 1, EFI System, 1.0 MB",
            "Partition 2, home, 2.1 MB"
        ]
    );
    for line in &about {
        assert!(line.chars().count() <= 40, "{line}");
    }

    // A partition that holds no file system says so in the table's words.
    let error = f.list(&at(&root, &["partition-2"])).unwrap_err();
    let line = words(&error);
    assert!(
        line.contains("it holds Linux filesystem, which has no files"),
        "{line}"
    );
    assert!(f.list(&at(&root, &["partition-9"])).is_err());
    assert!(f.list(&at(&root, &["partition-01"])).is_err());

    // The volumes list names each partition, meaning first.
    let provider = PluginProvider::new(&f.host, "disk", "disk").unwrap();
    let volumes = provider.volumes(&root, &OpContext::new()).unwrap();
    let rows: Vec<String> = volumes.iter().map(|v| v.words()).collect();
    assert_eq!(rows.len(), 2);
    assert!(
        within_cells(&rows[0], "Partition 1, EFI System"),
        "{rows:?}"
    );
    assert_eq!(volumes[1].total, Some(2 * 1024 * 1024));
    let one = provider
        .volume_of(&at(&root, &["partition-2"]), &OpContext::new())
        .unwrap()
        .unwrap();
    assert!(one.name.starts_with("Partition 2, home"), "{}", one.name);
}

#[test]
fn a_bitlocker_partition_is_shown_locked_and_never_opened() {
    let f = Fixture::new();
    let mut locked = vec![0u8; 64 * 1024];
    locked[3..11].copy_from_slice(b"-FVE-FS-");
    let disk = crate::make::gpt_with(
        4 * 1024 * 1024,
        &[(crate::make::guid::BASIC_DATA, "", &locked)],
    );
    let root = f.image("locked.img", &disk);
    let entries = f.list(&root).unwrap();
    let about = entries[0]
        .extra
        .get("disk.about")
        .cloned()
        .unwrap_or_default();
    assert_eq!(about, "Partition 1, BitLocker, 65.5 KB, locked");
    let line = words(&f.list(&at(&root, &["partition-1"])).unwrap_err());
    assert!(
        line.contains("it is locked by BitLocker, and diroxide does not unlock it"),
        "{line}"
    );
}
