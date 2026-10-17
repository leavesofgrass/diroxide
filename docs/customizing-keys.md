# Customizing keys

This guide covers how to see every key diroxide has, change one, test what your terminal sends, and switch to a set of keys modeled on another file manager. It describes the Keys list, Change key, the key test, the presets, `diro keys`, and the `keymap.toml` file behind them. It comes with alpha.3. Before it, you changed keys by editing `keymap.toml`, which still works and is described below.

What this guide says comes from the design and from the tests that check each sentence. Nothing in alpha.3 had been heard on a screen reader when this guide was written, so the lines below are what diroxide is built to say. [The accessibility statement](accessibility.md) says what has been checked.

<!-- to confirm: this whole guide is written from the research plan (the keymap presets and the Keys list) while the keys lane (W3k) and the pre-wiring step (P3b) are still running. The final pass checks every sentence, key, preset name and file name against their notes and the regenerated keyboard page. -->

## How the keys are layered

The keys diroxide uses are built in three layers, one on top of the next:

1. The built-in keys, the diroxide defaults.
2. A preset, if you chose one. It changes only the commands it names.
3. Your own changes, in `keymap.toml`.

A key in a higher layer wins. Choosing a preset sets its name in `config.toml` (the `preset` setting, `diroxide` by default) and clears nothing of yours. Changing a key writes one line to `keymap.toml`. So a preset can be updated without losing your changes, and you can go back to the defaults without losing them either.

`diro keys print` writes the keys that are in effect now, from all three layers, as `keymap.toml` text.

Every key is one key with its modifiers. There are no key sequences and no leader keys, so a key never waits for a second one. Insert and Caps Lock are never bound, because they are the screen readers' keys.

There are two columns of keys, because a terminal cannot send everything a window can. The terminal column holds the keys a terminal can send. The window column, used when the window comes with alpha.6, also holds keys such as Ctrl+Enter. Most keys are the same in both, and the lists say "in both".

## The Keys list

The Keys list shows every command and the key it has. Open it from Settings (Alt+T, then S), in the Keys group, or from the palette. It has no key of its own.

It is a list like every other: type a letter to jump, Home and End, Shift+F1 for help, Ctrl+D for where you are, and the position said last. The title says the preset and how many keys you changed: "Keys, preset Total Commander, 3 changed".

Each row puts the command first, then its key or keys, then a state word:

- "Copy to other panel, F5 in both". A built-in key has no state word.
- "Rename, Shift+F6 in both, changed". The word "changed" means you changed it. The word "preset" means a preset did.
- "Paste, Ctrl+V in both, check terminal". The words "check terminal" mean the terminal key is one that many terminals keep or never send, so it may not work for you. You learn this before you press it.
- "Compare files, no key". A command can be reached from the palette and the menus without a key.

Typing narrows the list. A word matches command names. Text that reads as a key, such as `ctrl+f5`, matches the commands that have that key. The words "unbound", "changed" and "conflicts" show those rows.

Enter on a row opens its actions: Change key, Change the terminal key, Change the window key, Add a second key, Remove a key, Reset this key and Test keys. The list has its own commands too, in the palette: Choose a preset, Reset all keys (it asks, and the question starts on No), and Single letters.

### Single letters

The Single letters setting says what a key with no Control or Alt does in a panel. It has three values:

- Type to find (`type_ahead`, the default): a letter jumps to a name.
- Commands (`commands`): a letter runs a command. The vim-style preset needs this.
- Off: single keys do nothing. Use this with dictation software, which types letters.

Every command still has a key that is not a letter, a menu entry and a palette entry.

## Change a key, step by step

Choose a row, press Enter, and choose Change key.

1. Which column. In a terminal, only the terminal column can be changed by pressing a key. The window column is changed by typing the key's name, because a terminal cannot produce keys such as Ctrl+Enter that only the window sends. The action list says so: "The window column is changed by typing the key's name".
2. Press the new key. diroxide says "Press the new key now. Escape cancels." The next key press is taken before the keymap sees it. A release and a bare modifier are ignored. A pasted text is refused: "Pasted text is not a key". The first key press ends the capture.
3. If nothing arrives in 10 seconds, it says "No key heard. The terminal may keep it." and offers the typed name. The typed name, for example `ctrl+f5`, is an equal way in. It is read with the same code as `keymap.toml`. To bind Escape itself, type its name.
4. It says the key back, in words: "Got Control plus F5".
5. It checks the key, one sentence for each problem, in this order:
   - "Insert is the screen reader key." The key is refused.
   - "The terminal cannot send Ctrl+Enter." Then "Use it for the window column only? y or n".
   - "Windows Terminal keeps Ctrl+V." This is said for a key the terminal takes before diroxide, even though such a key can never arrive to be captured. You can still type its name.
   - "NVDA and JAWS use Ctrl+Alt+Left." Then the question.
   - "Ctrl+F5 already sorts by modified time." Then "Take it, swap, or cancel?" Taking leaves the other command without a key in that column. Swapping gives it this command's old key. Two commands cannot share a key in one layer.
   - "A letter key works only while letters run commands. Go on? y or n".
6. It writes one line to `keymap.toml`, safely, and says "Ctrl+F5 saved for Sort by modified". Your comments and the order of your lines stay.

Reset this key removes that line. Reset all keys removes your changes, after a question that starts on No. A file that cannot be read keeps its old copy as `keymap.toml.bad`, and diroxide says so in words. A command or a key in the file that diroxide does not know is kept and reported, never dropped.

A keymap moved to a computer with another keyboard layout may need a look, because symbols depend on the layout and `keymap.toml` stores the character.

<!-- to confirm: the Keys list's wording, the order of the checks, the action names and the timeout are written from the research plan. Confirm each against W3k's notes. -->

## The key test

Test keys says what the terminal sent and what it would do now: "Got Control plus F5. Runs Sort by modified." It stays open until you press Escape, or until ten seconds pass with no key. It answers "my key does nothing". Either the terminal never sent the key, or it sent a different one.

It is in the Keys list, in the Tools menu (Alt+T, then K), in the palette, and in a shell:

```bash
diro keys test
```

The test also tells you which kind of terminal you have: one that reports keys exactly, or one that merges some of them, such as Ctrl+I with Tab. While it runs, diroxide asks the terminal for the more exact key reports, and puts the terminal back as it was when the test ends. Nothing is ever moved off a key a terminal can send because of this.

## Presets

A preset is a set of keys modeled on another file manager. It is the middle layer, so it changes only the commands it names, and you can change any key it sets.

- diroxide: the built-in keys. The Norton row of function keys, Tab to switch panels, F9 for the menus.
- Total Commander: Alt+F7 and Ctrl+F for search, Shift+F6 to rename, Shift+F8 to delete for good, Alt+F5 and Alt+F6 for pack and unpack, Alt+F1 for the drives, Ctrl+L for the size of a folder, and the keypad's plus, minus, star and slash for the selection. Ctrl+Q stays the quit key in a terminal.
- Midnight Commander: its Alt chords. F10 quits in the window. Backslash unselects by pattern. Alt+Y and Alt+U are Back and Forward, and Alt+C asks for a folder.
- Windows Explorer and the Finder: Backspace goes back, Ctrl+E searches, Alt+P turns on the quick view, and on a Mac the Command keys do what the Finder does.
- Worker: the plus, minus and star keys select all, none and invert, Backspace goes back, a slash jumps to a name, and the backquote shows hidden files.
- vim-style, for ranger, lf, yazi and vifm: h, j, k and l move, a capital H and L go back and forward, y copies, x cuts, p pastes, d trashes, r renames, a makes a folder, o opens the Sort list, slash filters, and a colon opens the palette. It runs letters as commands, so it turns type-ahead off. The Norton row and the other keys stay, so a mistake in the letters never takes your keys away.

Each preset keeps the keys that make diroxide usable by ear: Say status, Say other panel, the palette, help, the menus and quit. Where a source manager uses one of those keys for something else, such as Ctrl+D in Total Commander, the preset keeps diroxide's meaning.

A preset holds no key sequences. Midnight Commander's Ctrl+X family and Worker's Ctrl-x family are left out, and the vim-style doubled letters (gg, dd, yy, pp) become single letters (g, d, y, p).

<!-- to confirm: the exact keys of each preset come from the research plan; the keys lane (W3k) writes the files for diroxide, Total Commander, Midnight Commander and vim-style, and the wiring lane (W3a, commit 41) the files for Explorer and the Finder and for Worker. If the last two slip out of alpha.3, remove their lines here and say so in the known limits. -->

### Choose a preset

Choose a preset, in the Keys list and the palette, lists them. Pick one, and diroxide asks: "Apply Total Commander keys? y or n". Your own lines in `keymap.toml` stay. Choose the diroxide preset to go back.

In a shell:

```bash
diro keys preset
```

<!-- to confirm: the exact form of the preset subcommand, and whether it lists, applies or both. -->

### Every preset has a sheet

`cargo xtask keyboard` writes one sheet of keys for each preset, with the key first on every line, as it does for the defaults in [every key](keyboard.md). The preset sheets are named `keyboard-NAME.md` in the docs folder.

<!-- to confirm: the preset sheet names and whether they are linked from here once they exist. -->

### What every preset passes

Before a preset ships, tests check that every command has a key or is reachable from the palette; that no terminal key is one a terminal never sends; that no key is bound twice in one layer; that no screen reader key is bound; that Say status, Say other panel, the palette, help, the menus and quit have keys; that no key is a sequence; and that a preset in type-ahead mode binds no letter except plus, minus, star and slash. A preset that fails does not ship.

## The keymap.toml file

You can still edit `keymap.toml` by hand. It is in the config folder ([Settings and where the files live](settings.md) says where). diroxide writes the built-in keys beside it as `keymap-defaults.toml`, a reference it rewrites at each start.

- A key is written in plain words joined by plus signs, in lowercase: `ctrl+f5`, `shift+f3`, `alt+home`, `alt+period`, `kp-plus` for the keypad's plus. The word `none` unbinds a command.
- The file has one line for each command and each place it works, in tables named for the column and the layer: `terminal.panel`, `terminal.global`, `terminal.list` and `terminal.dialog`, and the same for `window`.
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

`diro keys check` reads your `keymap.toml`, or the file you name after `check`, and says every problem and every conflict in words, one a line. The problem comes first. Each preset can be checked the same way. It exits with 0 when the file is clean, 1 when it found problems, and 2 when it could not read the file.

```bash
diro keys print
```

`diro keys print` writes the keys in effect now, in both columns, as `keymap.toml` text.

```bash
diro keys test
```

`diro keys test` is the key test described above, and in [Terminals](terminals.md).

## The palette remembers

The palette lists the commands you used last first when the field is empty. A command you used yesterday counts a little less each day, so the one you used a minute ago is on top. Typing narrows it as before.

<!-- to confirm: the recently-used ranking is commit 32 (W3k); the aging rate is the plan's 0.8 a day. Remove the section if it did not ship. -->

## See also

- [Every key](keyboard.md)
- [Terminals](terminals.md)
- [Settings and where the files live](settings.md)
- [The panels](panels.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
