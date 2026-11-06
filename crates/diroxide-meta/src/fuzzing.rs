//! Entry points for the fuzz targets (`fuzz/fuzz_targets/meta_*.rs`), so
//! the fuzz workspace calls the crate's own code through a public door:
//! every reader of file contents gets one (R28 3.2). Each must never
//! panic, hang or allocate past its limits on any input.
//!
//! Stubs from alpha 4's contract step (P4): they hold the shape and the
//! fuzz workspace's two targets, and the metadata lane points them at the
//! readers and the checksum-list parser as it builds them (commits 45 and
//! 49).

/// One reader's input: any bytes, as a file's contents. The stub reads
/// nothing yet; the metadata lane runs every reader of the pipeline here.
pub fn file_bytes(bytes: &[u8]) -> usize {
    bytes
        .len()
        .min(crate::ReadLimits::default().max_bytes as usize)
}

/// A checksum list's text (`SHA256SUMS`, a BSD tagged list, `.sfv`): any
/// bytes, read as text. The stub counts its lines; the checksum engine's
/// parser takes its place.
pub fn checksum_list(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes).lines().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_points_take_any_bytes() {
        assert_eq!(file_bytes(&[0xFF; 16]), 16);
        assert_eq!(checksum_list(b"ab12  a.txt\n\xFF\xFE\n"), 2);
    }
}
