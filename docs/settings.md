# Settings and where the files live

This guide says where diroxide keeps its files, how to change a setting, which settings most people look for, and how to change keys, menus and themes. It describes the settings in words. [The settings reference](settings-reference.md) lists every setting with its exact name, default, range and help, and is generated from the program's own list of settings, so it is always right. This guide is the story, and that page is the dictionary.

## Where the files live

diroxide keeps two folders. The config folder holds the files you edit. The state folder holds what diroxide remembers for you.

- Windows: the config folder is `%APPDATA%\diroxide` and the state folder is `%LOCALAPPDATA%\diroxide`.
- Linux: the config folder is `~/.config/diroxide` and the state folder is `~/.local/state/diroxide`. If you have set `XDG_CONFIG_HOME` or `XDG_STATE_HOME`, diroxide uses those.
- macOS: both are in `~/Library/Application Support/diroxide`.
- With `diro --portable`, the folders are `config` and `state` beside the program, so you can carry everything on a drive.

In the config folder, alpha.1 uses these files:

- `config.toml` holds the settings.
- `keymap.toml` holds your changes to the keys. diroxide writes the built-in keys beside it as `keymap-defaults.toml`, a read-only reference that it rewrites at each start; changes to that copy are not read.
- `menus.toml` holds your changes to the menus. The built-in menus are written beside it as `menus-defaults.toml`, also read-only. Copy it to `menus.toml` and change it there to add, remove and reorder items and submenus.
- `banks.toml` holds the banks of command buttons, including the function key bar.
- `filetypes.toml` is a placeholder that later alphas fill in.
- A `themes` folder holds your own theme files.

Later alphas add more files, such as the tools list and the plugins folder. They are described when they arrive.

The state folder holds each panel's folder, focused row, sort and history, and the layout, from your last session (`session.toml`); the history of each prompt (the `history` folder); the folders path jump learned from; the list of recent jobs for Undo; `terminal.log`; and, since alpha.2, a `spool` folder, where an archive inside another archive is kept while it is open when it is too big for memory. diroxide uses it instead of the system's temporary folder, and the `archives.spool_folder` setting can name another place. Nothing in it needs editing.

## How diroxide treats these files

- It writes a file only when you make an explicit change, one value at a time, and it writes atomically, so a crash cannot leave half a file.
- It keeps what it does not know. A setting that a newer or older version wrote stays in the file, and your comments and the order of your lines stay too.
- A file it cannot read is never thrown away. diroxide uses the defaults for that file, keeps the old one beside it with `.bad` added to its name, and tells you in words at start.
- Every unknown command or key it finds in `keymap.toml` or `menus.toml` is reported in words at start, and never dropped silently. The status line says how many problems there were: "keymap.toml: 2 problems. Config problems in the Help menu lists them." Config problems, in the Help menu (Alt+H, then C) and the palette, lists each one with its file.

## Changing a setting

In the program, open Settings from the Tools menu (Alt+T, then S) or the palette. It is a list with one row for each setting, in the order of the reference. Each row says the setting's name, then its value in words, and says "comes with alpha.N" for a setting that does nothing yet. Enter flips a switch, or opens the choices with "Back to default" last, or opens a field for a number or text. The help key (Shift+F1 or the question mark) on a row gives that setting's help. The change is saved at once, and diroxide says it: "Layout: stacked", or "Layout back to side by side".

By file, open `config.toml` in any text editor. diroxide reads it the next time it starts. A setting is one line. A name with a dot belongs to a table: `sort.key` is `key` under `[sort]`.

```toml
layout = "stacked"
```

```toml
[sort]
key = "size"
```

If a value is not one diroxide understands, it says so in words at start and uses the default for that setting.

## The settings most people look for

Names are in code type. Defaults are in parentheses.

- Layout (`layout`): `side_by_side` (default), `stacked` or `single`. [The panels guide](panels.md) says which suits a screen reader.
- Screen reader mode (`screen_reader_mode`): `auto` (default), `on` or `off`. It slows the progress line to the interval below, redraws the other panel only on demand, keeps the candidate list closed until you press Tab, and keeps code colors off in the viewer. diroxide does not detect your screen reader yet, so `auto` behaves as on. Set `off` for the full look.
- Key bar (`key_bar`, on): the line at the bottom that names the function keys in words, "F2 Rename  F3 View  F4 Edit  F5 Copy  F6 Move  F7 Folder  F8 Trash  F9 Menu  Ctrl+Q Quit". It is drawn in every mode, screen-reader mode included, and the width decides how many pairs fit. Say key bar in the Help menu reads it. <!-- to confirm with the wiring (W2f): the setting's name, its default, how it meets compact, and the exact line. -->
- Quiet screen (`quiet_screen`, off): draws as little as possible, so a screen reader that reads every change stays quiet.
- Progress interval (`progress_interval_seconds`, 10; 1 to 600): in screen-reader mode, the shortest time between two progress lines. A job's start, its end and its questions are said at once.
- Single letters (`single_letters`): what a key without Control or Alt does in a panel. `type_ahead` (default) jumps to a name, `commands` lets letters run commands for the key presets that come with alpha.3, and `off` does nothing. The Filter command still filters when it is off. Turn it off if you use dictation software.
- Hidden files (`show_hidden`, off).
- Sorting: what a new panel sorts by (`sort.key`: name by default, or extension, size, modified, created, accessed, kind, attributes, owner, label or unsorted), the direction (`sort.direction`), where folders go (`sort.folders`: first by default, mixed or last), capitals (`sort.case`: ignore by default, or respect) and numbers in names (`sort.natural`, on, so `file2` comes before `file10`).
- Columns (`columns`): the details said after each name, in order. The default is type, size and modified.
- Confirmations (`confirm.copy`, `confirm.move`, `confirm.trash` and `confirm.quit_with_jobs`, all on): whether those four questions are asked. Every question that is asked starts on No, and deleting for good always asks.
- Trash (`trash`, system) and the drives that use it (`trash_drives`, fixed): on Windows only fixed drives keep trashed files safely. Elsewhere the delete is listed and asks before it deletes for good. Replaced files go to the trash first (`replace_to_trash`, on), so Undo can bring them back.
- Portable names (`portable_names`, on): refuses names that other systems cannot store, such as a colon or a trailing space, when diroxide makes or renames an entry.
- Dialog settle time (`dialog_settle_ms`, 250; 0 to 2000): keys that arrive this soon after a dialog opens are ignored.
- Completion list (`completion_list`): `auto` (default) lists candidates as you type in the window and on request in a terminal, `as_you_type`, or `on_tab`.
- Cursor fallback (`cursor_fallback`, off): parks the cursor on the status line instead of the row, for a screen reader that follows it better there.
- Compact (`compact`, off): hides the key bar and the title line in a terminal, and keeps the status and jobs lines.
- Editor (`editor`, empty): the program F4 starts, a name on the path or a full path with no arguments. If it is empty, diroxide tries VISUAL, then EDITOR, then the system's text editor.
- Password echo (`secret_echo`, asterisks) and Passwords shown (`secret_show_toggle`, off): what the masked password prompt shows for each key, which archives use. `silent` shows nothing. A password is never read back, logged or kept after the job.
- Code colors (`highlighting`): `auto` (default) colors code in the viewer except in screen-reader mode, `on` colors it always, and `off` never does. The code commands that say each word's kind work whichever you choose. [The viewer](viewer.md) describes them.

How long a copy retries a lost network, how long it waits before it says "stalled", and whether it keeps dates or checks a move are built into the job engine. They are not settings.

### The viewer, the quick view, archives and split sizes

Alpha.2 adds four groups of settings. [The settings reference](settings-reference.md) lists every one with its default and range, under Viewer and quick view and under Archives and split.

- The viewer (`viewer.*`): wrap (`viewer.wrap`, on) and the width it wraps at (`viewer.wrap_width`, 0 for the screen's width, or a number such as 40 for a Braille line), line numbers (`viewer.line_numbers`, off), follow a growing file (`viewer.follow`, off), the hex row (`viewer.hex_row`: `auto` is eight bytes in screen-reader mode and sixteen otherwise), how much of an archive member it keeps decoded (`viewer.member_cache_mib`, 64), and the four caps on code colors (`viewer.highlight_max_kib`, `viewer.highlight_max_lines`, `viewer.highlight_max_line_chars` and `viewer.highlight_deadline_ms`). [The viewer](viewer.md) says what each does.
- The quick view (`quick_view.*`): how long it waits after the cursor stops (`quick_view.wait_ms`, 500), how much of a file it reads (`quick_view.max_kib`, 1024), how long reading may take (`quick_view.deadline_ms`, 2000), whether it follows changes on disk (`quick_view.follow_changes`, on) and line numbers (`quick_view.line_numbers`, off). [The quick view](quick-view.md) says what each does.
- Archives (`archives.*`): nineteen settings, the limits that protect you from an archive made to fill a drive or a memory, and the two provider settings, `archives.ask_read_ahead_mib` and `archives.spool_folder`. In the file they are one table, `[archives]`. The defaults suit ordinary use. [Archives](archives.md) lists each limit with the question it asks.
- Split sizes (`split_presets`): the sizes Split offers, a list of presets. In the Settings list the row opens a list; in `config.toml` you add your own with `[[split_presets]]` tables. [Split and join](split-join.md) has the built-in sizes and how to add one.

```toml
[archives]
max_ratio = 2000
```

```toml
[viewer]
wrap_width = 40
hex_row = "narrow"
```

```toml
[quick_view]
wait_ms = 800
```

<!-- to confirm with the wiring (W2f): that Enter on the Split presets row opens a list and not a text field, and that the settings list shows the four new groups in this order. -->

Some settings are kept now and do nothing yet. The reference marks each with the alpha that brings it: the key preset (alpha.3), the filter's case, pause and first day of the week (alpha.3), the search limits (alpha.3), plugins (alpha.4), the components list (alpha.4), thumbnails (alpha.7), and hiding the menu bar and focus mode in the window (alpha.6).

## Themes and colors

diroxide comes with 24 themes. Their names, for the `theme` setting, are in the reference. The default is `galaxy`. Among the others are its light partner `galaxy-light`, `contrast` and `high-contrast`, `lamplight`, `sepia`, `amber`, `phosphor`, dark and light pairs of Gruvbox, One and Solarized, and `nord`, `dracula` and `tokyo-night`.

- Choose one in Settings (`theme`), or for one session with `diro --theme NAME`. An unknown name says "Unknown theme" and Galaxy is used. Next theme, in the View menu (Alt+V, then T) and the palette, steps through them.
- When you choose a theme, diroxide says whether it meets contrast in words: "Theme Nord, meets AA", or "Theme Nord, below AA: 2 checks fall short". A high-contrast theme is held to AAA.
- Nothing is shown by color alone. Every highlight is also bold, underlined, reversed or marked in text, such as the word "selected" on a marked row.
- Themes come in light and dark partners, and `follow_system_appearance` (off) can switch between them with your system.
- Your own theme is a TOML file in the `themes` folder in the config folder. It is listed after the built-in ones.
- To change one color, use the `[colors]` table in `config.toml` with a role's name and a color as `#rrggbb`, such as `folder = "#ffaa00"`. The reference lists the roles. diroxide checks the contrast and says the result in words.
- `NO_COLOR` is honored: with it set, diroxide uses attributes alone. The default look draws no boxes. The `borders` setting (off) turns on box drawing for sighted users.

## Changing keys

Every key is yours to change, with no sequences and no leader keys: each binding is one key with its modifiers. You change them in `keymap.toml`. The Keys list inside the program comes with alpha.3.

- A key is written in plain words joined by plus signs, in lowercase: `ctrl+f5`, `shift+f3`, `alt+home`, `alt+period`, `kp-plus` for the keypad's plus. The word `none` unbinds a command.
- The file has one line for each command and each place it works, in tables named for the column and the layer: `terminal.panel`, `terminal.global`, `terminal.list` and `terminal.dialog`, and the same for `window`. A value is one key, or a list of keys. The built-in defaults are written beside it in `keymap-defaults.toml`, so you can copy a line and change it.
- A line looks like this:

```toml
[terminal.panel]
"file.copy" = "f5"
```

- Command ids such as `file.copy` are in parentheses on [the keyboard page](keyboard.md).
- Insert is never available, because it is the screen readers' key.
- Two commands cannot share a key in one place. diroxide tells you which command already has it.
- diroxide knows which keys a terminal usually cannot send, such as Ctrl+Enter and most Ctrl+Shift letters, and which keys screen readers keep. A terminal binding that uses one is reported in words when diroxide starts.
- The `single_letters` setting turns every key without Control or Alt off.

Three subcommands help, and none opens the program:

```bash
diro keys check
```

`diro keys check` reads your `keymap.toml`, or the file you name after `check`, and says every problem and every conflict in words, one a line. It exits with 0 when the file is clean, 1 when it found problems, and 2 when it could not read the file.

```bash
diro keys print
```

`diro keys print` writes the current keys of both columns as `keymap.toml` text. Saved as `keymap.toml`, it gives the same keys, so a keymap can live in your own dotfiles and travel between machines.

```bash
diro keys test
```

`diro keys test` is the key test, described in [Terminals](terminals.md).

## Menus and banks

`menus.toml` uses the same command ids as the keymap. Add an item with a command id, remove or reorder items, and add submenus. Each menu has an access key, the letter after Alt, and diroxide reports unknown items in words. A menu item can be shown only where it makes sense, for example only where the place has a trash.

A bank is a named bar of command buttons. The default bank is the function key bar. A button runs one command.

## Starting fresh

To go back to the defaults, move the config folder aside by renaming it, and start diroxide. It writes new files when you change something. Your old files stay where you put them, so you can bring a setting back.

## See also

- [The settings reference](settings-reference.md)
- [The panels](panels.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Every key](keyboard.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
