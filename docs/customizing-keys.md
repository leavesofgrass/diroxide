# Customizing keys

This guide covers how to see every key diroxide has, change one, test what your terminal sends, and switch to a set of keys modeled on another file manager. It describes the Keys list, Change key, the key test, the six presets, `diro keys`, and the `keymap.toml` file behind them. You can still change keys by editing `keymap.toml` by hand, and that is described below too.

What this guide says comes from the design and from the tests that check each sentence. Nothing in alpha 1 had been heard on a screen reader when this guide was written, because the listening tests come after the build, so the lines below are what diroxide is built to say. [The accessibility statement](accessibility.md) says what has been checked.

## How the keys are layered

The keys diroxide uses are built in three layers, one on top of the next:

1. The built-in keys, the diroxide defaults.
2. A preset, if you chose one. It changes only the commands it names.
3. Your own changes, in `keymap.toml`.

A key in a higher layer wins. Choosing a preset sets its name in `config.toml` (the `preset` setting, `diroxide` by default) and clears nothing of yours. Changing a key writes one line to `keymap.toml`. So a preset can be updated without losing your changes, and you can go back to the defaults without losing them either.

`diro keys print` writes the keys that are in effect now, from all three layers, as `keymap.toml` text.

Every key is one key with its modifiers. There are no key sequences and no leader keys, so a key never waits for a second one. Insert and Caps Lock are never bound, because they are the screen readers' keys.

There are two columns of keys, because a terminal cannot send everything a graphical interface (GUI) can. The terminal column holds the keys a terminal can send. The GUI column, used when the GUI comes in a later alpha, also holds keys such as Ctrl+Enter. Most keys are the same in both, and the lists say "in both".

## The Keys list

The Keys list shows every command and the key it has. Open it from Settings (Alt+T, then S), where its row reads "Keys list, every command and its key", from the Tools menu's Keys submenu (Alt+T, then E, then L), or from the palette. It has no key of its own.

It is a list like every other: type a letter to jump, Home and End, Shift+F1 for help, Ctrl+D for where you are, and the position said last. The title says the preset and how many commands you changed: "Keys, preset diroxide defaults, 0 changed".

The list's own rows come first: Choose a preset, Reset all keys and Single letters. Then one row for each command, with the command first, then its key or keys, then state words:

- "Copy to other panel, F5 in both". A built-in key has no state word.
- "Rename, Shift+F6 in both, changed, check terminal". The word "changed" means you changed it. The word "preset" means a preset did: "Rename, F2 in both, preset".
- "Paste, Ctrl+V in both, check terminal". The words "check terminal" mean the terminal key is one that many terminals keep, merge with another key or send differently, so it may not work for you. You learn this before you press it. Every Shift plus function key says it, because some terminals send those as F13 to F24.
- "Compare files, no key". A command can be reached from the palette and the menus without a key.
- When the terminal and the GUI differ: "Sort by size, Ctrl+F9 in a terminal, Ctrl+F6 in the GUI". With no terminal key: "Settings, no key in a terminal, Ctrl+Comma in the GUI".

Typing narrows the list. A word matches command names. Text that reads as a key, such as `ctrl+f5`, matches the commands that have that key. The words "unbound", "changed", "preset" and "conflicts" show those rows. The first row and the count are said: "Copy path, no key. 4 matches". Nothing matching says "No command matches".

Enter on a row opens its actions: Change key (press the new key), Change the GUI key (type its name), Add a second key, Remove a key, Reset this key (back to the preset) and Test keys. Remove a key, on a command with two keys, asks which.

### Choose a preset, Reset all keys, Single letters

- Choose a preset opens the list of presets (below).
- Reset all keys takes every change of yours out of `keymap.toml`. It asks, starting on No: "Reset all keys? No is the default. y or n". Then "All keys reset: 3 commands back to the default." Your comments and unknown lines stay in the file. With nothing changed: "No key was changed. Nothing to reset."
- Single letters says what a key with no Control or Alt does in a panel. It has three values: type to find (`type_ahead`, the default), where a letter jumps to a name; commands (`commands`), where a letter runs a command, which the vim-style preset needs; and off, where single keys do nothing, the choice for dictation software, which types letters. Every command still has a key that is not a letter, a menu entry and a palette entry.

## Change a key, step by step

Choose a row, press Enter, and choose Change key.

1. Which column. In a terminal, only the terminal column can be changed by pressing a key. The GUI column is changed by typing the key's name, because a terminal cannot produce keys such as Ctrl+Enter that only the GUI sends. That is the action "Change the GUI key, type its name".
2. Press the new key. diroxide says "Press the new key now. Escape cancels." The next key press is taken before the keymap sees it. A key release and a bare modifier are ignored. A pasted text is refused: "Pasted text is not a key. Press one key." The first key press ends the capture.
3. diroxide says the key back, in words: "Got Control plus F9". On a terminal that sends some keys as others, it adds what it knows: "Got Tab. This terminal sends Ctrl+I as Tab too."
4. If nothing arrives in 10 seconds, it says "No key heard. The terminal may keep it." and opens a field: "Type the key's name, such as ctrl+f5". The typed name is an equal way in. It is read with the same code as `keymap.toml`, so `ctrl+f9` and `alt+home` work, and to bind Escape itself you type its name. A name that is not a key says so: "Not a key name: hyper+q. Write it as keymap.toml does, such as ctrl+f5." Two keys in a row are refused: "Two keys in a row: ctrl+x ctrl+f. A key is one key with its modifiers."
5. It checks the key, in this order. Each check that needs your answer is a question, and every question starts on No:
   - "Insert is the screen reader key." The key is refused.
   - A key the terminal cannot send, in the terminal column: "The terminal cannot send Ctrl+Enter. Use it for the GUI column only? No is the default. y or n".
   - A key a terminal program keeps, which is said and the change goes on: "Windows Terminal keeps Ctrl+V: it pastes text, so the key never arrives."
   - A key a screen reader owns: "Screen readers keep Ctrl+Alt+Up: NVDA, JAWS and Narrator. Use it anyway? No is the default. y or n".
   - A key kept for a command that comes in a later alpha. diroxide names the command, as in "Ctrl+T is kept for New tab", and asks "Use it anyway? No is the default. y or n".
   - A key another command has, where this command's keys are looked up: "F5 already copies to the other panel. Take it, swap, or cancel?" A list opens with the focus on Cancel. Take it leaves the other command without the key. Swap gives it this command's old key. Cancel changes nothing: "Nothing changed." Two commands cannot share a key in one layer. A command you need to find your way, such as Say status, the palette, help, the menus and quit, is never left without a key, so only Swap is offered: "Ctrl+D already says the status. Swap, or cancel? Say status needs a key."
   - A letter: "A letter key works only while letters run commands. Go on? No is the default. y or n".
6. It writes one line to `keymap.toml`, safely, and says "Ctrl+F9 saved for Sort by size." Your comments and the order of your lines stay. After a take or a swap it also says what happened to the other command: "Copy to other panel has no key now."

On Windows a text paste arrives as typed keys, so during the capture its first character is taken as the key. The questions after it start on No and ignore keys that come right after they open, so a paste cannot answer for you. Paste files with Paste in the Edit menu.

Reset this key removes that line and says "Rename back to F2." A file that cannot be read keeps its old copy as `keymap.toml.bad`, and diroxide says so in words. A command or a key in the file that diroxide does not know is kept and reported, never dropped.

A keymap moved to a computer with another keyboard layout may need a look, because symbols depend on the layout and `keymap.toml` stores the character.

## The key test

Test keys says what the terminal sent and what it would do now: "Got Control plus F5. Runs Sort by modified. Sent F5 with Control, a press." It stays open until you press Escape, or until ten seconds pass with no key. It answers "my key does nothing". Either the terminal never sent the key, or it sent a different one. [Terminals](terminals.md) describes the key test and the probe.

It is in the Keys list's actions, in the Tools menu (Alt+T, then K), in the palette, and in a shell:

```bash
diro keys test
```

While Change key waits for a key, diroxide asks a terminal that can report keys exactly to do so, and puts the terminal back as it was as soon as the key came. With it, Tab and Ctrl+I, and Enter and Ctrl+M, are told apart, and the capture says which kind of terminal you have: "This terminal reports keys exactly." or "This terminal merges Ctrl+I with Tab, and Ctrl+M with Enter." Nothing is ever moved off a key a terminal can send because of this. On Windows keys arrive as key records and the capture says nothing about the terminal.

## Presets

A preset is a set of keys modeled on another file manager. It is the middle layer, so it changes only the commands it names, and you can change any key it sets. There are six. Each has a sheet of its keys, generated from the preset file, with the key first on every line.

- diroxide defaults: the keys most people know, the CUA keys, Explorer's and the Finder's, and the Norton row. This is [the keyboard page](keyboard.md).
- Total Commander: the Norton row, Shift+F6 to rename, Alt with the function keys for archives, Alt+F7 and Ctrl+F to find files, Ctrl+L for the size of folders, Shift+F2 to select the differences, and the keypad's plus, minus, star and slash for the selection. In the GUI, Alt with plus selects the same extension. Ctrl+Q stays the quit key in a terminal. [The sheet](keyboard-total-commander.md).
- Midnight Commander: the Norton row with its Alt keys for history and paths. Alt+Y and Alt+U are Back and Forward, Alt+C asks for a folder, Alt with a question mark finds a file, backslash unselects by pattern, and in the GUI F10 quits and Ctrl+Space sizes the folder. [The sheet](keyboard-midnight-commander.md).
- Explorer and Finder: Backspace goes back, Ctrl+E finds files, Alt+P turns on the preview, and on a Mac the Command keys do what the Finder does. Command+D is not carried, because on a Mac it is Say status. In this preset Control with 1 to 7 sets the favorite labels (see [the favorite labels](labels-bookmarks.md)). macOS may keep those keys for Mission Control. That will be checked when the GUI arrives on a Mac; until then the Labels submenu in the Edit menu (Alt+E, then B, then 1 to 7) is the sure way. [The sheet](keyboard-explorer-finder.md).
- Worker: the plus and minus keys select all and none, and star inverts (on the keypad in the GUI), Backspace goes back, the backquote and the caret show hidden files, Alt+N and Alt+T sort by name and by modified time, Ctrl+E opens a shell here, Ctrl+O shows this folder in the other panel, and Alt+Q quits. Alt+T sorts in a panel, so the Tools menu is F9, then T. [The sheet](keyboard-worker.md).
- vim-style, for ranger, lf, yazi and vifm: h, j, k and l move, g and a capital G go to the first and last row, a capital H and L go back and forward, y copies, x cuts, p pastes, d trashes, r renames, a makes a folder, e edits, i views, v inverts the selection, a capital V marks a range, s finds files, o opens the Sort list, slash filters, a colon opens the palette, t sets a label, m bookmarks this folder, and q quits. It runs letters as commands, so choosing it turns type-ahead off, and choosing another preset turns it back on. The Norton row and the other keys stay, so a mistake in the letters never takes your keys away. Ctrl+B pages up in the GUI only, because tmux keeps Ctrl+B for itself in a terminal. [The sheet](keyboard-vim-style.md).

Each preset keeps the keys that make diroxide usable by ear: Say status, Say other panel, the palette, help, the menus and quit. Where a source manager uses one of those keys for something else, such as Ctrl+D in Total Commander, the preset keeps diroxide's meaning.

A preset holds no key sequences. Midnight Commander's Ctrl+X family and Worker's Ctrl-x family are left out, and the vim-style doubled letters (gg, dd, yy, pp) become single letters (g, d, y, p). Each preset file lists the sequences it held back.

The presets are modeled on written sources, not on the programs themselves. A key may differ from the version you know. [Known limits](known-limits.md) says which sources.

### Choose a preset

Choose a preset, in the Keys list, the Tools menu's Keys submenu (Alt+T, then E, then P) and the palette, opens a list named Key presets. Each row has the preset's name, then its description, and the one in use says "current". Pick one, and diroxide asks, starting on No: "Apply Total Commander keys? No is the default. y or n". Then "Total Commander keys applied. Your 1 change still applies." Your own lines in `keymap.toml` stay on top. A preset that changes how letters work adds "Single letters: commands." Choosing the diroxide defaults again says "The diroxide default keys applied." Choosing the one in use says "Total Commander keys are in use already."

In a shell, `diro keys preset` lists the presets, and `diro keys preset NAME` applies one:

```bash
diro keys preset vim-style
```

The names are `diroxide`, `total-commander`, `midnight-commander`, `explorer-finder`, `worker` and `vim-style`.

### What every preset passes

Before a preset ships, tests check that every command has a key or is reachable from the palette; that no terminal key is one a terminal never sends; that no key is bound twice in one layer; that no screen reader key is bound; that Say status, Say other panel, the palette, help, the menus and quit have keys; that no key is a sequence; and that a preset in type-ahead mode binds no letter except plus, minus, star and slash. `diro keys check` with no file runs them: "Preset diroxide defaults: OK." and one line for each preset, then your `keymap.toml`.

## The keymap.toml file

You can still edit `keymap.toml` by hand. It is in the config folder ([Settings and where the files live](settings.md) says where). diroxide writes the built-in keys beside it as `keymap-defaults.toml`, a reference it rewrites at each start.

- A key is written in plain words joined by plus signs, in lowercase: `ctrl+f5`, `shift+f3`, `alt+home`, `alt+period`, `kp-plus` for the keypad's plus. The word `none` unbinds a command.
- The file has one line for each command and each place it works, in tables named for the column and the layer: `terminal.panel`, `terminal.global`, `terminal.list` and `terminal.dialog`, and the same for `gui`.
- A value is one key, or a list of keys.

```toml
[terminal.panel]
"file.copy" = "f5"
```

Command ids such as `file.copy` are in parentheses in [every key](keyboard.md).

## diro keys

None of these opens the program.

```bash
diro keys check
```

`diro keys check` with no file checks every preset, then your `keymap.toml`. With a file name after `check`, it reads that file and says every problem and every conflict in words, one a line, the problem first. It exits with 0 when everything is clean, 1 when it found problems, and 2 when it could not read the file.

```bash
diro keys print
```

`diro keys print` writes the keys in effect now, in both columns, as `keymap.toml` text.

```bash
diro keys preset
```

`diro keys preset` lists the presets, the one in use said first. `diro keys preset NAME` applies one.

```bash
diro keys test
```

`diro keys test` is the key test described above, and in [Terminals](terminals.md).

```bash
diro keys probe
```

`diro keys probe` asks for each key of the delivery table in turn: "Press Control plus V now, 3 of 95." It notes what arrived, the key itself, another key, or nothing within ten seconds, which counts as kept by the terminal. It ends with one line for each key, such as `ctrl+v = kept`. Send those lines back to the project, so that your terminal's column of the delivery table gets filled in. Most of the table still says "to probe".

## The palette remembers

The palette lists the command you used last first when the field is empty. After it come the commands you use most, each use counting less as it ages: a use is worth 0.8 of what it was a day before. Typing narrows it, with names that start with your text first. The uses are kept in the state folder.

## See also

- [Every key](keyboard.md)
- [Terminals](terminals.md)
- [Settings and where the files live](settings.md)
- [The panels](panels.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
