# Quick start

This guide takes you from a built program to copying, renaming and deleting a file, in about ten minutes. It uses the terminal, because alpha 1 has no GUI yet. If you have not built diroxide yet, start with [installing diroxide](install.md).

## How keys are written

Keys are written the way diroxide writes them on its status line: Ctrl+F5, Alt+Home, Shift+Delete. Your screen reader reads Ctrl+F5 as "control plus F5", and diroxide's own messages say a key the same way. Keys that are not letters are spelled out: Up, Down, Left, Right, Tab, Enter, Escape, Space, Backspace.

Every command can also be found by name in the command palette and in the menus, so if your terminal keeps a key for itself, nothing is lost. [Terminals](terminals.md) lists the keys that often do not arrive.

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

On the very first start, the status line says "diroxide alpha 1. Shift+F1 for help. Ctrl+Q quits.", then "The key bar on the last line names the keys.", and then "Layout: side by side. Ctrl+P, Layout." The title line says where you are, the folder first: "Documents, left panel, in D:\work". Then comes the first row. You should hear the title and the first row, not the whole screen.

If your build can offer optional components besides the example plugins, a list named "Optional components to get now" opens once at that first start. Nothing is chosen in it. Space marks a row, Enter asks once and starts the downloads you marked, and Escape skips the list: "Nothing downloaded. Tools, Manage optional components has them." A build made from source has nothing to offer, so you may never see it. [Optional components](components.md) describes the list and how to bring it back.

## Look around

- Up and Down move one row. Home and End go to the first and last row. Page Up and Page Down move a page.
- Right goes into the folder under the cursor. Left goes to the parent folder. On a file, Right says what kind of file it is and opens nothing.
- Enter opens the focused entry: a folder opens in the panel, and a file opens with your system's program for it. Backspace goes to the parent too.
- Each row says its name first, then its kind, size and modified time, and its position last: "report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40".
- Type a letter or a digit to find a row by name. The cursor jumps to the first row that starts with what you typed, and the status line says "report.pdf, 1 of 3 matches".
- Ctrl+D says where you are: the folder, how many items, how many are selected, and the focused row with its position.

## Switch panels

Tab moves to the other panel and says it in one line: "Right panel: Backup, 12 items". The panel you are not in is the target of a copy or a move. Alt+O says the other panel's folder and its focused row without leaving yours.

## Mark rows

Space marks the focused row and moves down. The status line says "Selected report.pdf. 1 selected." and the row now says "selected". Ctrl+A marks every row. With nothing marked, a command acts on the focused row.

## Copy to the other panel

Mark a few files and press F5. diroxide asks, with the target first: "Copy 3 items to Backup, 312 bytes? No is the default. y or n". Press y to go ahead, or n or Escape to say no.

If a file with the same name is already there, a short list opens: "Name taken: report.pdf", how the two files differ, the facts about each, and then your choices, each starting with its verb. The focus is on "Skip this one", and Escape also skips, so a stray key cannot end a long copy. Down moves to "Keep both" or "Replace this one", and Enter chooses.

The jobs line tells you what is happening: "Copy started: 3 items to Backup", and at the end "Copy finished: 3 done." Escape cancels a job that is running. Ctrl+D says its progress whenever you ask.

## Rename

Press F2 on a file. diroxide says "Rename report.pdf to" with the name in the field and the cursor before the extension. Type the new name and press Enter: "Renamed report.pdf to report-2026.pdf". If the new name could cause trouble, such as a name that ends in a space, diroxide says "Name needs care: ends in a dot or space. Nothing renamed." and asks again.

## Make a folder

Press F7, type a name and press Enter. The cursor lands on the new folder: "Created folder Reports".

## Delete and undo

Press F8 or Delete. diroxide lists what it will move and asks: "Move 1 item to the trash, 2.1 KB? No is the default. y or n". Press y. The file is gone from the list but waits in the trash.

Press Ctrl+Z to undo. "Undid trash: 1 item is back", and the file is in the list again. If there is nothing to undo, diroxide says "Nothing to undo".

Shift+Delete deletes for good. It lists every path first, spelling out hidden characters, and the question starts on No. On a drive without a trash, such as a USB stick, F8 offers the same listed delete: "This drive has no trash. Delete 3 items for good? No is the default. y or n". [Operations and undo](operations.md) explains why.

## Sort

Ctrl+F3 sorts by name, Ctrl+F4 by extension, Ctrl+F5 by modified time, Ctrl+F6 by size and Ctrl+F7 not at all. The same key again reverses. The status line says the result first: "Sorted by size, largest first". Your cursor stays on the same entry.

## Filter and search

Press Ctrl+S and type what you want to see. `report` shows names that hold report. `*.pdf` shows PDFs. `size over 1 MB` shows big files, and `modified last week` shows what changed then. You hear nothing while you type. When you stop, one line says the count and the first match: "37 matches, first report.pdf". Enter keeps the filter: "Filter on, 4 of 9 shown". Escape in the panel takes it off: "Filter off, 9 shown".

To look inside folders below this one, press Ctrl+F. Find files opens a list of settings named Search, with the focus on its Name or filter row. Type a letter to open that field, give it the same words, and press Enter: "Searching work for *.txt". Results arrive as a list of real files, and Escape stops it: "Stopped at 214 found." [Search and the filter language](search.md) has the whole language and `diro search` for the shell.

## Label and bookmark

Alt+L puts a label on the marked rows, or the focused row: type `urgent` and press Enter. diroxide says "Label urgent added to 3 items", and Ctrl+Z undoes it. Ctrl+B opens the bookmarks. Its first row, "Add this folder", bookmarks the folder you are in. From then on Ctrl+B and then Enter on its row take you back, or its alias letter if you gave it one. [Labels and bookmarks](labels-bookmarks.md) has the rest.

## Compare two files

Mark two files and run Compare files from the palette (Ctrl+P, then "compare"). diroxide says the answer first, "5 changes between old.txt and report.txt", then the counts, then one row for each change. [Compare files](compare.md) explains it, and `diro diff` does the same in a shell.

## Open an archive

Move to a zip, a 7z, a tar or another archive and press Enter. It opens like a folder, and the status line says "Opened course.zip: 10 items, 1.8 KB." Move around as you do in any folder. Left, at the top of the archive, goes back to the folder that holds it. Mark members and press F5 to unpack them into the other panel. Nothing in an archive can be changed: "Archives are read only here. Use Pack to make a new one." To make a new archive from marked files, run Pack from the palette, or from the File menu under Archive. [Archives](archives.md) has the rest.

## Read a file

Press F3 on a file to read it in the viewer. Up and Down move a line, Ctrl+D says where you are, slash searches, and Escape closes the viewer with your cursor back on the file's row. Press Shift+F3 to turn the other panel into a live preview of the file under your cursor ("Quick view on"), and Shift+F3 again to turn it off. Tab goes into the preview and Tab or Escape comes back. [The viewer](viewer.md) and [the quick view](quick-view.md) have the rest.

## Find any command

- Ctrl+P opens the command palette. Type part of a command's name and press Enter. Each entry says its name first and its key after it.
- F9 opens the menu bar. Alt plus a menu's first letter opens it directly: File, Edit, View, Go, Select, Tools, Plugins and Help. The Search menu has no Alt letter, because Alt+S is Select; open it with F9, then R. Every menu item shows its live key. The Go menu has Volumes (Alt+G, then V) and Trash (Alt+G, then C). The Plugins menu has Manage plugins and Get example plugins, then the commands of each plugin you turned on.
- Shift+F1 or the question mark opens help: a list of commands with their keys.

## Quit

Press Ctrl+Q. diroxide does not ask unless a job is running or a prompt still holds what you typed: "Quit? 1 job is running. No is the default. y or n". If you quit with a job running, the job is cancelled.

## Leave your shell in the folder you were in

Most file managers can change your shell's folder when you quit. diroxide does it through a small function that you add to your shell's startup file. The function is printed by diroxide itself, so it always matches the program. For bash:

```bash
diro init bash
```

`diro init` takes the name of your shell: bash, zsh, dash (or sh), fish, powershell (or pwsh) or cmd. It prints a function named `dcd`. Start diroxide with `dcd` instead of `diro`, and Ctrl+Q takes your shell to the active panel's folder. To quit and leave the shell where it was, use "Quit, shell stays where it was" from the File menu or the palette; it has no key of its own.

## Where to go next

- [The panels](panels.md): layouts, moving, sorting, filtering and marking.
- [Operations and undo](operations.md): every operation, its question and how to reverse it.
- [Volumes and the trash](volumes-trash.md): your drives, ejecting a stick, and getting things back from the trash.
- [Archives](archives.md), [the viewer](viewer.md) and [the quick view](quick-view.md): opening, packing, reading and previewing.
- [Search and the filter language](search.md), [labels and bookmarks](labels-bookmarks.md) and [compare files](compare.md): finding, remembering and comparing.
- [Customizing keys](customizing-keys.md): changing a key, testing what your terminal sends, and key presets.
- [Watching a folder](watching.md): hearing when a new file has finished arriving.
- [Split and join](split-join.md): big files in parts.
- [Using diroxide with a screen reader](screen-readers.md): the settings to check in NVDA and JAWS, and what you should hear.
- [Settings](settings.md): where the files live and what to change.
- [Troubleshooting](troubleshooting.md), when something does not work.

## See also

- [Installing diroxide](install.md)
- [Every key](keyboard.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
