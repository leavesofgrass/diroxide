# Directory Oxide (diroxide): the plan for the first alpha

Written on Friday, October 9, 2026, by the planning session (Fable 5.1), for the owner. It is a proposal: the owner adopts it, or parts of it, and the orchestrator applies it to the task log. Nothing in it starts until the owner says go. Every date and weekday in it was computed from the machine's clock.

What it is built on: the Worker 5.4.0 source at `D:\worker-5.4.0` (191,399 lines of C++ under `src/`, GPL-2.0-or-later) and its documentation; the owner's wiki, above all the textweaver hub, its wave planning pages, the Research Agent Rules and the Wave 4 orchestration plan; the textweaver repository's conventions (`CLAUDE.md`, the shared preamble in the planning repository, the guard, `safe-remove.ps1`, the build profiles, the Docker setup, ADRs 0027, 0043 and 0045); abax's earlier Worker-style file manager; and today's measurements of this machine. The two surveys that fed it (Worker's architecture, and the textweaver method) are kept with the planning records.

## The short version

- **What it is.** A two-panel file manager in pure Rust, in the orthodox style of Worker and Directory Opus, built accessibility-first for the owner's screen readers and Braille display, with a terminal frontend, a window frontend on AccessKit, and a plugin system that takes bash, Python and Lua.
- **License.** Apache-2.0 OR MIT, the Rust convention, decided by the owner on Friday, October 9, 2026 (question 2). Worker is GPL-2.0-or-later, so this is a reimplementation of its behavior, never a port of its code. The clean-room rule in section 3 keeps that true.
- **Name.** The project is diroxide. The short command cannot be `do`: `do` is a reserved word in sh, bash, zsh and PowerShell, so typing it at a prompt is a syntax error. The command is `diro`, decided by the owner on Friday, October 9, 2026 (question 1).
- **Shape.** One app core that holds all state and returns effects; three frontends over it: the terminal reader on ratatui, the window on Masonry and AccessKit (the stack textweaver already carries), and a headless JSON-RPC server for scripts and tests. The same list model, menu model and keymap serve all three, as textweaver's do.
- **Plugins.** Process plugins over JSON-RPC 2.0 on standard input and output, in any language; "script commands" for one-file bash and Python scripts with no protocol code at all; an optional in-process Lua tier for the fastest hooks; a transport designed so WebAssembly can be added later without changing the plugin API.
- **The must-haves.** Archives in both directions: zip, 7z, tar, tar.gz, tar.xz and xz, all in pure Rust. Regex search at ripgrep speed, on names and contents. ImageMagick, FFmpeg and pandoc found and driven through one tools registry, for previews that are text first and for conversions that run as jobs.
- **The alphas.** Seven: two panels (alpha.1); archives, search and the viewer (alpha.2); plugins and SSH (alpha.3); the window (alpha.4); previews and conversions (alpha.5); the modern features, tabs, flat view, collections, compare, batch rename, volumes (alpha.6); and the feature-complete final alpha (alpha.7). The final alpha is what most projects call 1.0.
- **The method.** The textweaver method, unchanged where it worked: a Phase 0 contract first, waves of three to five building agents in their own worktrees with crate ownership, one app-wiring agent per sub-wave, batch merges, CI watched by run id, the guard and `safe-remove.ps1`, dates from the clock, and the owner's sessions as follow-ups rather than gates. Fable plans, Opus builds, Sonnet researches and writes prose, Haiku watches.
- **Pace.** textweaver went from an empty folder to a feature-complete alpha in two weeks with this method. diroxide is smaller in scope and reuses that infrastructure, so four to six weeks of wall time to the final alpha is plausible if the owner keeps the same pace. Nothing here is a promise or a date.
- **Twelve questions** for the owner are in section 13, each with a default so that a "go" alone is enough to start.

## 1. What diroxide is

### The pitch

An orthodox file manager: two panels side by side, one active, one the target. Everything is a command, every command has a key, a name in the palette and a place in the menus, and every command can be bound to a button in a bank, as Worker's can. It is fast because it is Rust and because it never blocks the interface: listing, searching, copying and archiving run as jobs that report progress in words. It is safe because the owner's own rules from September 26, 2026 are built into the product (section 5.12). It is modern because it runs on Wayland, Windows and macOS through winit with no X11 code, and because the things Worker left to Lua are open to any language.

### Who it is for

The owner first: blind, reading through JAWS and NVDA and a 40-cell HumanWare Mantis Q40, and red-green color-blind. Every row of a panel puts its meaning in the first 40 cells. Status is words, never a symbol or a color. Then power users who want Worker or Directory Opus without X11 and without a proprietary license.

### Names

- **Project:** diroxide, "Directory Oxide". Folder `D:\diroxide`, crates `diroxide-*`, lowercase everywhere, as textweaver is written.
- **Command:** `diro`. The owner suggested `do`, which is a reserved word in every shell the project targets (`for ... do ... done` in sh, bash and zsh; `do { } while` in PowerShell), so a binary named `do` cannot be run by typing its name. `diro` is four letters, reads as one word in speech, and is free on PATH on all three platforms. Alternatives if the owner prefers: `dxo`, `dro`. (Question 1.)
- **Config folder:** `diroxide` under the platform's config directory (`%APPDATA%\diroxide`, `~/.config/diroxide`, `~/Library/Application Support/diroxide`).

### License

Apache-2.0 OR MIT, dual, as Rust crates are. Every dependency in section 6 is compatible. `cargo deny` enforces the allow-list from the first commit. The copyright line follows textweaver's precedent: the owner's name in `NOTICE`, the README and `diro --version`; "the owner" everywhere else. (Question 2.)

### The icon

A rusty file cabinet: two drawers for the two panels, the top one pulled open a little, rust orange on steel blue (a color-blind-safe pair), with no text in the mark. One design agent produces three SVG concepts in the planning repository, as was done for the textweaver logo, and the owner picks one. The chosen mark is rasterized by `cargo xtask icons` with resvg (pure Rust) into PNG at 16 to 512 pixels, a Windows `.ico`, a macOS `.icns` and a Linux `.desktop` entry, and gets an alt text that says what it is. It lands in alpha.4 with the window.

## 2. What Worker does, and what carries over

Worker is 191,399 lines of C++ in 517 source files, carrying its own X11 widget toolkit (AGUIX), its own configuration grammar (a yacc and lex parser), its own JSON parser, and the glue for AVFS, libmagic, HAL and uDisks. The Wave 0 parity inventory (section 9) verifies and completes the list below from the source, with file and line citations, the way textweaver's Star inventory did. What follows is the planning session's reading of the source listing and the documentation; items marked "verify" are expected but not yet confirmed line by line.

### Carried over as features

- **Two listers** with an active and an inactive side, Tab between them, each with its own path, sort order, filter, selection and column set.
- **Columns** chosen per lister: name, size, type, permissions, owner, group, modification time, link destination, and more. Verify the full set.
- **Buttons in banks**: configurable rows of buttons, each running a list of commands, switched by bank. In diroxide a bank is a named command bar that both frontends show and the keymap can switch.
- **Hotkeys** bound to commands, global and per context. In diroxide: one keymap with layers (section 5.7).
- **Commands as the unit of configuration**: copy, delete, rename, make directory, chmod, chown, chtime, change symlink, change label, change layout, change columns, clipboard, command menu, text view mode, enter and parent directory, go to, select all, none, invert, by pattern, quick search, path jump, bookmarks, dirsize, search, filter, show hidden, own command, run script. The source listing shows one file pair per command (`copyop`, `chmodop`, `changelabelop` and the rest); the inventory lists them all.
- **File types**: recognized by name pattern, by content (libmagic) and by a conditional "deep" check, with actions per type: double-click, show, raw show, user actions. diroxide keeps the model with a pure-Rust detector (section 5.6).
- **Path jump**: type a path with completion and history. **Bookmarks** with a database and filter (`bookmarkdb*`). **Directory history** per lister.
- **Dirsize**: the size of each folder in the listing on demand.
- **Search**: by name and by content, with results as a list to act on.
- **Text view**: a built-in viewer, with a hex mode (verify), search inside it, and a "raw" mode for file types that need it.
- **Labels** on files, colored in Worker. In diroxide a label is a word first, shown in a column, and colored only in addition.
- **Filters** per lister: show and hide patterns.
- **Volume manager**: list, mount and unmount volumes through HAL or uDisks. In diroxide: a volumes list on every platform (drives on Windows, mounts on Linux and macOS), with mount and unmount through a plugin where the platform needs D-Bus.
- **Layouts**: which parts of the window are shown. **Clipboard** operations between listers and other programs.
- **Background copy** (`bgcopy_*`, `async_job_limiter`): operations that run while the panels stay usable. In diroxide every operation is a job.
- **Lua scripting** (Worker 3 and later): scripts run from buttons, hotkeys and file-type actions, with an API into the listers. In diroxide the same places run plugins in any language (section 5.8).

### Modernized, in Directory Opus's direction

- **Folder tabs** per panel, with their own histories.
- **Flat view**: a folder and all its subfolders as one list, with the relative path in a column.
- **Collections**: virtual folders that hold search results, selections and drops, and that every command can act on.
- **Compare and synchronize** two panels: same, different, only here, only there, with a plan the owner reads before anything is copied.
- **Find duplicates** by size and hash.
- **Batch rename** with regex and a preview list of old and new names, nothing renamed until confirmed.
- **Metadata**: a details view of the focused entry (section 5.9), text first.
- **Command line everywhere**: the palette takes commands, and `diro` exposes the same commands from a shell.

### Dropped or replaced

- AGUIX and every X11 call: replaced by ratatui in the terminal and by Masonry, winit, Vello and AccessKit in the window.
- The configuration grammar: replaced by TOML with a schema that generates its own reference page.
- AVFS: replaced by diroxide's own virtual file system with archive providers that also write.
- libmagic: replaced by a pure-Rust detector (`infer` plus diroxide's own table) with libmagic as an optional plugin.
- HAL and uDisks linkage: replaced by `sysinfo` for listing and a plugin for mounting.
- autotools: replaced by Cargo, xtask and the release workflow.
- Lua as the only scripting language: replaced by the plugin system, with Lua as one tier.

## 3. The clean-room rule

Worker's code is GPL-2.0-or-later. diroxide is Apache-2.0 OR MIT. A port of Worker's code would be a derivative work and could not carry that license. So:

1. **Two roles, never one agent.** Research agents (Sonnet) read Worker's source and documentation and write specifications: what a feature does, its inputs, outputs, keys, edge cases and defaults, in their own words, with file and line citations for the orchestrator's verification. Building agents (Opus) never read Worker's source; they build from the specifications and from the public documentation.
2. **No Worker code, names of internal types, comments or configuration snippets** go into the repository. Default key bindings and user-facing behavior are not copyrightable and may match.
3. **Every brief says which role it is.** A building agent's brief does not name `D:\worker-5.4.0`. The guard denies reads of that folder to building agents' worktrees (section 11).
4. **The owner's own code is different.** textweaver is GPL-3.0-or-later, but the owner holds its copyright, so the owner may release any of it under Apache-2.0 OR MIT for diroxide. The pieces worth carrying over are named in section 7. The orchestrator confirms with the owner that no third party holds copyright in those pieces before they move (question 3).
5. **The records say so.** `docs/dev/clean-room.md` records the rule and the list of specifications; `THIRD-PARTY-NOTICES.md` is generated by `cargo about` as in textweaver.

## 4. Design principles

1. **Accessibility first.** A row says its meaning in the first 40 cells; status is words; color never carries meaning alone; every control has a name and a role; every change is announced; everything works from the keyboard. If an accessible way is not possible, the report says so; nothing inaccessible ships quietly.
2. **One app core, many frontends.** The core holds every panel, job and dialog and returns effects; frontends draw and announce. A feature exists when the core has it; a frontend only shows it. Tests drive the core without a terminal or a window.
3. **Everything is a command.** A command has an id, a name, a key, a menu place and a palette entry, from one model. Buttons, plugins and `diro` call the same commands.
4. **Never block the input thread.** Listing, stat, search, copy, archive and preview run on worker threads or the async runtime; the core receives messages. A slow network share pauses one job, not the program.
5. **Pure Rust first, tools second.** Archives, search, detection, images and SSH are in-process Rust. ImageMagick, FFmpeg and pandoc are found and used when present, never required. The owner's textweaver spirit applies: push the envelope with Rust, accept alpha crates, keep the gates, record each bold choice and its fallback in an ADR.
6. **Safe by construction.** Built-in operations never go through a shell. Paths are literal. Deletes list what they will remove. The trash is the default. Confirmations default to No. See section 5.12.
7. **Plain configuration.** TOML files a person can read and diff, unknown keys preserved, atomic writes, a schema that generates the reference page.
8. **Measure, then optimize.** A bench gate from alpha.1 (list 100,000 entries, search a 1 GB tree, copy 10 GB) fails in both directions, as textweaver's does.
9. **Lean.** No crate for a few lines. No option nobody asked for. The smallest complete change ("Ponytail", from the textweaver beta brief).
10. **US English, no feature numbers in user docs, em dashes sparingly, dates from the clock.**

## 5. Architecture

### 5.1 The workspace

One Cargo workspace, edition 2024, resolver 3, toolchain pinned to the installed stable (1.98.1 today), `[workspace.dependencies]` declared once, one crate per job, dependency direction checked by `cargo xtask deps --check`:

- `diroxide-core`: ids, errors, `Place` and `Entry`, the list model, commands and effects, messages and announcement kinds, the clock trait. No I/O.
- `diroxide-vfs`: the `Provider` trait, places, capabilities, cancellation, the registry, and the local provider.
- `diroxide-archive`: the archive provider for zip, 7z, tar, tar.gz, tar.xz and xz, reading and writing, with limits.
- `diroxide-sftp`: the SFTP provider on russh.
- `diroxide-ops`: the job engine: copy, move, delete, rename, make directory, links, attributes, times, trash, conflicts, progress, undo log.
- `diroxide-search`: name and content search, dirsize, duplicates, results as a lister.
- `diroxide-filetype`: detection and the file-type registry with actions.
- `diroxide-config`: settings, keymap, file types, banks and plugins configuration, in TOML.
- `diroxide-plugins`: the protocol types, the host, script commands, and the Lua tier behind a feature.
- `diroxide-tools`: the external tools registry (ImageMagick, FFmpeg, pandoc, and others), detection and safe running.
- `diroxide-preview`: previewers, text first, over tools and pure-Rust readers.
- `diroxide-app`: the app core: two panels, tabs, commands, keymap dispatch, menus, palette, dialogs, jobs, announcements.
- `diroxide-tui`: the terminal frontend on ratatui and crossterm.
- `diroxide-gui`: the window on Masonry and AccessKit.
- `diroxide-cli`: the `diro` binary: open, serve, search, convert, plugin tooling.
- `xtask`: deps, regen, keyboard, settings reference, notices, icons, dist, bench.

Sixteen crates. textweaver reached 36; a file manager needs fewer. A crate is added only when a wave's brief says so.

### 5.2 The core model

- **Place.** A location any provider can serve, written as a path with an optional scheme: `D:\photos`, `/home/owner`, `course.zip!week1/` (an archive member path, the form textweaver and star use, so nothing is extracted to browse), `sftp://host/path`. A place has a display form that says its kind first in the title line ("week1, in course.zip").
- **Entry.** Name, kind (folder, file, link, archive, special), size, times, mode or attributes, owner, link target, labels, and a display name with control, format and direction characters replaced by a question mark (the real name is kept for operations).
- **Lister.** A place, its entries, sort, filter, columns, selection, cursor, history, and the tab it belongs to. Two listers are live; one is active.
- **The list model.** One list type, with its title, rows, focus, letter jump, Home and End, F1 help, Say Status, and position said last ("report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40"). It is the same shape as textweaver's `ListModel` (ADR-0045), reused here for panels, search results, bookmarks, jobs, menus and every chooser, so every list reads the same way.
- **Commands and effects.** `Command` ids with arguments; `dispatch` returns `Effect`s (redraw, announce, open dialog, start job, run plugin). Frontends execute effects.

### 5.3 The virtual file system

A `Provider` trait with `list`, `stat`, `read`, `write`, `create_dir`, `rename`, `remove`, `set_attributes`, `copy_hint` (for same-provider fast paths), `watch` (optional), each taking a cancellation token and reporting progress; a `Capabilities` set (writable, trash, links, attributes, case-sensitive, max name length, reserved names); and a registry that resolves a place to a provider. Providers:

- **Local** (in `diroxide-vfs`): `std::fs` with platform details behind a small `sys` module: Windows attributes, long paths, reparse points, drive listing from `GetLogicalDrives` (never touching the drive, as textweaver's browser does); Unix modes, owners and groups; macOS resource forks ignored.
- **Archive** (`diroxide-archive`): zip through the `zip` crate (deflate, bzip2, zstd, lzma and xz entries), 7z through `sevenz-rust2` (LZMA, LZMA2, BZIP2, PPMd, with writing), tar and its gzip and xz wrappers through `tar`, `flate2` and `liblzma` or `lzma-rust2` (question 6), plain `.xz` as a one-member archive. Reading lists and reads members in place; writing packs a selection into a new archive or adds to one, as a job with progress. Limits from textweaver's archive module: member size, bytes decompressed, 7z dictionary size, entry count, nesting depth of four, normalized names so a member called `../../x` cannot leave the archive, junk entries hidden.
- **SFTP** (`diroxide-sftp`): russh and russh-sftp, key and agent authentication, known hosts checked, a connection per host kept open, listings and transfers streamed. Password entry goes through the frontends' prompt, never a command line.
- **Plugin providers** (section 5.8): S3, WebDAV, MTP and anything else, served by a process plugin that implements the provider methods.

### 5.4 The job engine

Every operation that touches more than one entry, or that can take more than a moment, is a job in `diroxide-ops`: a plan (what will happen, computed first and shown on request), a queue, progress in words ("copying 12 of 300, 1.2 of 4.0 GB, about two minutes left"), pause, cancel, a conflict policy asked once per job (ask, skip, overwrite, keep both, rename), and a result list (done, skipped, failed, with reasons). Copy preserves times and attributes, uses reflinks or server-side copy where the provider offers them, and moves within one provider by rename. Delete goes to the trash through the `trash` crate; permanent delete is a separate command that lists every path first. Renames and moves are recorded in an undo log that the Undo command replays for the last job. Nothing in the engine runs a shell.

### 5.5 Search

`diroxide-search` uses ripgrep's own crates: `ignore` for the parallel walker (with gitignore rules optional), `globset` for name patterns, `grep-regex` and `grep-searcher` for contents, so a content search runs at ripgrep's speed in-process. Filters: name glob or regex, content regex, size, dates, kind, depth, hidden. Results arrive as they are found into a results lister that every command can act on (copy, move, open, collect), and the search can be saved as a collection or a filter. Dirsize walks with the same walker. Duplicates by size, then by a fast hash (`blake3`), then by full comparison for the hits.

### 5.6 File types and actions

Detection in `diroxide-filetype`: extension first, then magic bytes through `infer` and diroxide's own table (text, scripts, archives, documents), then an optional content check. Types are in `filetypes.toml`, user-extendable, with a priority order and actions: open (the default), view, preview, convert, and user actions, each a command or a plugin call. Actions that run a program use the tools registry (section 5.9), never a shell string.

### 5.7 Commands, keys, banks, menus and the palette

One model in the app core, as textweaver's ADR-0043: menus (File, Edit, View, Go, Select, Tools, Plugins, Help), the palette (ranked by name), and banks (named command bars) all read command names and keys from the keymap, so no menu or button can show an unbound key. The keymap has layers (global, panel, list, viewer, dialog), a `keymap.toml` override, and a generated `docs/keyboard.md`. WCAG 2.1.4 applies: single-letter keys can be turned off. The defaults follow Worker and the Norton Commander line where screen readers allow: Tab switches panels, Enter opens, Backspace goes up, Space selects and moves on, F5 copy, F6 move, F7 make folder, F8 delete, F3 view, F4 edit, F2 rename, F9 menus, Ctrl+P the palette. **Insert is not used for selection,** because Insert is the NVDA and JAWS modifier key. The keymap is checked for conflicts with NVDA's and JAWS's default keys as textweaver's is.

### 5.8 The plugin system

The owner wants to automate file management in bash and Python before Lua, and to reach other protocols such as SSH. So the plugin system is a protocol, not a language.

**Tiers.**

1. **Process plugins.** A plugin is a folder with a `plugin.toml` manifest and a program in any language. diroxide starts it on demand and talks JSON-RPC 2.0 over its standard input and output, one JSON object per line, UTF-8, in both directions. The manifest names what the plugin provides: commands (with names, descriptions and default keys), file-type actions, providers (the methods of section 5.3), previewers, converters, and hooks (before and after copy, delete, rename, enter folder, select). The host offers the plugin: `ui/announce`, `ui/ask` (yes or no, choose from a list, prompt for text or a path, all through the frontends' accessible dialogs), `ui/progress`, `panel/get` and `panel/set` (place, selection, cursor, refresh), `fs/*` (list, read, write, copy, move, delete through the job engine, so a plugin's operations get the same safety, trash, progress and undo as built-in ones), `tools/run` (run a registered tool), `config/get`, and `log`. The plugin answers `initialize`, `command/run`, `provider/*`, `preview/render`, `convert/run`, `hook/event` and `shutdown`. The owner can write one in Python with the standard library alone: read a line, `json.loads`, answer. Reference plugins ship in `examples/plugins/` in bash, Python and Lua, and `diro plugin new --language python` writes a working skeleton.
2. **Script commands.** No protocol at all. A one-file script in `plugins/scripts/` with a header comment (`# diro: name = "Make thumbnails"`, `# diro: key = "ctrl+alt+t"`) becomes a command. diroxide runs it with the context in environment variables (`DIRO_ACTIVE`, `DIRO_OTHER`, `DIRO_CURSOR`, `DIRO_SELECTION_FILE` naming a file with one selected path per line) and as JSON on standard input. Standard output goes to the output pane, except lines that start with `diro:`, which are directives: `diro:announce ...`, `diro:refresh`, `diro:cd PATH`, `diro:select PATH`, `diro:progress N of M`. The exit code is the status. Worker-style one-line commands with placeholders (`{file}`, `{files}`, `{dir}`, `{other}`, `{dest}`) are the smallest script command; abax did the same in Python.
3. **In-process Lua.** `mlua` with a vendored Lua 5.4 (MIT), behind the `lua` feature, for the few things where process start-up is too slow: column formatters, filters that run per row, key macros. Same API names as the protocol, so a script can be moved from one tier to another. Worker's Lua scripts are not compatible (section 3), though a compatibility shim could be a plugin one day.
4. **WebAssembly.** Not in the first alpha. The host's transport is a trait (`PluginTransport`: stdio process today), so a component on wasmtime, or an Extism plugin, can be added as a second transport without changing the plugin API. Recorded as the fallback in the ADR.

**Why not embed Python.** PyO3 ties the binary to one Python version and makes a plugin crash a diroxide crash. A process plugin uses whichever Python the owner has, is isolated, and matches how the owner already works.

**Safety.** A plugin runs with the user's rights, so the manifest's capability list is shown on first enable and nothing is enabled by default. A plugin crash or timeout is reported and the app continues. `--no-plugins` starts clean. Plugins never receive a shell string; `tools/run` takes an argument list. Each plugin has its own log in the state folder.

**Why this works for SSH.** SSH is built in through `diroxide-sftp` because the owner needs it from the start. The provider protocol lets the same thing be done by a Python plugin with paramiko for a protocol diroxide does not carry.

### 5.9 Previews, tools and conversions

**The tools registry** (`diroxide-tools`): one table of external programs (`magick` and `convert`, `ffmpeg` and `ffprobe`, `pandoc`, `7z`, `pdftotext`, `xdg-open` and its platform equivalents), each with how to find it (PATH through one lookup rule, then `tools.toml` overrides), how to read its version, and how to run it: an argument list, no shell, no console window on Windows, output captured, cancellable. Settings shows each tool as found with its version or not found with how to install it, in words. This is the "plays nice" layer: nothing requires a tool, and everything that can use one says so.

**Previews** (`diroxide-preview`) are text first, because the owner cannot see a thumbnail, and because text is what a Braille line shows. The details view of the focused entry says, for an image, "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB" from the `image` crate's header read and EXIF from `kamadak-exif`, with ImageMagick for formats `image` does not read; for audio and video, "MP3, 3 minutes 41 seconds, 320 kbps, stereo" from `ffprobe`'s JSON; for documents, the first lines of text through pandoc, or `pdftotext` for PDF; for archives, the listing; for text, the text, with a hex mode. The window also renders the image and plays nothing; a preview never blocks, and a newer request cancels an older one, as textweaver's browser preview does.

**Conversions** are commands over recipes in `conversions.toml`: image formats and resizing through ImageMagick; audio and video transcoding through FFmpeg; Markdown, HTML, DOCX, EPUB and PDF through pandoc; pack and unpack through the archive provider. A conversion runs as a job, writes to the other panel by default (the orthodox rule), reports progress in words, and leaves a report of what was made and what failed. Users add recipes, and plugins register converters.

### 5.10 Frontends

- **Terminal** (`diroxide-tui`): ratatui on crossterm, raw mode. Two panels, a title line that says the active place first, a status line that says the key fact first, a jobs line, and the minibuffer for prompts. The hardware cursor sits on the focused row so screen readers and Braille displays follow it. The default theme uses no box drawing; a "lines" theme adds it for sighted users. Highlights carry an attribute (reverse or bold) and a text marker (a leading asterisk on selected rows), never color alone. No spinners; progress is a sentence that changes at most twice a second. Screen-reader mode detection and a quiet-screen option follow textweaver's terminal reader.
- **Window** (`diroxide-gui`): Masonry, `masonry_winit`, Vello, Parley and AccessKit, vendored from textweaver's `third_party/xilem` at the same revision and with the same patches (question 7), so the AccessKit floor that lets Orca find the app, and the Windows AcceleratorKey patch, come along. Each panel is an AccessKit grid with real row and cell nodes, named columns, a selection state, and a focus that the screen reader follows; menus are native (ADR-0046's approach); announcements go through a live region; dialogs are modal, named, close on Escape and return focus. Wayland, Windows and macOS through winit; X11 only through winit's X11 backend, with no X11-specific code anywhere.
- **Headless** (`diro serve --stdio`): the app core over JSON-RPC, the same protocol the plugins speak, for tests, scripts and other programs.

### 5.11 Configuration and state

`config.toml` (settings), `keymap.toml`, `filetypes.toml`, `banks.toml`, `conversions.toml`, `tools.toml` and `plugins/` in the config folder; bookmarks, history, labels, collections and the undo log in the state folder; every write atomic and only on an explicit change; unknown keys preserved; a settings schema that generates `docs/settings-reference.md` and the in-app Settings list with F1 help, as textweaver's does. A `--portable` flag keeps everything beside the binary.

### 5.12 Safety, built in

The owner lost most of a user profile on September 26, 2026 to a delete that read a hidden character as a drive letter. A file manager is the one program whose whole job is such operations, so the rules become features:

- **Every delete, move and overwrite lists what it will touch first,** with hidden, control, private-use and direction characters spelled out as `U+XXXX` and the count said in words. The confirmation starts on No.
- **Paths are literal.** Nothing a user types or a plugin sends is expanded by a shell. Built-in operations use argument lists and `std::fs`.
- **The trash is the default.** Permanent delete is its own command with its own key and its own listing.
- **Dangerous names are flagged** on creation and rename: reserved characters, trailing dots and spaces, Windows device names, shell-special characters, names that differ only by case from a sibling. The row says "name needs care" in words.
- **Boundaries are respected.** A recursive operation never follows a symbolic link or junction out of its root; an archive member never writes outside its destination folder.
- **Undo** replays the last job's renames and moves; a delete to the trash is undone by restoring from it.
- **Nothing escapes a drive letter.** A path such as `C:` without a slash is refused as ambiguous; the app always resolves to a full path before acting and shows it.

### 5.13 Platforms

Windows 11 (Windows Terminal; the classic console is best effort), Linux (Wayland and X11 through winit; any terminal; Orca through AT-SPI), macOS (VoiceOver through NSAccessibility). Linux builds and tests run in Docker from day one; macOS runs in CI as textweaver's does.

## 6. Technology choices

Each line: the need, the crate, and the fallback. Versions are those textweaver pins today where it already uses the crate, so the two projects share a cache.

- Regex and search: `regex` 1.13, `ignore`, `globset`, `grep-regex`, `grep-searcher` (ripgrep's crates, MIT and Unlicense). Fallback: none needed.
- Zip: `zip` 8.6 with deflate through zlib-rs, lzma, xz, bzip2 and ppmd features (textweaver's pins). 7z: `sevenz-rust2` 0.23 (Apache-2.0). Tar: `tar` 0.4.46; gzip: `flate2` 1.1; xz: `liblzma` (bindings, proven) or `lzma-rust2` (pure Rust, newer); question 6.
- SSH: `russh` and `russh-sftp` (Apache-2.0 and MIT); fallback `openssh-sftp-client` over the system `ssh`.
- Watching: `notify` 8. Trash: `trash`. Volumes and system info: `sysinfo`. Config folders: `dirs`. Hashing: `blake3`.
- Detection: `infer` plus diroxide's table; fallback a libmagic plugin.
- Images: `image` and `kamadak-exif`; SVG rasterization for the icon: `resvg`.
- Async: `tokio` for the plugin host, SFTP and jobs; the app core stays synchronous and message-driven. Question 8.
- Lua: `mlua` with the `lua54` and `vendored` features, behind the `lua` feature flag.
- Terminal: `ratatui` 0.30 and `crossterm`. Window: textweaver's vendored Masonry with AccessKit 0.25.1 and `accesskit_winit` 0.34.1.
- Serialization and config: `serde`, `serde_json`, `toml` 1.1. Errors: `thiserror` 2 in libraries, `anyhow` in binaries. CLI: `clap` 4.6. Logging: `tracing`.
- Tests: `cargo-nextest`, `insta`, `proptest`, `cargo-fuzz` (archive readers, the protocol parser, the name sanitizer), `cargo-deny`, `cargo-about`.

## 7. Quick wins folded in

Each is reuse rather than new work, and each saves a wave's worth of mistakes already made once.

- **The guard, `safe-remove.ps1`, `settings.json` and the project `CLAUDE.md`** from `D:\star\wiki\claude\files\textweaver-guard\`, with the folder allow-list changed to `D:\diroxide`, `D:\diroxide-planning`, `D:\star\wiki` and the memory folder, and a deny on reads of `D:\worker-5.4.0` for building agents (section 3). Its tests come with it.
- **The build settings**: `CARGO_INCREMENTAL=0`, line-tables-only debug info, dependencies at `opt-level = 1` with no debug info, `release` without LTO for contributors and a `dist` profile with fat LTO for packages, sccache in `D:\sccache`, Cargo and rustup homes on D:. Note: sccache is not on PATH on this machine today; installing it is pre-approved and is a Wave 0 step.
- **The Docker setup**: the `compose.yaml` pattern with per-agent target volumes, 6 GB and 4 CPUs per container, at most three building, and `tools/build-hygiene.ps1`. The image adds ImageMagick, FFmpeg, pandoc, p7zip (for cross-checking archives) and an OpenSSH server (for SFTP tests).
- **CI workflow shapes**: fmt, clippy with warnings denied, `missing_docs`, generated files checked, dependency direction, notices, cargo-deny, tests on Windows, Ubuntu and macOS, rustdoc, the accessibility tree dump compared with main, nightly fuzzing and Miri, the bench ratchet, and the release workflow with SHA256SUMS and attestations.
- **xtask**: `deps --check`, `regen`, `keyboard`, `settings-doc`, `notices`, `dist`, `bench`, plus the new `icons`.
- **The list model, the menu and palette model, the keymap with layers and generated docs, the dialogs contract, and the message style rules** (meaning in 40 cells, errors as "Could not VERB OBJECT: CAUSE. NEXT STEP.", questions ending "y or n"). Designs from textweaver's ADRs 0006, 0043 and 0045; code relicensed by the owner where it fits (question 3).
- **The archive module** from textweaver's formats crate: already lists and reads zip, tar, tar.gz and 7z with nesting, limits, hostile-name handling and junk filtering. diroxide adds writing and xz.
- **The browser's Windows drive listing** that never touches a drive, and its background preview with cancellation.
- **The ADR template, the docs index shape, `docs/dev/messages.md`, the Braille 40-cell label test, the pseudo-locale test** (if translations are wanted later), and the screen-reader session scripts.
- **ripgrep's crates** for search instead of a search engine of our own; **`trash`** instead of platform code for the recycle bin; **`dirs`** for config folders.
- **One PATH lookup rule and process helpers with no console window**, which textweaver's Wave 8 cleanup settled.
- **The research and orchestrator rules** as written, including dates from the clock, `git status` after every hand-back, and the single-output rule for research agents.

## 8. The alpha series

Each alpha has a goal, contents, what the owner checks, and exit criteria. "Readiness" means the orchestrator hands the owner one page: the packages built on a branch, the checks dated, the known gaps, the open decisions. The owner decides every tag; nothing tags on its own.

### alpha.1: two panels

- **Goal.** A terminal file manager the owner can use every day on local folders.
- **Contents.** The workspace and contract; `Place`, `Entry`, `Lister`, the list model; the local provider with drives, attributes and watching; the job engine with copy, move, rename, make folder, delete to trash, permanent delete with its listing, conflicts, progress and undo; sort, filter, selection by pattern, quick search, path jump, history; the keymap, menus, palette, banks and settings; the terminal frontend with its accessibility rules; `diro PATH` and `diro serve --stdio`; the bench gate; the docs skeleton.
- **The owner checks.** Ten minutes with NVDA, then JAWS, then the Mantis: open, move around, select, copy to the other panel, rename, delete, undo; the status line on the display; the palette.
- **Exit.** All checks green on three OSes; the readiness page; the owner's findings filed for alpha.2.

### alpha.2: archives, search and the viewer

- **Contents.** The archive provider, reading and writing all six formats, with limits and fuzz targets; pack and unpack as jobs; search by name and content with the results lister, dirsize, saved searches; the text viewer with hex mode and search; labels; bookmarks.
- **The owner checks.** Enter a zip and a 7z, copy out of one and into the other, pack a selection, search a large folder for a regex, read a file in the viewer.

### alpha.3: plugins and SSH

- **Contents.** The protocol, the host, script commands, the Lua tier, `diro plugin` (list, new, enable, disable, run, log), the three example plugins, the plugin author's guide, the SFTP provider with key and agent authentication and known hosts, the Plugins menu.
- **The owner checks.** Write a five-line bash script command and a Python plugin that adds a command; connect to a host over SFTP and copy both ways.

### alpha.4: the window

- **Contents.** The Masonry window: panels as accessible grids, native menus, the palette, dialogs, jobs, settings, the viewer, themes with contrast checked; the icon; packaging for all three platforms with the release workflow.
- **The owner checks.** The same ten minutes as alpha.1, in the window, with NVDA, JAWS and the display; the UI Automation report.

### alpha.5: previews and conversions

- **Contents.** The tools registry and its Settings page; previewers for images, audio, video, documents, archives and text; the details view; conversions with recipes, reports and the Convert command; file-type actions that use tools.
- **The owner checks.** Focus a photo, a song and a PDF and read their details; convert a folder of images and a document.

### alpha.6: the modern file manager

- **Contents.** Folder tabs; flat view; collections; compare and synchronize with a plan; duplicates; batch rename with regex and a preview; the volumes list with mount and unmount through a plugin; directory history across sessions; the plugin hooks.
- **The owner checks.** Synchronize two folders from the plan; rename fifty files with a regex and read the preview; find duplicates.

### alpha.7: the final alpha

- **Contents.** The findings from every session; the feature-complete list reviewed and closed; performance against the budget; thirty days of clean nightly fuzzing; documentation complete and checked; packages started on every platform; a clean-install smoke test; the known-limits page.
- **The owner checks.** Two weeks of daily use.

## 9. The waves and the agents

### How a wave runs

The textweaver method, as it stands after Wave 9 and the beta brief:

- **Phase 0 is the contract.** The orchestrator and one Opus agent write a workspace in which every crate compiles, the core crate is finished, every owned crate has its public types with deliberately naive bodies, the ADRs are drafted, CI and Docker run, and the task log exists. Building agents keep the public signatures and replace the bodies.
- **Ownership by crate.** Each brief names the crates, binary subcommand files and fixture folders the agent owns. No one edits the core crate, the root `Cargo.toml`, the toolchain file, `.github/`, `docker/` or another agent's paths. Contract changes are written as requests in the report; the orchestrator resolves them at integration.
- **Three to five builders at a time,** by the machine's budget (section 12), plus research and docs agents that do not build. One agent per sub-wave touches the app core's messages and keys; one GUI lane at a time.
- **Worktrees and branches.** `D:\diroxide\.claude\worktrees\<id>` on `wave<N>/<letter>-<name>`, each with `CARGO_TARGET_DIR=D:/diroxide/target/<id>` and its own Docker volume. Agents never push, merge, rebase onto main or tag.
- **Integration.** The orchestrator merges in batches of three or four, the widest app change first, runs one full native and container check per batch, pushes, and watches CI by run id before calling a merge done. `cargo xtask regen` rebuilds every generated file after a merge; nothing generated is hand-merged.
- **Between waves.** `git status --short` in both repositories after every hand-back; strays removed by `safe-remove.ps1` after a dry run, the same hour, and recorded; build output flushed by the hygiene script; Docker restarted gracefully; D: and RAM checked; the task log's status lines written with clock-substituted times; the wiki updated by the orchestrator (agents never write it); a readiness page per alpha.
- **The owner's sessions are follow-ups, not gates.** Findings go into the next wave. Small reversible decisions take a recorded default. Tags, releases, new licenses, accounts, downloads outside the pre-approved list, guard changes and deletes outside the task's own output wait for the owner.
- **Models.** Fable plans and orchestrates; Opus builds; Sonnet researches, surveys, and writes docs and prose; Haiku runs timers, CI watches and status polls.
- **Time boxes.** Every brief has one. An agent that reaches it stops and reports what is left.

### Wave 0: the contract, the inventories and the setup

Orchestrator (Fable) with one Opus builder and four Sonnet researchers, about three days.

- **P0 (Opus): the contract.** The sixteen crates, `diroxide-core` complete (ids, `Place`, `Entry`, the list model, commands and effects, messages, the clock), naive bodies elsewhere, the lints, the profiles, `xtask` with `deps`, `regen` and `keyboard`, CI, the Docker image and compose file, `docs/README.md`, `docs/dev/architecture.md`, ADRs 0001 to 0008 (workspace and dependencies; the core model; the provider trait; the job engine; commands, keys and menus from one model; the plugin protocol; frontends over one core; safety rules). Acceptance: `cargo test --workspace` green natively and in the container with nothing implemented; `deps --check` green.
- **R1 (Sonnet): the Worker parity inventory.** Reads `D:\worker-5.4.0` and the documentation; writes `research/worker-parity.md`: every command, key, column, file-type rule, configuration block, Lua API function and viewer feature, each with what it does and a file and line citation, and a list of what diroxide keeps, changes or drops. Research rules apply: one file, read-only shell.
- **R2 (Sonnet): the modern features inventory.** Directory Opus, Double Commander, Total Commander, yazi, lf, ranger and nnn: tabs, flat view, collections, synchronize, duplicates, batch rename, previews, and how each does plugins and scripting (yazi's Lua, lf's shell, ranger's Python, nnn's scripts). Writes `research/modern-features.md` with the shape diroxide should take and the keys users expect.
- **R3 (Sonnet): the plugin protocol review.** Reads the Language Server Protocol, the Debug Adapter Protocol, dprint's process and WebAssembly plugins, and the Extism and wasmtime component model; reviews section 5.8's draft; writes `research/plugin-protocol.md` with the message list, the manifest schema, the script-command directives, and the WebAssembly path, plus what each decision costs.
- **R4 (Sonnet): the accessibility stack check.** The current state of Masonry and AccessKit upstream against textweaver's vendored revision and patches; ratatui under NVDA, JAWS, Orca and VoiceOver for a two-panel layout; the grid and table roles in UIA, AT-SPI and NSAccessibility; writes `research/accessibility-stack.md` with the floor and the risks.
- **Orchestrator:** the planning repository `D:\diroxide-planning` with `history/tasks.md` (the shared preamble adapted from textweaver's, section 10) and `research/`; the guard, `safe-remove.ps1` and `settings.json` installed and tested; sccache installed; the license files; the wiki hub updated; the owner's answers to section 13 recorded.

### Wave 1: two panels (alpha.1)

Four Opus builders, one Sonnet docs agent, about five days, two sub-waves.

- **1a, three builders at once:**
  - **W1a (core and local VFS):** `diroxide-vfs`: the provider trait finished, the local provider with drives, attributes, long paths, links and watching; the name sanitizer; proptests over path normalization; fuzz target for the sanitizer. Owns `diroxide-vfs`, `fixtures/vfs/`.
  - **W1b (the job engine):** `diroxide-ops`: plans, the queue, copy, move, rename, make folder, trash, permanent delete with its listing, conflicts, progress, cancel, undo log; a fake provider for tests; proptests that a plan equals what ran. Owns `diroxide-ops`.
  - **W1c (configuration, keys and commands):** `diroxide-config` and the command model in `diroxide-app` (commands, keymap with layers, menus, palette, banks, settings schema and generated docs, NVDA and JAWS conflict check). Owns `diroxide-config`, `diroxide-app/src/{commands,keymap,menu,palette,settings}.rs`, `xtask/src/{keyboard,settings_doc}.rs`.
- **1b, two builders and one writer:**
  - **W1d (the app and the terminal):** the app core's panels, dispatch and effects, dialogs, jobs wiring, announcements; the terminal frontend with its accessibility rules; `diro PATH` and `diro serve --stdio`; the scripted ratatui test (open, move, select, copy, rename, delete, undo, relaunch and find the cursor restored); the bench gate. Owns the rest of `diroxide-app`, `diroxide-tui`, `diroxide-cli`. This is the one agent in the app's messages.
  - **W1e (the CLI and the bench):** `diro search`, `diro ls` (plain output), `cargo xtask bench` with the three scenarios and the ratchet. Owns `diroxide-cli/src/cmd/`, `xtask/src/bench.rs`. Light.
  - **W1f (docs, Sonnet):** quickstart, panels, operations, keyboard (generated), settings (generated), screen readers, accessibility, troubleshooting; the docs checks. Owns `docs/` except `docs/dev/` and `docs/adr/`.

### Wave 2: archives, search and the viewer (alpha.2)

Four Opus builders, one Sonnet docs agent, about four days.

- **W2a (archives):** `diroxide-archive`, reading and writing the six formats; the limits; junk and hostile names; nesting; pack and unpack as jobs through `diroxide-ops`; fuzz targets for each reader; cross-checks against p7zip and `tar` in the container. Owns `diroxide-archive`, `fixtures/archives/`, `fuzz/archive_*`.
- **W2b (search):** `diroxide-search`: the walker, name and content search, filters, the results lister, saved searches, dirsize, duplicates; the bench scenario. Owns `diroxide-search`.
- **W2c (the viewer, labels, bookmarks):** the text viewer with hex mode and search, as a list over lines; labels and bookmarks in the state folder. Owns `diroxide-app/src/{viewer,labels,bookmarks}.rs`, `diroxide-tui/src/viewer.rs`.
- **W2d (app wiring):** the Enter-an-archive flow, the Search dialog and results, the viewer keys, the pack and unpack commands, the announcements. Starts after 2a to 2c report; the one app-message agent.
- **W2e (docs, Sonnet):** archives, search, the viewer, labels and bookmarks guides.

### Wave 3: plugins and SSH (alpha.3)

Four Opus builders, one Sonnet writer, about five days.

- **W3a (the protocol and the host):** `diroxide-plugins`: the JSON-RPC types with a fuzzed parser, the manifest, the host (spawn, lifecycle, routing, timeouts, logs, crash isolation), the host-side services (`ui/*`, `panel/*`, `fs/*` through the job engine, `tools/run`, `config/get`), the `PluginTransport` trait with the stdio transport; a test plugin in Rust that records everything. Owns `diroxide-plugins` except `script.rs` and `lua.rs`.
- **W3b (script commands and Lua):** script commands with the header, environment, stdin JSON, directives and exit codes; the Lua tier behind the feature, with the same API names; the three example plugins; `diro plugin` subcommands and `new --language`. Owns `diroxide-plugins/src/{script,lua}.rs`, `examples/plugins/`, `diroxide-cli/src/cmd/plugin.rs`.
- **W3c (SFTP):** `diroxide-sftp` with authentication, known hosts, connection reuse, streaming transfers; tests against the container's OpenSSH server. Owns `diroxide-sftp`, `docker/sftp/`.
- **W3d (app wiring):** the Plugins menu, plugin commands in the palette and banks, the connect-to-host flow with the password prompt, hooks; after 3a to 3c report.
- **W3e (the plugin author's guide, Sonnet):** `docs/plugins.md`, `docs/writing-a-plugin.md` with the three worked examples, `docs/ssh.md`.

### Wave 4: the window (alpha.4)

Two GUI lanes in sequence, one packaging agent beside them, about six days.

- **W4a (the window's foundation):** the vendored stack brought in; the window, the two panels as accessible grids with named columns, row and cell nodes, selection and focus; the live region; native menus from the menu model; the palette; the accessibility tree test harness and the UI Automation report. Owns `diroxide-gui`, `third_party/`.
- **W4b (the window's dialogs and jobs):** after 4a: dialogs under the dialogs contract, jobs and progress, settings, the viewer, themes with the contrast check in words, the theme-following title bar and menus. Owns `diroxide-gui` after 4a hands back.
- **W4c (the icon and packaging):** the three concepts for the owner's choice (a design task first), then `xtask icons`, `dist`, the Windows zip, the macOS tarball, the Linux AppImage and tarball, the release workflow with SHA256SUMS, the clean-install smoke test. Owns `xtask/src/{icons,dist}.rs`, `assets/`, `.github/workflows/release.yml` (the one exception to the `.github` rule, by brief).
- **W4d (docs, Sonnet):** the window guide, install, themes, the window in pictures with alt text.

### Wave 5: previews and conversions (alpha.5)

Three Opus builders, one Sonnet writer, about four days.

- **W5a (tools):** `diroxide-tools`: the registry, detection, versions, overrides, safe running, the Settings page data. Owns `diroxide-tools`.
- **W5b (previews and the details view):** `diroxide-preview`: the previewers in section 5.9, the details view model, cancellation. Owns `diroxide-preview`.
- **W5c (conversions and file-type actions):** recipes, the Convert command as a job with reports, the file-type registry's actions through the tools, the Convert dialog in both frontends. Owns `diroxide-filetype`, `diroxide-app/src/convert.rs`, both frontends' convert dialogs.
- **W5d (docs, Sonnet):** tools and previews, converting, file types.

### Wave 6: the modern file manager (alpha.6)

Four Opus builders, one Sonnet writer, about five days.

- **W6a (tabs, flat view, collections):** the tab model per panel, flat view as a lister mode, collections in the state folder. Owns `diroxide-app/src/{tabs,flat,collections}.rs`.
- **W6b (compare, synchronize, duplicates, batch rename):** in `diroxide-ops` and `diroxide-search`; the plan views; the rename preview. Owns those files.
- **W6c (volumes, history, hooks):** the volumes list, mount and unmount through a plugin, history across sessions, plugin hooks wired to the job engine. Owns `diroxide-vfs/src/volumes.rs`, `diroxide-plugins/src/hooks.rs`, the example mount plugin.
- **W6d (app and window wiring):** after 6a to 6c.
- **W6e (docs, Sonnet).**

### Wave 7: the final alpha (alpha.7)

Stabilization, as textweaver's Wave 8c and 9: the findings from the owner's sessions; a whole-codebase review by a research agent; performance against the budget; the gates list (two-way checks, report-only steps with reasons, release asset checks, the sentinel against tests touching real data, the clean-install smoke); a docs sweep; the known-limits page; the feature-complete list closed with the owner. Three builders at a time from a rolling queue.

## 10. Rules for every agent

The shared preamble in `D:\diroxide-planning\history\tasks.md` carries these, adapted from textweaver's; every brief points at it. In short:

- **Privacy.** No personal identifier leaves the machine or enters a file; the owner is "the owner"; the neutral User-Agent; placeholder names in test data.
- **Where to work.** `D:\diroxide`, `D:\diroxide-planning`; read-only in `D:\star\wiki`; nothing on drive C; building agents never read `D:\worker-5.4.0`.
- **Deleting.** Only through `safe-remove.ps1` after a dry run; never `rm`, `git clean` or a one-line `Remove-Item`; `git rm` of tracked files is allowed; never a delete in Bash; never an escape sequence or an odd name typed.
- **Plain names only** in anything created.
- **Ownership and the contract** as in section 9; new dependencies only from `[workspace.dependencies]`; contract change requests in the report.
- **Quality bar.** Rustdoc on every public item; `unsafe_code` denied except in named platform modules reviewed at integration; no `unwrap` or `expect` on input or I/O in libraries; `thiserror` errors; no `todo!`, `unimplemented!` or `dbg!`; accessibility is the product: every user-visible change announceable, every string readable aloud and meaningful in 40 cells.
- **Checks before reporting,** on the crates changed: `cargo fmt --all --check`, `cargo clippy -p <crate> --all-targets --all-features -- -D warnings`, `cargo nextest run -p <crate> --all-features`, `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps`, the same in the container with the agent's own target folder, and `cargo xtask regen --check` after a settings, key or catalog change.
- **Git.** Small logical commits on the agent's branch, each ending `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; no push, merge, rebase or tag.
- **Dates and times from the clock,** substituted in the command that writes them.
- **No scratch files** outside the worktree and target folder; `git status --short` in both repositories before reporting, listing every untracked file created, accidents included.
- **Report format.** Summary in five lines; files by crate; checks with Pass or Fail; GUI wiring needed; contract change requests; open issues; what the next wave should do first; at most five things for the owner to try with NVDA and JAWS, or "nothing to hear".
- **Research agents** follow the Research Agent Rules unchanged: one output file, a read-only shell, named folders only, pages not files, small outputs.

## 11. Repository conventions

- **Layout.** `crates/diroxide-*`, `xtask/`, `docs/` (user guides at the top, `dev/`, `adr/`, `site/` later), `fixtures/`, `fuzz/`, `examples/plugins/`, `assets/`, `docker/`, `scripts/` (`dev-check.ps1` and `.sh`), `tools/` (build hygiene, accessibility checks), `third_party/` (the vendored window stack), `.claude/` (the guard, tools, settings; worktrees and temp folders ignored).
- **Project `CLAUDE.md`,** untracked as in textweaver, master copy in the wiki: the owner overrides rules; privacy; where to work and deleting; plain names; the guard; the clean-room rule; accessibility first; color never alone; writing; the pointer to the planning preamble.
- **The guard** from textweaver with the new allow-list and the Worker-folder read denial for building agents; its test file updated; installed by the owner, since the hook refuses to edit itself.
- **Docs rules.** User guides have no tables, one command per fenced block, end with "See also"; generated files never hand-edited; ADRs keep their decisions with dated status updates; no feature numbers in user docs.
- **Tests.** `tests/it/main.rs` per crate; nextest profiles; insta snapshots of listings and plans; proptests for paths, names, plans and the protocol; fuzz targets nightly; real-tool tests `#[ignore]` unless `DIROXIDE_TOOLS=1`, with CI naming required tools so a missing tool fails instead of skipping; a sentinel against tests touching real folders; tests never touch the real trash or the real config folder (an injected state folder).
- **Changelog.** Agents write under a scratch heading with their id; the orchestrator regroups by area before an alpha.
- **Versioning.** `0.1.0-alpha.N` in `[workspace.package]`; the final alpha is feature-complete; beta and stable only when the owner starts them.

## 12. Resource budget

Measured on this machine on Friday, October 9, 2026:

- D: 579.7 GB free. Floor: 200 GB. Each native builder's target folder is budgeted at 8 GB for a project this size (textweaver's lean build measured 16 GB for a far larger workspace); five builders and the orchestrator fit with room.
- RAM: 37.6 GB free of 63.8. Floor: 20 GB free. Docker's machine has about 31 GB: at most three containers at 6 GB, and 26 GB of host RAM free before a container build.
- Toolchain: rustc and cargo 1.98.1 installed; Docker 29.8.0; sccache not on PATH (install in Wave 0; pre-approved).
- Docker: restart gracefully between waves; never purge; compact with the owner past 300 GB.
- Compute: textweaver found six agents' full checks at once made one clippy run take 70 minutes. Agents check only the crates they changed; the full workspace check runs once per merge batch.

## 13. Questions for the owner, each with a default

1. **The command name.** Default `diro`. (`do` is a shell reserved word.)
2. **The license.** Default Apache-2.0 OR MIT.
3. **Relicensing textweaver pieces.** Default yes for the list model, the archive module, the keymap, the dialogs contract and the message rules, on the owner's confirmation that no third party holds copyright in them.
4. **The remote.** Default: local repository only until the owner asks for one (the global rule).
5. **The planning repository.** Default `D:\diroxide-planning`, local, never pushed, mirrored to the wiki after each wave.
6. **xz.** Default `liblzma` (bindings to the proven library) in alpha.2, with `lzma-rust2` tried in the final alpha if its write path is solid by then.
7. **The window stack.** Default: vendor textweaver's Masonry snapshot and patches; R4 reports whether upstream has moved far enough to vendor fresh instead.
8. **tokio.** Default yes, confined to the plugin host, SFTP and jobs.
9. **Lua.** Default in alpha.3 as an optional feature; drop it if R3 finds the script-command tier covers the owner's needs.
10. **The icon.** Default three concepts in Wave 4; the owner picks; words-first alt text.
11. **Insert for selection.** Default no, because Insert is the screen reader key; Space selects.
12. **Self-voicing.** Default none: diroxide relies on the screen reader. A speaking mode through textweaver's speech crates could come later as a plugin.

## 14. What happens when the owner says go

1. The orchestrator records the answers above (defaults where none is given) at the end of this file under "The owner's answers", with the date from the clock.
2. Wave 0 starts: the planning repository, the guard and settings, sccache, the license files, the Phase 0 builder and the four researchers, all the same day.
3. The wiki hub and the planning record are updated at the end of Wave 0 and after every wave.
4. Wave 1 starts when P0's workspace is green natively and in the container, and R1 and R3 have handed back.

## The owner's answers

Given on Friday, October 9, 2026, in the planning session:

1. **The command name:** `diro`.
2. **The license:** Apache-2.0 OR MIT.
3. **Relicensing textweaver pieces:** yes. The list model, the archive module, the keymap, the dialogs contract and the message rules may move to diroxide under Apache-2.0 OR MIT. Before any piece moves, the orchestrator confirms with the owner that no third party holds copyright in it.
4. **This plan file** lives in the repository at `docs/plan.md`, not in the Claude application's plan folder on drive C.

Questions 4 to 12 are open and take their defaults until the owner says otherwise.

## Bookkeeping left from the planning session

Done before this section was written: the repository initialized with this plan as its first commit; the planning repository `D:\diroxide-planning` initialized (not yet committed) with the textweaver-method survey in `research/`; the wiki hub `domains/diroxide.md` and the record `meta/diroxide-releases/diroxide first alpha plan.md` written.

Still to do, once the owner approves:

- Fold the Worker 5.4.0 source survey into section 2 when the research agent reports, and save the survey as `research/worker-survey.md` in the planning repository.
- Wiki bookkeeping: the diroxide lines in `index.md`, `domains/domains index.md` and `meta/meta index.md`; a log entry at the top of `log.md`; a section in `hot.md`; the owner's answers in the wiki record.
- A project memory (`diroxide-project-state`) in the Claude memory folder, with its copy under `claude/memory/D--diroxide/` in the wiki and a line in `Claude Memory`.
- Commit the planning repository; commit this plan's update in the project repository.
- Report the Obsidian MCP server's timeout to the owner (the wiki was read and written directly on disk this session).

## Related

- The wiki: the diroxide hub and the first alpha planning record; the textweaver hub, its wave planning pages, the Research Agent Rules and the Wave 4 orchestration plan; Claude Global Rules.
- In the repository: `docs/dev/clean-room.md` (Wave 0), `docs/adr/` (Wave 0), the task log in the planning repository.
