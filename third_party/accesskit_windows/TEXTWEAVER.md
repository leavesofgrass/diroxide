# Vendored accesskit_windows

A copy of [accesskit_windows](https://github.com/AccessKit/accesskit) 0.35.1 (AccessKit's Windows adapter), from crates.io, with one change for textweaver's GUI. The decision is [ADR-0033](../../docs/adr/0033-gui-session-2-and-edit-mode.md). Licence: MIT OR Apache-2.0, as upstream.

## Why

Every button in the GUI has a key from the keymap, which the owner wants in the button's standard keyboard shortcut property, so NVDA and JAWS say it only when their "report shortcut keys" setting is on. AccessKit has the property (`Node::set_keyboard_shortcut`), but its Windows adapter 0.35.1 never gives it to UI Automation, so no screen reader could see it.

## textweaver's change

`textweaver.patch` holds it as a unified diff against the crates.io release: `src/node.rs` answers UI Automation's `AcceleratorKey` property (`UIA_AcceleratorKeyPropertyId`) with the node's keyboard shortcut, and raises a property change when it changes (the adapter's `properties!` table does both). Nothing else differs.

It is small enough to send upstream as one pull request: "Windows: expose the keyboard shortcut as AcceleratorKey". Once a release has it, remove this folder and the `[patch.crates-io]` line in the root `Cargo.toml`.

## Left out

The crate's own `Cargo.lock` and packaging files. It is its own package; the root workspace excludes it and patches crates.io's `accesskit_windows` with it.
