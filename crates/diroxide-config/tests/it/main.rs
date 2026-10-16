//! Integration tests of the configuration's contract, on injected folders
//! only: no test reads or writes the real config or state folder.

#![allow(clippy::unwrap_used)]

use std::fs;

use diroxide_config::{
    KeymapConfig, Layout, Paths, SCHEMA, Settings, SingleLetters, load, load_session,
    save_settings, set_setting,
};
use diroxide_core::SortKey;

fn folders() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    (dir, paths)
}

#[test]
fn settings_and_keymap_read_together() {
    let settings =
        Settings::from_toml("single_letters = \"off\"\n[sort]\nkey = \"modified\"\n").unwrap();
    assert_eq!(settings.sort.key, SortKey::Modified);
    assert_eq!(settings.single_letters, SingleLetters::Off);
    let keymap = KeymapConfig::from_toml("[terminal.global]\n\"app.quit\" = \"none\"\n").unwrap();
    assert_eq!(keymap.overrides.len(), 1);
    assert!(keymap.overrides[0].keys.is_empty());
}

#[test]
fn a_whole_folder_loads_saves_and_loads_again() {
    let (_dir, paths) = folders();
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(
        paths.config_file(Paths::SETTINGS),
        "# The owner's notes.\nlayout = \"stacked\"\n",
    )
    .unwrap();
    fs::write(
        paths.config_file(Paths::KEYMAP),
        "# Mine.\n[terminal.panel]\n\"file.copy\" = \"f5\"\n",
    )
    .unwrap();
    let config = load(&paths);
    assert_eq!(config.problems, Vec::new());
    assert_eq!(config.settings.layout, Layout::Stacked);
    assert_eq!(config.keymap.overrides.len(), 1);
    let mut settings = config.settings;
    settings.theme = "nord".to_owned();
    save_settings(&paths, &settings).unwrap();
    set_setting(
        &paths,
        "progress_interval_seconds",
        toml::Value::Integer(20),
    )
    .unwrap();
    let text = fs::read_to_string(paths.config_file(Paths::SETTINGS)).unwrap();
    assert!(
        text.starts_with("# The owner's notes.\nlayout = \"stacked\"\n"),
        "{text}"
    );
    let again = load(&paths);
    assert_eq!(again.settings.theme, "nord");
    assert_eq!(again.settings.progress_interval_seconds, 20);
    assert_eq!(again.problems, Vec::new());
    let session = load_session(&paths);
    assert!(session.problems.is_empty());
}

#[test]
fn the_schema_covers_every_setting_this_wave_and_the_reserved_ones() {
    for key in [
        "layout",
        "screen_reader_mode",
        "progress_interval_seconds",
        "single_letters",
        "theme",
        "follow_system_appearance",
        "colors",
        "show_hidden",
        "sort.key",
        "confirm.trash",
        "trash",
        "trash_drives",
        "portable_names",
        "secret_show_toggle",
        "filter_case",
        "week_starts_on",
        "filter_pause_ms",
        "search_row_cap",
        "search_respect_ignore",
        "preset",
        "completion_list",
        "dialog_settle_ms",
        "auto_hide_menu",
        "focus_mode",
        "compact",
        "highlighting",
        "thumbnails",
        "cursor_fallback",
        "components_source",
        "borders",
    ] {
        assert!(SCHEMA.iter().any(|s| s.key == key), "{key}");
    }
    let settle = SCHEMA.iter().find(|s| s.key == "dialog_settle_ms").unwrap();
    assert_eq!(settle.default, "250");
}
