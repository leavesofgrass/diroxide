# Troubleshooting

This guide lists common problems, one heading each, with what to try. It ends with how to report a bug. Where the cause is a known limit, the entry links to it.

## A key does nothing

The terminal may have kept the key. Run the key test, press the key, and read what diroxide says:

```bash
diro keys test
```

If the terminal sent nothing, use the palette (Ctrl+P, then type the command's name) or the menus (F9). Windows Terminal keeps Ctrl+V, the Alt arrows and Alt+Enter; [Terminals](terminals.md) lists them with the sure way around each, and a snippet that gives the keys back.

If the key arrives but runs another command, check your own `keymap.toml`:

```bash
diro keys check
```

## Typing moves the cursor when I did not mean it

With single-letter keys on, typing a letter finds a row by name. That also happens when text is pasted into a panel, or when dictation software types. Turn it off in Settings (`single_letters`). Every command still has a key that is not a letter, a menu entry and a palette entry.

## Nothing is spoken, or it is spoken twice

- Check that your screen reader reads new text in the terminal. NVDA: Report dynamic content changes (NVDA+5) on. JAWS: Screen echo set to all (Insert+S). [Using diroxide with a screen reader](screen-readers.md) has the full list.
- Check that `screen_reader_mode` is on in Settings.
- For NVDA in Windows Terminal, try both choices of "Speak new text in Windows Terminal via".
- If your screen reader speaks the cursor move and diroxide's message, you hear it twice. The `cursor_fallback` setting parks the cursor on the status line.

## Tab or a refresh speaks a lot

A screen reader that reads new text speaks everything that changes on screen. The stacked layout changes less on Tab than side by side. Press Ctrl+P, type "layout" and press Enter. The `layout` setting keeps the choice.

## The Braille display does not follow the row

- In NVDA, set Tether braille to automatically or to focus. In JAWS, turn on Braille follows active cursor.
- The display follows the hardware cursor, which diroxide keeps on the first cell of the focused row. If it does not follow, report your screen reader, its version and the terminal.
- NVDA and JAWS messages can cover the line for a few seconds. Shorten the message timeout in your screen reader's Braille settings.

## Rows look shifted or characters are missing

A known bug in the library that paints the terminal can shift the rest of a row after an emoji or a Chinese, Japanese or Korean name in some terminals. Try the stacked layout, or `diro --inline`. If it happens, report the terminal's name and version. [Known limits](known-limits.md) has the details.

## There is no color, or the colors are wrong

- If `NO_COLOR` is set in your environment, diroxide uses bold, underline and reverse only. Unset it to get colors.
- Choose another theme in Settings, or start with `diro --theme NAME`. Each theme says whether it meets contrast in words when you choose it.
- Nothing in diroxide depends on color alone, so every state is also in words.

## The terminal is in a strange state after diroxide ended

diroxide restores the terminal when it quits, crashes or receives a signal. If it was killed in a way that gives it no chance, the terminal may be left in raw mode, with no echo. On Linux and macOS, type `reset` and press Enter. On Windows, close that tab and open a new one.

## The shell stays in the old folder after I quit

The folder hand-back needs the small function that `diro init` prints, and it must run in your shell's startup file. See [the quick start](quickstart.md). If you quit with "Quit and keep the shell where it was", the shell is meant to stay.

## "Folder not watched. Reload with Ctrl+R."

diroxide cannot watch a network drive, and it checks a removable drive only when you return to it and every two seconds. Press Ctrl+R to list the folder again. [Known limits](known-limits.md) has the details.

## "This drive has no trash" or "Path is too long for the trash"

On Windows the trash is used only on a fixed drive inside the computer, for a path of 259 characters or fewer. Anywhere else diroxide offers the listed permanent delete, starting on No. If you do not want that, copy the files somewhere with a trash first. [Operations and undo](operations.md) explains why.

## "Deleted but not found in the trash."

diroxide checked after a trash call and could not find the item in the trash. The item may be gone for good. Do not trust it to be recoverable. Please report what drive and what path length were involved.

## "Cannot start a program in a folder this deep"

On Windows a program cannot start in a folder whose path is 259 characters or longer. Go to a shallower folder, or move the files up. The same limit applies to opening a file: "Cannot open a file this deep".

## A copy did not finish

- The jobs line says what happened: "Copy had 8 failures: 290 done, 2 skipped". The entries that failed stay selected, so you can see what to try again.
- A failure names the file and the cause, and says what to do. "Could not copy report.pdf: the drive is full. Free some space, then retry."
- A stalled copy says "Copy stalled: no data for 20 seconds" and offers to keep waiting, to give up on the file, or to cancel the job.
- If diroxide was killed during a job, the next start says a job was interrupted. A leftover file whose name begins with `.diro-part-` is a partly written copy. Delete it yourself; diroxide never does.

## Undo says it could not undo something

Undo checks that an entry still has the size and time the job left, and that the old name is free. If you changed the entry since, or another file took its name, that step is skipped and the message says why. A permanent delete and a move across drives cannot be undone. [Operations and undo](operations.md) lists what can.

## diroxide asks before it quits

It asks only when a job is running or a prompt still holds text you typed: "Quit? 1 job is running. y or n". Answer y to quit anyway. Answer n and press Escape to cancel the job first.

## A settings or keymap file has a problem

diroxide never throws a file away. It reports the problem at start in words, uses the defaults for that file, and keeps the old one with `.bad` added to its name. Open the Problems list from the palette to see every problem. [Settings](settings.md) has the details.

## `diro --gui` says the window is not built

That is true in alpha.1. The window comes with alpha.6. Use the terminal for now.

## Starting clean

To start with factory settings, move the config folder aside by renaming it and start diroxide. [Settings](settings.md) says where the folder is. Your old files stay where you put them.

## Report a bug

When you report a problem, say:

- what you did, what you expected and what happened, in words;
- the output of `diro --version`;
- your system, the terminal's name and version, your screen reader's name and version, and whether a Braille display was in use;
- the lines from the log file that matter, with anything private removed. The log can hold the names of files involved in an error. [Privacy](privacy.md) says what the log holds.

If a key is involved, add what `diro keys test` said. If an accessibility problem is involved, say so first. It is a bug, not a preference.

## See also

- [Terminals](terminals.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Known limits](known-limits.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
