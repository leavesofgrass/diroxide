# Quick start

This guide takes you from a built program to copying, renaming and deleting a file, in about ten minutes. It uses the terminal, because alpha.1 has no window yet. If you have not built diroxide yet, start with [installing diroxide](install.md).

## How keys are written

Keys are written the way diroxide writes them on its status line: Ctrl+F5, Alt+Home, Shift+Delete. Your screen reader reads Ctrl+F5 as "control plus F5", and diroxide's own messages say a key the same way. Keys that are not letters are spelled out: Up, Down, Left, Right, Tab, Enter, Escape, Space, Backspace.

Every command can also be found by name in the command palette, so if your terminal keeps a key for itself, nothing is lost. [Terminals](terminals.md) lists the keys that often do not arrive.

## Start diroxide

Run `diro` with no arguments to open the current folder in the left panel:

```bash
diro
```

The right panel opens where it was when you last quit. The first time, it opens on the same folder as the left panel, so nothing unexpected is on screen.

To open a folder in the left panel, give its path:

```bash
diro D:\work
```

To open both panels, give two paths. Put quotes around any path with spaces in it:

```bash
diro D:\work D:\backup
```

When diroxide starts, the status line names the version and the key for help, then names the layout and the command that changes it. The title line says where you are, the folder first: "Documents, left panel, in D:\work". Then comes the first row. You should hear the title and the first row, not the whole screen.

## Look around

- Up and Down move one row. Home and End go to the first and last row. Page Up and Page Down move a page.
- Right goes into the folder under the cursor. Left goes to the parent folder. On a file, Right says what kind of file it is and opens nothing.
- Enter opens the focused entry: a folder opens in the panel, and a file opens with your system's program for it. Backspace goes to the parent too.
- Each row says its name first, then its kind, size and modified time, and its position last: "report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40".
- Type a letter or a digit to find a row by name. The cursor jumps to the first row that starts with what you typed, and the status line says "3 of 12 matches".
- Ctrl+D says where you are: the folder, how many items, how many are selected, and the focused row with its position.

## Switch panels

Tab moves to the other panel and says it in one line: "Right panel: Backup, 12 items". The panel you are not in is the target of a copy or a move. Alt+O says the other panel's folder and its focused row without leaving yours.

## Mark rows

Space marks the focused row and moves down. The status line says "Selected report.pdf. 1 selected." and the row now says "selected". Ctrl+A marks every row. With nothing marked, a command acts on the focused row.

## Copy to the other panel

Mark a few files and press F5. diroxide asks, with the target first: "Copy 3 files to Backup? y or n". The answer starts on No. Press y to go ahead, or n or Escape to say no.

If a file with the same name is already there, a short list opens: "Name taken: report.pdf", how the two files differ, and then your choices, each starting with its verb. The focus is on "Skip this one", and Escape also skips, so a stray key cannot end a long copy. Down moves to "Keep both" or "Replace this one", and Enter chooses.

The jobs line tells you what is happening: "Copy started: 3 files to Backup", and at the end "Copy finished: 3 done". Escape cancels a job that is running. Ctrl+D says its progress whenever you ask.

## Rename

Press F2 on a file. diroxide says "Rename report.pdf to:" with the name in the field and the extension left alone. Type the new name and press Enter: "Renamed report.pdf to report-2026.pdf". If the new name could cause trouble, such as a name that ends in a space, diroxide says "Name needs care: ends with a space" and renames nothing.

## Make a folder

Press F7, type a name and press Enter. The cursor lands on the new folder: "Created folder Reports".

## Delete and undo

Press F8 or Delete. diroxide lists what it will move and asks: "Move report-2026.pdf to the trash? y or n". Press y. The file is gone from the list but waits in the trash.

Press Ctrl+Z to undo. "Undid trash: 1 item is back", and the file is in the list again. If there is nothing to undo, diroxide says "Nothing to undo".

Shift+Delete deletes for good. It lists every path first, spelling out hidden characters, and the question starts on No. On a drive without a trash, such as a USB stick, F8 offers the same listed delete: "This drive has no trash. Delete 3 items for good? y or n". [Operations and undo](operations.md) explains why.

## Sort

Ctrl+F3 sorts by name, Ctrl+F4 by extension, Ctrl+F5 by modified time, Ctrl+F6 by size and Ctrl+F7 not at all. The same key again reverses. The status line says the result first: "Sorted by size, largest first". Your cursor stays on the same entry.

## Find any command

- Ctrl+P opens the command palette. Type part of a command's name and press Enter. Each entry says its name first and its key after it, such as "Sort by size, Ctrl+F6". In a terminal that lets Ctrl+Shift+P through, that key works too.
- F9 opens the menus. Alt plus a menu's first letter opens it directly: File, Edit, View, Go, Select, Tools and Help. Every menu item shows its live key.
- Shift+F1 or the question mark opens help.

## Quit

Press Ctrl+Q. diroxide does not ask unless a job is running or a prompt still holds what you typed: "Quit? 1 job is running. y or n".

## Leave your shell in the folder you were in

Most file managers can change your shell's folder when you quit. diroxide does it through a small function that you add to your shell's startup file. The function is printed by diroxide itself, so it always matches the program. For bash:

```bash
diro init bash
```

`diro init` takes the name of your shell. It knows bash, zsh, dash, fish, PowerShell (7 and 5.1) and cmd. With the function in place, Ctrl+Q takes your shell to the active panel's folder. To quit and leave the shell where it was, use "Quit and keep the shell where it was" from the File menu or the palette; it has no key of its own.

## Where to go next

- [The panels](panels.md): layouts, moving, sorting, filtering and marking.
- [Operations and undo](operations.md): every operation, its question and how to reverse it.
- [Using diroxide with a screen reader](screen-readers.md): the settings to check in NVDA and JAWS, and what you should hear.
- [Settings](settings.md): where the files live and what to change.
- [Troubleshooting](troubleshooting.md), when something does not work.

## See also

- [Installing diroxide](install.md)
- [Every key](keyboard.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
