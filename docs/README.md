# diroxide documentation

diroxide (Directory Oxide) is a two-panel file manager written in pure Rust, keyboard first and accessible first. Its command is `diro`. It is in its alpha series: the guides here describe what alpha 1 does, and each later alpha adds its own guides.

This is the index of every diroxide document, grouped by audience:

- [People who use diroxide](#for-people-who-use-diroxide): installing, moving around, operations, files and archives, finding things, keys, settings and screen readers.
- [People who build diroxide](#for-people-who-build-diroxide): the plan, the architecture, building and the clean-room rule.
- [The decisions](#decisions): the architecture decision records.

If you are new, begin with the [quick start](quickstart.md).

## Start here

- [Quick start](quickstart.md): from a built program to copying, renaming and deleting a file, in ten minutes.
- [Known limits](known-limits.md): what does not work yet, and what has not been tested.
- [Privacy](privacy.md): what stays on your computer, and when diroxide uses the network. In alpha 1 it does not.
- [Accessibility statement](accessibility.md): what diroxide promises, how it is checked, and what has and has not been heard.
- [What is new](../CHANGELOG.md): the changelog.

## For people who use diroxide

### Getting started

- [Quick start](quickstart.md): starting, moving, marking, copying, renaming, deleting, undoing and quitting, and a first look at search, labels, compare, archives and the viewer.
- [Installing diroxide](install.md): building from source today (a Rust toolchain and a C compiler), the one file per platform that comes in a later alpha, and `cargo install diro` with the first beta.
- [Using diroxide with a screen reader](screen-readers.md): how diroxide talks to NVDA, JAWS, Orca and VoiceOver, what you should hear, the settings to check, and the Braille display.
- [Terminals](terminals.md): which terminal to use, the keys terminals keep, Windows Terminal's own keys and how to give them back, the key test, and `diro --inline`, `diro ls` and `diro search` for when the full screen fails.
- [Troubleshooting](troubleshooting.md): common problems, and how to report a bug.

### Using the panels

- [The panels](panels.md): the three layouts, what a row says, moving, going to a path or a place, path jump, sorting, filtering and marking rows.
- [Operations and undo](operations.md): copy, move, rename, make folder, duplicate, the trash, deleting for good, the file clipboard, prompts with completion, jobs, progress and undo.

### Archives, files and previews

- [Archives](archives.md): opening zip, 7z, tar and xz archives like folders, copying out, unpacking, packing, testing, passwords, split archives, the limits that protect you, and `diro archive`.
- [The viewer](viewer.md): F3, lines, wrap, jump, search, hex mode, following a log, large files, and code read by ear.
- [The quick view](quick-view.md): a live preview of the focused file in the other panel, by heading, table and link, and the formats it reads.
- [Split and join](split-join.md): cutting a big file into parts for mail, GitHub, a disc or FAT32, joining them back, the checksum file, `diro split` and `diro join`.
- [Links, permissions and times](links-permissions.md): symbolic links and junctions, permissions said in words, the recursive option, times, and the details list.

### Finding, remembering and comparing

- [Search and the filter language](search.md): the one filter language that the panel filter, select by pattern, Search and saved searches share, twenty examples, results, the Space list, newest files, and `diro search`.
- [Labels and bookmarks](labels-bookmarks.md): labels on files, bookmarks on folders, the History list, a view saved for one folder, and saved searches.
- [Compare files](compare.md): what two files differ in, said in words first, the change list, the cases that sound the same, binary and large files, and `diro diff`.
- [Watching a folder](watching.md): hearing when a new file has finished arriving, in the program and with `diro watch`.

### Keys and settings

- [Every key](keyboard.md): the default keys for the terminal and the graphical interface (GUI), the keys a terminal may keep and the keys screen readers keep, generated from the keymap. Each key preset has its own sheet, linked from [customizing keys](customizing-keys.md).
- [Customizing keys](customizing-keys.md): the Keys list, changing a key, the key test, the key presets, `diro keys` and `keymap.toml`.
- [Settings and where the files live](settings.md): the config and state folders, the settings most people look for, themes, changing keys, menus and banks.
- [Settings reference](settings-reference.md): every setting with its default, range and help, generated from the program's own list of settings.

### Coming in later alphas

These guides are not written yet, because the features they describe are not in alpha 1. Each arrives with the alpha that ships its feature.

- Plugins, writing a plugin, script commands and optional components.
- SSH and remote places.
- The GUI.
- Tools and conversions, previews of more kinds of document, and git.
- Tabs and the folder tree, comparing and synchronizing two folders, saved tasks, duplicates, batch rename, disk images and GitHub.

## For people who build diroxide

- [The plan](plan.md): what diroxide is, its architecture, the alphas and the waves. Its alpha names are the plan's own stages; the first three ship together as alpha 1.
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
- [ADR-0007: Frontends over one core](adr/0007-frontends-over-one-core.md): the terminal, the GUI and the headless server over one app core.
- [ADR-0008: Safety rules](adr/0008-safety-rules.md): lists before deletes, literal paths, the trash by default and confirmations that start on No.
- [ADR-0009: Secrets and the masked prompt](adr/0009-secrets-and-the-masked-prompt.md): one secret type that is wiped and never logged, and a password prompt that says what it is for.

## See also

- [The project README](../README.md)
- [Quick start](quickstart.md)
