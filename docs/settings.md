# Settings and where the files live

This guide says where diroxide keeps its files, how to change a setting, which settings most people look for, and how to change keys, menus and themes. It describes the settings in words. [The settings reference](settings-reference.md) lists every setting with its exact name, default, range and help, and is generated from the program's own list of settings, so it is always right. This guide is the story, and that page is the dictionary.

## Where the files live

diroxide keeps two folders. The config folder holds the files you edit. The state folder holds what diroxide remembers for you.

- Windows: the config folder is `%APPDATA%\diroxide` and the state folder is `%LOCALAPPDATA%\diroxide`.
- Linux: the config folder is `~/.config/diroxide` and the state folder is `~/.local/state/diroxide`. If you have set `XDG_CONFIG_HOME` or `XDG_STATE_HOME`, diroxide uses those.
- macOS: both are in `~/Library/Application Support/diroxide`.
- With `diro --portable`, the folders are `config` and `state` beside the program, so you can carry everything on a drive.

The config folder holds these files:

- `config.toml` holds the settings.
- `keymap.toml` holds your changes to the keys. diroxide writes the built-in keys beside it as `keymap-defaults.toml`, a read-only reference that it rewrites at each start; changes to that copy are not read.
- `menus.toml` holds your changes to the menus. The built-in menus are written beside it as `menus-defaults.toml`, also read-only. Copy it to `menus.toml` and change it there to add, remove and reorder items and submenus.
- `banks.toml` holds the banks of command buttons, including the function key bar.
- `filetypes.toml` is a placeholder that later alphas fill in.
- A `themes` folder holds your own theme files.
- A `plugins` folder holds your plugins, one folder each, and its `scripts` folder holds your script commands. [Plugins](plugins.md) and [script commands](script-commands.md) describe them.

Later alphas add more files, such as the tools list. They are described when they arrive.

The state folder holds each panel's folder, focused row, sort and history, and the layout, from your last session (`session.toml`); the history of each prompt (the `history` folder); the folders path jump learned from; the list of recent jobs for Undo; `terminal.log`; and a `spool` folder, where an archive inside another archive is kept while it is open when it is too big for memory. diroxide uses it instead of the system's temporary folder, and the `archives.spool_folder` setting can name another place. It also holds `labels.toml` (your labels and where they are), `bookmarks.toml`, `folder-views.toml` (the view saved for a folder) and a `saved` folder with one file for each saved search. [Labels and bookmarks](labels-bookmarks.md) describes the first three. Nothing in it needs editing, though the label file is the one place to rename or recolor a label for now. A `plugins` folder holds what you turned on (`consent.toml`), a folder for each plugin with its log and its data, and a `runs` folder for the short-lived files of a running script. The SFTP places plugin keeps `sites.toml`, the list of your sites, in its folder there. `partials.json` holds the partial files a lost network left, so a copy can go on from them ([the transfer queue](transfer-queue.md) says how). A `components` folder holds the optional components you installed, one folder each. [Optional components](components.md) describes them.

## How diroxide treats these files

- It writes a file only when you make an explicit change, one value at a time, and it writes atomically, so a crash cannot leave half a file.
- It keeps what it does not know. A setting that a newer or older version wrote stays in the file, and your comments and the order of your lines stay too.
- A file it cannot read is never thrown away. diroxide uses the defaults for that file, keeps the old one beside it with `.bad` added to its name, and tells you in words at start.
- Every unknown command or key it finds in `keymap.toml` or `menus.toml` is reported in words at start, and never dropped silently. The status line says how many problems there were: "keymap.toml: 2 problems. Config problems in the Help menu lists them." Config problems, in the Help menu (Alt+H, then C) and the palette, lists each one with its file.

## Changing a setting

In the program, open Settings from the Tools menu (Alt+T, then S) or the palette. It is a list with one row for each setting, in the order of the reference. Each row says the setting's name, then its value in words, and, for a setting that does nothing yet, that it comes with a later alpha. Enter flips a switch, or opens the choices with "Back to default" last, or opens a field for a number or text. The help key (Shift+F1 or the question mark) on a row gives that setting's help. The change is saved at once, and diroxide says it: "Layout: stacked", or "Layout back to side by side".

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
- Key bar (`key_bar`, on): the line at the bottom that names the function keys in words, "F2 Rename  F3 View  F4 Edit  F5 Copy  F6 Move  F7 Folder  F8 Trash  F9 Menu  Ctrl+Q Quit". It is drawn in every mode, screen-reader mode included, and the width decides how many pairs fit. The Compact setting hides it, and a screen too short to hold it leaves it out. Say key bar, in the Help menu and the palette, reads it. In the Settings list it is the Key bar switch in the Look group.
- Quiet screen (`quiet_screen`, off): draws as little as possible, so a screen reader that reads every change stays quiet.
- Progress interval (`progress_interval_seconds`, 10; 1 to 600): in screen-reader mode, the shortest time between two progress lines. A job's start, its end and its questions are said at once.
- Single letters (`single_letters`): what a key without Control or Alt does in a panel. `type_ahead` (default) jumps to a name, `commands` lets letters run commands, which the vim-style key preset needs ([customizing keys](customizing-keys.md)), and `off` does nothing. The Filter command still filters when it is off. Turn it off if you use dictation software.
- Hidden files (`show_hidden`, off).
- Sorting: what a new panel sorts by (`sort.key`: name by default, or extension, size, modified, created, accessed, kind, attributes, owner, label or unsorted), the direction (`sort.direction`), where folders go (`sort.folders`: first by default, mixed or last), capitals (`sort.case`: ignore by default, or respect) and numbers in names (`sort.natural`, on, so `file2` comes before `file10`).
- Columns (`columns`): the details said after each name, in order. The default is type, size and modified.
- Confirmations (`confirm.copy`, `confirm.move`, `confirm.trash` and `confirm.quit_with_jobs`, all on): whether those four questions are asked. Every question that is asked starts on No, and deleting for good always asks.
- Trash (`trash`, system) and the drives that use it (`trash_drives`, fixed): on Windows only fixed drives keep trashed files safely. Elsewhere the delete is listed and asks before it deletes for good. Replaced files go to the trash first (`replace_to_trash`, on), so Undo can bring them back. The trash browser lists what is in the system's trash; [volumes and the trash](volumes-trash.md) describes it.
- Hidden volumes (`hidden_volumes`, none; in the Panels group): the volumes the Volumes list leaves out, by device or place, as its Hide action keeps them. Remove an entry to list the volume again.
- Portable names (`portable_names`, on): refuses names that other systems cannot store, such as a colon or a trailing space, when diroxide makes or renames an entry.
- Dialog settle time (`dialog_settle_ms`, 250; 0 to 2000): keys that arrive this soon after a dialog opens are ignored.
- Completion list (`completion_list`): `auto` (default) lists candidates as you type in the graphical interface (GUI) and on request in a terminal, `as_you_type`, or `on_tab`.
- Cursor fallback (`cursor_fallback`, off): parks the cursor on the status line instead of the row, for a screen reader that follows it better there.
- Compact (`compact`, off): hides the key bar and the title line in a terminal, and keeps the status and jobs lines.
- Editor (`editor`, empty): the program F4 starts, a name on the path or a full path with no arguments. If it is empty, diroxide tries VISUAL, then EDITOR, then the system's text editor.
- Password echo (`secret_echo`, asterisks) and Passwords shown (`secret_show_toggle`, off): what the masked password prompt shows for each key, which archives use. `silent` shows nothing. A password is never read back, logged or kept after the job.
- Code colors (`highlighting`): `auto` (default) colors code in the viewer except in screen-reader mode, `on` colors it always, and `off` never does. The code commands that say each word's kind work whichever you choose. [The viewer](viewer.md) describes them.

How long it waits before it says "stalled" (20 seconds), and whether a copy keeps dates or checks a move, are built into the job engine. They are not settings. The tries on a lost network and the queue are, below.

### Jobs and transfers

The Settings list has a group of seven settings for the queue, retries, the speed limit and errors, each with F1 help. [The transfer queue](transfer-queue.md) explains them in use.

- Jobs at once per server (`jobs.per_host`, 2; 1 to 8): how many jobs run at once on one remote server. More wait their turn. Jobs on this computer always start at once.
- Remote jobs at once (`jobs.remote`, 4; 1 to 16): how many jobs on remote servers run at once in all.
- Speed limit (`jobs.speed_limit_kb`, 0; 0 to 10000000): the most a job may read and write in a second, in KB. 0 is off. The Speed limit command in the palette changes it, and the Jobs list sets a limit for one job.
- Speed limit for (`jobs.speed_limit_scope`, remote): which jobs the limit slows, `remote` or `all`.
- Tries on a lost network (`jobs.network_retries`, 3; 0 to 10): how many times a lost network is tried again, without a word, before the job asks. 0 is off: the job asks at once.
- First wait before a try (`jobs.network_retry_seconds`, 5; 1 to 600): how long the first wait is. Each wait is three times the one before, so 5 gives waits of 5, 15 and 45 seconds. No wait is longer than ten minutes.
- On errors (`jobs.on_errors`, ask): what a new job does when an item fails. `ask` asks each time, `continue_and_list` goes on and lists the failures at the end, and `stop_at_first` stops.

```toml
[jobs]
per_host = 2
network_retries = 3
on_errors = "continue_and_list"
```

### Search, filters, compare and keys

These settings are for the filter language, search, compare and the key presets. [The settings reference](settings-reference.md) lists every one with its default and range, under Search and filter and under Keys.

- Filter case (`filter_case`): how case decides a match in filters and searches. `smart` (default) ignores case unless the text has a capital letter, `ignore` never cares and `match` always does.
- Filter pause (`filter_pause_ms`, 400; 0 to 5000): how long the panel filter waits after your last key before it says the count. A longer pause is quieter.
- First day of the week (`week_starts_on`, sunday): the day that "this week" and "last week" start on. The choices are sunday, monday and saturday. The read-back always spells out the real days.
- Search result limit (`search_row_cap`, 100000; 1000 to 10000000): the most rows a search lists. The count goes on past it.
- Search skips ignored files (`search_respect_ignore`, off): leaves out what `.gitignore` and similar files name. A file manager is not a code search, so it is off.
- Key preset (`preset`, diroxide): the set of keys under your own changes. Choose it in the Keys list or with `diro keys`. [Customizing keys](customizing-keys.md) describes the presets.
- Compare (`compare.*`): eleven settings, in one table, `[compare]`. Six leave differences out: `compare.ignore_line_endings` (on), `compare.ignore_trailing_space`, `compare.ignore_space_amount`, `compare.ignore_all_space`, `compare.ignore_blank_lines` and `compare.ignore_case` (all off). `compare.algorithm` is patience, histogram or myers (patience). `compare.deadline_ms` (2000) is how long a compare looks for the fewest changes, `compare.text_limit_mib` (16) the largest file compared as text, `compare.large_files` (ask, head or bytes) what to do past that, and `compare.context` (2; 0 to 9) the lines shown around a change. [Compare files](compare.md) says what each does.
- Label colors (`label_blue`, `label_orange`, `label_purple`, `label_yellow`, `label_gray` and `label_brown`): the theme roles that color a label's word. They are keys of the `[colors]` table, like the other roles.

### Plugins and optional components

[The settings reference](settings-reference.md) lists five settings under Plugins. You turn a plugin on and off in Manage plugins, in the Plugins menu, or with `diro plugin enable` and `diro plugin disable`. The record, with the fingerprints, is kept in `consent.toml` in the state folder. `plugins.enabled` is reserved and does nothing.

- Plugin time limit (`plugins.timeout_ms`, 10000; 1000 to 600000): how long a plugin may take to answer before it is reported and left.
- Components list (`components_source`, empty): where you keep your own copy of the optional components, a folder or a repository written `owner/name`. Empty means the public list only. It is tried before the public address. Nothing else names it, and the question that asks to download says only "From your own components source." [Optional components](components.md) describes it.
- First-run list shown (`components_first_run_shown`, off): whether the first-run list of optional components was shown. It turns on by itself when the list opens, and the list is not shown again at a first start.
- Ask about components again (`components_ask_again`, off): on, the first-run list opens once at the next start, with nothing chosen. The setting turns itself off when the list opens. Ask first-run choices again, in the Tools menu (Alt+T, then F), opens the list now. Any opening of the list turns First-run list shown on and this setting off. [Optional components](components.md) describes the list.

The SFTP places plugin's settings (which ssh to use, timeouts, how many requests are in flight) are listed in [remote places](remote-places.md). A plugin's own settings are listed in its manifest. The Settings list has a row, "Plugin settings", that opens them: pick a plugin, then a setting. A change reaches the plugin at once, and the F1 help says what each setting does. The values are kept in `settings.toml` in the plugin's folder of the state folder. [Plugins](plugins.md) has the details.

### The viewer, the quick view, archives and split sizes

Four groups of settings cover the viewer, the quick view, archives and split sizes. [The settings reference](settings-reference.md) lists every one with its default and range, under Viewer and quick view and under Archives and split.

- The viewer (`viewer.*`): wrap (`viewer.wrap`, on) and the width it wraps at (`viewer.wrap_width`, 0 for the screen's width, or a number such as 40 for a Braille line), line numbers (`viewer.line_numbers`, off), follow a growing file (`viewer.follow`, off), the hex row (`viewer.hex_row`: `auto` is eight bytes in screen-reader mode and sixteen otherwise), how much of an archive member it keeps decoded (`viewer.member_cache_mib`, 64), and the four caps on code colors (`viewer.highlight_max_kib`, `viewer.highlight_max_lines`, `viewer.highlight_max_line_chars` and `viewer.highlight_deadline_ms`). [The viewer](viewer.md) says what each does.
- The quick view (`quick_view.*`): how long it waits after the cursor stops (`quick_view.wait_ms`, 500), how much of a file it reads (`quick_view.max_kib`, 1024), how long reading may take (`quick_view.deadline_ms`, 2000), whether it follows changes on disk (`quick_view.follow_changes`, on) and line numbers (`quick_view.line_numbers`, off). [The quick view](quick-view.md) says what each does.
- Archives (`archives.*`): nineteen settings, the limits that protect you from an archive made to fill a drive or a memory, and the two provider settings, `archives.ask_read_ahead_mib` and `archives.spool_folder`. In the file they are one table, `[archives]`. The defaults suit ordinary use. [Archives](archives.md) lists each limit with the question it asks.
- Split sizes (`split_presets`): the sizes Split offers, a list of presets. In the Settings list the row opens a list you can read, not a field; in `config.toml` you add your own with `[[split_presets]]` tables. [Split and join](split-join.md) has the built-in sizes and how to add one.

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

Some settings are kept now and do nothing yet: thumbnails, and hiding the menu bar and focus mode in the GUI. The reference marks each as coming with a later alpha.

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

Every key is yours to change, with no sequences and no leader keys: each binding is one key with its modifiers. You change them in `keymap.toml`, or in the program, in the Keys list (Settings, then Keys), which writes the same file for you. [Customizing keys](customizing-keys.md) is the guide to the Keys list, the key test and the presets.

- A key is written in plain words joined by plus signs, in lowercase: `ctrl+f5`, `shift+f3`, `alt+home`, `alt+period`, `kp-plus` for the keypad's plus. The word `none` unbinds a command.
- The file has one line for each command and each place it works, in tables named for the column and the layer: `terminal.panel`, `terminal.global`, `terminal.list` and `terminal.dialog`, and the same for `gui`. A value is one key, or a list of keys. The built-in defaults are written beside it in `keymap-defaults.toml`, so you can copy a line and change it.
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
- [Customizing keys](customizing-keys.md)
- [Search and the filter language](search.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Plugins](plugins.md)
- [The transfer queue](transfer-queue.md)
- [Remote places over SFTP](remote-places.md)
- [Volumes and the trash](volumes-trash.md)
- [Optional components](components.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
