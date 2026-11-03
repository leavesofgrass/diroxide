# Vendored code

diroxide carries two folders of third-party Rust code under `third_party/`, copied from textweaver, the owner's other project, which maintains them: the GUI toolkit and AccessKit's Windows adapter, each with the patches textweaver carries against upstream. The GUI crate (`crates/diroxide-gui`) builds on them from the GUI alpha on (plan section 5, "GUI", and ADR-0007).

## What is here

- `third_party/xilem`: a trimmed copy of Linebender's Xilem repository at upstream revision 271a27a6d4a930f7878d404f9014e3c50a3a9b88 ("xilem: Expose imaging backends as features", Monday, September 14, 2026): the crates `masonry`, `masonry_core`, `masonry_winit`, `masonry_imaging`, `masonry_testing`, `tree_arena`, `include_doc_path`, `xilem`, `xilem_core` and `xilem_masonry`, and Parley 0.11.1 from crates.io. It is its own Cargo workspace. `third_party/xilem/TEXTWEAVER.md` lists every change textweaver made, and `textweaver.patch` holds them as one unified diff against upstream: AccessKit 0.25.1 and accesskit_winit 0.34.1, the accessible label on a text area, actions on nodes a widget adds itself, the tag released when a widget is removed, the tree update handed to AccessKit before the frame is rendered, and the rest.
- `third_party/accesskit_windows`: AccessKit's Windows adapter 0.35.1 from crates.io with one change, in `textweaver.patch`: the node's keyboard shortcut answered as UI Automation's AcceleratorKey property, so a screen reader says a button's key only when its own setting asks for it.

## The sync rule

- The copy is taken whole from textweaver's `main` with `git archive`, never edited by hand here. A change diroxide needs goes to textweaver first, where the patches live and are tested against its GUI, then the copy is refreshed and this page's revision line updated. The first copy was made on Monday, November 2, 2026, from textweaver's commit abd9585b, with every file's hash checked against textweaver's tree.
- The root workspace excludes `third_party/xilem` and depends on its crates by path, with `[patch.crates-io]` lines for `parley` and `accesskit_windows`, as textweaver's root manifest does (the GUI alpha's contract step wires them).
- The licenses are upstream's: Apache-2.0 for Xilem, Masonry and Parley, Apache-2.0 OR MIT for AccessKit; `deny.toml` lists them, and `cargo xtask native --check` keeps the inventory of anything in the stack that compiles C.
- Both folders stay out of the clean-room grep, the docs check and the format check, which cover diroxide's own code and pages.

## See also

- [Architecture](architecture.md)
- [ADR-0007: Frontends over one core](../adr/0007-frontends-over-one-core.md)
