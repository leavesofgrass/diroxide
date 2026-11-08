//! Entry points for the fuzz targets (`fuzz/fuzz_targets/meta_*.rs`), so
//! the fuzz workspace calls the crate's own code through a public door:
//! every reader of file contents gets one (R28 3.2). Each must never
//! panic, hang or allocate past its limits on any input.
//!
//! The readers run here directly, never under the guard's catch, so a
//! panic reaches the fuzzer as the failure it is.

use std::time::Duration;

use diroxide_core::{Entry, EntryKind, Place};

use crate::guard::Bounded;
use crate::reader::{ReadLimits, ReadRequest};

/// The limits a fuzzed read is held to: 4 MiB and ten seconds.
const LIMITS: ReadLimits = ReadLimits {
    max_bytes: 4 * 1024 * 1024,
    deadline: Duration::from_secs(10),
};

/// One file's contents through every reader of the crate, whatever the
/// file's name or type would say, and through the Exif reader and the
/// internet mark's readers as a block of their own: the number of fields
/// and facts they gave.
pub fn file_bytes(bytes: &[u8]) -> usize {
    let mut found = usize::from(crate::exif::parse(bytes).is_some());
    found += usize::from(crate::origin::InternetMark::from_zone_identifier(bytes).is_some());
    found += usize::from(crate::origin::InternetMark::from_quarantine(bytes).is_some());
    found += usize::from(crate::json::parse(&String::from_utf8_lossy(bytes)).is_some());
    found + readers_on(bytes)
}

/// A picture's bytes through the strip: when it is written, its copy is
/// read again, and nothing the strip takes out may still be in it.
pub fn strip(bytes: &[u8]) -> usize {
    match crate::strip::strip_bytes(bytes.to_vec()) {
        Ok(stripped) => {
            if let Ok(left) = crate::strip::left_in_copy(&stripped.bytes) {
                assert!(left.still.is_empty(), "{:?}", left.still);
            }
            stripped.bytes.len()
        }
        Err(_) => 0,
    }
}

fn readers_on(bytes: &[u8]) -> usize {
    let place = Place::parse(if cfg!(windows) {
        "C:\\fuzz\\input"
    } else {
        "/fuzz/input"
    });
    let Ok(place) = place else {
        return 0;
    };
    let entry = Entry::new("input", EntryKind::File);
    let request = ReadRequest {
        place: &place,
        entry: &entry,
        type_id: None,
        group: None,
        cancel: None,
        tools: None,
    };
    let mut fields = 0;
    for reader in crate::readers::builtin() {
        let mut source = Bounded::from_bytes(bytes.to_vec(), &LIMITS);
        if let Ok(found) = reader.read(&mut source, &request, &LIMITS) {
            fields += found.len();
        }
    }
    fields
}

/// A checksum list's text (`SHA256SUMS`, a BSD tagged list, a one-file
/// list, `.sfv`): any bytes, read under each kind of list name. No name
/// taken may be absolute or leave the list's folder, and a GNU list
/// written from what was read reads back with the same names: either
/// failing is a panic the fuzzer reports. The number of entries taken.
pub fn checksum_list(bytes: &[u8]) -> usize {
    use crate::checksum::Digest;
    use crate::checksum::list::{ListForm, parse_list, write_list};

    let mut taken = 0;
    for list_name in [
        "SHA256SUMS",
        "list.sfv",
        "report.pdf.sha256",
        "checksums.txt",
    ] {
        let list = parse_list(bytes, list_name);
        for entry in &list.entries {
            assert!(!entry.name.starts_with('/'), "{:?}", entry.name);
            assert!(!entry.name.contains('\\'), "{:?}", entry.name);
            assert!(!entry.parts().contains(&".."), "{:?}", entry.name);
            assert!(!entry.parts().is_empty(), "{:?}", entry.name);
        }
        let written: Vec<(String, Digest)> = list
            .entries
            .iter()
            .filter_map(|entry| {
                let algorithm = *entry.candidates().first()?;
                let bytes = (0..entry.hex.len() / 2)
                    .map(|i| u8::from_str_radix(entry.hex.get(2 * i..2 * i + 2)?, 16).ok())
                    .collect::<Option<Vec<u8>>>()?;
                Some((entry.name.clone(), Digest { algorithm, bytes }))
            })
            .collect();
        if let Ok(text) = write_list(&written, ListForm::Gnu) {
            let again = parse_list(text.as_bytes(), "SHA256SUMS");
            let before: Vec<&str> = written.iter().map(|(name, _)| name.as_str()).collect();
            let after: Vec<&str> = again.entries.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(before, after);
        }
        taken += list.entries.len();
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_points_take_any_bytes() {
        assert!(file_bytes(&[0xFF; 16]) > 0);
        assert!(file_bytes(b"") > 0);
        assert_eq!(strip(b"not a picture"), 0);
        assert!(strip(&crate::image::tests::jpeg(&[(0xFE, b"note".to_vec())])) > 0);
        assert_eq!(checksum_list(b"ab12  a.txt\n\xFF\xFE\n"), 0);
        let line = format!(
            "{}  a.txt\n\\{}  b\\nc\n{}  ../x\n",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        );
        assert_eq!(checksum_list(line.as_bytes()), 6);
    }
}
