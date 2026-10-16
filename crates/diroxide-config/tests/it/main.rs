//! Integration tests of the configuration's contract.

use diroxide_config::{KeymapConfig, Settings, SortKey};

#[test]
fn settings_and_keymap_read_together() {
    let settings =
        Settings::from_toml("sort = \"modified\"\nsingle-letter-keys = false\n").unwrap();
    assert_eq!(settings.sort, SortKey::Modified);
    assert!(!settings.single_letter_keys);
    let keymap = KeymapConfig::from_toml("[terminal.global]\n\"app.quit\" = \"none\"\n").unwrap();
    assert_eq!(keymap.overrides.len(), 1);
    assert!(keymap.overrides[0].keys.is_empty());
}
