# ADR-0005: Commands, keys and menus from one model

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

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
