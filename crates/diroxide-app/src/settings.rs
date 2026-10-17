//! The Settings list: every setting of the schema as a row, name first and
//! its value in words after it ("Layout, side by side"), with F1 help that
//! says what the setting does, its default and what it can hold. The
//! schema itself lives in the configuration crate, which checks
//! `config.toml` against it; `cargo xtask regen` writes
//! `docs/settings-reference.md` from the same table.
//!
//! The theme's row also says its contrast verdict with the owner's colors
//! applied: "Theme, Galaxy, meets AA".

pub use diroxide_config::schema::{
    Choice, SCHEMA, SettingGroup, SettingInfo, SettingKind, list_words, setting,
};
pub use diroxide_config::theme::builtin_names as theme_names;
pub use diroxide_config::{BUILTIN_SPLIT_PRESETS, ColumnKind, DRIVE_KINDS, SplitPreset};
pub use diroxide_theme::PanelRole;

use diroxide_config::theme::{self, ColorOverride};
use diroxide_config::{Paths, Settings};
use diroxide_core::{ListModel, ListRow};

/// The theme the settings choose, with the owner's colors, and the verdict
/// in words: "Theme Galaxy, meets AA". An unknown theme name falls back to
/// the default, as the startup problems already said.
pub fn theme_line(settings: &Settings, paths: Option<&Paths>) -> String {
    let registry = match paths {
        Some(paths) => theme::registry(&paths.themes_dir(), Paths::SETTINGS).0,
        None => diroxide_theme::Registry::builtin(),
    };
    let (chosen, _) = theme::resolve(&registry, &settings.theme, Paths::SETTINGS);
    let (overrides, _): (Vec<ColorOverride>, _) =
        theme::read_overrides(&settings.colors, Paths::SETTINGS);
    theme::chosen_line(chosen, &overrides)
}

/// The value said in the Settings list, with "comes with" for a setting
/// that is reserved for a later alpha.
fn value_details(settings: &Settings, info: &SettingInfo, theme_verdict: &str) -> Vec<String> {
    let mut details = vec![settings.value_words(info.key)];
    if info.key == "theme" && !theme_verdict.is_empty() {
        details.push(theme_verdict.to_owned());
    }
    if let Some(alpha) = info.arrives {
        details.push(format!("comes with {alpha}"));
    }
    details
}

/// The filter language's options from the settings (W3a): `filter_case`
/// as the case rule and `week_starts_on` as the first day of the week.
pub fn filter_options(settings: &Settings) -> diroxide_core::filter::FilterOptions {
    use diroxide_config::{FilterCase, WeekStart};
    use diroxide_core::Weekday;
    use diroxide_core::pattern::CaseRule;
    diroxide_core::filter::FilterOptions {
        case: match settings.filter_case {
            FilterCase::Smart => CaseRule::Smart,
            FilterCase::Ignore => CaseRule::Ignore,
            FilterCase::Match => CaseRule::Match,
        },
        week_start: match settings.week_starts_on {
            WeekStart::Sunday => Weekday::Sunday,
            WeekStart::Monday => Weekday::Monday,
            WeekStart::Saturday => Weekday::Saturday,
        },
    }
}

/// The id of the Settings list's row that opens the Keys list (W3k).
pub const KEYS_ROW: &str = "\u{1}keys";

/// The Settings list, one row per setting in schema order: the name, then
/// the value in words. Each row's id is the setting's key, so F1 on it
/// gives [`setting_help`] and Enter can change it. The Keys list's row,
/// "Keys list, every command and its key", comes before the keys'
/// settings; its id is [`KEYS_ROW`].
pub fn settings_list(settings: &Settings) -> ListModel {
    let line = theme_line(settings, None);
    let verdict = line.split_once(", ").map_or("", |(_, v)| v).to_owned();
    let mut rows = Vec::with_capacity(SCHEMA.len() + 1);
    for info in &SCHEMA {
        if info.key == "single_letters" {
            rows.push(
                ListRow::new(KEYS_ROW, "Keys list").with_details(["every command and its key"]),
            );
        }
        rows.push(
            ListRow::new(info.key, info.name).with_details(value_details(settings, info, &verdict)),
        );
    }
    let mut list = ListModel::new("Settings").with_rows(rows);
    list.set_help("Up and Down choose, Enter changes, F1 says what a setting does, Escape closes.");
    list
}

/// F1 help for one setting: its help, its group, its default and what it
/// can hold, in plain sentences.
pub fn setting_help(key: &str) -> Option<String> {
    let info = setting(key)?;
    let default = match info.kind {
        SettingKind::Choice(choices) => {
            let value = info.default.trim_matches('"');
            choices
                .iter()
                .find(|c| c.value == value)
                .map_or_else(|| value.to_owned(), |c| c.words.to_owned())
        }
        SettingKind::Flag if info.default == "true" => "on".to_owned(),
        SettingKind::Flag => "off".to_owned(),
        SettingKind::SplitPresets => format!(
            "the built-in presets, {}",
            Settings::default().value_words(info.key)
        ),
        _ if info.default == "\"\"" || info.default == "[]" || info.default == "{}" => {
            "none".to_owned()
        }
        _ => info.default.trim_matches('"').to_owned(),
    };
    let mut text = format!(
        "{}: {} In {}. Default: {default}. Holds {}.",
        info.name,
        info.help,
        info.group.title(),
        info.kind.words()
    );
    if let Some(alpha) = info.arrives {
        text.push_str(&format!(" Kept now; it comes with {alpha}."));
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    #[test]
    fn the_list_says_each_setting_and_its_value() {
        let settings = Settings::default();
        let list = settings_list(&settings);
        assert_eq!(list.len(), SCHEMA.len() + 1, "and the Keys list's row");
        assert_eq!(
            list.announce_focus(),
            format!("Layout, side by side, 1 of {}", SCHEMA.len() + 1)
        );
        let keys = list.rows().iter().position(|r| r.id == KEYS_ROW).unwrap();
        assert_eq!(list.rows()[keys + 1].id, "single_letters");
        assert_eq!(
            list.rows()[keys].spoken(),
            "Keys list, every command and its key"
        );
        let theme = list.rows().iter().find(|r| r.id == "theme").unwrap();
        assert_eq!(theme.spoken(), "Theme, galaxy, meets AA");
        let preset = list.rows().iter().find(|r| r.id == "preset").unwrap();
        assert_eq!(preset.spoken(), "Key preset, diroxide");
        for row in list.rows() {
            assert!(within_cells(&row.spoken(), &row.label), "{}", row.spoken());
        }
    }

    #[test]
    fn f1_says_what_a_setting_does() {
        assert_eq!(
            setting_help("progress_interval_seconds").unwrap(),
            "Progress interval: In screen reader mode, the shortest time between two progress lines. A job's start, its end and its questions are said at once. In Speech and Braille. Default: 10. Holds a number of seconds, 1 to 600."
        );
        assert!(
            setting_help("layout")
                .unwrap()
                .contains("Default: side by side.")
        );
        assert!(
            setting_help("plugins.enabled")
                .unwrap()
                .ends_with("it comes with alpha.4.")
        );
        assert!(setting_help("nothing").is_none());
        let presets = setting_help("split_presets").unwrap();
        assert!(
            presets.contains("Default: the built-in presets, Email attachment, GitHub"),
            "{presets}"
        );
        assert!(
            !presets.contains("bytes = "),
            "no TOML in the help: {presets}"
        );
        for info in &SCHEMA {
            let help = setting_help(info.key).unwrap();
            assert!(within_cells(&help, info.name), "{help}");
        }
    }

    #[test]
    fn the_theme_line_counts_the_owners_colors() {
        let mut settings = Settings::default();
        assert_eq!(theme_line(&settings, None), "Theme Galaxy, meets AA");
        let page = diroxide_theme::builtin::default_theme()
            .color(diroxide_theme::ColorRole::Background)
            .hex();
        settings.colors.insert("folder".to_owned(), page);
        assert_eq!(
            theme_line(&settings, None),
            "Theme Galaxy, below AA: 1 check falls short"
        );
    }
}
