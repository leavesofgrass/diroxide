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
/// file's name or type would say: the number of fields they gave.
pub fn file_bytes(bytes: &[u8]) -> usize {
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

/// A checksum list's text (`SHA256SUMS`, a BSD tagged list, `.sfv`): any
/// bytes, read as text. The stub counts its lines until the checksum
/// engine's parser takes its place.
pub fn checksum_list(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes).lines().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_points_take_any_bytes() {
        assert!(file_bytes(&[0xFF; 16]) > 0);
        assert!(file_bytes(b"") > 0);
        assert_eq!(checksum_list(b"ab12  a.txt\n\xFF\xFE\n"), 2);
    }
}
