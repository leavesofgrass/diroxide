//! The keymap: keys bound to commands in layers, from the catalog's
//! defaults and `keymap.toml`'s lines, one column at a time (the terminal's
//! or the window's). A key is looked up in the most specific layer first.
//! A binding is one key with its modifiers; there are no sequences (plan
//! question 56).
//!
//! On macOS the window column uses Command where the others use Control,
//! leaves Ctrl+F3 to Ctrl+F7 to the system (the Sort list, the View menu
//! and the palette sort there), and leaves Ctrl+Space to the system too.
//!
//! The checks say each problem in words, meaning first: a key the
//! terminal cannot deliver ([`tables`]), a key a screen reader owns, a key
//! kept for a later alpha, and a key another command already has, which
//! the Keys list offers as "take it, swap, or cancel".

pub mod names;
pub mod tables;

pub use names::{Platform, spoken, written};

use diroxide_config::{ConfigProblem, KeyOverride, KeymapConfig, Paths, SingleLetters};
use diroxide_core::{CommandId, Key, KeyCode, KeyColumn, Layer};

use crate::commands::{self, CATALOG};
use tables::{ReaderKind, TerminalKind};

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

/// Keys that start screen readers, kept from Phase 0 for callers that
/// list them; the full table is [`tables::screen_reader_table`].
pub const SCREEN_READER_KEYS: [(&str, &str); 3] = [
    ("ctrl+alt+n", "starts NVDA"),
    ("ctrl+alt+j", "starts JAWS"),
    ("ctrl+alt+s", "is taken by some screen reader setups"),
];

/// One problem with a key for a command, as the Keys list, `diro keys
/// check` and the startup check say it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyConcern {
    /// Insert, the screen readers' modifier key: refused.
    ScreenReaderKey,
    /// The terminal cannot deliver the key.
    TerminalCannot {
        /// Why, in words.
        why: String,
    },
    /// A terminal program may keep the key.
    TerminalProgram {
        /// The program.
        program: String,
        /// What it does with the key.
        why: String,
    },
    /// Some terminals send the key as another.
    ArrivesDifferently {
        /// How, in words.
        why: String,
    },
    /// A screen reader takes the key.
    ScreenReaderOwns {
        /// The readers, in words.
        readers: String,
    },
    /// A screen reader takes the key with Num Lock off.
    NeedsNumLock {
        /// The readers, in words.
        readers: String,
    },
    /// The key is kept for a later alpha's command.
    Reserved {
        /// The command's name.
        name: String,
        /// The alpha that brings it.
        alpha: String,
    },
    /// Another command already has the key in this layer.
    AlreadyUsed {
        /// That command.
        command: CommandId,
    },
    /// A single printable key, which works only while letters run commands
    /// (WCAG 2.1.4).
    SingleLetter,
}

impl KeyConcern {
    /// The sentence, meaning first: "Windows Terminal keeps Ctrl+V: it
    /// pastes text, so the key never arrives."
    pub fn sentence(&self, key: &Key, platform: Platform) -> String {
        let k = written(key, platform);
        match self {
            KeyConcern::ScreenReaderKey => "Insert is the screen reader key.".to_owned(),
            KeyConcern::TerminalCannot { why } => {
                format!("The terminal cannot send {k}: {why}.")
            }
            KeyConcern::TerminalProgram { program, why } => format!("{program} keeps {k}: {why}."),
            KeyConcern::ArrivesDifferently { why } => {
                format!("{k} may arrive as another key: {why}.")
            }
            KeyConcern::ScreenReaderOwns { readers } => {
                format!("Screen readers keep {k}: {readers}.")
            }
            KeyConcern::NeedsNumLock { readers } => {
                format!("Num Lock on needed for {k}: {readers} review with it off.")
            }
            KeyConcern::Reserved { name, alpha } => {
                format!("{k} is kept for {name}, coming in {alpha}.")
            }
            KeyConcern::AlreadyUsed { command } => {
                format!("{k} already runs {}.", commands::name_of(command))
            }
            KeyConcern::SingleLetter => {
                "A letter key works only while letters run commands.".to_owned()
            }
        }
    }

    /// Whether the key must not be bound at all: only Insert.
    pub fn refuses(&self) -> bool {
        matches!(self, KeyConcern::ScreenReaderKey)
    }
}

/// What the owner chose when a key is taken: the plan's "take it, swap, or
/// cancel" (two commands cannot share a key in one layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyChoice {
    /// The other command loses the key.
    Take,
    /// The other command gets this command's old key.
    Swap,
    /// Nothing changes.
    Cancel,
}

/// The question when a key is taken, after the "already runs" sentence.
pub const TAKE_SWAP_CANCEL: &str = "Take it, swap, or cancel?";

/// The keymap of one column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    column: KeyColumn,
    platform: Platform,
    bindings: Vec<Binding>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

fn parse_defaults(keys: &[&str], layer: Layer, command: &CommandId, out: &mut Vec<Binding>) {
    for text in keys {
        if let Ok(key) = Key::parse(text) {
            out.push(Binding {
                layer,
                key,
                command: command.clone(),
            });
        }
    }
}

impl Keymap {
    /// The terminal column's default keys from the command catalog: the
    /// keymap alpha.1's frontend uses.
    pub fn defaults() -> Self {
        Self::defaults_for(KeyColumn::Terminal)
    }

    /// One column's default keys from the command catalog. The window
    /// column is the Windows and Linux one; [`Keymap::for_platform`] makes
    /// the macOS one.
    pub fn defaults_for(column: KeyColumn) -> Self {
        let mut bindings = Vec::new();
        for info in &CATALOG {
            let keys = match column {
                KeyColumn::Terminal => info.terminal,
                KeyColumn::Window => info.window,
            };
            parse_defaults(keys, info.layer, &info.id, &mut bindings);
        }
        Self {
            column,
            platform: Platform::Other,
            bindings,
        }
    }

    /// The same keymap for a system. In the window column on macOS,
    /// Command replaces Control, and the keys the system keeps are left
    /// out: Ctrl+F3 to Ctrl+F8 (the Sort list, the View menu and the
    /// palette sort instead) and Ctrl+Space. The terminal column keeps
    /// Control everywhere, because the terminal program owns Command.
    pub fn for_platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        if self.column == KeyColumn::Window && platform == Platform::MacOs {
            self.bindings.retain(|b| {
                let system_f_key = b.key.mods.ctrl
                    && !b.key.mods.alt
                    && !b.key.mods.shift
                    && matches!(b.key.code, KeyCode::F(3..=8));
                let ctrl_space = b.key.mods.ctrl && b.key.code == KeyCode::Char(' ');
                !system_f_key && !ctrl_space
            });
            for binding in &mut self.bindings {
                if binding.key.mods.ctrl {
                    binding.key.mods.ctrl = false;
                    binding.key.mods.meta = true;
                }
            }
        }
        self
    }

    /// The same keymap with `keymap.toml`'s lines for this column applied:
    /// each line replaces its command's keys in its layer, and takes each of
    /// its keys from whatever command had it there. A line with no keys
    /// unbinds the command. Lines for unknown commands, and Insert, are not
    /// bound; [`check_keymap`] says so in words.
    pub fn with_overrides(mut self, config: &KeymapConfig) -> Self {
        for line in config.column(self.column) {
            if commands::info(&line.command).is_none() {
                continue;
            }
            let keys: Vec<Key> = line
                .keys
                .iter()
                .copied()
                .filter(|k| k.code != KeyCode::Insert)
                .collect();
            self.bindings.retain(|b| {
                !(b.layer == line.layer && (b.command == line.command || keys.contains(&b.key)))
            });
            for key in keys {
                self.bindings.push(Binding {
                    layer: line.layer,
                    key,
                    command: line.command.clone(),
                });
            }
        }
        self
    }

    /// The same keymap for the single-letters setting. Type-ahead drops the
    /// letter and digit bindings, so those keys jump to names; off drops
    /// every single-character binding (WCAG 2.1.4); commands keeps them
    /// all. A dialog's own keys (y, n) stay, since they act only while the
    /// dialog has the focus.
    pub fn with_single_letters(mut self, mode: SingleLetters) -> Self {
        self.bindings.retain(|b| {
            if b.layer == Layer::Dialog || !b.key.is_single_character() {
                return true;
            }
            match mode {
                SingleLetters::Commands => true,
                SingleLetters::Off => false,
                SingleLetters::TypeAhead => {
                    !matches!(b.key.code, KeyCode::Char(c) if c.is_alphanumeric())
                }
            }
        });
        self
    }

    /// The same keymap without single-character shortcuts, as WCAG 2.1.4
    /// lets the user choose.
    pub fn without_single_characters(self) -> Self {
        self.with_single_letters(SingleLetters::Off)
    }

    /// The column.
    pub fn column(&self) -> KeyColumn {
        self.column
    }

    /// The system the keys are named for.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Every binding.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// What a key means, looking in `layers` in order, most specific first.
    pub fn lookup(&self, layers: &[Layer], key: &Key) -> Lookup {
        for layer in layers {
            if let Some(found) = self.binding(*layer, key) {
                return Lookup::Command(found.command.clone());
            }
        }
        Lookup::NoMatch
    }

    /// The binding of a key in one layer.
    pub fn binding(&self, layer: Layer, key: &Key) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.layer == layer && b.key == *key)
    }

    /// The keys bound to a command, in any layer, in order.
    pub fn keys_for(&self, command: &CommandId) -> Vec<&Key> {
        self.bindings
            .iter()
            .filter(|b| &b.command == command)
            .map(|b| &b.key)
            .collect()
    }

    /// The first key of a command as written ("F5", "Ctrl+F5"), or an
    /// empty string when it has none.
    pub fn key_text(&self, command: &CommandId) -> String {
        self.keys_for(command)
            .first()
            .map(|k| written(k, self.platform))
            .unwrap_or_default()
    }

    /// Every key of a command as written, joined: "F8 or Delete".
    pub fn keys_text(&self, command: &CommandId) -> String {
        self.keys_for(command)
            .iter()
            .map(|k| written(k, self.platform))
            .collect::<Vec<_>>()
            .join(" or ")
    }

    /// The first key of a command as said ("Control plus F5"), or an empty
    /// string when it has none. Messages that name a key take it from here.
    pub fn spoken_key(&self, command: &CommandId) -> String {
        self.keys_for(command)
            .first()
            .map(|k| spoken(k, self.platform))
            .unwrap_or_default()
    }

    /// Keys bound twice in one layer, in words.
    pub fn conflicts(&self) -> Vec<String> {
        let mut found = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.layer == b.layer && a.key == b.key && a.command != b.command {
                    found.push(format!(
                        "{} runs two commands in the {} layer: {} and {}.",
                        written(&a.key, self.platform),
                        a.layer,
                        commands::name_of(&a.command),
                        commands::name_of(&b.command)
                    ));
                }
            }
        }
        found
    }

    /// Bindings that a screen reader takes first, in words. Insert is
    /// always one, because it is the NVDA and JAWS key. Keys a reader takes
    /// only with Num Lock off are not listed here: they work with Num Lock
    /// on, and [`Keymap::only_num_lock_keys`] checks they are never alone.
    pub fn screen_reader_conflicts(&self) -> Vec<String> {
        let mut found = Vec::new();
        for binding in &self.bindings {
            for concern in self.reader_concerns(&binding.key) {
                if matches!(
                    concern,
                    KeyConcern::ScreenReaderKey | KeyConcern::ScreenReaderOwns { .. }
                ) {
                    found.push(format!(
                        "{} ({})",
                        concern.sentence(&binding.key, self.platform),
                        commands::name_of(&binding.command)
                    ));
                }
            }
        }
        found
    }

    /// Commands whose only keys work under a screen reader only with Num
    /// Lock on (the keypad), in words; empty when every such command has
    /// another key.
    pub fn only_num_lock_keys(&self) -> Vec<String> {
        let mut found = Vec::new();
        for info in &CATALOG {
            let keys = self.keys_for(&info.id);
            if !keys.is_empty()
                && keys.iter().all(|k| {
                    tables::reader_entries(k, self.platform)
                        .iter()
                        .any(|r| r.kind == ReaderKind::NumLock)
                })
            {
                found.push(format!("{} has only keypad keys.", info.name));
            }
        }
        found
    }

    fn reader_concerns(&self, key: &Key) -> Vec<KeyConcern> {
        if key.code == KeyCode::Insert {
            return vec![KeyConcern::ScreenReaderKey];
        }
        tables::reader_entries(key, self.platform)
            .into_iter()
            .map(|r| match r.kind {
                ReaderKind::Owned => KeyConcern::ScreenReaderOwns {
                    readers: r.readers.clone(),
                },
                ReaderKind::NumLock => KeyConcern::NeedsNumLock {
                    readers: r.readers.clone(),
                },
            })
            .collect()
    }

    /// Everything to say about binding `key` to `command` in `layer` of
    /// this column, in the order the Keys list asks: Insert, the terminal,
    /// the screen readers, a reserved key, a key already used, a single
    /// letter. Empty when the key is free and safe.
    pub fn concerns(&self, layer: Layer, key: &Key, command: &CommandId) -> Vec<KeyConcern> {
        let mut found = self.reader_concerns(key);
        if found.first().is_some_and(KeyConcern::refuses) {
            return found;
        }
        if self.column == KeyColumn::Terminal
            && let Some(row) = tables::terminal_entry(key)
        {
            let concern = match row.kind {
                TerminalKind::Unsafe => KeyConcern::TerminalCannot {
                    why: row.why.clone(),
                },
                TerminalKind::Program => KeyConcern::TerminalProgram {
                    program: row.program.clone(),
                    why: row.why.clone(),
                },
                TerminalKind::Differs => KeyConcern::ArrivesDifferently {
                    why: row.why.clone(),
                },
            };
            found.insert(0, concern);
        }
        if let Some(row) = tables::reserved_entry(key, self.column)
            && row.command != *command
        {
            found.push(KeyConcern::Reserved {
                name: row.name.clone(),
                alpha: row.alpha.clone(),
            });
        }
        if let Some(binding) = self.binding(layer, key)
            && binding.command != *command
        {
            found.push(KeyConcern::AlreadyUsed {
                command: binding.command.clone(),
            });
        }
        if layer != Layer::Dialog && key.is_single_character() {
            found.push(KeyConcern::SingleLetter);
        }
        found
    }

    /// Binds `key` to `command` in `layer` after the owner's choice, and
    /// says what happened. Take leaves the other command without the key;
    /// swap gives it this command's old first key in the layer.
    pub fn assign(
        &mut self,
        layer: Layer,
        key: Key,
        command: &CommandId,
        choice: KeyChoice,
    ) -> String {
        let k = written(&key, self.platform);
        let name = commands::name_of(command);
        if key.code == KeyCode::Insert {
            return KeyConcern::ScreenReaderKey.sentence(&key, self.platform);
        }
        if choice == KeyChoice::Cancel {
            return "Nothing changed.".to_owned();
        }
        let other = self
            .binding(layer, &key)
            .filter(|b| &b.command != command)
            .map(|b| b.command.clone());
        let old = self
            .bindings
            .iter()
            .find(|b| b.layer == layer && &b.command == command)
            .map(|b| b.key);
        self.bindings
            .retain(|b| !(b.layer == layer && (&b.command == command || b.key == key)));
        self.bindings.push(Binding {
            layer,
            key,
            command: command.clone(),
        });
        match (other, choice, old) {
            (None, _, _) => format!("{k} now runs {name}."),
            (Some(other), KeyChoice::Swap, Some(old)) => {
                self.bindings.push(Binding {
                    layer,
                    key: old,
                    command: other.clone(),
                });
                format!(
                    "{k} now runs {name}; {} moved to {}.",
                    commands::name_of(&other),
                    written(&old, self.platform)
                )
            }
            (Some(other), _, _) => format!(
                "{k} now runs {name}; {} has no key here.",
                commands::name_of(&other)
            ),
        }
    }

    /// The keymap as `keymap.toml` lines: every catalog command, with
    /// "none" for one that has no key in this column.
    pub fn lines(&self) -> Vec<KeyOverride> {
        let mut lines = Vec::new();
        for layer in Layer::ALL {
            for info in CATALOG.iter().filter(|i| i.layer == layer) {
                let keys: Vec<Key> = self
                    .bindings
                    .iter()
                    .filter(|b| b.layer == layer && b.command == info.id)
                    .map(|b| b.key)
                    .collect();
                lines.push(KeyOverride {
                    column: self.column,
                    layer,
                    command: info.id.clone(),
                    keys,
                    line: 0,
                });
            }
        }
        for binding in &self.bindings {
            let known = commands::info(&binding.command).is_some_and(|i| i.layer == binding.layer);
            if !known {
                match lines
                    .iter_mut()
                    .find(|l| l.layer == binding.layer && l.command == binding.command)
                {
                    Some(line) => line.keys.push(binding.key),
                    None => lines.push(KeyOverride {
                        column: self.column,
                        layer: binding.layer,
                        command: binding.command.clone(),
                        keys: vec![binding.key],
                        line: 0,
                    }),
                }
            }
        }
        lines
    }
}

/// Both columns' keymaps for a system, with `keymap.toml` applied.
pub fn keymaps(config: &KeymapConfig, platform: Platform) -> (Keymap, Keymap) {
    let terminal = Keymap::defaults_for(KeyColumn::Terminal)
        .for_platform(platform)
        .with_overrides(config);
    let window = Keymap::defaults_for(KeyColumn::Window)
        .for_platform(platform)
        .with_overrides(config);
    (terminal, window)
}

/// What the key test says for a key the terminal sent: "Got Control plus
/// F5. Runs Sort by modified." or "Got Control plus J. Nothing runs on
/// it."
pub fn key_test(keymap: &Keymap, layers: &[Layer], key: &Key) -> String {
    let said = spoken(key, keymap.platform());
    if key.code == KeyCode::Insert {
        return format!("Got {said}. Insert is the screen reader key.");
    }
    match keymap.lookup(layers, key) {
        Lookup::Command(id) => format!("Got {said}. Runs {}.", commands::name_of(&id)),
        Lookup::NoMatch => format!("Got {said}. Nothing runs on it."),
    }
}

/// What the key test says for a key typed by its name, the equal way in
/// for keys a terminal cannot capture: "hyper+q" gives "Not a key name:
/// hyper+q."
pub fn key_test_typed(keymap: &Keymap, layers: &[Layer], text: &str) -> String {
    match Key::parse(text) {
        Ok(key) => key_test(keymap, layers, &key),
        Err(diroxide_core::CoreError::KeySequence(_)) => format!(
            "Two keys in a row: {}. A key is one key with its modifiers.",
            text.trim()
        ),
        Err(_) => format!("Not a key name: {}.", text.trim()),
    }
}

/// Every problem with `keymap.toml`: the file's own (which the config
/// crate found while reading it) followed by the app's
/// ([`app_keymap_problems`]). `diro keys check FILE` says these.
pub fn check_keymap(config: &KeymapConfig, platform: Platform) -> Vec<ConfigProblem> {
    let mut problems = config.problems.clone();
    problems.extend(app_keymap_problems(config, platform));
    problems
}

/// The app's checks of `keymap.toml`, which need the catalog and the key
/// tables: unknown commands, Insert, keys the terminal cannot deliver in
/// the terminal column, keys a screen reader owns, a key given twice in a
/// table, and keys bound twice in a layer once the file is applied. Each
/// is one sentence, meaning first, with its line where it has one. At
/// startup the app adds these to `diroxide_config::load`'s problems, which
/// already hold the file's own.
pub fn app_keymap_problems(config: &KeymapConfig, platform: Platform) -> Vec<ConfigProblem> {
    let file = Paths::KEYMAP;
    let mut problems: Vec<ConfigProblem> = Vec::new();
    let at = |line: usize| {
        if line == 0 {
            String::new()
        } else {
            format!(", line {line}")
        }
    };
    for line in &config.overrides {
        if commands::info(&line.command).is_none() {
            problems.push(ConfigProblem::new(
                file,
                format!("Unknown command: {}{}.", line.command, at(line.line)),
            ));
            continue;
        }
        for key in &line.keys {
            let probe = Keymap {
                column: line.column,
                platform,
                bindings: Vec::new(),
            };
            for concern in probe.concerns(line.layer, key, &line.command) {
                let report = matches!(
                    concern,
                    KeyConcern::ScreenReaderKey
                        | KeyConcern::TerminalCannot { .. }
                        | KeyConcern::ScreenReaderOwns { .. }
                );
                if report {
                    let sentence = concern.sentence(key, platform);
                    problems.push(ConfigProblem::new(
                        file,
                        format!("{}{}.", sentence.trim_end_matches('.'), at(line.line)),
                    ));
                }
            }
        }
    }
    for (i, a) in config.overrides.iter().enumerate() {
        for b in &config.overrides[i + 1..] {
            if a.column != b.column || a.layer != b.layer || a.command == b.command {
                continue;
            }
            for key in a.keys.iter().filter(|k| b.keys.contains(k)) {
                problems.push(ConfigProblem::new(
                    file,
                    format!(
                        "{} is given twice in [{}.{}]: {} and {}; {} has it{}.",
                        written(key, platform),
                        a.column,
                        a.layer,
                        commands::name_of(&a.command),
                        commands::name_of(&b.command),
                        commands::name_of(&b.command),
                        at(b.line)
                    ),
                ));
            }
        }
    }
    let (terminal, window) = keymaps(config, platform);
    for keymap in [&terminal, &window] {
        for conflict in keymap.conflicts() {
            problems.push(ConfigProblem::new(
                file,
                format!("{} column: {conflict}", capitalized(keymap.column().word())),
            ));
        }
    }
    problems
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The current keys of both columns as `keymap.toml` text, every command
/// with its keys or "none": what `diro keys print` writes, so a keymap can
/// live in the owner's dotfiles. Read back, it gives the same keys.
pub fn print_keys(terminal: &Keymap, window: &Keymap) -> String {
    let mut lines = terminal.lines();
    lines.extend(window.lines());
    KeymapConfig::render(
        &lines,
        "The keys of diroxide, both columns, as diro keys print wrote them.\nKeep this file as keymap.toml to use these keys; change a line to change a key.",
    )
}

/// Writes the two read-only references beside the owner's files,
/// `keymap-defaults.toml` and `menus-defaults.toml`, each only when its
/// text changed. The app calls it at startup.
pub fn write_references(paths: &Paths) -> Result<(), diroxide_config::ConfigError> {
    diroxide_config::write_reference(paths, Paths::KEYMAP_DEFAULTS, &defaults_reference())?;
    diroxide_config::write_reference(
        paths,
        Paths::MENUS_DEFAULTS,
        &crate::menu::defaults_reference(),
    )?;
    Ok(())
}

/// The built-in keys as `keymap.toml` text, written beside the owner's
/// file as `keymap-defaults.toml`, a read-only reference.
pub fn defaults_reference() -> String {
    let mut lines = Keymap::defaults_for(KeyColumn::Terminal).lines();
    lines.extend(Keymap::defaults_for(KeyColumn::Window).lines());
    KeymapConfig::render(
        &lines,
        "The built-in keys of diroxide, written by diroxide at each start.\nThis copy is not read: copy a line to keymap.toml and change it there.\nOne key per command and layer, or a list, or \"none\" to unbind.\nOn macOS the window column uses Command where this says ctrl.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ids;
    use diroxide_core::format::within_cells;

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
            line: 0,
        }
    }

    fn config(lines: Vec<KeyOverride>) -> KeymapConfig {
        KeymapConfig {
            overrides: lines,
            problems: Vec::new(),
        }
    }

    #[test]
    fn the_defaults_are_clean_in_both_columns_on_every_system() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Other] {
            for column in KeyColumn::ALL {
                let keymap = Keymap::defaults_for(column).for_platform(platform);
                assert_eq!(
                    keymap.conflicts(),
                    Vec::<String>::new(),
                    "{column} {platform:?}"
                );
                assert_eq!(
                    keymap.screen_reader_conflicts(),
                    Vec::<String>::new(),
                    "{column} {platform:?}"
                );
                assert_eq!(keymap.only_num_lock_keys(), Vec::<String>::new());
            }
        }
        assert_eq!(Keymap::defaults().key_text(&ids::COPY), "F5");
        assert_eq!(Keymap::defaults().keys_text(&ids::TRASH), "F8 or Delete");
    }

    #[test]
    fn the_terminal_column_never_uses_an_unsafe_key() {
        let terminal = Keymap::defaults();
        for binding in terminal.bindings() {
            if let Some(row) = tables::terminal_entry(&binding.key) {
                assert_ne!(
                    row.kind,
                    TerminalKind::Unsafe,
                    "{} on {}",
                    binding.command,
                    binding.key
                );
            }
        }
    }

    #[test]
    fn a_key_a_terminal_may_keep_has_a_menu_as_the_sure_way() {
        let menus = crate::menu::Menus::builtin();
        for binding in Keymap::defaults().bindings() {
            if tables::terminal_entry(&binding.key).is_some() {
                assert!(
                    menus.reaches(&binding.command),
                    "{} on {} has no menu item",
                    binding.command,
                    binding.key
                );
            }
        }
    }

    #[test]
    fn every_command_is_reachable_in_a_terminal() {
        let menus = crate::menu::Menus::builtin();
        let terminal = Keymap::defaults();
        let typing = Keymap::defaults().with_single_letters(SingleLetters::Off);
        let palette = crate::palette::palette("", &terminal);
        for info in &CATALOG {
            let keys = terminal.keys_for(&info.id);
            assert!(
                !keys.is_empty() || info.palette_only,
                "{} has no terminal key and is not palette-only",
                info.id
            );
            if info.layer == Layer::Dialog {
                assert!(!keys.is_empty(), "{}", info.id);
                continue;
            }
            assert!(
                palette.rows().iter().any(|r| r.id == info.id.as_str()),
                "{} is not in the palette",
                info.id
            );
            if typing.keys_for(&info.id).is_empty() && !info.palette_only {
                assert!(
                    menus.reaches(&info.id),
                    "{} has only single-character keys and no menu item",
                    info.id
                );
            }
        }
    }

    #[test]
    fn the_later_alphas_keys_stay_free() {
        for column in KeyColumn::ALL {
            for binding in Keymap::defaults_for(column).bindings() {
                if let Some(row) = tables::reserved_entry(&binding.key, column) {
                    assert_eq!(row.command, binding.command, "{column} {}", binding.key);
                }
            }
        }
    }

    #[test]
    fn a_keypad_key_is_never_a_commands_only_key() {
        for column in KeyColumn::ALL {
            for info in &CATALOG {
                let keys = Keymap::defaults_for(column).keys_for(&info.id).len();
                let keypad = Keymap::defaults_for(column)
                    .keys_for(&info.id)
                    .iter()
                    .filter(|k| {
                        matches!(
                            k.code,
                            KeyCode::KeypadPlus
                                | KeyCode::KeypadMinus
                                | KeyCode::KeypadMultiply
                                | KeyCode::KeypadDivide
                        )
                    })
                    .count();
                assert!(keypad == 0 || keypad < keys, "{column} {}", info.id);
            }
        }
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
        assert_eq!(
            keymap.lookup(&[Layer::Dialog, Layer::Global], &key("enter")),
            Lookup::Command(ids::ACCEPT)
        );
    }

    #[test]
    fn the_macos_window_column_uses_command_and_leaves_the_system_keys() {
        let mac = Keymap::defaults_for(KeyColumn::Window).for_platform(Platform::MacOs);
        assert_eq!(mac.key_text(&ids::QUIT), "Cmd+Q");
        assert_eq!(mac.spoken_key(&ids::QUIT), "Command plus Q");
        assert!(mac.keys_for(&ids::SORT_MODIFIED).is_empty());
        assert_eq!(mac.key_text(&ids::SORT_LIST), "Cmd+Shift+S");
        assert_eq!(mac.keys_text(&ids::CONTEXT_MENU), "Shift+F10 or Menu");
        let terminal = Keymap::defaults().for_platform(Platform::MacOs);
        assert_eq!(terminal.key_text(&ids::QUIT), "Ctrl+Q");
        assert_eq!(terminal.key_text(&ids::SORT_MODIFIED), "Ctrl+F5");
    }

    #[test]
    fn a_line_moves_a_command_to_new_keys() {
        let keymap = Keymap::defaults().with_overrides(&config(vec![line(
            Layer::Panel,
            ids::RELOAD,
            &["f12"],
        )]));
        assert_eq!(
            keymap.lookup(&PANEL, &key("f12")),
            Lookup::Command(ids::RELOAD)
        );
        assert_eq!(keymap.keys_for(&ids::RELOAD), vec![&key("f12")]);
        let window_only = config(vec![KeyOverride {
            column: KeyColumn::Window,
            ..line(Layer::Panel, ids::COPY, &[])
        }]);
        let keymap = Keymap::defaults().with_overrides(&window_only);
        assert_eq!(
            keymap.lookup(&PANEL, &key("f5")),
            Lookup::Command(ids::COPY)
        );
        let unknown = config(vec![line(
            Layer::Panel,
            CommandId::from_static("file.copi"),
            &["f5"],
        )]);
        let keymap = Keymap::defaults().with_overrides(&unknown);
        assert_eq!(
            keymap.lookup(&PANEL, &key("f5")),
            Lookup::Command(ids::COPY)
        );
    }

    #[test]
    fn single_letters_follow_the_setting() {
        let with_letter =
            Keymap::defaults().with_overrides(&config(vec![line(Layer::Panel, ids::QUIT, &["q"])]));
        assert_eq!(
            with_letter.lookup(&PANEL, &key("q")),
            Lookup::Command(ids::QUIT)
        );
        let typing = with_letter
            .clone()
            .with_single_letters(SingleLetters::TypeAhead);
        assert_eq!(typing.lookup(&PANEL, &key("q")), Lookup::NoMatch);
        assert_eq!(
            typing.lookup(&PANEL, &key("*")),
            Lookup::Command(ids::SELECT_INVERT)
        );
        let off = with_letter.clone().without_single_characters();
        assert_eq!(off.lookup(&PANEL, &key("q")), Lookup::NoMatch);
        assert_eq!(off.lookup(&PANEL, &key("*")), Lookup::NoMatch);
        assert_eq!(
            off.lookup(&PANEL, &key("space")),
            Lookup::Command(ids::SELECT_TOGGLE)
        );
        assert_eq!(
            off.lookup(&[Layer::Dialog], &key("y")),
            Lookup::Command(ids::YES)
        );
        let commands = with_letter.with_single_letters(SingleLetters::Commands);
        assert_eq!(
            commands.lookup(&PANEL, &key("q")),
            Lookup::Command(ids::QUIT)
        );
    }

    #[test]
    fn concerns_are_said_in_order_and_in_words() {
        let terminal = Keymap::defaults();
        let p = Platform::Windows;
        let said = |k: &str, cmd: CommandId| -> Vec<String> {
            terminal
                .concerns(Layer::Panel, &key(k), &cmd)
                .iter()
                .map(|c| c.sentence(&key(k), p))
                .collect()
        };
        assert_eq!(
            said("insert", ids::COPY),
            ["Insert is the screen reader key."]
        );
        assert_eq!(
            said("ctrl+enter", ids::COPY),
            ["The terminal cannot send Ctrl+Enter: terminals send it as Enter or a line feed."]
        );
        assert_eq!(
            said("ctrl+v", ids::COPY),
            [
                "Windows Terminal keeps Ctrl+V: it pastes text, so the key never arrives.",
                "Ctrl+V already runs Paste."
            ]
        );
        assert_eq!(
            said("ctrl+alt+left", ids::COPY),
            [
                "Windows Terminal keeps Ctrl+Alt+Left: it moves to the previous pane.",
                "Screen readers keep Ctrl+Alt+Left: NVDA, JAWS and Narrator."
            ]
        );
        assert_eq!(
            said("ctrl+f5", ids::COPY),
            [
                "macOS keeps Ctrl+F5: the system uses Ctrl+F3 to Ctrl+F8 for the Dock, the menu bar and windows.",
                "Ctrl+F5 already runs Sort by modified."
            ]
        );
        assert_eq!(
            said("ctrl+f", ids::COPY),
            ["Ctrl+F is kept for Find files, coming in alpha.3."]
        );
        assert!(said("ctrl+f", CommandId::from_static("search.find")).is_empty());
        assert_eq!(
            said("q", ids::COPY),
            ["A letter key works only while letters run commands."]
        );
        assert!(said("f12", ids::COPY)[0].starts_with("The terminal cannot send F12:"));
        assert!(said("ctrl+k", ids::COPY).is_empty());
        let window = Keymap::defaults_for(KeyColumn::Window);
        assert!(
            window
                .concerns(Layer::Panel, &key("ctrl+enter"), &ids::GO_PATH)
                .is_empty()
        );
        assert_eq!(
            window
                .concerns(Layer::Panel, &key("kp-plus"), &ids::SELECT_ALL)
                .iter()
                .map(|c| c.sentence(&key("kp-plus"), p))
                .collect::<Vec<_>>(),
            ["Num Lock on needed for Keypad Plus: NVDA, JAWS and Orca review with it off."]
        );
        for row in tables::terminal_table() {
            for k in &row.keys {
                for concern in terminal.concerns(Layer::Panel, k, &ids::COPY) {
                    let sentence = concern.sentence(k, p);
                    let meaning = sentence.split(':').next().unwrap_or(&sentence);
                    let meaning = meaning.split(" already").next().unwrap_or(meaning);
                    assert!(within_cells(&sentence, meaning), "{sentence}");
                }
            }
        }
    }

    #[test]
    fn take_swap_and_cancel() {
        let mut keymap = Keymap::defaults();
        let f5 = key("ctrl+f5");
        assert_eq!(
            keymap.assign(Layer::Panel, f5, &ids::SORT_NAME, KeyChoice::Cancel),
            "Nothing changed."
        );
        assert_eq!(
            keymap.assign(Layer::Panel, f5, &ids::SORT_NAME, KeyChoice::Swap),
            "Ctrl+F5 now runs Sort by name; Sort by modified moved to Ctrl+F3."
        );
        assert_eq!(
            keymap.lookup(&PANEL, &key("ctrl+f3")),
            Lookup::Command(ids::SORT_MODIFIED)
        );
        assert_eq!(
            keymap.assign(
                Layer::Panel,
                key("ctrl+f3"),
                &ids::SORT_SIZE,
                KeyChoice::Take
            ),
            "Ctrl+F3 now runs Sort by size; Sort by modified has no key here."
        );
        assert!(keymap.keys_for(&ids::SORT_MODIFIED).is_empty());
        assert_eq!(keymap.conflicts(), Vec::<String>::new());
        assert_eq!(
            keymap.assign(
                Layer::Panel,
                key("ctrl+k"),
                &ids::SORT_KIND,
                KeyChoice::Take
            ),
            "Ctrl+K now runs Sort by kind."
        );
    }

    #[test]
    fn the_key_test_says_what_arrived_and_what_runs() {
        let keymap = Keymap::defaults().for_platform(Platform::Windows);
        assert_eq!(
            key_test(&keymap, &PANEL, &key("ctrl+f5")),
            "Got Control plus F5. Runs Sort by modified."
        );
        assert_eq!(
            key_test(&keymap, &PANEL, &key("ctrl+k")),
            "Got Control plus K. Nothing runs on it."
        );
        assert_eq!(
            key_test(&keymap, &PANEL, &key("insert")),
            "Got Insert. Insert is the screen reader key."
        );
        assert_eq!(
            key_test_typed(&keymap, &PANEL, "hyper+q"),
            "Not a key name: hyper+q."
        );
        assert_eq!(
            key_test_typed(&keymap, &PANEL, "ctrl+x ctrl+f"),
            "Two keys in a row: ctrl+x ctrl+f. A key is one key with its modifiers."
        );
        assert_eq!(
            key_test_typed(&keymap, &PANEL, "alt+home"),
            "Got Alt plus Home. Runs Home folder."
        );
    }

    #[test]
    fn ten_mistakes_give_ten_sentences_inside_40_cells() {
        let text = "[terminal.panel]
\"file.copy\" = \"hyper+c\"
\"file.copi\" = \"f5\"
\"go.find\" = \"ctrl+x ctrl+f\"
\"file.view\" = \"ctrl+enter\"
\"file.edit\" = \"insert\"
\"file.move\" = \"ctrl+alt+n\"
\"Bad.Id\" = \"f2\"
\"file.rename\" = 7
[terminal.nowhere]
\"app.quit\" = \"f1\"
[tv.panel]
\"app.quit\" = \"f1\"
";
        let config = KeymapConfig::from_toml(text).unwrap();
        let problems = check_keymap(&config, Platform::Windows);
        let said: Vec<&str> = problems.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(said.len(), 10, "{said:#?}");
        for sentence in &said {
            let meaning = sentence.split(':').next().unwrap_or(sentence);
            assert!(within_cells(sentence, meaning), "{sentence}");
        }
        assert!(said.contains(&"Unknown command: file.copi, line 3."));
        assert!(said.contains(&"Insert is the screen reader key, line 6."));
        assert!(said.contains(&"Screen readers keep Ctrl+Alt+N: NVDA, line 7."));
        assert!(said.contains(
            &"The terminal cannot send Ctrl+Enter: terminals send it as Enter or a line feed, line 5."
        ));
        let twice = KeymapConfig::from_toml(
            "[terminal.panel]\n\"file.view\" = \"f5\"\n\"file.edit\" = \"f5\"\n",
        )
        .unwrap();
        let said: Vec<String> = check_keymap(&twice, Platform::Windows)
            .into_iter()
            .map(|p| p.text)
            .collect();
        assert_eq!(
            said,
            ["F5 is given twice in [terminal.panel]: View and Edit; Edit has it, line 3."]
        );
    }

    #[test]
    fn print_read_back_is_clean_and_the_same() {
        let (terminal, window) = keymaps(&KeymapConfig::default(), Platform::Windows);
        let printed = print_keys(&terminal, &window);
        let config = KeymapConfig::from_toml(&printed).unwrap();
        assert_eq!(check_keymap(&config, Platform::Windows), Vec::new());
        let (t2, w2) = keymaps(&config, Platform::Windows);
        let sorted = |k: &Keymap| {
            let mut b: Vec<(String, String, String)> = k
                .bindings()
                .iter()
                .map(|b| {
                    (
                        b.layer.to_string(),
                        b.key.to_string(),
                        b.command.to_string(),
                    )
                })
                .collect();
            b.sort();
            b
        };
        assert_eq!(sorted(&t2), sorted(&terminal));
        assert_eq!(sorted(&w2), sorted(&window));
        assert!(printed.contains("\n[terminal.panel]\n"));
        assert!(printed.contains("\"app.key-test\" = \"none\"\n"));
        assert!(defaults_reference().starts_with("# The built-in keys of diroxide"));
    }

    #[test]
    fn the_references_are_written_beside_the_owners_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
        write_references(&paths).unwrap();
        let keys = std::fs::read_to_string(paths.config_file(Paths::KEYMAP_DEFAULTS)).unwrap();
        assert_eq!(keys, defaults_reference());
        let menus = std::fs::read_to_string(paths.config_file(Paths::MENUS_DEFAULTS)).unwrap();
        assert_eq!(menus, crate::menu::defaults_reference());
        assert!(KeymapConfig::from_toml(&keys).unwrap().problems.is_empty());
    }

    #[test]
    fn rebinding_changes_what_is_said() {
        let keymap = Keymap::defaults().with_overrides(&config(vec![line(
            Layer::Panel,
            ids::RELOAD,
            &["f12"],
        )]));
        assert_eq!(keymap.spoken_key(&ids::RELOAD), "F12");
        assert_eq!(
            diroxide_core::catalog::folder_not_watched(&keymap.key_text(&ids::RELOAD)).text,
            "Folder not watched. Reload with F12."
        );
    }
}
