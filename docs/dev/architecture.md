# Architecture

diroxide is one Cargo workspace. One app core holds all state and returns effects; three frontends show it (the terminal, the graphical interface (GUI), and a headless JSON-RPC server). Every crate has one job. This page is the crate map: what each crate does, who owns it after Phase 0, and which workspace crates it may depend on. `cargo xtask deps --check` enforces the dependency lists below; the table it checks is in `xtask/src/deps.rs`, and the two must change together.

## How a key press travels

1. A frontend turns its own key event into a `diroxide_core::Key` and hands it to `App::handle` as `AppEvent::Key`.
2. The app looks the key up in the keymap's layers (panel, list, global; a dialog first when one is open), in the frontend's column (terminal or GUI). A bound key gives a command; there are no key sequences (ADR-0005); an unbound printable key jumps to the next row starting with that letter.
3. `App::dispatch` changes the app's state and returns `Effect`s.
4. The frontend handles the screen and speech effects (`Redraw`, `Announce`, `OpenDialog`, `CloseDialog`, `Quit`) and text for the clipboard (`SetClipboardText`), since each frontend owns its clipboard. The `Runtime` handles the I/O effects (`List`, `StartJob`, `CancelJob`, `RunPlugin`, `OpenExternally`, `SetClipboard`) and `AppRequests`, where `Runtime::carry_out` takes the app's own queued requests (plan a job, run a confirmed plan, answer a question, watch a folder, save the session), and turns their results into `AppEvent`s, which go back to `App::handle`.
5. A listing or job answer carries the request id it answers, so a late answer to an old request is dropped.

The app core never does I/O and never blocks; tests drive it without a terminal or the GUI.

## The crates

Each entry gives the crate's job, its owner after Phase 0, and the workspace crates it may depend on.

- `diroxide-core`: ids, `Place`, `Entry`, the list model (marks by row id, the frozen selection), commands and effects, the job vocabulary (conflict answers, job states, error actions), messages and their importance, the message catalog of the safety sentences, keys (one key with its modifiers), layers and the keymap's two columns, sorting, patterns, secrets, detail rows, the clock, errors and the `Explain` trait. No I/O. Finished by Phase 0 and the Wave 1 contract (P0b); changed only through a contract change request. Depends on no workspace crate.
- `diroxide-theme`: the themes, ported from textweaver's theme crate: the color and style roles, the 24 built-in themes, user themes, the WCAG contrast check with its verdict in words, the terminal renderer, the system appearance, and the file manager's roles (`PanelRole`). Owners: W1c for the role overrides and settings, W1d for applying them in the terminal. Depends on no workspace crate.
- `diroxide-vfs`: the `Provider` trait (the committing writer, seekable reads, links, restore, space, volumes), `OpContext` (cancellation, progress and secrets), capabilities, the trash gate and its post-check, the depth limits, the registry, the name checks, and the local provider with its `sys` platform module (the one module where `unsafe` may be allowed). Owner: W1a. May depend on core.
- `diroxide-config`: settings, the keymap file, file types, banks, conversions and tools, in TOML, with the config and state paths, and the session state restored at the next start. Owner: W1c. May depend on core and theme.
- `diroxide-ops`: the job engine: plans, runs, progress in words, conflicts one at a time, errors with their choices, job control, reports with the reconciliation, the undo log, and the fake provider for tests. Owner: W1b. May depend on core and vfs.
- `diroxide-archive`: the archive provider for zip, 7z, tar, tar.gz, tar.xz and xz, with limits, member name rules and the pack and unpack requests. Owner: W2a. May depend on core, vfs and ops.
- `diroxide-search`: name and content search, dirsize, duplicates, the results list. Owner: W2b. May depend on core and vfs.
- `diroxide-tools`: the external tools registry and safe running. Owner: W5a. May depend on core, vfs and config.
- `diroxide-filetype`: file type detection and the type registry with actions. Owner: W5c. May depend on core, vfs, config and tools.
- `diroxide-preview`: previews, text first. Owner: W5b. May depend on core, vfs, tools and filetype.
- `diroxide-diff`: compare two files in words (research/diffing.md): the line engine on `similar` behind one module, the change model and its counts, the ignore options, a deadline on every run, the word pass and the change window, what differs when old and new sound the same, the file-wide facts, the verdict and the rows at 40 cells, equality with an early exit, binary runs, and the driver that reads two files through the providers. Owner: W3d. May depend on core, vfs and preview (preview for the decoder's byte-order mark and encoding).
- `diroxide-plugins`: the JSON-RPC protocol, manifests, the host with its services, providers, listers and the conformance suite, the transport and script commands; no in-process tier (Lua scripts run through a Lua runner plugin). Owners: W4a-1, W4a-2 and W4b (alpha 2). May depend on core, vfs, ops, tools and config.
- `diroxide-components`: the optional components: the pins and the embedded list, the fetchers (the built-in client behind the `fetch` feature, then curl and wget from the PATH), the downloader with ask-first, resume and the hash check, install from a file, and the spoken lines. Owner: W4c (alpha 2). May depend on core, vfs, archive, tools and config.
- `diroxide-app`: the app core: panels, commands, keymap dispatch, menus, the palette, banks, dialogs, jobs, announcements and the runtime. Owners: W1c for `commands.rs`, `keymap.rs`, `menu.rs`, `palette.rs` and `settings.rs`; W1d for the rest; later waves add their own files. May depend on core, vfs, archive, ops, search, filetype, config, plugins, tools, preview, theme and diff.
- `diroxide-tui`: the terminal frontend. Owner: W1d. May depend on core, app and theme.
- `diroxide-gui`: the GUI frontend. Owners: W4a, then W4b. May depend on core, app and theme.
- `diroxide-cli`: the command line behind `diro`: open, serve, search, convert, plugin tooling. Owners: W1d, with W1e for `src/cmd/` and W3b for `src/cmd/plugin.rs`. May depend on every `diroxide-*` crate.
- `diro`: the thin binary that `cargo install diro` installs. May depend on cli only, and on `ureq` behind its default `fetch` feature, an entry of the native gate's inventory (ADR-0001).
- `xtask`: the workspace's own tasks. Owner: the orchestrator, with W1c, W1e and W4c adding their tasks. May depend on core, vfs, ops, archive, search, config, plugins, app and theme (plugins for the protocol reference, generated from its methods module; ops for the bench, which copies through the engine; archive for the bench's archive scenarios and the demo set, which pack, list and unpack through the provider and its runners, P2b; search for the bench's search scenarios, which run the search engine itself, W3s). Nothing depends on it.

Plan section 5.1 counts sixteen crates with `diroxide-cli` as "the diro binary". Phase 0 splits that into the library `diroxide-cli`, which holds the command line and is tested as a library, and the binary crate `diro`, so that `cargo install diro` is the install line (plan section 1). The Wave 1 contract adds the seventeenth crate of the plan's count, `diroxide-theme`, and moves the SFTP provider out of the main workspace (below). The compare crate of plan section 6, `diroxide-diff`, joined in the plan's alpha.3 (W3d), which ships as part of alpha 1. The main workspace has eighteen packages: sixteen `diroxide-*` crates, `diro` and `xtask`.

## Plugins, outside the main workspace

The plugin boundary is the language boundary (plan section 5.8, item 3; ADR-0001). Anything that needs C, assembly, another language or a license the main binary cannot carry is a plugin: a separate program in a workspace of its own under `plugins/`, which the root manifest's `exclude` keeps out, so `cargo build --workspace` and `cargo install diro` never see it. Pure Rust is a goal, not a rule (answer 46): a crate that carries C may enter the main workspace only through the inventory in `xtask/src/native.rs`, with its reason, and `cargo xtask native --check` fails on any other `cc`, `cmake` or `cxx` build dependency. The inventory today: `ring`, for the `fetch` feature, and `psm` and `stacker`, carta's stack growth for the quick view's readers (ADR-0001).

- `plugins/diro-sftp`: the SFTP provider plugin (Owner: W3c). Depends on core and vfs by path; russh and russh-sftp join its own manifest in the plan's alpha.5. Built with `cargo build --manifest-path plugins/diro-sftp/Cargo.toml`.

## Rules every crate keeps

- Third-party crates come from `[workspace.dependencies]` in the root `Cargo.toml`, taken with `name.workspace = true`.
- Public items have rustdoc; `missing_docs` and `unsafe_code` are denied workspace-wide.
- Library code returns `thiserror` errors that implement `diroxide_core::Explain`, so the app can say "Could not VERB OBJECT: CAUSE. NEXT STEP." the same way everywhere.
- No `unwrap` or `expect` outside tests; no `todo!`, `unimplemented!` or `dbg!`.
- Every user-visible string puts its meaning in its first 40 characters.

## See also

- [Building](building.md)
- [ADR-0001: Workspace and dependencies](../adr/0001-workspace-and-dependencies.md)
- [ADR-0007: Frontends over one core](../adr/0007-frontends-over-one-core.md)
