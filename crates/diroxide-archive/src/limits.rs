//! The limits that keep a hostile archive from filling the disk or the
//! memory (research/archives.md section 6), one to one with the
//! `[archives]` keys of `config.toml`.
//!
//! The keys, by field: `max_member_size` is `max_member_gib`,
//! `ask_total_bytes` is `ask_total_gib`, `max_total_bytes` is
//! `max_total_gib`, `max_ratio` is `max_ratio`, `ratio_floor_bytes` is
//! `ratio_floor_mib`, `ask_dictionary_bytes` is `ask_dictionary_mib`,
//! `max_dictionary` is `max_dictionary_mib`, `max_header_bytes` is
//! `max_header_mib`, `max_index_bytes` is `max_index_mib`, `max_entries`,
//! `max_name_bytes`, `max_component_bytes` and `max_link_target_bytes` are
//! themselves, `max_memory_bytes` is `max_memory_mib`,
//! `max_spool_memory_bytes` is `spool_memory_mib`, `max_spool_disk_bytes`
//! is `spool_disk_gib`, and `max_nesting` is itself. The defaults match
//! the schema's; the app's mapping is a straight copy with the units.

/// A mebibyte, in bytes.
pub(crate) const MIB: u64 = 1 << 20;
/// A gibibyte, in bytes.
pub(crate) const GIB: u64 = 1 << 30;

/// Limits that keep a hostile archive from filling the disk or memory
/// (research/archives.md section 6). Each becomes a key of the
/// `[archives]` table of `config.toml` (W2a). "Ask" limits are questions
/// ending "y or n", asked once per job; the others refuse with a reason in
/// words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Ask before reading a member larger than this, or one behind this
    /// much compressed data in a sequential archive (soft).
    pub max_member_size: u64,
    /// Ask before an operation decompresses more than this (soft).
    pub ask_total_bytes: u64,
    /// Refuse an operation that would decompress more than this (hard).
    pub max_total_bytes: u64,
    /// Ask when one member's running ratio passes this many to one, once
    /// it has written `ratio_floor_bytes`.
    pub max_ratio: u64,
    /// The output a member writes before its ratio is judged.
    pub ratio_floor_bytes: u64,
    /// Ask before a dictionary or window larger than this (7z, xz,
    /// Zstandard).
    pub ask_dictionary_bytes: u64,
    /// Refuse a dictionary or window larger than this; replaces Phase 0's
    /// 7z-only limit.
    pub max_dictionary: u64,
    /// Refuse a container header or central directory larger than this.
    pub max_header_bytes: u64,
    /// Refuse an index that takes more memory than this.
    pub max_index_bytes: u64,
    /// Refuse an archive with more entries than this.
    pub max_entries: usize,
    /// Skip a member whose name is longer than this, in bytes.
    pub max_name_bytes: usize,
    /// Skip a member with a path component longer than this, in bytes.
    pub max_component_bytes: usize,
    /// Skip a link whose target is longer than this, in bytes.
    pub max_link_target_bytes: usize,
    /// Refuse an operation that needs more memory than this.
    pub max_memory_bytes: u64,
    /// Spool a nested archive in memory up to this size.
    pub max_spool_memory_bytes: u64,
    /// Spool on disk up to this much per job, shared across nesting levels.
    pub max_spool_disk_bytes: u64,
    /// How many archives deep a place may go.
    pub max_nesting: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_member_size: 8 * GIB,
            ask_total_bytes: 32 * GIB,
            max_total_bytes: 1 << 40,
            max_ratio: 1_000,
            ratio_floor_bytes: 64 * MIB,
            ask_dictionary_bytes: 256 * MIB,
            max_dictionary: 1_536 * MIB,
            max_header_bytes: 256 * MIB,
            max_index_bytes: 256 * MIB,
            max_entries: 1_000_000,
            max_name_bytes: 4_096,
            max_component_bytes: 255,
            max_link_target_bytes: 4_096,
            max_memory_bytes: GIB,
            max_spool_memory_bytes: 64 * MIB,
            max_spool_disk_bytes: 4 * GIB,
            max_nesting: 4,
        }
    }
}

impl Limits {
    /// Small limits for the fuzz runs, so a bomb is found in seconds.
    pub fn strict() -> Self {
        Self {
            max_member_size: 16 * MIB,
            ask_total_bytes: 32 * MIB,
            max_total_bytes: 64 * MIB,
            max_ratio: 100,
            ratio_floor_bytes: MIB,
            ask_dictionary_bytes: 16 * MIB,
            max_dictionary: 64 * MIB,
            max_header_bytes: 4 * MIB,
            max_index_bytes: 16 * MIB,
            max_entries: 10_000,
            max_name_bytes: 1_024,
            max_component_bytes: 255,
            max_link_target_bytes: 1_024,
            max_memory_bytes: 128 * MIB,
            max_spool_memory_bytes: 8 * MIB,
            max_spool_disk_bytes: 64 * MIB,
            max_nesting: 2,
        }
    }
}

/// A limit's size in words, in the binary units the settings use: "64
/// MiB", "1.5 GiB", "1 TiB", "512 bytes".
pub fn limit_words(bytes: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1 << 40, "TiB"),
        (GIB, "GiB"),
        (MIB, "MiB"),
        (1 << 10, "KiB"),
    ];
    for (unit, name) in UNITS {
        if bytes >= unit {
            let whole = bytes / unit;
            let tenths = (bytes % unit) * 10 / unit;
            return if tenths == 0 || whole >= 100 {
                format!("{whole} {name}")
            } else {
                format!("{whole}.{tenths} {name}")
            };
        }
    }
    diroxide_core::format::count(bytes, "byte", "bytes").replace("no bytes", "0 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_read_in_binary_units() {
        assert_eq!(limit_words(64 * MIB), "64 MiB");
        assert_eq!(limit_words(1_536 * MIB), "1.5 GiB");
        assert_eq!(limit_words(1 << 40), "1 TiB");
        assert_eq!(limit_words(4_096), "4 KiB");
        assert_eq!(limit_words(255), "255 bytes");
        assert_eq!(limit_words(u64::from(u32::MAX)), "3.9 GiB");
    }

    #[test]
    fn the_defaults_are_the_schemas() {
        // The `[archives]` defaults in diroxide-config's schema, in its
        // units, so the app's mapping can be a straight copy.
        let limits = Limits::default();
        assert_eq!(limits.max_member_size, 8 * GIB);
        assert_eq!(limits.ask_total_bytes, 32 * GIB);
        assert_eq!(limits.max_total_bytes, 1_024 * GIB);
        assert_eq!(limits.max_ratio, 1_000);
        assert_eq!(limits.ratio_floor_bytes, 64 * MIB);
        assert_eq!(limits.ask_dictionary_bytes, 256 * MIB);
        assert_eq!(limits.max_dictionary, 1_536 * MIB);
        assert_eq!(limits.max_header_bytes, 256 * MIB);
        assert_eq!(limits.max_index_bytes, 256 * MIB);
        assert_eq!(limits.max_entries, 1_000_000);
        assert_eq!(limits.max_name_bytes, 4_096);
        assert_eq!(limits.max_component_bytes, 255);
        assert_eq!(limits.max_link_target_bytes, 4_096);
        assert_eq!(limits.max_memory_bytes, 1_024 * MIB);
        assert_eq!(limits.max_spool_memory_bytes, 64 * MIB);
        assert_eq!(limits.max_spool_disk_bytes, 4 * GIB);
        assert_eq!(limits.max_nesting, 4);
    }
}
