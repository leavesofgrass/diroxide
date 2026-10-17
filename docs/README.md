# diroxide documentation

diroxide (Directory Oxide) is a two-panel file manager written in pure Rust, keyboard first and accessible first. Its command is `diro`. It is in its first alpha series: the guides here describe what alpha.1 and alpha.2 do, and each later alpha adds its own guides.

This is the index of every diroxide document, grouped by audience:

- [People who use diroxide](#for-people-who-use-diroxide): installing, moving around, operations, settings and screen readers.
- [People who build diroxide](#for-people-who-build-diroxide): the plan, the architecture, building and the clean-room rule.
- [The decisions](#decisions): the architecture decision records.

If you are new, begin with the [quick start](quickstart.md).

## Start here

- [Quick start](quickstart.md): from a built program to copying, renaming and deleting a file, in ten minutes.
- [Known limits](known-limits.md): what does not work yet, and what has not been tested.
- [Privacy](privacy.md): what stays on your computer, and when diroxide uses the network. In alpha.1 it does not.
- [Accessibility statement](accessibility.md): what diroxide promises, how it is checked, and what has and has not been heard.
- [What is new](../CHANGELOG.md): the changelog.

## For people who use diroxide

### Getting started

- [Quick start](quickstart.md): starting, moving, marking, copying, renaming, deleting, undoing and quitting.
- [Installing diroxide](install.md): building from source today (a Rust toolchain and a C compiler), the one file per platform that comes with alpha.6, and `cargo install diro` at beta.1.
- [Using diroxide with a screen reader](screen-readers.md): how diroxide talks to NVDA, JAWS, Orca and VoiceOver, what you should hear, the settings to check, and the Braille display.
- [Terminals](terminals.md): which terminal to use, the keys terminals keep, Windows Terminal's own keys and how to give them back, the key test, and `diro --inline`, `diro ls` and `diro search` for when the full screen fails.
- [Troubleshooting](troubleshooting.md): common problems, and how to report a bug.

### Using the panels

- [The panels](panels.md): the three layouts, what a row says, moving, going to a path or a place, path jump, sorting, filtering and marking rows.
- [Operations and undo](operations.md): copy, move, rename, make folder, duplicate, the trash, deleting for good, the file clipboard, prompts with completion, jobs, progress and undo.
- [Every key](keyboard.md): the default keys for the terminal and the window, the keys a terminal may keep and the keys screen readers keep, generated from the keymap.

### Archives, files and previews (alpha.2)

- [Archives](archives.md): opening zip, 7z, tar and xz archives like folders, copying out, unpacking, packing, testing, passwords, split archives, the limits that protect you, and `diro archive`.
- [The viewer](viewer.md): F3, lines, wrap, jump, search, hex mode, following a log, large files, and code read by ear.
- [The quick view](quick-view.md): a live preview of the focused file in the other panel, by heading, table and link, and the formats it reads.
- [Split and join](split-join.md): cutting a big file into parts for mail, GitHub, a disc or FAT32, joining them back, the checksum file, `diro split` and `diro join`.
- [Links, permissions and times](links-permissions.md): symbolic links and junctions, permissions said in words, the recursive option, times, and the details list.

### Settings

- [Settings and where the files live](settings.md): the config and state folders, the settings most people look for, themes, changing keys, menus and banks.
- [Settings reference](settings-reference.md): every setting with its default, range and help, generated from the program's own list of settings.

### Coming with later alphas

These guides are not written yet, because the features they describe are not in alpha.1. Each arrives with the alpha that ships its feature.

- Search, filters, labels and bookmarks, the Keys list and file compare: comes with alpha.3.
- Plugins, writing a plugin, script commands and optional components: comes with alpha.4.
- SSH and remote places: comes with alpha.5.
- The window: comes with alpha.6.
- Tools and conversions, previews, and git: comes with alpha.7.
- Tabs and the folder tree, synchronize and saved tasks, duplicates, batch rename, disk images and GitHub: comes with alpha.8.

## For people who build diroxide

- [The plan for the first alpha](plan.md): what diroxide is, its architecture, the alphas and the waves.
- [Architecture](dev/architecture.md): the crates, what each does, and which way dependencies point.
- [Building](dev/building.md): the toolchain, the checks, the profiles and the tasks.
- [Docker](dev/docker.md): the Linux container for builds and tests.
- [The bench](dev/bench.md): the speed measurements that gate a change.
- [The clean-room rule](dev/clean-room.md): how diroxide stays a reimplementation of Worker's behavior, never a port of its code.
- [The changelog](../CHANGELOG.md): what changed in each alpha.

## Decisions

Each architecture decision record keeps one decision: the context, the choice, the alternatives and the fallback. A dated status update says what changed later. [The index of decisions](adr/README.md) lists them with their statuses.

- [ADR-0001: Workspace and dependencies](adr/0001-workspace-and-dependencies.md): one crate per job, one table of approved dependencies, and the pure-Rust rule.
- [ADR-0002: The core model](adr/0002-the-core-model.md): places, entries, display names and the list model.
- [ADR-0003: The provider trait](adr/0003-the-provider-trait.md): how every kind of place is listed, read and written through one trait.
- [ADR-0004: The job engine](adr/0004-the-job-engine.md): plans, progress, conflicts and undo.
- [ADR-0005: Commands, keys and menus from one model](adr/0005-commands-keys-and-menus.md): one model for menus, the palette, banks and keys.
- [ADR-0006: The plugin protocol](adr/0006-the-plugin-protocol.md): plugins over JSON-RPC in any language.
- [ADR-0007: Frontends over one core](adr/0007-frontends-over-one-core.md): the terminal, the window and the headless server over one app core.
- [ADR-0008: Safety rules](adr/0008-safety-rules.md): lists before deletes, literal paths, the trash by default and confirmations that start on No.
- [ADR-0009: Secrets and the masked prompt](adr/0009-secrets-and-the-masked-prompt.md): one secret type that is wiped and never logged, and a password prompt that says what it is for.

## See also

- [The project README](../README.md)
- [Quick start](quickstart.md)
