# Settings and where the files live

This guide says where diroxide keeps its files, how to change a setting, which settings most people look for, and how to change keys, menus and themes. It describes the settings in words. The settings reference page lists every setting with its exact name, default, range and help, and is generated from the program's own list of settings, so it is always right. This guide is the story, and that page is the dictionary.

## Where the files live

diroxide keeps two folders. The config folder holds the files you edit. The state folder holds what diroxide remembers for you.

- Windows: the config folder is `%APPDATA%\diroxide` and the state folder is `%LOCALAPPDATA%\diroxide`.
- Linux: the config folder is `~/.config/diroxide` and the state folder is `~/.local/state/diroxide`. If you have set `XDG_CONFIG_HOME` or `XDG_STATE_HOME`, diroxide uses those.
- macOS: both are in `~/Library/Application Support/diroxide`.
- With `diro --portable`, the folders are `config` and `state` beside the program, so you can carry everything on a drive.

In the config folder, alpha.1 uses these files:

- `config.toml` holds the settings.
- `keymap.toml` holds your changes to the keys.
- `menus.toml` holds the menus. The built-in menus are written as its default, and you edit it to add, remove and reorder items.
- `banks.toml` holds the banks of command buttons, including the function key bar.
- `filetypes.toml` is a placeholder that later alphas fill in.

Later alphas add more files, such as the tools list and the plugins folder. They are described when they arrive.

The state folder holds each panel's folder, focused row, sort, filter and the layout from your last session; the history of each prompt; the list of folders path jump learned from; the list of recent jobs for Undo; and the log file. Nothing in it needs editing.

## How diroxide treats these files

- It writes a file only when you make an explicit change, and it writes atomically, so a crash cannot leave half a file.
- It keeps what it does not know. A setting that a newer or older version wrote stays in the file, and your comments and the order of your lines stay too.
- A file it cannot read is never thrown away. diroxide uses the defaults for that file, keeps the old one beside it with `.bad` added to its name, and tells you in words at start: "Could not read keymap.toml: line 12. Using defaults; the old file was kept as keymap.toml.bad."
- Every unknown command or key it finds in `keymap.toml` or `menus.toml` is reported in words at start, and never dropped silently. The status line says how many problems there were, such as "keymap.toml: 2 problems. Ctrl+P, Problems", and the palette has a Problems entry that lists them.

## Changing a setting

In the program, open Settings from the Tools menu or the palette. It is a list. Each row says the setting's name, its value and its default within the first 40 characters. Enter changes the value, and the help key gives the setting's full help. Changes are saved at once.

By file, open `config.toml` in any text editor. diroxide reads it the next time it starts. A setting is one line:

```toml
layout = "stacked"
```

If a value is not one diroxide understands, it says so in words at start and uses the default for that setting.

## The settings most people look for

Names are in code type. Defaults are in parentheses.

- Layout (`layout`): `side_by_side` (default), `stacked` or `single`. [The panels guide](panels.md) says which suits a screen reader.
- Screen reader mode (`screen_reader_mode`): turn it on when you use a screen reader. It slows the progress line to the interval below, hides the key bar, redraws the other panel only on demand, and keeps the candidate list closed until you press Tab. diroxide does not try to detect your screen reader. Check this setting yourself.
- Progress interval (`progress_interval_seconds`, 10): in screen-reader mode, how many seconds pass between progress updates. A change of state is always announced at once.
- Single-letter keys (`single_letters`): what typing a letter does in a panel. `type-ahead` (default) finds a row by name, and `off` does nothing. A third value, `commands`, lets single letters run commands, for the key presets that come with alpha.3. Turn it off if you use dictation software.
- Show hidden files (`show_hidden`, off).
- Sorting: the key (`sort`), its direction (`sort_direction`), where folders go (`folders`: first by default, or mixed or last), case (`sort_case`, ignored by default) and natural order (`sort_natural`, on, so `file2` comes before `file10`).
- Confirmations (`confirm`): how each kind of question is asked. Every question starts on No.
- Keep dates (`keep_dates`, on): a copy keeps the modified time, and the created time where the system allows it.
- Verify moves (`verify_moves`, on): a move across drives checks the copy before it removes the original. Verify copies (`verify_copies`, off) does the same check for copies, at the cost of reading each file twice.
- Network and stalls: retries (`network_retry_count`, 3), seconds between them (`network_retry_seconds`, 5) and how long without data counts as a stall (`stall_seconds`, 20).
- Dialog settle time (`dialog_settle_ms`, 250): keys that arrive this soon after a dialog opens are ignored.
- Completion list (`completion_list`): whether candidates show as you type or only on Tab. In the terminal the default is on Tab.
- Cursor fallback (`cursor_fallback`): parks the cursor on the status line instead of the row, for a screen reader that talks too much when the cursor moves.
- Compact (`compact`): hides the key bar and the title line, and keeps the status and jobs lines.
- Editor (`editor`): the program F4 starts. If it is empty, diroxide tries VISUAL, then EDITOR, then the system's text editor.
- Portable names (`portable_names`, on): flags names that would not work on every system, such as a name that ends in a space.
- Windows dotfiles hidden (`windows.dotfiles_hidden`, off): treat names that start with a dot as hidden on Windows too.

## Themes and colors

diroxide comes with 24 themes: Galaxy (the default) and its light partner, Contrast and High Contrast, Lamplight, Sepia, Amber, Phosphor, Gruvbox, One and Solarized in dark and light, and others such as Nord, Dracula and Tokyo Night.

- Choose one in Settings (`theme`), or for one session with `diro --theme NAME`. Next theme in the View menu and the palette steps through them.
- When you choose a theme, diroxide says whether it meets contrast in words: "meets AA", or "below AA: 2 checks fall short".
- Nothing is shown by color alone. Every highlight is also bold, underlined, reversed or marked in text, such as the asterisk on selected rows.
- Themes come in light and dark partners, and `follow_system_appearance` can switch between them with your system.
- Your own theme is a TOML file in the config folder; it is listed after the built-in ones.
- To change one color, use the `[colors]` table in `config.toml` with a role's name. diroxide checks the contrast and says the result in words.
- `NO_COLOR` is honored: with it set, diroxide uses attributes alone. The default look draws no boxes. The `borders` setting (off) turns on box drawing for sighted users.

## Changing keys

Every key is yours to change, with no sequences and no leader keys: each binding is one key with its modifiers. You change them in `keymap.toml`. The Keys list inside the program comes with alpha.3.

- A key is written in plain words joined by plus signs: `ctrl+f5`, `shift+f3`, `alt+home`. The word `none` unbinds a command.
- The file has one line for each command and each place it works, in two columns: the terminal and the window. The built-in defaults are written beside it in a read-only reference file that diroxide regenerates at each start, so you can see what you are changing.
- A line looks like this:

```toml
[terminal.panel]
"file.copy" = "f5"
```

- Command ids such as `file.copy` are listed on [the keyboard page](keyboard.md).
- Insert is never available, because it is the screen readers' key.
- Two commands cannot share a key in one place. diroxide tells you which command already has it.
- diroxide knows which keys a terminal usually cannot send, such as Ctrl+Enter and most Ctrl+Shift letters. A terminal binding that uses one is reported in words when diroxide starts.

Two subcommands help, and neither opens the program:

```bash
diro keys check
```

`diro keys check` reads your file and says every problem and every conflict in words. It exits with 0 when the file is clean, 1 when it found problems, and 2 when it could not read the file.

```bash
diro keys print
```

`diro keys print` writes the current keys as two lists, one for the terminal and one for the window, so a keymap can live in your own dotfiles and travel between machines.

## Menus and banks

`menus.toml` uses the same command ids as the keymap. Add an item with a command id, remove or reorder items, and add submenus. Each menu has an access key, the letter after Alt, and diroxide checks the access keys for conflicts and reports unknown items in words.

A bank is a named bar of command buttons. The default bank is the function key bar, with a button for each of F2 to F9. A button runs one command.

## Starting fresh

To go back to the defaults, move the config folder aside by renaming it, and start diroxide. It writes new files when you change something. Your old files stay where you put them, so you can bring a setting back.

## See also

- [The panels](panels.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Every key](keyboard.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
