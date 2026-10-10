//! The keymap: chords bound to commands in layers, from the catalog's
//! defaults and `keymap.toml`'s overrides. A key is looked up in the most
//! specific layer first. Two-key chords work in the Emacs style: after the
//! first key, the status line says the continuations.
//!
//! Wave 1 (W1c) owns this module, and completes the check against NVDA's
//! and JAWS's default keys.

use diroxide_config::KeymapConfig;
use diroxide_core::{CommandId, Key, KeyChord, KeyCode, Layer};

use crate::commands::{self, CATALOG};

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Where it applies.
    pub layer: Layer,
    /// The key or chord.
    pub chord: KeyChord,
    /// The command it runs.
    pub command: CommandId,
}

/// What the keys pressed so far mean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// A whole chord: run this command.
    Command(CommandId),
    /// The start of one or more chords: wait for the next key. Lists each
    /// next key with the command it would run.
    Prefix(Vec<(Key, CommandId)>),
    /// Nothing is bound to these keys.
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
                if let Ok(chord) = KeyChord::parse(keys) {
                    bindings.push(Binding {
                        layer: *layer,
                        chord,
                        command: info.id.clone(),
                    });
                }
            }
        }
        Self { bindings }
    }

    /// The same keymap with `keymap.toml`'s overrides applied: each
    /// override first removes what the chord had in its layer, then binds
    /// the new command, if any.
    pub fn with_overrides(mut self, config: &KeymapConfig) -> Self {
        for change in &config.overrides {
            self.bindings
                .retain(|b| !(b.layer == change.layer && b.chord == change.chord));
            if let Some(command) = &change.command {
                self.bindings.push(Binding {
                    layer: change.layer,
                    chord: change.chord.clone(),
                    command: command.clone(),
                });
            }
        }
        self
    }

    /// The same keymap without single-character shortcuts, as WCAG 2.1.4
    /// lets the user choose.
    pub fn without_single_characters(mut self) -> Self {
        self.bindings
            .retain(|b| !(b.chord.keys().len() == 1 && b.chord.keys()[0].is_single_character()));
        self
    }

    /// Every binding.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// What the keys pressed so far mean, looking in `layers` in order,
    /// most specific first.
    pub fn lookup(&self, layers: &[Layer], pressed: &[Key]) -> Lookup {
        for layer in layers {
            let in_layer = self.bindings.iter().filter(|b| b.layer == *layer);
            if let Some(exact) = in_layer.clone().find(|b| b.chord.keys() == pressed) {
                return Lookup::Command(exact.command.clone());
            }
            let continuations: Vec<(Key, CommandId)> = in_layer
                .filter(|b| b.chord.starts_with(pressed))
                .map(|b| (b.chord.keys()[pressed.len()], b.command.clone()))
                .collect();
            if !continuations.is_empty() {
                return Lookup::Prefix(continuations);
            }
        }
        Lookup::NoMatch
    }

    /// The chords bound to a command, in any layer.
    pub fn keys_for(&self, command: &CommandId) -> Vec<&KeyChord> {
        self.bindings
            .iter()
            .filter(|b| &b.command == command)
            .map(|b| &b.chord)
            .collect()
    }

    /// The first key of a command as said ("F5"), or an empty string.
    pub fn key_text(&self, command: &CommandId) -> String {
        self.keys_for(command)
            .first()
            .map(ToString::to_string)
            .unwrap_or_default()
    }

    /// Chords bound twice in one layer, in words.
    pub fn conflicts(&self) -> Vec<String> {
        let mut found = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.layer == b.layer && a.chord == b.chord && a.command != b.command {
                    found.push(format!(
                        "{} in the {} layer runs both {} and {}",
                        a.chord,
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
        let taken: Vec<(KeyChord, &str)> = SCREEN_READER_KEYS
            .iter()
            .filter_map(|(keys, why)| KeyChord::parse(keys).ok().map(|c| (c, *why)))
            .collect();
        let mut found = Vec::new();
        for binding in &self.bindings {
            if binding
                .chord
                .keys()
                .iter()
                .any(|k| k.code == KeyCode::Insert)
            {
                found.push(format!(
                    "{} uses Insert, the screen reader key",
                    binding.chord
                ));
            }
            if let Some((_, why)) = taken.iter().find(|(chord, _)| *chord == binding.chord) {
                found.push(format!("{} {why}", binding.chord));
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
            keymap.lookup(&PANEL, &[key("f5")]),
            Lookup::Command(ids::COPY)
        );
        assert_eq!(
            keymap.lookup(&PANEL, &[key("down")]),
            Lookup::Command(ids::LIST_DOWN)
        );
        assert_eq!(
            keymap.lookup(&PANEL, &[key("ctrl+q")]),
            Lookup::Command(ids::QUIT)
        );
        assert_eq!(
            keymap.lookup(&[Layer::Global], &[key("f5")]),
            Lookup::NoMatch
        );
    }

    #[test]
    fn chords_wait_for_their_second_key() {
        let config = KeymapConfig {
            overrides: vec![KeyOverride {
                layer: Layer::Global,
                chord: KeyChord::parse("ctrl+x ctrl+f").unwrap(),
                command: Some(ids::FIND),
            }],
        };
        let keymap = Keymap::defaults().with_overrides(&config);
        assert_eq!(
            keymap.lookup(&PANEL, &[key("ctrl+x")]),
            Lookup::Prefix(vec![(key("ctrl+f"), ids::FIND)])
        );
        assert_eq!(
            keymap.lookup(&PANEL, &[key("ctrl+x"), key("ctrl+f")]),
            Lookup::Command(ids::FIND)
        );
    }

    #[test]
    fn overrides_unbind_and_single_characters_can_go() {
        let config = KeymapConfig {
            overrides: vec![KeyOverride {
                layer: Layer::Panel,
                chord: KeyChord::parse("f5").unwrap(),
                command: None,
            }],
        };
        let keymap = Keymap::defaults().with_overrides(&config);
        assert_eq!(keymap.lookup(&PANEL, &[key("f5")]), Lookup::NoMatch);
        let with_letter = Keymap::defaults().with_overrides(&KeymapConfig {
            overrides: vec![KeyOverride {
                layer: Layer::Panel,
                chord: KeyChord::parse("q").unwrap(),
                command: Some(ids::QUIT),
            }],
        });
        assert_eq!(
            with_letter.lookup(&PANEL, &[key("q")]),
            Lookup::Command(ids::QUIT)
        );
        let without = with_letter.without_single_characters();
        assert_eq!(without.lookup(&PANEL, &[key("q")]), Lookup::NoMatch);
        assert_eq!(
            without.lookup(&PANEL, &[key("space")]),
            Lookup::Command(ids::SELECT_TOGGLE)
        );
    }
}
