//! The conflict analysis: everything Change key says about a key before it
//! is bound, in the order the Keys list asks (research/alpha3-plan.md,
//! "Change key, step by step"), each one sentence with its meaning first.
//!
//! 1. **Refused.** Insert is the screen readers' key and is never bound.
//!    A key that is already the command's own changes nothing.
//! 2. **The terminal.** In the terminal column a key the terminal cannot
//!    send asks "Use it for the window column only?"; a key a terminal
//!    program keeps, or that some terminals send as another, is said and
//!    the change goes on. A program's row counts only on its own system:
//!    macOS's keys on macOS, the Windows console's on Windows.
//! 3. **The screen readers.** A key NVDA, JAWS, Narrator, Orca or
//!    VoiceOver owns asks "Use it anyway?"; a keypad key that needs Num
//!    Lock on is said.
//! 4. **Kept for later.** A key a later alpha's command is promised asks
//!    "Use it anyway?".
//! 5. **Already used.** A key another command holds where this command's
//!    keys are looked up (its own layer, any list and everywhere) says
//!    what it does: "Ctrl+F5 already sorts by modified time." Then "Take
//!    it, swap, or cancel?": taking leaves the other command without the
//!    key, swapping gives it this command's old key. Two commands cannot
//!    share a key in one layer, so the plan's "keep both" is not offered.
//!    An accessibility-critical command (Say status, Say other panel, the
//!    palette, help, the menus, quit) is never left without a key: only a
//!    swap is offered then.
//! 6. **A letter.** A single printable key asks "Go on?", since it works
//!    only while letters run commands.
//!
//! Every question starts on No; Escape answers it.

use diroxide_config::SingleLetters;
use diroxide_core::{CommandId, Key, KeyCode, Layer};

use super::names::Platform;
use super::{Binding, KeyConcern, Keymap, written};
use crate::commands::{self, ids};

/// The commands that keep a key in every keymap and every preset, so a
/// screen reader user can always find their way (plan section 5.7; the
/// presets' critical set).
pub const CRITICAL: [CommandId; 6] = [
    ids::SAY_STATUS,
    ids::SAY_OTHER,
    ids::PALETTE,
    ids::HELP,
    ids::MENU,
    ids::QUIT,
];

/// Whether a command is in the critical set.
pub fn is_critical(command: &CommandId) -> bool {
    CRITICAL.contains(command)
}

/// What a yes-or-no question of the analysis asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    /// The terminal cannot send the key: use it in the window column only.
    WindowOnly,
    /// A screen reader owns the key, or a later alpha is promised it.
    Anyway,
    /// A single printable key.
    Letter,
}

impl AskKind {
    /// The question, without its "? y or n": "Use it for the window column
    /// only".
    pub const fn words(self) -> &'static str {
        match self {
            AskKind::WindowOnly => "Use it for the window column only",
            AskKind::Anyway => "Use it anyway",
            AskKind::Letter => "Go on",
        }
    }
}

/// Another command holds the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Taken {
    /// The command that holds it.
    pub other: CommandId,
    /// The layer it holds it in.
    pub layer: Layer,
    /// "F5 already copies to the other panel."
    pub sentence: String,
    /// Whether taking it is offered: not when the other command is in the
    /// critical set and has no other key.
    pub take: bool,
    /// The key a swap would give the other command (this command's old
    /// key), or `None` when there is nothing to give.
    pub swap: Option<Key>,
}

impl Taken {
    /// The question after the sentence: "Take it, swap, or cancel?", with
    /// only the choices offered.
    pub fn question(&self) -> String {
        match (self.take, self.swap.is_some()) {
            (true, true) => "Take it, swap, or cancel?".to_owned(),
            (true, false) => "Take it, or cancel?".to_owned(),
            (false, true) => format!(
                "Swap, or cancel? {} needs a key.",
                commands::name_of(&self.other)
            ),
            (false, false) => format!(
                "{} needs a key, so nothing changed.",
                commands::name_of(&self.other)
            ),
        }
    }
}

/// One thing the analysis says about a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// The key is not bound; the sentence says why.
    Refused(String),
    /// Said, and the change goes on.
    Note(String),
    /// A question, starting on No: the sentence, then the question.
    Ask {
        /// The sentence, meaning first: "The terminal cannot send
        /// Ctrl+Enter."
        line: String,
        /// What it asks.
        kind: AskKind,
    },
    /// Another command holds the key.
    Taken(Taken),
}

impl Finding {
    /// The finding as it is said: the sentence, and for a question the
    /// question after it ("... Use it anyway? y or n").
    pub fn said(&self) -> String {
        match self {
            Finding::Refused(line) | Finding::Note(line) => line.clone(),
            Finding::Ask { line, kind } => format!("{line} {}? y or n", kind.words()),
            Finding::Taken(taken) => format!("{} {}", taken.sentence, taken.question()),
        }
    }
}

/// Whether keys bound in layer `a` and in layer `b` can meet in one
/// lookup: the panel, the viewer, the preview and a dialog each look in
/// their own layer, then any list, then everywhere.
pub fn layers_meet(a: Layer, b: Layer) -> bool {
    a == b || matches!(a, Layer::List | Layer::Global) || matches!(b, Layer::List | Layer::Global)
}

/// The binding of `key` that a binding of it for `command` in `layer`
/// would meet: its own layer first, then any layer it shares a lookup
/// with. `None` when the key is free there, or is the command's own.
pub fn holder<'a>(
    keymap: &'a Keymap,
    layer: Layer,
    key: &Key,
    command: &CommandId,
) -> Option<&'a Binding> {
    let held = |b: &&Binding| b.key == *key && b.command != *command;
    keymap
        .bindings()
        .iter()
        .filter(|b| b.layer == layer)
        .find(held)
        .or_else(|| {
            keymap
                .bindings()
                .iter()
                .filter(|b| b.layer != layer && layers_meet(b.layer, layer))
                .find(held)
        })
}

/// What a command does, after a key's name: "copies to the other panel",
/// "sorts by modified time". A command with no phrase "runs" its name.
pub fn does(command: &CommandId) -> String {
    let phrase = match command.as_str() {
        "app.quit" => "quits",
        "app.help" => "says help",
        "app.say-status" => "says the status",
        "app.say-other" => "says the other panel",
        "app.say-key-bar" => "says the key bar",
        "app.palette" => "opens the command palette",
        "app.menu" => "opens the menus",
        "app.settings" => "opens the settings",
        "app.key-test" => "tests keys",
        "app.cancel" => "cancels",
        "app.layout" => "changes the layout",
        "edit.undo" => "undoes the last job",
        "jobs.history" => "opens the job history",
        "panel.switch" => "switches panels",
        "panel.swap" => "swaps the panels",
        "panel.show-in-other" => "shows the folder in the other panel",
        "panel.this-in-other" => "shows this folder in the other panel",
        "panel.enter" => "opens the focused row",
        "panel.into" => "goes into the folder",
        "panel.up" => "goes up one folder",
        "panel.reload" => "reloads the folder",
        "archive.pack" => "packs into an archive",
        "archive.unpack" => "unpacks an archive",
        "archive.test" => "tests an archive",
        "go.back" => "goes back",
        "go.forward" => "goes forward",
        "go.home" => "goes to the home folder",
        "go.path" => "goes to a path",
        "go.places" => "goes to a place",
        "go.history" => "opens the folder history",
        "list.up" => "moves up a row",
        "list.down" => "moves down a row",
        "list.page-up" => "moves up a page",
        "list.page-down" => "moves down a page",
        "list.home" => "goes to the first row",
        "list.end" => "goes to the last row",
        "select.toggle" => "selects or unselects",
        "select.all" => "selects all",
        "select.none" => "selects none",
        "select.invert" => "inverts the selection",
        "select.pattern" => "selects by pattern",
        "select.unpattern" => "unselects by pattern",
        "select.next" => "goes to the next selected",
        "select.previous" => "goes to the previous selected",
        "select.restore" => "restores the selection",
        "file.view" => "views the file",
        "file.edit" => "edits the file",
        "file.copy" => "copies to the other panel",
        "file.move" => "moves to the other panel",
        "file.copy-to" => "copies to a place",
        "file.move-to" => "moves to a place",
        "folder.make" => "makes a folder",
        "file.trash" => "moves to the trash",
        "file.delete-permanently" => "deletes for good",
        "file.rename" => "renames",
        "file.duplicate" => "duplicates",
        "file.details" => "shows the details",
        "file.context-menu" => "opens the context menu",
        "file.run-command" => "runs a command",
        "file.shell-here" => "opens a shell here",
        "clipboard.copy" => "copies to the clipboard",
        "clipboard.cut" => "cuts to the clipboard",
        "clipboard.paste" => "pastes",
        "view.hidden" => "shows hidden files",
        "view.filter" => "filters the panel",
        "view.filter-clear" => "clears the filter",
        "view.sort-name" => "sorts by name",
        "view.sort-extension" => "sorts by extension",
        "view.sort-modified" => "sorts by modified time",
        "view.sort-size" => "sorts by size",
        "view.sort-unsorted" => "leaves the folder unsorted",
        "view.sort-created" => "sorts by created time",
        "view.sort-accessed" => "sorts by accessed time",
        "view.sort-kind" => "sorts by kind",
        "view.sort-attributes" => "sorts by attributes",
        "view.sort-owner" => "sorts by owner",
        "view.sort-label" => "sorts by label",
        "view.sort-list" => "opens the Sort list",
        "view.quick-view" => "turns the quick view on or off",
        "dialog.yes" => "answers yes",
        "dialog.no" => "answers no",
        "dialog.accept" => "chooses the focused answer",
        id => {
            if let Some(menu) = id.strip_prefix("menu.") {
                let title = commands::Menu::ALL
                    .iter()
                    .find(|m| m.id() == menu)
                    .map_or(menu, |m| m.title());
                return format!("opens the {title} menu");
            }
            return format!("runs {}", commands::name_of(command));
        }
    };
    phrase.to_owned()
}

/// The "already" sentence: "F5 already copies to the other panel."
pub fn already(key: &Key, other: &CommandId, platform: Platform) -> String {
    format!("{} already {}.", written(key, platform), does(other))
}

/// The sentence of a concern without its reason: "The terminal cannot
/// send Ctrl+Enter." from "... Ctrl+Enter: terminals send it as Enter."
fn first_clause(sentence: &str) -> String {
    match sentence.split_once(": ") {
        Some((head, _)) => format!("{head}."),
        None => sentence.to_owned(),
    }
}

/// Whether a terminal program's row holds on this system.
fn program_holds(program: &str, platform: Platform) -> bool {
    match program {
        "macOS" => platform == Platform::MacOs,
        "Windows" => platform == Platform::Windows,
        "Windows Terminal" => platform != Platform::MacOs,
        _ => true,
    }
}

/// Everything to say about binding `key` to `command` in `layer` of this
/// keymap's column, in order (see the module documentation). `letters` is
/// the single-letter setting: in commands mode a letter needs no
/// question. An empty list means the key is free and safe.
pub fn analyze(
    keymap: &Keymap,
    layer: Layer,
    key: &Key,
    command: &CommandId,
    letters: SingleLetters,
) -> Vec<Finding> {
    let platform = keymap.platform();
    let k = written(key, platform);
    if key.code == KeyCode::Insert {
        return vec![Finding::Refused(
            KeyConcern::ScreenReaderKey.sentence(key, platform),
        )];
    }
    let name = commands::name_of(command);
    if keymap
        .bindings()
        .iter()
        .any(|b| b.layer == layer && b.key == *key && b.command == *command)
    {
        return vec![Finding::Refused(format!(
            "{k} is already the key of {name}. Nothing changed."
        ))];
    }
    let mut findings = Vec::new();
    for concern in keymap.concerns(layer, key, command) {
        match &concern {
            KeyConcern::TerminalCannot { .. } => findings.push(Finding::Ask {
                line: first_clause(&concern.sentence(key, platform)),
                kind: AskKind::WindowOnly,
            }),
            KeyConcern::TerminalProgram { program, .. } => {
                if program_holds(program, platform) {
                    findings.push(Finding::Note(concern.sentence(key, platform)));
                }
            }
            KeyConcern::ArrivesDifferently { .. } | KeyConcern::NeedsNumLock { .. } => {
                findings.push(Finding::Note(concern.sentence(key, platform)));
            }
            KeyConcern::ScreenReaderOwns { .. } => findings.push(Finding::Ask {
                line: concern.sentence(key, platform),
                kind: AskKind::Anyway,
            }),
            KeyConcern::Reserved { .. } => findings.push(Finding::Ask {
                line: concern.sentence(key, platform),
                kind: AskKind::Anyway,
            }),
            KeyConcern::ScreenReaderKey => {
                return vec![Finding::Refused(concern.sentence(key, platform))];
            }
            // The holder below looks wider than one layer.
            KeyConcern::AlreadyUsed { .. } => {}
            KeyConcern::SingleLetter => {
                if letters != SingleLetters::Commands {
                    findings.push(Finding::Ask {
                        line: concern.sentence(key, platform),
                        kind: AskKind::Letter,
                    });
                }
            }
        }
    }
    if let Some(held) = holder(keymap, layer, key, command) {
        let other_keys = keymap
            .bindings()
            .iter()
            .filter(|b| b.command == held.command && b.key != *key)
            .count();
        let swap = keymap
            .bindings()
            .iter()
            .find(|b| b.layer == layer && b.command == *command && b.key != *key)
            .map(|b| b.key)
            .filter(|old| {
                // The old key must be free in the other command's layer.
                keymap
                    .bindings()
                    .iter()
                    .all(|b| !(b.layer == held.layer && b.key == *old && b.command != *command))
            });
        let taken = Taken {
            other: held.command.clone(),
            layer: held.layer,
            sentence: already(key, &held.command, platform),
            take: !(is_critical(&held.command) && other_keys == 0),
            swap,
        };
        // The letter question, when there is one, comes after this one.
        let at = findings
            .iter()
            .position(|f| {
                matches!(
                    f,
                    Finding::Ask {
                        kind: AskKind::Letter,
                        ..
                    }
                )
            })
            .unwrap_or(findings.len());
        findings.insert(at, Finding::Taken(taken));
    }
    findings
}

/// Whether any finding refuses the key, or a taken key can be neither
/// taken nor swapped: nothing can change.
pub fn blocked(findings: &[Finding]) -> bool {
    findings.iter().any(|f| match f {
        Finding::Refused(_) => true,
        Finding::Taken(taken) => !taken.take && taken.swap.is_none(),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    fn said(keymap: &Keymap, layer: Layer, k: &str, command: &CommandId) -> Vec<String> {
        analyze(keymap, layer, &key(k), command, SingleLetters::TypeAhead)
            .iter()
            .map(Finding::said)
            .collect()
    }

    #[test]
    fn a_table_of_keys_and_what_is_said() {
        let terminal = Keymap::defaults().for_platform(Platform::Windows);
        let size = ids::SORT_SIZE;
        let cases: [(&str, &[&str]); 10] = [
            ("ctrl+f9", &[]),
            (
                "ctrl+f5",
                &["Ctrl+F5 already sorts by modified time. Take it, swap, or cancel?"],
            ),
            (
                "f5",
                &["F5 already copies to the other panel. Take it, swap, or cancel?"],
            ),
            ("insert", &["Insert is the screen reader key."]),
            (
                "ctrl+enter",
                &["The terminal cannot send Ctrl+Enter. Use it for the window column only? y or n"],
            ),
            (
                "ctrl+v",
                &[
                    "Windows Terminal keeps Ctrl+V: it pastes text, so the key never arrives.",
                    "Ctrl+V already pastes. Take it, swap, or cancel?",
                ],
            ),
            (
                "ctrl+alt+up",
                &[
                    "Screen readers keep Ctrl+Alt+Up: NVDA, JAWS and Narrator. Use it anyway? y or n",
                ],
            ),
            (
                "ctrl+f",
                &["Ctrl+F is kept for Find files, coming in alpha.3. Use it anyway? y or n"],
            ),
            (
                "q",
                &["A letter key works only while letters run commands. Go on? y or n"],
            ),
            (
                "ctrl+d",
                &["Ctrl+D already says the status. Swap, or cancel? Say status needs a key."],
            ),
        ];
        for (k, expected) in cases {
            assert_eq!(said(&terminal, Layer::Panel, k, &size), expected, "{k}");
        }
        assert_eq!(
            said(&terminal, Layer::Panel, "ctrl+f6", &size),
            ["Ctrl+F6 is already the key of Sort by size. Nothing changed."]
        );
    }

    #[test]
    fn a_programs_keys_count_only_on_its_system() {
        let mac = Keymap::defaults().for_platform(Platform::MacOs);
        let windows = Keymap::defaults().for_platform(Platform::Windows);
        let free = CommandId::from_static("app.say-key-bar");
        assert_eq!(
            said(&mac, Layer::Global, "ctrl+f8", &free),
            [
                "macOS keeps Ctrl+F8: the system uses Ctrl+F3 to Ctrl+F8 for the Dock, the menu bar and windows."
            ]
        );
        assert!(said(&windows, Layer::Global, "ctrl+f8", &free).is_empty());
        assert!(
            said(&mac, Layer::Global, "ctrl+v", &free)
                .iter()
                .all(|s| !s.starts_with("Windows Terminal"))
        );
        assert_eq!(
            said(&windows, Layer::Global, "menu", &free)[0],
            "Windows keeps Menu: crossterm never delivers the Menu key in a Windows terminal; Shift+F10 is the way."
        );
        assert_eq!(
            said(&mac, Layer::Global, "ctrl+b", &free),
            [
                "tmux keeps Ctrl+B: it is tmux's prefix key, so the key arrives only when pressed twice.",
                "Ctrl+B is kept for Bookmarks, coming in alpha.3. Use it anyway? y or n"
            ]
        );
    }

    #[test]
    fn a_key_held_in_a_layer_that_meets_this_one_is_already_used() {
        let terminal = Keymap::defaults().for_platform(Platform::Windows);
        // Ctrl+Q quits everywhere: a panel command taking it would hide
        // Quit in the panels, and Quit has no other key.
        assert_eq!(
            said(&terminal, Layer::Panel, "ctrl+q", &ids::RELOAD)[1],
            "Ctrl+Q already quits. Swap, or cancel? Quit needs a key."
        );
        assert_eq!(
            said(&terminal, Layer::Panel, "ctrl+u", &ids::RELOAD),
            ["Ctrl+U already swaps the panels. Take it, swap, or cancel?"]
        );
        // The viewer's F7 and the panel's F7 never meet.
        assert!(
            said(&terminal, Layer::Viewer, "f7", &ids::VIEWER_WRAP)
                .iter()
                .any(|s| s.starts_with("F7 already runs Search in file."))
        );
        assert!(holder(&terminal, Layer::Panel, &key("w"), &ids::RELOAD).is_none());
        assert!(layers_meet(Layer::Panel, Layer::Global));
        assert!(!layers_meet(Layer::Panel, Layer::Viewer));
    }

    #[test]
    fn take_and_swap_are_offered_only_when_they_can_be() {
        let terminal = Keymap::defaults().for_platform(Platform::Windows);
        let findings = analyze(
            &terminal,
            Layer::Panel,
            &key("f5"),
            &ids::SORT_SIZE,
            SingleLetters::TypeAhead,
        );
        let Finding::Taken(taken) = &findings[0] else {
            panic!("{findings:?}")
        };
        assert_eq!(taken.other, ids::COPY);
        assert!(taken.take);
        assert_eq!(taken.swap, Some(key("ctrl+f6")));
        // A command with no key has nothing to swap.
        let findings = analyze(
            &terminal,
            Layer::Panel,
            &key("f5"),
            &ids::SORT_KIND,
            SingleLetters::TypeAhead,
        );
        assert_eq!(
            findings[0].said(),
            "F5 already copies to the other panel. Take it, or cancel?"
        );
        // Help keeps its other key, so taking one of its keys is fine.
        let findings = analyze(
            &terminal,
            Layer::Global,
            &key("shift+f1"),
            &ids::SORT_KIND,
            SingleLetters::TypeAhead,
        );
        assert!(matches!(&findings[..], [.., Finding::Taken(t)] if t.take));
        // The only key of a critical command, with nothing to swap.
        let findings = analyze(
            &terminal,
            Layer::Global,
            &key("ctrl+q"),
            &ids::SORT_KIND,
            SingleLetters::TypeAhead,
        );
        assert!(blocked(&findings));
        assert_eq!(
            findings.last().unwrap().said(),
            "Ctrl+Q already quits. Quit needs a key, so nothing changed."
        );
        // In commands mode a letter needs no question.
        assert!(
            analyze(
                &terminal,
                Layer::Panel,
                &key("w"),
                &ids::RELOAD,
                SingleLetters::Commands
            )
            .is_empty()
        );
    }

    #[test]
    fn every_command_with_a_key_has_words_and_each_sentence_fits() {
        let platform = Platform::Windows;
        for info in commands::CATALOG.iter() {
            for text in info.terminal.iter().chain(info.window) {
                let k = key(text);
                let sentence = already(&k, &info.id, platform);
                assert!(sentence.ends_with('.'), "{sentence}");
                if info.layer != Layer::Viewer && info.layer != Layer::Preview {
                    assert!(
                        !does(&info.id).starts_with("runs "),
                        "{} has no phrase",
                        info.id
                    );
                }
                // The key, "already" and the verb come first.
                let verb = does(&info.id);
                let verb = verb.split(' ').next().unwrap_or_default();
                let meaning = format!("{} already {verb}", written(&k, platform));
                assert!(within_cells(&sentence, &meaning), "{sentence}");
            }
        }
        assert_eq!(does(&ids::MENU_TOOLS), "opens the Tools menu");
        assert_eq!(does(&CommandId::from_static("x.y")), "runs x.y");
        for (k, other) in [("f5", ids::COPY), ("ctrl+f5", ids::SORT_MODIFIED)] {
            let line = already(&key(k), &other, platform);
            assert!(line.chars().count() <= 40, "{line}");
        }
    }
}
