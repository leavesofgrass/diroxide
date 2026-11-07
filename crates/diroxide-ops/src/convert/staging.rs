//! The staging name a tool writes to (R28 5.3, spike V5): `.diro-part-`,
//! eight hexadecimal digits and the output's extension, in the
//! destination folder, renamed to the real name only when the tool
//! succeeded. The leading dot is what the arrival tracker ignores, so a
//! hot folder watching the destination never sees a half-made output;
//! the extension is what every tool reads the output's format from
//! (ImageMagick, FFmpeg, pandoc and pdftotext all took
//! `.diro-part-1a2b3c4d.jpg` and its kin, Friday, November 6, 2026).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use diroxide_vfs::names::UNFINISHED_PREFIX;

/// A token no other staging file of this process has had, and unlikely in
/// any other: a counter mixed with the process's random hash keys.
pub fn token() -> u32 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u32(std::process::id());
    let hash = hasher.finish();
    (hash ^ (hash >> 32)) as u32
}

/// A staging name: `.diro-part-1a2b3c4d.jpg`.
pub fn staging_name(token: u32, extension: &str) -> String {
    format!("{UNFINISHED_PREFIX}{token:08x}.{extension}")
}

/// The name of a source's copy in the spool: `in-`, eight hexadecimal
/// digits and the source's extension, a plain name whatever the source's
/// was, so the tool never misreads it.
pub fn spool_input_name(token: u32, extension: &str) -> String {
    format!("in-{token:08x}.{extension}")
}

/// Whether a name is a conversion's staging name: the prefix, eight
/// hexadecimal digits, a dot and a plain extension.
pub fn is_staging_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(UNFINISHED_PREFIX) else {
        return false;
    };
    let Some((digits, extension)) = rest.split_once('.') else {
        return false;
    };
    digits.len() == 8
        && digits.bytes().all(|b| b.is_ascii_hexdigit())
        && !extension.is_empty()
        && extension.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::is_ignored;

    #[test]
    fn staging_names_are_plain_and_the_tracker_ignores_them() {
        let name = staging_name(0x1a2b_3c4d, "jpg");
        assert_eq!(name, ".diro-part-1a2b3c4d.jpg");
        assert!(is_staging_name(&name));
        assert!(is_ignored(&name));
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        );
        assert!(!is_staging_name(".diro-part-1a2b3c4d"));
        assert!(!is_staging_name(".diro-part-1a2b3c4.jpg"));
        assert!(!is_staging_name("photo.jpg"));
        assert_eq!(spool_input_name(0x2a, "png"), "in-0000002a.png");
        assert!(!is_staging_name(&spool_input_name(0x2a, "png")));
        let tokens: std::collections::BTreeSet<u32> = (0..1000).map(|_| token()).collect();
        assert!(tokens.len() > 990);
    }
}
