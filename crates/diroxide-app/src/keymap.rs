//! The keymap: keys bound to commands in layers, from the catalog's
//! defaults and `keymap.toml`'s lines. A key is looked up in the most
//! specific layer first. A binding is one key with its modifiers; there
//! are no sequences (plan question 56).
//!
//! Wave 1 (W1c) owns this module: it builds the two columns (terminal and
//! window), completes the check against NVDA's and JAWS's default keys,
//! and reports every problem in `keymap.toml` in words. Phase 0 keeps one
//! column, the terminal's, which is the frontend alpha.1 ships.

use diroxide_config::KeymapConfig;
use diroxide_core::{CommandId, Key, KeyCode, KeyColumn, Layer};

use crate::commands::{self, CATALOG};

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Where it applies.
    pub layer: Layer,
    /// The key, with its modifiers.
    pub key: Key,
    /// The command it runs.
    pub command: CommandId,
}

/// What a key press means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Run this command.
    Command(CommandId),
    /// Nothing is bound to the key.
    NoMatch,
}

/// Keys that screen readers take for themselves, which the keymap must not
/// bind. Phase 0 lists a few; W1c completes it from NVDA's and JAWS's
/// documented defaults.
pub const SCREEN_READER_KEYS: [(&str, &str); 3] = [
    ("ctrl+alt+n", "starts NVDA"),
    ("ctrl+alt+j", "starts JAWS"),
    ("ctrl+alt+s", "is taken by some screen reader setups"),
];

/// The keymap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// The default keys from the command catalog.
    pub fn defaults() -> Self {
        let mut bindings = Vec::new();
        for info in &CATALOG {
            for (layer, keys) in info.keys {
                if let Ok(key) = Key::parse(keys) {
                    bindings.push(Binding {
                        layer: *layer,
                        key,
                        command: info.id.clone(),
                    });
                }
            }
        }
        Self { bindings }
    }

    /// The same keymap with `keymap.toml`'s terminal column applied: each
    /// line replaces its command's keys in its layer, and takes each of its
    /// keys from whatever command had it there. A line with no keys
    /// unbinds the command.
    pub fn with_overrides(mut self, config: &KeymapConfig) -> Self {
        for line in config.column(KeyColumn::Terminal) {
            self.bindings.retain(|b| {
                !(b.layer == line.layer
                    && (b.command == line.command || line.keys.contains(&b.key)))
            });
            for key in &line.keys {
                self.bindings.push(Binding {
                    layer: line.layer,
                    key: *key,
                    command: line.command.clone(),
                });
            }
        }
        self
    }

    /// The same keymap without single-character shortcuts, as WCAG 2.1.4
    /// lets the user choose.
    pub fn without_single_characters(mut self) -> Self {
        self.bindings.retain(|b| !b.key.is_single_character());
        self
    }

    /// Every binding.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// What a key means, looking in `layers` in order, most specific first.
    pub fn lookup(&self, layers: &[Layer], key: &Key) -> Lookup {
        for layer in layers {
            if let Some(found) = self
                .bindings
                .iter()
                .find(|b| b.layer == *layer && b.key == *key)
            {
                return Lookup::Command(found.command.clone());
            }
        }
        Lookup::NoMatch
    }

    /// The keys bound to a command, in any layer.
    pub fn keys_for(&self, command: &CommandId) -> Vec<&Key> {
        self.bindings
            .iter()
            .filter(|b| &b.command == command)
            .map(|b| &b.key)
            .collect()
    }

    /// The first key of a command as said ("F5"), or an empty string.
    pub fn key_text(&self, command: &CommandId) -> String {
        self.keys_for(command)
            .first()
            .map(ToString::to_string)
            .unwrap_or_default()
    }

    /// Keys bound twice in one layer, in words.
    pub fn conflicts(&self) -> Vec<String> {
        let mut found = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.layer == b.layer && a.key == b.key && a.command != b.command {
                    found.push(format!(
                        "{} in the {} layer runs both {} and {}",
                        a.key,
                        a.layer,
                        commands::name_of(&a.command),
                        commands::name_of(&b.command)
                    ));
                }
            }
        }
        found
    }

    /// Bindings that a screen reader would take first, in words. Insert is
    /// always one, because it is the NVDA and JAWS key.
    pub fn screen_reader_conflicts(&self) -> Vec<String> {
        let taken: Vec<(Key, &str)> = SCREEN_READER_KEYS
            .iter()
            .filter_map(|(keys, why)| Key::parse(keys).ok().map(|k| (k, *why)))
            .collect();
        let mut found = Vec::new();
        for binding in &self.bindings {
            if binding.key.code == KeyCode::Insert {
                found.push(format!(
                    "{} uses Insert, the screen reader key",
                    binding.key
                ));
            }
            if let Some((_, why)) = taken.iter().find(|(key, _)| *key == binding.key) {
                found.push(format!("{} {why}", binding.key));
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ids;
    use diroxide_config::KeyOverride;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    const PANEL: [Layer; 3] = [Layer::Panel, Layer::List, Layer::Global];

    fn line(layer: Layer, command: CommandId, keys: &[&str]) -> KeyOverride {
        KeyOverride {
            column: KeyColumn::Terminal,
            layer,
            command,
            keys: keys.iter().map(|k| key(k)).collect(),
        }
    }

    #[test]
    fn the_defaults_are_clean() {
        let keymap = Keymap::defaults();
        assert_eq!(keymap.conflicts(), Vec::<String>::new());
        assert_eq!(keymap.screen_reader_conflicts(), Vec::<String>::new());
        assert_eq!(keymap.key_text(&ids::COPY), "F5");
    }

    #[test]
    fn keys_are_found_in_the_most_specific_layer() {
        let keymap = Keymap::defaults();
        assert_eq!(
            keymap.lookup(&PANEL, &key("f5")),
            Lookup::Command(ids::COPY)
        );
        assert_eq!(
            keymap.lookup(&PANEL, &key("down")),
            Lookup::Command(ids::LIST_DOWN)
        );
        assert_eq!(
            keymap.lookup(&PANEL, &key("ctrl+q")),
            Lookup::Command(ids::QUIT)
        );
        assert_eq!(keymap.lookup(&[Layer::Global], &key("f5")), Lookup::NoMatch);
    }

    #[test]
    fn a_line_moves_a_command_to_new_keys() {
        let config = KeymapConfig {
            overrides: vec![line(Layer::Panel, ids::FIND, &["ctrl+f"])],
        };
        let keymap = Keymap::defaults().with_overrides(&config);
        assert_eq!(
            keymap.lookup(&PANEL, &key("ctrl+f")),
            Lookup::Command(ids::FIND)
        );
        assert_eq!(keymap.keys_for(&ids::FIND), vec![&key("ctrl+f")]);
        let window_only = KeymapConfig {
            overrides: vec![KeyOverride {
                column: KeyColumn::Window,
                ..line(Layer::Panel, ids::COPY, &[])
            }],
        };
        let keymap = Keymap::defaults().with_overrides(&window_only);
        assert_eq!(
            keymap.lookup(&PANEL, &key("f5")),
            Lookup::Command(ids::COPY)
        );
    }

    #[test]
    fn overrides_unbind_and_single_characters_can_go() {
        let config = KeymapConfig {
            overrides: vec![line(Layer::Panel, ids::COPY, &[])],
        };
        let keymap = Keymap::defaults().with_overrides(&config);
        assert_eq!(keymap.lookup(&PANEL, &key("f5")), Lookup::NoMatch);
        let with_letter = Keymap::defaults().with_overrides(&KeymapConfig {
            overrides: vec![line(Layer::Panel, ids::QUIT, &["q"])],
        });
        assert_eq!(
            with_letter.lookup(&PANEL, &key("q")),
            Lookup::Command(ids::QUIT)
        );
        let without = with_letter.without_single_characters();
        assert_eq!(without.lookup(&PANEL, &key("q")), Lookup::NoMatch);
        assert_eq!(
            without.lookup(&PANEL, &key("space")),
            Lookup::Command(ids::SELECT_TOGGLE)
        );
    }
}
