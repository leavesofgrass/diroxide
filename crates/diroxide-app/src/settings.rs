//! The settings schema: every setting's key, name, help and default, which
//! generates the Settings list with F1 help and the reference page.
//!
//! Wave 1 (W1c) owns this module and generates `docs/settings-reference.md`
//! from it.

/// One setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingInfo {
    /// The key in `config.toml`.
    pub key: &'static str,
    /// The name in the Settings list.
    pub name: &'static str,
    /// One sentence for F1 help.
    pub help: &'static str,
    /// The default, as written in TOML.
    pub default: &'static str,
}

/// Every setting, in Settings list order.
pub const SCHEMA: [SettingInfo; 9] = [
    SettingInfo {
        key: "show-hidden",
        name: "Show hidden files",
        help: "Lists entries that are hidden by their name or attributes.",
        default: "false",
    },
    SettingInfo {
        key: "folders-first",
        name: "Folders first",
        help: "Lists folders before files.",
        default: "true",
    },
    SettingInfo {
        key: "sort",
        name: "Sort by",
        help: "Sorts new panels by name, size, modified, type or extension.",
        default: "\"name\"",
    },
    SettingInfo {
        key: "columns",
        name: "Columns",
        help: "Says these details after each name, in order.",
        default: "[\"type\", \"size\", \"modified\"]",
    },
    SettingInfo {
        key: "single-letter-keys",
        name: "Single-letter keys",
        help: "Lets keys without Control or Alt run commands.",
        default: "true",
    },
    SettingInfo {
        key: "screen-reader",
        name: "Screen reader mode",
        help: "Behaves for a screen reader: auto, on or off.",
        default: "\"auto\"",
    },
    SettingInfo {
        key: "quiet-screen",
        name: "Quiet screen",
        help: "Draws as little as possible.",
        default: "false",
    },
    SettingInfo {
        key: "theme",
        name: "Theme",
        help: "Plain has no box drawing; lines adds it.",
        default: "\"plain\"",
    },
    SettingInfo {
        key: "progress-interval-ms",
        name: "Progress interval",
        help: "The shortest time between two progress sentences, in milliseconds.",
        default: "500",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_config::Settings;

    #[test]
    fn every_default_matches_the_config_defaults() {
        let mut text = String::new();
        for setting in &SCHEMA {
            text.push_str(&format!("{} = {}\n", setting.key, setting.default));
        }
        assert_eq!(Settings::from_toml(&text).unwrap(), Settings::default());
    }
}
