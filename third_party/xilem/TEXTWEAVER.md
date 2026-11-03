# Vendored Xilem and Masonry

This folder is a trimmed copy of [Xilem](https://github.com/linebender/xilem), Linebender's all-Rust GUI toolkit, used by textweaver's GUI crate, `crates/textweaver-xilem`. The decision is [ADR-0027](../../docs/adr/0027-xilem-gui.md).

## What is here

- **Upstream revision:** `271a27a6d4a930f7878d404f9014e3c50a3a9b88` on `main`, "xilem: Expose imaging backends as features (#1847)", committed on Monday, September 14, 2026. The crates call themselves version 0.4.0, the last release (October 29, 2025), but main has moved well past it.
- **Crates:** `masonry`, `masonry_core`, `masonry_winit`, `masonry_imaging`, `masonry_testing`, `tree_arena`, and `include_doc_path`, which textweaver uses; and `xilem`, `xilem_core`, and `xilem_masonry`, Xilem's reactive layer, kept so it can be adopted without another vendoring step.
- **Parley 0.11.1** (`parley/`), from crates.io (August 16, 2026), for its `accesskit` feature on AccessKit 0.25. The root `Cargo.toml` patches crates.io's `parley` with it. It replaced Parley 0.8.0 in Wave 5 (Agent W5a4); see ADR-0027's status update. Parley's main branch has since removed the `accesskit` feature (its integration moved to an example), so the next upgrade past 0.11 means Masonry's `TextArea` builds its own accessibility nodes; textweaver's document view already does.
- **Left out:** `xilem_web`, `placehero`, the examples, benches, stress tests, and the screenshot folders used by Masonry's own tests.

It is its own Cargo workspace. The root workspace excludes it (`exclude = ["third_party/xilem"]`) and depends on its crates by path.

## textweaver's changes

`textweaver.patch` holds every change against upstream, as a unified diff. In short:

1. **AccessKit 0.25.1 and accesskit_winit 0.34.1** (from 0.24 and 0.32.2), and accesskit_consumer 0.39.1 (from 0.35). This brings the Orca-detection fix (`accesskit_unix` 0.22 and later) that Xilem issue #1733 is about, plus text attributes on macOS and the AT-SPI EditableText and Document interfaces.
2. `masonry_core`: `accesskit::Tree` became `TreeInfo` in AccessKit 0.25.
3. `masonry_testing`: `accesskit_consumer::Node` became `NodeRef`. `TestHarness::access_node` now uses `node_by_tree_local_id` (AccessKit PR #707) instead of an `unsafe` write into a private `NodeId`, which closes the workaround for AccessKit issue #701.
4. **Parley 0.11.1**: its optional `accesskit` dependency moved from 0.24.0 to 0.25.1 (`parley/Cargo.toml`, the published manifest; `Cargo.toml.orig` is left as published). One source change: `bidi.rs` allows the deprecation of `BidiClass::to_icu4c_value` in icu_properties 2.3 (the workspace resolves 2.3; Parley asks for 2.1), because CI builds with `-D warnings`.
5. Manifests: the removed crates, examples, tests, and benches are gone from the workspace member list and the crate manifests, so the copy builds without them.

These are small enough to send upstream as one pull request: "Update to AccessKit 0.25 and accesskit_winit 0.34". See ADR-0027, "Upstream".

textweaver's own fixes, each small, each a candidate for its own pull request:

6. `masonry`: `TextArea` takes an accessible label (`with_accessible_label`), so a prompt's field has a name.
7. `masonry_winit`: `DriverCtx::exit` is honoured after an async action too, so a window can close on a timer or a background message.
8. `masonry_testing`: the harness registers fonts and draws at the scale factor it lays out at (the screenshots at 200%).
9. **Actions on nodes a widget adds itself** (Wave 3, second half). A widget may add AccessKit nodes that are not widgets: text runs, list options, the settings form's rows. Masonry sent an action aimed at one of them to a widget id that does not exist, which panics in a debug build. Now the accessibility pass records which widget added each such node (`RenderRootState::access_node_owners`), and the action goes to that widget, with the node named in the new `AccessEvent::node` field. The appended part of `textweaver.patch` (it applies after the earlier parts) holds this change.
10. `masonry`: a one-line `TextArea` (`InsertNewline::Never`) leaves Up and Down to its parent, as a one-line edit does on Windows, so a prompt can recall earlier answers with them.
11. `masonry`: forced by Parley 0.9 (Wave 5, W5a4). `Layout::align` no longer takes a width, so `Label` aligns its lines within the width they were broken at (or the layout's own width when it was not broken). Start-aligned labels, which are all textweaver uses, are unchanged; a centered or end-aligned label reused from the cache at a different width can sit differently. Upstream Masonry makes the same change when it moves to Parley 0.9.
12. `masonry_core`: a widget's tag is released when the widget is removed (`remove_child`). Before, the tag kept pointing at the removed widget, so a second dialog whose field or list had the same tag was not registered (an error in a release build, a panic in a debug one), and the next edit through the tag (a key in the second list, the palette's filter) reached a widget that no longer existed and panicked (Wave 6, W6a5). The appended part of `textweaver.patch` holds it. Worth sending upstream as its own pull request: "Release a widget's tag when the widget is removed".
13. `masonry_core`: the full ("dense") trace log of a debug build is written only to the folder the `MASONRY_DENSE_LOG_DIR` environment variable names (`app::DENSE_LOG_DIR_VAR`); unset or empty, no file is written. Upstream wrote `masonry-<time>-dense.log` to the system's temporary folder on every start and in every test program that uses the harness, which filled the temporary folder on the system drive (Wave 7, W7x; issue linebender/xilem#1556 tracks a better subscriber). A file that cannot be created is reported on standard error instead of panicking. The appended part of `textweaver.patch` holds it.

14. `parley`: a cluster's offset into its run's text (`ClusterData::text_offset`) is a `u32`, not a `u16` (Wave 8b, W8b-g). A run is text in one style, script, and font, so one very long line of plain text is one run; past 65,535 bytes the offset wrapped, and clusters, line ranges, cursors, and selection rectangles pointed at the wrong text. The frame-time probe found it as a line start inside a two-byte character on a 1 MB one-line file, and past 64 KB the spoken word's band was drawn at the wrong place. The appended part of `textweaver.patch` holds it. Worth sending upstream as its own pull request: "Allow runs longer than 64 KB".

15. `masonry_winit`: a redraw hands the accessibility tree update to AccessKit before it renders the frame, not after (Wave 8c, W8c-w). `redraw` rendered, presented, and waited for the GPU to finish, and only then sent the update, so the caret move that a screen reader and a Braille display follow arrived after the pixels, behind the GPU. The tree is complete before rendering starts, so the update is the same; only its timing moves. A screen magnifier may now move a few milliseconds before the pixels. The appended part of `textweaver.patch` holds it. Worth sending upstream as its own pull request: "Send the accessibility update before rendering".

16. `masonry_winit`: a window whose drawing surface cannot be created panics with a message that says so, starting with `GRAPHICS_FAILURE` ("the window could not start its graphics"), instead of a bare `unwrap` (Wave 9, W9a-w; GPU report QW4). textweaver's panic hook recognizes it and says, in the listener's language, that the window could not start its graphics and that the terminal reader needs none: in a message box when the window was started from a shortcut. The appended part of `textweaver.patch` holds it.

17. `masonry_winit`: the graphics adapter the first window's device was created on is kept, and `app::graphics_adapter()` returns its name, driver, graphics API and kind, with `software` true for a CPU renderer (Wave 9, W9a-w; GPU report QW2). textweaver writes it under `--log` and warns in `textweaver.log` when the adapter is a software renderer. The appended part of `textweaver.patch` holds it. Items 16 and 17 together are worth sending upstream as one pull request: "Report graphics start-up failures and the adapter in use".

18. `masonry`: the cursor's blink cycle in every `TextArea` is set by the app (`widgets::set_caret_blink_period`; `None` keeps the cursor steady) instead of a fixed second (Wave 9, W9a-w; GPU report QW6). textweaver sets it from Windows' cursor blink rate, steady when that is "none", which closes upstream's "should be reading from the system settings" note. The ten-second stop is unchanged. The appended part of `textweaver.patch` holds it.

19. `masonry_winit`: an opaque window gets an opaque drawing surface (alpha.9 fix, the owner's report). Upstream took the first alpha mode the surface offered, post- or premultiplied before opaque, whatever the window (its own TODO said so: winit cannot be asked whether a window is transparent). Vulkan on an NVIDIA card under Windows offers premultiplied alpha for an ordinary window and then composites the whole window with per-pixel alpha; Windows draws the native menu bar with alpha 0, so the menu bar was see-through, and white behind the window hid its text. Direct3D 12 offers only opaque for a window, which is why `--graphics dx12` looked right. Now the runner remembers which windows were created with `with_transparent(true)`, and only those get an alpha-blending surface; every other window gets `Opaque` when the surface offers it (it always does), which also saves power. The AMD blit rule is unchanged. `app::graphics_adapter()` also reports the first surface's alpha mode (`GraphicsAdapter::alpha_mode`), which textweaver writes on its `--log` "graphics adapter" line. The appended part of `textweaver.patch` holds it. Worth sending upstream as its own pull request: "Use an opaque surface for an opaque window".

20. `masonry_core`: `app::try_init_tracing_with_span_times(name, report)` installs the same trace output as `try_init_tracing`, plus a layer that calls `report` with how long each span named `name` lasted, from its creation to its close (beta 1, G1-a). The layer has its own filter, so it turns on no other span or event. textweaver times `masonry_winit`'s `redraw` span under `--log` and writes the median, the 95th percentile, and the worst frame time every 200 frames. The appended part of `textweaver.patch` holds it.

21. `masonry_winit`: only the plain paste chord (Ctrl+V, or Cmd+V on macOS) is turned into a `ClipboardPaste` event (beta 3). Upstream took Ctrl (or Cmd) with V and any other modifier, so Ctrl+Shift+V and Ctrl+Alt+V never reached the app as keys. With Shift or Alt the key now arrives as a keyboard event, so textweaver can bind Ctrl+Shift+V to Paste as plain text and Ctrl+Alt+V to Choose a voice. The appended part of `textweaver.patch` holds it.

22. `masonry`: a `TextArea` can be a password field, and tells its owner where an edit was (beta 5). `set_password` makes an editable area a `PasswordInput` to accessibility APIs (UIA's `IsPassword` on Windows), so a screen reader does not echo what is typed into a masked field. `selection_byte_range` gives the selection, and `selection_before_edit` the selection just before the latest text event (a key, a paste, an input method's text). textweaver's masked prompt shows one star per character, and with these two it works out what each edit replaced, so typing or deleting in the middle of a password changes the middle and a typed `*` is an asterisk. The appended part of `textweaver.patch` holds it. Worth sending upstream as its own pull request: "A password mode for TextArea".

## Updating

1. Pick a new upstream revision and copy the crates listed above over this folder, leaving out the same things.
2. Apply `textweaver.patch` (parts may already be upstream; drop those).
3. Build and test `textweaver-xilem`, then regenerate the patch and update the revision here, in the workspace manifest's comment, and in ADR-0027.

The licences travel with the code: Xilem and Masonry are Apache-2.0 (`LICENSE`), Parley is Apache-2.0 or MIT (`parley/LICENSE-APACHE`, `parley/LICENSE-MIT`).
