# ADR-0005: Commands, keys and menus from one model

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Friday, October 16, 2026 (the Wave 1 contract, P0b, from the owner's decisions of Saturday, October 10 and Sunday, October 11, 2026, questions 56 and 63, answers 29, 33 and 34, R19 and R21):

- **No sequences.** There are no chords and no leader keys: a binding is one key with its modifiers, and a two-key entry is refused in words (`CoreError::KeySequence`). The decision "Two-key chords work in the Emacs style" above no longer holds; the prefix lookup and the continuations line are gone from the app.
- **Two columns.** The keymap has a terminal column and a GUI column (`KeyColumn`), each with the layers above.
- **`keymap.toml` is command to key**: one line per command, in a table per column and layer (`[terminal.panel]` with `"file.copy" = "f5"`; a list of keys, or `"none"` to unbind). The built-in defaults are written beside it as a read-only reference, and every unknown command or key is reported in words, never dropped.
- **The defaults change as decided.** F10 no longer quits (Ctrl+Q does); it opens the menus in the GUI, and the terminal column does not use it. Ctrl+I and Ctrl+H are gone; Ctrl+Shift+S no longer says the status, and in the GUI it opens the Sort list; Say Status is on Ctrl+D, details on Alt+Enter alone; the palette is on Ctrl+Shift+P, and Ctrl+P too in the terminal; sorting is on Ctrl+F3 to Ctrl+F7. The menus are a data file, `menus.toml`, with the same command ids.
- Done by W1c on Friday, October 16, 2026: the catalog carries the decided keys in both columns, and the terminal-unsafe, screen-reader and reserved tables are data files under `crates/diroxide-app/keys/`, each entry with its source; the owner's key probe amends them.

Status update, Saturday, October 17, 2026 (the plan's alpha.3, the keys lane W3k, from answer 34 and research/alpha3-plan.md, "The keymap presets" and "The Keys list"):

- **Presets are layers.** The keys that apply are the catalog's defaults, then the chosen preset, then the owner's `keymap.toml`. A preset is a data file under `crates/diroxide-app/keys/presets/`: a `[preset]` table, then only the lines it changes in `keymap.toml`'s own tables, a `[macos]` section for keys that hold on macOS only, and the source's key sequences held back as `[[held_back]]`. Applying one sets `preset` in `config.toml` and clears nothing. Six ship: the diroxide defaults, Total Commander, Midnight Commander, Explorer and Finder, Worker and vim-style, whose single letters run commands. Every preset passes the same checks in CI (each command reachable, no key a terminal cannot send, a menu as the sure way for a key a terminal may keep, no key twice in a layer, no screen reader's key, the critical set kept, no keypad-only command, no letter in a type-ahead preset), and `cargo xtask keyboard` writes a sheet for each, `docs/keyboard-NAME.md`.
- **The Keys list changes keys in the app.** Change key captures one key, says it back, and runs the conflict analysis before binding it: Insert refused; a key the terminal cannot send offered for the GUI column only; a key a terminal program keeps, or a screen reader owns, said; a key another command holds where this one's keys are looked up answered with "take it, swap, or cancel", since two commands cannot share a key in one layer (this replaces the plan's "replace, keep both, or cancel"); a critical command never left without a key. Every change writes only the owner's lines it touched, through `toml_edit`, so comments, order and unknown lines survive.
- **The key-delivery matrix** (`keys/terminal-delivery.toml`) says what Windows Terminal, the desktop app's built-in terminal, tmux and the classic console do with each key the presets use, and feeds the terminal table; `diro keys probe` fills it. The enhancement protocol is turned on only while a capture waits.

## Context

Everything in diroxide is a command, and every command has a key, a name in the palette and a place in the menus. If menus, buttons and help each kept their own names and keys, a menu could show a key that is not bound, which a screen reader user would follow and find broken. This follows the design of textweaver's ADR-0043.

## Decision

- The command catalog (`diroxide_app::commands::CATALOG`) is one table: each command's id, name, one sentence of help, menu, and default keys with their layers.
- The keymap is built from the catalog's defaults, then `keymap.toml`'s overrides: an override first removes what its chord had in its layer, then binds the new command, or none.
- Layers are global, panel, list, viewer and dialog. A key is looked up in the most specific layer first: a dialog when one is open, then panel, list and global.
- Two-key chords work in the Emacs style. After the first key, the status line says the continuations, and they are announced: "Ctrl+X, then: Ctrl+F Find files".
- Menus, the palette and button banks read names and keys from the catalog and the keymap, so none can show an unbound key. The palette ranks names that start with the query, then names with a word that starts with it, then names that contain it.
- WCAG 2.1.4: single-character shortcuts can be turned off with a setting; the keymap then drops them.
- Insert is never bound, because it is the NVDA and JAWS key. The keymap checks itself for chords bound twice in a layer and for keys screen readers take; W1c completes the table of NVDA's and JAWS's default keys.
- Defaults follow Worker and the Norton Commander line where screen readers allow: Tab switches panels, Enter opens, Backspace goes up, Space selects and moves on, F2 renames, F3 views, F4 edits, F5 copies, F6 moves, F7 makes a folder, F8 and Delete move to the trash, Shift+Delete deletes for good, F9 opens the menus, Ctrl+P the palette, F1 help, Ctrl+Shift+S Say Status, Ctrl+Q and F10 quit; keypad plus, minus, star and slash select all, none, invert and by pattern, with Control letters on laptops.
- `docs/keyboard.md` is generated from the keymap's defaults by `cargo xtask keyboard`, key first on every line, no tables.

## Alternatives

- **Keys kept per frontend.** Each frontend would drift, and the docs would have to be written three times.
- **Commands registered only at runtime.** Plugins add commands at runtime (W3d), but built-in commands as a static table give the docs, the tests and the menus one source.

## Fallback

If a default key conflicts with a screen reader on some setup, the owner rebinds it in `keymap.toml`, and the conflict table gains it so the defaults change for everyone.

## Consequences

- Phase 0's catalog has 33 commands; most say "not built yet" until their wave.
- W1c owns the catalog, the keymap, the menus, the palette and the settings schema; W1d wires them into dispatch.

## See also

- [Keyboard](../keyboard.md)
- [ADR-0007: Frontends over one core](0007-frontends-over-one-core.md)
