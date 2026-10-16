# Terminals

In alpha.1 diroxide runs in a terminal. This guide says which terminals to use, what a terminal can and cannot send, the keys Windows Terminal keeps for itself and how to get around them, and what to do when the full screen does not work for you.

## Which terminal

- Windows: Windows Terminal is the main target. It is the default terminal on Windows 11. The classic console, the window that cmd opens in older setups, is best effort. diroxide reads the keyboard the same way in both, as key records rather than escape sequences. Which one a screen reader reads better has not been tried yet.
- Linux: any terminal works. With Orca, use one built on VTE, such as GNOME Terminal, Console or Ptyxis. Terminals that draw with the graphics card, such as Alacritty and kitty, give Orca nothing to read. On the text console, BRLTTY, Speakup and Fenrir follow the hardware cursor.
- macOS: Terminal.app and iTerm2 are best effort.
- Over SSH or on a server: diroxide runs in the terminal, which is where it opens when there is no display.

diroxide sets the terminal's title to the active place, so a screen reader that speaks the window title says where you are.

## What diroxide asks of a terminal

- It switches to the alternate screen and restores your screen when it exits, when it crashes and when it receives a signal.
- It never captures the mouse.
- It honors `NO_COLOR`.
- It asks the terminal once whether it supports an enhanced keyboard protocol and writes the answer to the log. It never relies on the answer, so every key works without it.
- It ignores key releases, so one press is one action.

## Keys a terminal cannot send

Terminals were designed for text, so some keys never arrive, or arrive as another key. diroxide's terminal column avoids all of them, and the palette and the menus reach every command.

- Some keys are other keys in disguise: Ctrl+I is Tab, Ctrl+M is Enter, Ctrl+H is often Backspace, Ctrl+J is a line feed and Ctrl+[ is Escape. diroxide never binds them.
- Shift+Enter, Ctrl+Enter, Ctrl+Tab and most Ctrl+Shift letters have no form a terminal can send. Ctrl+Space is not safe either. diroxide's terminal column does not use them. The window will use some of them.
- F1, F10, F11 and F12 are often kept by the terminal for help, a menu or full screen. Help in the terminal is Shift+F1 or the question mark, and the menus are on F9.
- The keypad's plus, minus, star and slash arrive as the main row's characters in a Windows terminal. A command never depends on a keypad-only key.
- Alt arrives as an Escape prefix on Linux and macOS. On macOS Terminal, turn on "Use Option as Meta key" in the profile's keyboard settings. The function keys need Fn on a Mac or a laptop unless you change the system setting. The system owns Ctrl+F3 to Ctrl+F8 on a Mac, so the sort keys do not reach diroxide there; use the Sort list, the menu or the palette.
- On Windows, AltGr arrives as Control plus Alt. diroxide treats Control plus Alt plus a printable character as typed text, so you can type a character that your keyboard makes with AltGr, such as an at sign on a German keyboard.

## Windows Terminal's own keys

Windows Terminal keeps some keys for itself, so diroxide never sees them. These come from Microsoft's list of Windows Terminal actions and its default settings. The first four lines are the ones that affect diroxide.

- Ctrl+V, Ctrl+Shift+V and Shift+Insert paste text. diroxide's file paste is on Ctrl+V, so it does not arrive. The clipboard's text arrives as typed letters instead, and with single-letter keys on, those letters move the cursor. Sure way: Paste in the Edit menu, Alt+E and then P.
- Alt+Left and Alt+Right move between panes, and Alt+Up and Alt+Down do too. diroxide uses Alt+Left and Alt+Right for back and forward and Alt+Up for the parent folder. With only one pane open, Windows Terminal may pass them on; that has not been verified. Sure ways: Left, Right and Backspace move between folders, and the Go menu (Alt+G) has back, forward and the parent.
- Alt+Enter and F11 switch to full screen. diroxide's details are on Alt+Enter. Sure way: the context menu on Shift+F10, or the palette.
- Ctrl+C copies when text is selected in Windows Terminal. With no selection, diroxide gets it and copies files.
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

Unbinding Ctrl+V means Windows Terminal no longer pastes text on that key in any program. Ctrl+Shift+V, Shift+Insert and right-click still paste. Unbinding Alt+Enter gives up Windows Terminal's full screen on that key, which F11 still gives.

### The menu is the sure way

If you would rather change nothing, the menus always work. F9 opens the menus, and Alt plus a menu's first letter opens it directly: File, Edit, View, Go, Select, Tools and Help. Every item shows its live key. Paste is in Edit, back, forward and parent are in Go, and the palette on Ctrl+P finds any command by name.

## The key test

When a key does nothing, ask the terminal what it sent. This command starts a small mode that waits for a key:

```bash
diro keys test
```

Press the key. diroxide says in words what the terminal sent, which modifiers came with it, and what command the key would run. If nothing arrives at all, the terminal kept the key. It tells you how to leave the test.

## The classic console

The classic console works and is best effort. Key input is the same as in Windows Terminal. The terminal drawing library had a bug with wide characters that was first seen in the classic console, so a name with an emoji may draw the rest of a row out of place; see [known limits](known-limits.md). Screen readers read it through the console support in their advanced settings. For NVDA that is "Windows Console support".

## Linux terminals

GNOME Terminal keeps F1 for help, F10 for its menu and F11 for full screen, and Alt plus a digit switches its tabs. This comes from its default settings and has not been checked here. diroxide's terminal column uses none of those keys.

## macOS

Terminal.app keeps the function keys as system keys unless you hold Fn. Turn on "Use Option as Meta key" so Alt chords work. Control plus the arrow keys are reported to belong to Mission Control. Neither has been checked here. If VoiceOver reads the full screen poorly, try `diro --inline` or `diro ls`.

## When the full screen does not work

Three fallbacks, from least to most plain.

`diro --inline` draws diroxide in your terminal's normal screen, in place, instead of switching to the alternate screen:

```bash
diro --inline
```

`diro ls` prints a folder as plain lines, one entry per line, with the name first and then the kind, size and date in words. `--long` adds more, `--json` writes JSON, and `--hidden` includes hidden entries. It works in any terminal, over any connection, and in a script:

```bash
diro ls work
```

`diro ls -` reads paths from standard input instead of a folder and lists them the same way. The paths are separated by NUL characters, which are safe for any name; add `--lines` if yours are separated by newlines. An empty list does nothing.

```bash
diro ls -
```

The `diro` subcommands exit with 0 when they succeed, 1 when they fail or find problems, 2 when the command line was wrong, and 3 when they did part of the job, such as a list in which some items failed. A script can tell "nothing changed" from "some changed".

## Colors and wide characters

`NO_COLOR` turns colors off and leaves attributes. `diro --theme NAME` picks a theme for one session. Names with emoji or Chinese, Japanese or Korean characters are drawn with their real width where the terminal allows it. If a row looks shifted after such a name, tell us the terminal's name and version.

## Leaving your shell in the folder you were in

`diro init` prints a small function for your shell that makes quitting with Ctrl+Q take the shell to the active panel's folder. The function starts `diro` with the terminal frontend and a file name through which diroxide hands the folder back. [The quick start](quickstart.md) has the command.

## See also

- [Using diroxide with a screen reader](screen-readers.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
