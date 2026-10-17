//! Split presets (plan section 5.4; research/alpha2-plan.md part 5.3):
//! the sizes Split offers, each a use and its size in bytes, said use
//! first ("Email attachment, 18.0 MB"). The built-in table holds published
//! figures, each with its source, tied to the figure by a test; the
//! optical sizes keep one percent free for the file system; the floppy
//! preset is the formatted disk's data area, with nothing more kept free.
//!
//! `config.toml` may add presets as `[[split_presets]]` tables, each with
//! a `name`, `bytes` and an optional `headroom_percent`. They are listed
//! after the built-in ones, and one with a built-in's name (in any case)
//! replaces it there, so a user adds a preset without copying the table.
//! "Equal parts" and "custom size" are choices of the split dialog, not
//! presets.

use std::collections::BTreeMap;

use diroxide_core::format::{grouped, human_size};
use serde::{Deserialize, Serialize};
use toml::Value;

/// One split preset: a use, a size in bytes, and the share of it kept free.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitPreset {
    /// The use, said first: "Email attachment".
    pub name: String,
    /// The size the use allows, in bytes: a limit or a medium's capacity.
    pub bytes: u64,
    /// The percent of `bytes` kept free, for a medium's file system; the
    /// parts are that much smaller. 0 to 50.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub headroom_percent: u8,
    /// Keys this version does not know, kept as they were.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

impl SplitPreset {
    /// A preset with no headroom.
    pub fn new(name: impl Into<String>, bytes: u64) -> Self {
        Self {
            name: name.into(),
            bytes,
            headroom_percent: 0,
            unknown: BTreeMap::new(),
        }
    }

    /// The size of each part but the last: `bytes` less the headroom,
    /// rounded down, at least one byte.
    pub fn part_bytes(&self) -> u64 {
        let keep = u128::from(100 - self.headroom_percent.min(50));
        ((u128::from(self.bytes) * keep / 100) as u64).max(1)
    }

    /// The preset's row, use first, with the size as people say it:
    /// "GitHub file limit, 100 MiB" for a built-in one (its
    /// [`BuiltinPreset::said`]), the part size in words for the user's own
    /// ("Zip disk, 101 MB"). The exact bytes are on the details line.
    pub fn row_text(&self) -> String {
        let said = BUILTIN_SPLIT_PRESETS
            .iter()
            .find(|builtin| builtin.preset() == *self)
            .map_or_else(|| human_size(self.part_bytes()), |b| b.said.to_owned());
        format!("{}, {said}", self.name)
    }

    /// The details line with the exact bytes: "Parts of 104,857,600
    /// bytes", or with headroom "Capacity 4,700,372,992 bytes, parts of
    /// 4,653,369,262".
    pub fn details_line(&self) -> String {
        if self.headroom_percent == 0 {
            format!("Parts of {} bytes", grouped(self.part_bytes()))
        } else {
            format!(
                "Capacity {} bytes, parts of {}, {} percent kept free",
                grouped(self.bytes),
                grouped(self.part_bytes()),
                self.headroom_percent
            )
        }
    }
}

/// A built-in preset with the published figure it comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinPreset {
    /// The use, said first.
    pub name: &'static str,
    /// The figure, in bytes.
    pub bytes: u64,
    /// The percent kept free.
    pub headroom_percent: u8,
    /// The size as people say it, for the row: "100 MiB", "4.7 GB".
    pub said: &'static str,
    /// Where the figure comes from, in words, with the figure in it.
    pub source: &'static str,
}

impl BuiltinPreset {
    /// The preset as configuration.
    pub fn preset(&self) -> SplitPreset {
        SplitPreset {
            headroom_percent: self.headroom_percent,
            ..SplitPreset::new(self.name, self.bytes)
        }
    }
}

/// The built-in presets of research/alpha2-plan.md part 5.3, in the order
/// the split dialog lists them, each with its source.
pub const BUILTIN_SPLIT_PRESETS: [BuiltinPreset; 7] = [
    BuiltinPreset {
        name: "Email attachment",
        bytes: 18_000_000,
        headroom_percent: 0,
        said: "18 MB",
        source: "18,000,000 bytes: a 25 MB mail limit after base64, which turns three bytes into four characters in lines of at most 76 (RFC 2045, section 6.8), about 37 percent more",
    },
    BuiltinPreset {
        name: "GitHub file limit",
        bytes: 104_857_600,
        headroom_percent: 0,
        said: "100 MiB",
        source: "104,857,600 bytes: GitHub blocks a file over 100 MiB (its large-files page)",
    },
    BuiltinPreset {
        name: "CD",
        bytes: 737_280_000,
        headroom_percent: 1,
        said: "700 MB",
        source: "737,280,000 bytes: an 80-minute CD-R, 360,000 sectors of 2,048 bytes",
    },
    BuiltinPreset {
        name: "FAT32's largest file",
        bytes: 4_294_967_295,
        headroom_percent: 0,
        said: "4 GiB less one byte",
        source: "4,294,967,295 bytes: 4 GiB less one byte, the most FAT32's 32-bit size field holds",
    },
    BuiltinPreset {
        name: "DVD",
        bytes: 4_700_372_992,
        headroom_percent: 1,
        said: "4.7 GB",
        source: "4,700,372,992 bytes: a single-layer DVD+R, 2,295,104 sectors of 2,048 bytes, the smaller of the two DVD figures",
    },
    BuiltinPreset {
        name: "Blu-ray",
        bytes: 25_025_314_816,
        headroom_percent: 1,
        said: "25 GB",
        source: "25,025,314,816 bytes: a single-layer Blu-ray disc, 12,219,392 sectors of 2,048 bytes",
    },
    BuiltinPreset {
        name: "Floppy",
        bytes: 1_457_664,
        headroom_percent: 0,
        said: "1.44 MB",
        source: "1,457,664 bytes: the data area of a formatted 1.44 MB floppy, 2,847 clusters of 512 bytes, the raw 1,474,560 less the boot sector, two FATs and the root folder",
    },
];

/// The built-in presets as configuration: the default of `split_presets`.
pub fn builtin_split_presets() -> Vec<SplitPreset> {
    BUILTIN_SPLIT_PRESETS
        .iter()
        .map(BuiltinPreset::preset)
        .collect()
}

/// The presets Split offers: the built-in ones with the user's merged in.
/// A user's preset with a built-in's name, in any case, replaces it in
/// place; the others follow in the user's order; a later one with the same
/// name as an earlier one replaces it.
pub fn merge_split_presets(user: Vec<SplitPreset>) -> Vec<SplitPreset> {
    let mut presets = builtin_split_presets();
    for preset in user {
        match presets
            .iter_mut()
            .find(|p| p.name.to_lowercase() == preset.name.to_lowercase())
        {
            Some(slot) => *slot = preset,
            None => presets.push(preset),
        }
    }
    presets
}

/// Checks one `[[split_presets]]` entry, `n` counting from 1; the error is
/// the reason in words, after the key.
pub(crate) fn check_preset(key: &str, n: usize, value: &Value) -> Result<(), String> {
    let Some(table) = value.as_table() else {
        return Err(format!(
            "{key} preset {n} needs a name and bytes, as a table."
        ));
    };
    let name = match table.get("name").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() && name.chars().count() <= 30 => name,
        Some(_) => {
            return Err(format!(
                "{key} preset {n} needs a name of 1 to 30 characters."
            ));
        }
        None => return Err(format!("{key} preset {n} needs a name in quotes.")),
    };
    match table.get("bytes").and_then(Value::as_integer) {
        Some(bytes) if bytes > 0 => {}
        _ => {
            return Err(format!(
                "{key} preset {name} needs bytes, a whole number above 0."
            ));
        }
    }
    if let Some(headroom) = table.get("headroom_percent") {
        match headroom.as_integer() {
            Some(h) if (0..=50).contains(&h) => {}
            _ => {
                return Err(format!(
                    "{key} preset {name} cannot keep {headroom} percent free: use 0 to 50."
                ));
            }
        }
    }
    Ok(())
}

/// The keys of a `[[split_presets]]` entry this version does not know,
/// with the preset's name: kept and said.
pub(crate) fn unknown_preset_keys(value: &Value) -> Vec<(String, String)> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_table)
        .flat_map(|table| {
            let name = table
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            table
                .keys()
                .filter(|k| !matches!(k.as_str(), "name" | "bytes" | "headroom_percent"))
                .map(move |k| (name.clone(), k.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    const KIB: u64 = 1 << 10;
    const MIB: u64 = 1 << 20;
    const SECTOR: u64 = 2_048;

    /// Each byte count equals the figure its source cites, computed
    /// independently, and the source says the figure in its first words.
    #[test]
    fn each_count_equals_its_cited_figure() {
        let figures: [(&str, u64); 7] = [
            ("Email attachment", 18 * 1_000_000),
            ("GitHub file limit", 100 * MIB),
            ("CD", 360_000 * SECTOR),
            ("FAT32's largest file", u64::from(u32::MAX)),
            ("DVD", 2_295_104 * SECTOR),
            ("Blu-ray", 12_219_392 * SECTOR),
            ("Floppy", 2_847 * 512),
        ];
        assert_eq!(BUILTIN_SPLIT_PRESETS.len(), figures.len());
        for (builtin, (name, figure)) in BUILTIN_SPLIT_PRESETS.iter().zip(figures) {
            assert_eq!(builtin.name, name);
            assert_eq!(builtin.bytes, figure, "{name}");
            assert!(
                builtin
                    .source
                    .starts_with(&format!("{} bytes: ", grouped(figure))),
                "{name}: {}",
                builtin.source
            );
        }
        assert_eq!(4 * KIB * MIB - 1, 4_294_967_295, "4 GiB less one byte");
        assert_eq!(1_474_560 / 1_024, 1_440, "the 1.44 MB of a floppy");
        assert_eq!(
            (80 * 2 * 18 - 1 - 9 * 2 - 14) * 512,
            1_457_664,
            "the data area: the raw sectors less the boot sector, two FATs and the root folder"
        );
    }

    #[test]
    fn the_optical_presets_keep_one_percent_free() {
        let dvd = BUILTIN_SPLIT_PRESETS[4].preset();
        assert_eq!(dvd.part_bytes(), 4_653_369_262);
        assert_eq!(
            dvd.details_line(),
            "Capacity 4,700,372,992 bytes, parts of 4,653,369,262, 1 percent kept free"
        );
        for builtin in &BUILTIN_SPLIT_PRESETS {
            let optical = matches!(builtin.name, "CD" | "DVD" | "Blu-ray");
            assert_eq!(
                builtin.headroom_percent,
                u8::from(optical),
                "{}",
                builtin.name
            );
            let preset = builtin.preset();
            assert!(within_cells(&preset.row_text(), builtin.name));
            assert!(preset.part_bytes() <= preset.bytes);
        }
        let github = BUILTIN_SPLIT_PRESETS[1].preset();
        assert_eq!(github.part_bytes(), 104_857_600);
        assert_eq!(github.details_line(), "Parts of 104,857,600 bytes");
        let rows: Vec<String> = builtin_split_presets()
            .iter()
            .map(SplitPreset::row_text)
            .collect();
        assert_eq!(
            rows,
            [
                "Email attachment, 18 MB",
                "GitHub file limit, 100 MiB",
                "CD, 700 MB",
                "FAT32's largest file, 4 GiB less one byte",
                "DVD, 4.7 GB",
                "Blu-ray, 25 GB",
                "Floppy, 1.44 MB",
            ]
        );
        assert_eq!(
            SplitPreset::new("Zip disk", 100_663_296).row_text(),
            "Zip disk, 101 MB"
        );
        let mut changed = github;
        changed.headroom_percent = 5;
        assert!(
            changed.row_text().starts_with("GitHub file limit, 99."),
            "{}",
            changed.row_text()
        );
    }

    #[test]
    fn a_users_presets_follow_the_built_in_ones_or_replace_them_by_name() {
        let zip_disk = SplitPreset::new("Zip disk", 100_663_296);
        let mut cd = SplitPreset::new("cd", 681_574_400);
        cd.headroom_percent = 2;
        let merged = merge_split_presets(vec![zip_disk.clone(), cd.clone()]);
        assert_eq!(merged.len(), 8);
        assert_eq!(merged[2], cd, "replaced in place, any case");
        assert_eq!(merged[7], zip_disk, "added after the built-in ones");
        assert_eq!(merge_split_presets(Vec::new()), builtin_split_presets());
        assert_eq!(
            merge_split_presets(builtin_split_presets()),
            builtin_split_presets()
        );
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
    }
}
