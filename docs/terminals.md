# Terminals

In alpha 1 diroxide runs in a terminal. This guide says which terminals to use, what a terminal can and cannot send, the keys Windows Terminal keeps for itself and how to get around them, and what to do when the full screen does not work for you.

## Which terminal

- Windows: Windows Terminal is the main target. It is the default terminal on Windows 11. The classic console, the window that cmd opens in older setups, is best effort. diroxide reads the keyboard the same way in both, as key records rather than escape sequences. Which one a screen reader reads better has not been tried yet.
- Linux: any terminal works. With Orca, use one built on VTE, such as GNOME Terminal, Console or Ptyxis. Terminals that draw with the graphics card, such as Alacritty and kitty, give Orca nothing to read. On the text console, BRLTTY, Speakup and Fenrir follow the hardware cursor.
- macOS: Terminal.app and iTerm2 are best effort.
- Over SSH or on a server: diroxide runs in the terminal, which is where it opens when there is no display.

diroxide sets the terminal's title to the active place, so a screen reader that speaks the window title could say where you are. Whether NVDA or JAWS speak it when it changes has not been verified.

## What diroxide asks of a terminal

- It switches to the alternate screen, and gives your screen back when it quits, when it meets an error and when it panics. It gives the terminal back when it is ended by a signal, by Ctrl+Break or by closing the window, and then exits with 130. [Troubleshooting](troubleshooting.md) says what to do if that leaves the terminal odd.
- It never captures the mouse.
- It honors `NO_COLOR`.
- It asks the terminal once whether it supports an enhanced keyboard protocol and writes the answer to `terminal.log` in the state folder. Every key works without it, so diroxide never relies on the answer. It turns the protocol on only while Change key in the Keys list waits for one key, so that key is reported exactly (Tab and Ctrl+I, Enter and Ctrl+M are told apart), and turns it off again as soon as the key came. On Windows keys arrive as key records, and diroxide does not ask.
- It ignores key releases, so one press is one action.

## Keys a terminal cannot send

Terminals were designed for text, so some keys never arrive, or arrive as another key. diroxide's terminal column avoids all of them, and the palette and the menus reach every command. [The keyboard page](keyboard.md) lists them with the reason for each.

- Some keys are other keys in disguise: Ctrl+I is Tab, Ctrl+M is Enter, Ctrl+H is often Backspace, Ctrl+J is a line feed and Ctrl+[ is Escape. diroxide never binds them.
- Shift+Enter, Ctrl+Enter, Ctrl+Tab and most Ctrl+Shift letters have no form a terminal can send. Ctrl+Space is not safe either. diroxide's terminal column does not use them. The graphical interface (GUI) will use some of them.
- F1, F10, F11 and F12 are often kept by the terminal for help, a menu or full screen. Help in the terminal is Shift+F1 or the question mark, and the menus are on F9.
- The keypad's plus, minus, star and slash arrive as the main row's characters in a Windows terminal. A command never depends on a keypad-only key.
- Alt arrives as an Escape prefix on Linux and macOS. On macOS Terminal, turn on "Use Option as Meta key" in the profile's keyboard settings. The function keys need Fn on a Mac or a laptop unless you change the system setting. The system owns Ctrl+F3 to Ctrl+F8 on a Mac, so the sort keys do not reach diroxide there; use the Sort submenu in the View menu or the palette. Mission Control keeps Control with the arrow keys, which diroxide uses for Folder to other panel and Next selected.
- On Windows, AltGr arrives as Control plus Alt. diroxide treats Control plus Alt plus a printable character as typed text, so you can type a character that your keyboard makes with AltGr, such as an at sign on a German keyboard.
- On Windows crossterm never delivers the Menu key to a terminal program. Shift+F10 opens the context menu instead.

## Windows Terminal's own keys

Windows Terminal keeps some keys for itself, so diroxide never sees them. These come from Microsoft's list of Windows Terminal actions and its default settings. The first four lines are the ones that affect diroxide.

- Ctrl+V, Ctrl+Shift+V and Shift+Insert paste text. diroxide's file paste is on Ctrl+V, so it does not arrive. The clipboard's text arrives as typed keys instead, because on Windows diroxide cannot tell a paste from typing: letters move the cursor to matching names, plus, minus and star open the pattern prompts, and a line break in the text acts as Enter. Sure way to paste files: Paste in the Edit menu, Alt+E and then P.
- Alt+Left and Alt+Right move between panes, and Alt+Up and Alt+Down do too. diroxide uses Alt+Left and Alt+Right for back and forward and Alt+Up for the parent folder. With only one pane open, Windows Terminal may pass them on; that has not been verified. Sure ways: Left, Right and Backspace move between folders, and the Go menu (Alt+G) has Back, Forward and Up one folder.
- Alt+Enter and F11 switch to full screen. diroxide's details are on Alt+Enter. Sure way: Details in the File menu (Alt+F, then D).
- Ctrl+C copies when text is selected in Windows Terminal. With no selection, diroxide gets it and puts the files on its clipboard. Sure way: Copy in the Edit menu.
- Ctrl+Shift+P opens Windows Terminal's own command palette, which is why diroxide's palette is on Ctrl+P in the terminal.
- Ctrl+Tab and Ctrl+Shift+Tab switch tabs, Ctrl+plus, Ctrl+minus and Ctrl+0 change the zoom, Alt+Shift+D and Alt+Shift+plus and minus split the window, and many Ctrl+Shift letters do other things. diroxide's terminal column uses none of them.
- Alt+F4 closes the window, and Alt+Space opens its window menu.

### Giving keys back to diroxide

You can tell Windows Terminal to stop keeping a key. Open its settings with Ctrl+comma, choose Open JSON file at the bottom of the list, and add lines like these to the `actions` list, with a comma between entries. Add only the keys you want:

```json
{ "command": "unbound", "keys": "ctrl+v" },
{ "command": "unbound", "keys": "alt+left" },
{ "command": "unbound", "keys": "alt+right" },
{ "command": "unbound", "keys": "alt+up" },
{ "command": "unbound", "keys": "alt+enter" }
```

The settings screen can also remove a key from an action under Actions. Which of the two ways works in your version has not been verified.

Unbinding Ctrl+V means Windows Terminal no longer pastes text on that key in any program, and in diroxide the key then pastes files. Ctrl+Shift+V, Shift+Insert and right-click still paste text. Unbinding Alt+Enter gives up Windows Terminal's full screen on that key, which F11 still gives.

### The menu is the sure way

If you would rather change nothing, the menus always work. F9 opens the menu bar, and Alt plus a menu's first letter opens it directly: File, Edit, View, Go, Select, Tools, Plugins and Help. The Search menu is F9, then R. Every item shows its live key. Paste is in Edit, back, forward and the parent folder are in Go, and the palette on Ctrl+P finds any command by name.

## The key test

When a key does nothing, ask the terminal what it sent. This command starts a small mode that waits for a key:

```bash
diro keys test
```

Press the key. diroxide says one line for it, with what the key would run first and what the terminal sent after: "Got Control plus F5. Runs Sort by modified. Sent F5 with Control, a press." or "Got Control plus J. Nothing runs on it." A key's release is said too, as "Released." followed by what was sent, and runs nothing. Insert is named as the screen reader key. Escape ends the test ("Escape: key test ended."), and so do ten seconds without a key ("No key for 10 seconds. Key test ended."). If nothing arrives at all, the terminal kept the key.

You can also start the test from inside diroxide, from the Tools menu (Alt+T, then K), the palette or the Keys list, as Test keys. It says the same, one line on the status line for each key, and ends with Escape or after ten seconds without a key.

The Keys list (Settings, then Keys list) marks a row "check terminal" when its key is one that many terminals keep or merge with another key, and on every Shift plus function key, because some terminals send those as F13 to F24. Change key checks a new key against the same table before it binds it. [Customizing keys](customizing-keys.md) describes both.

### The probe

To help fill in the table of what each terminal does with each key, run the guided form:

```bash
diro keys probe
```

It asks for each key in turn: "Press Control plus V now, 3 of 95." It notes what arrived, the key itself, another key, or nothing within ten seconds, which counts as kept by the terminal. It ends with one line for each key, such as `ctrl+v = kept`. Send those lines back with the name and version of your terminal. Nothing is written to a file for you.

### Key presets from a shell

`diro keys preset` lists the key presets, and `diro keys preset NAME` applies one. `diro keys check` with no file checks every preset and your `keymap.toml`. [Customizing keys](customizing-keys.md) has the details.

## tmux

tmux keeps Ctrl+B as its prefix key, so inside tmux the Bookmarks key arrives only when pressed twice. Alt+B opens the same list, and so does the Go menu (Alt+G, then K). The vim-style key preset pages up on Ctrl+B in the GUI only, for the same reason.

## The classic console

The classic console works and is best effort. Key input is the same as in Windows Terminal. The terminal drawing library had a bug with wide characters that was first seen in the classic console, so a name with an emoji may draw the rest of a row out of place; see [known limits](known-limits.md). Screen readers read it through the console support in their advanced settings. For NVDA that is "Windows Console support".

## Linux terminals

GNOME Terminal keeps F1 for help, F10 for its menu and F11 for full screen, and Alt plus a digit and Ctrl plus Page Up and Page Down switch its tabs. The keyboard page lists these as keys never used in a terminal. diroxide's terminal column uses none of them.

## macOS

Terminal.app keeps the function keys as system keys unless you hold Fn. Turn on "Use Option as Meta key" so Alt chords work. Control plus the arrow keys belong to Mission Control. If VoiceOver reads the full screen poorly, try `diro --inline` or `diro ls`.

## When the full screen does not work

Three fallbacks, from least to most plain.

`diro --inline` draws diroxide below the prompt, in your terminal's normal screen, instead of switching to the alternate screen. It uses at most 24 rows:

```bash
diro --inline
```

`diro ls` prints a folder as plain lines, one entry per line, with the name first and then the kind, size and date in words. It lists the current folder when you give no place. `--long` adds the exact size, times, attributes and owner, `--json` writes one JSON object a line, and `--hidden` includes hidden entries. Everything it says goes to standard output in order, its problems included, so a screen reader hears the listing and its problems as one story. It works in any terminal, over any connection, and in a script:

```bash
diro ls work
```

`diro ls -` reads paths from standard input instead of a folder and lists them the same way. The paths are separated by NUL characters, which are safe for any name; add `--lines` if yours are separated by newlines. An empty list does nothing and says so. `diro ls --help` gives the options.

```bash
diro ls -
```

`diro search` prints the places under a folder that match a filter, one a line, then a summary. The filter is the same language as the panel's filter: names, kinds, sizes, dates and file contents. [Search and the filter language](search.md) has it, with the options.

```bash
diro search work size over 1 MB and ext pdf
```

When its output is not a terminal, `diro` itself prints one screen of plain lines instead of opening the full screen.

The `diro` subcommands exit with 0 when they are done, 1 when they failed or found nothing, 2 when the command line was wrong, and 3 when they did part of the job, such as a list in which some items failed. `diro --help` says so. A script can tell "nothing changed" from "some changed".

## Colors and wide characters

`NO_COLOR` turns colors off and leaves attributes. `diro --theme NAME` picks a theme for one session. Names with emoji or Chinese, Japanese or Korean characters are drawn with their real width: each row is one run of text, and after a frame a changed row that holds a wide character is sent again from that character to the end of the row. If a row still looks shifted, tell us the terminal's name and version.

## Leaving your shell in the folder you were in

`diro init` prints a small function named `dcd` for your shell. `dcd` starts `diro` with the terminal frontend and a file name through which diroxide hands the folder back; Ctrl+Q then takes the shell to the active panel's folder. [The quick start](quickstart.md) has the command.

## See also

- [Using diroxide with a screen reader](screen-readers.md)
- [Customizing keys](customizing-keys.md)
- [Search and the filter language](search.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
