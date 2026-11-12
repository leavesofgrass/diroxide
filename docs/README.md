# diroxide documentation

diroxide (Directory Oxide) is a two-panel file manager written in pure Rust, keyboard first and accessible first. Its command is `diro`. It is in its alpha series: the guides here describe what alpha 1 does and what has been added since, and each later alpha adds its own guides.

This is the index of every diroxide document, grouped by audience:

- [People who use diroxide](#for-people-who-use-diroxide): installing, the GUI, moving around, operations, volumes and the trash, remote places and the queue, files, archives, disk images and previews, columns, checksums, encryption and signatures, photos, privacy before sharing, conversions and watched folders, finding things, plugins, git, hooks, scripts and optional components, keys, settings and screen readers.
- [People who build diroxide](#for-people-who-build-diroxide): the plan, the architecture, building and the clean-room rule.
- [The decisions](#decisions): the architecture decision records.

If you are new, begin with the [quick start](quickstart.md).

## Start here

- [Quick start](quickstart.md): from a built program to copying, renaming and deleting a file, in ten minutes.
- [Known limits](known-limits.md): what does not work yet, and what has not been tested.
- [Privacy](privacy.md): what stays on your computer, what plugins can see, where the secrets of remote places live, and when diroxide uses the network: a download that asks first, and the servers you open.
- [Security](security.md): what a plugin, a script and a downloaded component can and cannot do, what the SFTP places plugin trusts and never does, and what diroxide checks.
- [Accessibility statement](accessibility.md): what diroxide promises, how it is checked, and what has and has not been heard.
- [What is new](whats-new.md): what each alpha added, in plain words. [The changelog](../CHANGELOG.md) has the full list.

## For people who use diroxide

### Getting started

- [Quick start](quickstart.md): starting, moving, marking, copying, renaming, deleting, undoing and quitting, and a first look at search, labels, compare, archives and the viewer.
- [Installing diroxide](install.md): building from source (a Rust toolchain and a C compiler), the one file per platform, what the window needs and costs to build, a smaller or terminal-only build, and `cargo install diro` with the first beta.
- [The GUI](gui.md): the window version of diroxide: how `diro` chooses between the window and the terminal, how it draws with or without a graphics card, the window in reading order, its keys and menus, the viewer and the quick view, its dialogs, jobs and settings, what it says and when, focus mode and full screen, and what it does not do yet.
- [Using diroxide with a screen reader](screen-readers.md): how diroxide talks to NVDA, JAWS, Orca and VoiceOver, what you should hear in the terminal and in the window, the settings to check, plugin lists, and the Braille display.
- [Terminals](terminals.md): which terminal to use, the keys terminals keep, Windows Terminal's own keys and how to give them back, the key test, and `diro --inline`, `diro ls` and `diro search` for when the full screen fails.
- [Troubleshooting](troubleshooting.md): common problems, and how to report a bug.

### Using the panels

- [The panels](panels.md): the three layouts, what a row says, moving, going to a path or a place (with its site, volume and trash rows), path jump, sorting, filtering and marking rows.
- [Tabs](tabs.md): several folders in one panel, each kept as you left it, said in words ("Documents, tab 2 of 3"), the lock and the pin, Undo close tab, the tab list, tab sets saved by name, and what is restored at the next start.
- [Columns, notes and fields](columns.md): a song's artist or a document's pages said in the row, your own notes and fields, a pattern over the name, a plugin's column, the three loading modes, sorting by a column, and Fill columns.
- [Operations and undo](operations.md): copy, move, rename, make folder, duplicate, the trash, deleting for good, the file clipboard, prompts with completion, jobs, progress and undo. [The transfer queue](transfer-queue.md) goes further into jobs.
- [Volumes and the trash](volumes-trash.md): the Volumes list with its state-first rows, free space, Open, Mount, Unmount, Eject and Hide, the Volume actions plugin, the trash browser with Restore, Delete for good and Empty the trash, and free space in the copy plan and in Say Status.

### Remote places and the queue

- [Remote places over SFTP](remote-places.md): what you need, `diro-sftp doctor`, opening a place, the new server key and password questions, a changed key, saved sites, the three ways to keep a password, working in a remote folder, and what goes wrong.
- [The transfer queue](transfer-queue.md): the Jobs list, waiting your turn, a server that stops answering, retries and the error policy, resume, the speed limit, and quitting with jobs unfinished.

### Archives, files and previews

- [Archives](archives.md): opening zip, 7z, tar and xz archives like folders, copying out, unpacking, packing, testing, passwords, split archives, RAR and the other formats that 7-Zip reads, deleting, renaming and making folders inside a zip with the old copy kept in the trash, the limits that protect you, and `diro archive`.
- [Disk images](disk-images.md): opening an ISO, a raw disk or a virtual disk like a folder, its partitions, copying files out, what the plugin reads itself and what it hands to 7-Zip, the limits that protect you, and what it does not do yet.
- [The viewer](viewer.md): F3, lines, wrap, jump, search, hex mode, following a log, large files, and code read by ear.
- [The quick view](quick-view.md): a live preview of the focused file in the other panel, by heading, table and link, and the formats it reads.
- [Conversions](conversions.md): Convert and its recipe list, the plan you answer first, a report and Undo, the 31 built-in recipes, a file type that names its recipe, your own `conversions.toml`, and `diro convert`.
- [Previews and the quick view's formats](previews.md): what pictures, drawings, sound and video, archives, PDF, Word, EPUB, RTF and spreadsheets say, the Preview command and auto-preview, Read text in picture, and the limits that protect the quick view.
- [The details view](details.md): Alt+Enter, the facts in their order, who can do what, the Privacy and Checksum rows, Show all with ExifTool, Say details, Copy details and Copy all details.
- [File types and groups](file-types.md): what a row says about a file, how a type is found, the eight groups and the screenshot names, the retro pack, your own `filetypes.toml`, typed actions, Open with on each system, and the encoding chooser.
- [Privacy before sharing](before-sharing.md): Check before sharing says what a file carries that could say who made it, where or with what, Strip metadata into a copy for JPEG, PNG and WebP, and Remove the internet mark with Undo.
- [Checksums and verifying](checksums.md): Checksum with the value to read, copy, compare with a pasted one or save as `SHA256SUMS`, Verify checksum against the list beside a download, the verdict first and what a match does and does not show, and `diro sum`.
- [Encryption and signatures](encryption.md): the File menu's Encryption submenu: age with a passphrase or a key through the Encryption plugin, gpg to encrypt, decrypt and sign, Verify signature for gpg and minisign, the originals and the trash, what is never stored, and the signature sentences in Verify checksum.
- [Photos](photos.md): the photo columns, Sort by date taken, Next day and Previous day, selecting the screenshots, the bursts and the pictures with no date, Review pictures with one key an answer, and Move into month folders with one Undo.
- [Split and join](split-join.md): cutting a big file into parts for mail, GitHub, a disc or FAT32, joining them back, the checksum file, `diro split` and `diro join`.
- [Links, permissions and times](links-permissions.md): symbolic links and junctions, permissions said in words, the recursive option, who can do what on each system, Change time with a typed date, Change owner and its refusals, and the details list.

### Finding, remembering and comparing

- [Search and the filter language](search.md): the one filter language that the panel filter, select by pattern, Search and saved searches share, twenty examples, results, the Space list, newest files, and `diro search`.
- [Labels and bookmarks](labels-bookmarks.md): labels on files, bookmarks on folders, the History list, a view saved for one folder, and saved searches.
- [Compare files](compare.md): what two files differ in, said in words first, the change list, the cases that sound the same, binary and large files, and `diro diff`.
- [Watching a folder](watching.md): hearing when a new file has finished arriving, in the program and with `diro watch`.
- [Watched folders](watched-folders.md): a folder that converts, moves, copies or notes each file that arrives, its task file, the approval you give first, the Watched folders list, retries and pauses, and `diro watch --task`.

### Plugins, scripts and optional components

- [Plugins](plugins.md): what a plugin is, the three kinds, turning one on and what the question says, managing them, what you hear when one runs, the keys a plugin's commands get, the eleven examples, and `diro plugin`.
- [Hooks](hooks.md): a plugin asked before a copy, move, delete, rename or new folder, and told afterward; what a stop says; the Folder guard example.
- [Script commands](script-commands.md): a command from one file in bash, Python or PowerShell: the header, what the script is given, directives, output modes, exit codes, which program runs it on each system, and `--explain`.
- [Writing a plugin](writing-a-plugin.md): `diro plugin new`, the manifest, the protocol in a page, the services diroxide offers, lists of rows that are not files, places, previewers and converters, and the conformance checks.
- [Optional components](components.md): helper programs, plugins and data packs, among them the SFTP places, disk image, git and encryption plugins and the helper programs, fetched only after you say yes, checked before use, offered once in a first-run list, installed from a file with no network, and removed by a list. Includes `diro components`.
- [Git](git.md): the git plugin: the branch and the changes in the title line and the Git column, the status list, staging and committing, diffs, the log and blame, branches and stashes, fetch, pull and push, clone, and the trust rule for a repository you did not make.
- [Tools and diro doctor](tools.md): the helper programs diroxide can use (ImageMagick, FFmpeg, pandoc and the rest), how each is found, `tools.toml`, the Tools list in the program, the install command for each system, and `diro doctor` with its sections and exit codes.
- [Security](security.md): what diroxide checks, what it protects, and what a plugin could still do.

### Keys and settings

- [Every key](keyboard.md): the default keys for the terminal and the graphical interface (GUI), the keys a terminal may keep and the keys screen readers keep, generated from the keymap. Each key preset has its own sheet, linked from [customizing keys](customizing-keys.md).
- [Customizing keys](customizing-keys.md): the Keys list, changing a key, the key test, the key presets, `diro keys` and `keymap.toml`.
- [Settings and where the files live](settings.md): the config and state folders, the settings most people look for, themes, changing keys, menus and banks.
- [Themes and colors](themes.md): the 24 themes with their contrast verdicts in words, the ten that fall short and what the window does for them, your own colors, Windows' High Contrast, and following the system's light or dark setting.
- [Settings reference](settings-reference.md): every setting with its default, range and help, generated from the program's own list of settings.

### Coming in later alphas

These guides are not written yet, because the features they describe are not built, or are built in the library and not yet in the program. Each arrives with the alpha that ships its feature.

- The folder tree, comparing and synchronizing two folders, tasks other than watched folders such as synchronizing and renaming, duplicates, batch rename, thumbnails and a grid for the photo helpers, an editor for extended attributes, writing to disk images, and GitHub.

## For people who build diroxide

- [The plan](plan.md): what diroxide is, its architecture, the alphas and the waves. Its alpha names are the plan's own stages; the first three ship together as alpha 1.
- [Architecture](dev/architecture.md): the crates, what each does, and which way dependencies point.
- [Building](dev/building.md): the toolchain, the checks, the profiles and the tasks.
- [The plugin protocol reference](dev/plugin-protocol.md): every request and notification of protocol 1, generated from the code.
- [Docker](dev/docker.md): the Linux container for builds and tests, and its three stages: dev, gui and tools.
- [Components](dev/components.md): the definitions and the pins of the optional components, every field of a component and its files, `cargo xtask components` and its flags, the pin job and the monthly run.
- [The hot-folder engine](dev/hot-folders.md): the modules behind watched folders, one file from start to end, what a failure means, how a file still being written is told on each system, and the exit codes.
- [The bench](dev/bench.md): the speed measurements that gate a change.
- [The clean-room rule](dev/clean-room.md): how diroxide stays a reimplementation of Worker's behavior, never a port of its code.
- [Fuzzing](dev/fuzzing.md): the fuzz targets in five workspaces, running one, and the nightly run.
- [Vendored code](dev/vendoring.md): the two folders of third-party Rust code under `third_party/`, copied from textweaver, and the patches they carry.
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
- [ADR-0010: The optional components](adr/0010-the-optional-components.md): plugins, helper programs and data packs that come only after a yes.
- [ADR-0011: The keyring and stored secrets](adr/0011-the-keyring-and-stored-secrets.md): where stored secrets live.
- [ADR-0012: SFTP over the OpenSSH client](adr/0012-sftp-over-the-openssh-client.md): the SFTP places plugin and the askpass helper.
- [ADR-0013: The job queue and resume](adr/0013-the-job-queue-and-resume.md): the job queue, the connection gate and resume.
- [ADR-0014: Software drawing and the Windows subsystem](adr/0014-software-drawing-and-the-windows-subsystem.md): drawing without a graphics card, and one console program on Windows.
- [ADR-0015: Encryption and signatures](adr/0015-encryption.md): age in a plugin, gpg as a program and minisign in process, and why each is where it is.

## See also

- [The project README](../README.md)
- [Quick start](quickstart.md)
