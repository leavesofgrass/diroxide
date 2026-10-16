# The panels

This guide covers the two panels: how they are laid out, what a row says, and how to move, sort, filter, jump and mark. Copying, renaming, deleting and undo are in [operations and undo](operations.md).

## The two panels and the three layouts

diroxide shows two panels. One is active, and the other is the target of a copy or a move. Tab switches between them. A setting called layout decides how they are drawn.

- Side by side is the default: the panels sit next to each other, as in every orthodox file manager since Norton Commander.
- Stacked puts one panel above the other. Each screen line is one row at the full width of the terminal. It is the fallback for screen reader users and for narrow terminals.
- Single shows only the active panel. Tab then switches which panel is shown and says its place.

Which layout should a screen reader user choose? Try stacked first. In the side by side layout one screen line holds a row of each panel. A screen reader reads the whole line, and a 40-cell Braille display shows its first 40 cells, so the two panels mix. When the right panel is active, its focused row starts near column 41, after the left panel's row. In the stacked layout every line is one row, and Tab changes only the cursor, a marker and a title. This advice comes from how screen readers read a line. It has not yet been confirmed by listening on this project, and this guide will change if the listening checks say otherwise. [Using diroxide with a screen reader](screen-readers.md) has more.

To change the layout, press Ctrl+P, type "layout" and press Enter. Each choice cycles to the next layout and says it: "Layout: stacked". The `layout` setting makes the choice permanent, and diroxide remembers it between sessions.

In screen-reader mode the other panel is redrawn only when you ask, whatever the layout, so a change in the panel you are not using does not interrupt you.

## What is on the screen

- The title line comes first and puts the place first: the folder's own name, then which panel, then the parent: "Documents, left panel, in D:\work". A filter or a sort that is not the default comes at the end: "Documents, left panel, filter *.pdf" and ", sorted by size". A very long parent path is cut from the left with three dots. The whole path is the starting text of Go to a path.
- The rows of the panel follow. There is no ".." row; Left, Backspace and Alt+Up go to the parent. At the top of a drive, diroxide says "At the top already".
- The status line gives the latest message with its key fact first. Between messages, Ctrl+D tells you where you are.
- The jobs line says what jobs are doing, or "No jobs".
- The key bar is an optional line that names the function keys in words, such as "F5 copy, F6 move, F7 make folder". It is off by default in screen-reader mode, it is outside the order you move through, and the Say key bar command reads it. The compact setting hides it, with the title line.
- Prompts and their candidate lists appear at the bottom of the screen, in the minibuffer.

The hardware cursor always sits on the focused row's first cell, so a screen reader and a Braille display follow it. In a prompt it sits where the next character will go. In an empty folder it sits on the line that says "Empty folder".

## What a row says

A row puts its meaning first: the name, then the kind, the size and the modified time. "report.pdf, PDF, 2.1 MB, modified Tuesday". A folder says "folder" after its name. The position comes last when diroxide speaks the row: "3 of 40".

- A selected row begins with an asterisk and also says "selected" in words: "* report.pdf, selected, PDF, 2.1 MB". The asterisk is never the only signal.
- A name that could cause trouble says so: "name needs care, ends with a space". That covers reserved characters, trailing dots and spaces, Windows device names, invisible characters, and names that differ from a sibling only by case.
- Control, private-use, direction and other invisible characters in a name are shown as a question mark. The real name is kept for operations. Any listing before a delete spells such characters out as `U+XXXX`.
- A cloud placeholder says "online only". diroxide asks before it reads such a file's contents.
- An empty cell says a word such as "not set". It is never a blank or a lone dash.
- Hidden entries are not listed until you show them. A hidden entry says "hidden" in its details.

By default the columns are type, size and modified time. The palette has entries for choosing columns for each panel: extension, created and accessed times, permissions, mode, octal, owner, group, link target and others.

Permissions are read in words first. On Linux and macOS a column says who can do what: "read: everyone; write: owner". The full sentence in the details says "Owner can read and write, group can read, others can read." A folder says "list and enter: everyone; change: owner". The octal number is an extra column, never the first thing said. On Windows the column uses words such as read-only, hidden, system, compressed, encrypted and online only. You can see permissions in alpha.1, but you cannot change them until alpha.2.

## Moving around

- Up and Down move one row. Home and End go to the first and last row. Page Up and Page Down move a page.
- Left goes to the parent folder. Right goes into the folder under the cursor; on a file it says the kind of file and opens nothing.
- Enter opens the focused entry. Backspace and Alt+Up go to the parent as well.
- Alt+Left and Alt+Right go back and forward through the folders you visited. Where a terminal keeps those keys, use the Go menu (Alt+G).
- Alt+Home goes to your home folder.
- Ctrl+R lists the folder again: "Reloaded, 124 entries."

When you go to the parent with Left or Backspace, the cursor lands on the folder you just left. Back and forward return you to the row you were on, with the marks you had, folder by folder.

Enter on a folder goes in. Enter on a file hands it to your system's program for that kind of file: "Opening report.pdf." Enter on a program asks first, and the question starts on No: "Run setup.exe? y or n". F4 hands the file to your editor instead, with the screen suspended; when you come back diroxide says "Back in diroxide, Documents." The editor is the `editor` setting, or else the VISUAL or EDITOR variable, or else the system's text editor.

## Go to a path

Ctrl+L or Alt+D opens the path field with the current path in it. Type or paste a path and press Enter.

- Tab completes the path, and Up and Down recall the paths you typed before. [Operations and undo](operations.md) describes completion.
- Quoted paths work, such as a path copied from Explorer, and either kind of slash.
- A `~` or a variable such as `%TEMP%` is offered as a candidate that completes to the literal full path. Nothing is expanded when you press Enter.
- A path such as `D:` with no slash is refused as ambiguous. Type `D:\` for the root of the drive.
- If the folder does not exist, diroxide says why and stays where it was.

## Go to a place

Ctrl+G opens a list of places: the drives on Windows, the root and the mounts on Linux and macOS, your home folder and the temporary folder. A drive says its type in words and its free space: "C drive, disk, 120 GB free". The types are "disk", "removable drive" and "network drive". diroxide does not wake an idle drive to read it.

## Path jump

diroxide remembers the folders you visit, with how often and when. Path jump takes a few words and lists the folders whose names contain them, best first. Type the words in the order they appear in the path. The last word has to match the last folder. Your home folder is left out. Each match says its rank in words, with the folder's own name first: "Documents, good match, used 12 times, yesterday". Path jump is in the Go menu and the palette.

## The other panel

- Ctrl+Right shows the focused folder in the other panel: "Other panel now shows Documents." Ctrl+Left shows this panel's folder in the other panel.
- Ctrl+U swaps the two panels: "Panels swapped. This panel: Downloads. Other panel: Documents."
- Alt+O says the other panel's folder and its focused row without switching to it.
- `diro LEFT RIGHT` opens both panels on two folders from the start.

## Sorting

Sorting is quick and has many keys, because sorting is how you find things and decide what to do next.

- Ctrl+F3 sorts by name, Ctrl+F4 by extension, Ctrl+F5 by modified time, Ctrl+F6 by size and Ctrl+F7 not at all (the provider's own order). These are the Norton Commander keys that Total Commander and Double Commander also use.
- Choosing the same key again reverses the direction. Each key has a sensible first direction: names A to Z, sizes largest first, dates newest first, kinds A to Z.
- The Sort list has every key and marks the current one, saying its direction: "Name, A to Z, current". Type a letter to jump, press Enter to apply, and choose the same key again to reverse. In the terminal you reach it from the View menu (Alt+V, then S), because a terminal cannot reliably send Ctrl+Shift+S, which opens it in the window. The palette has a "Sort by" entry for every key, such as "Sort by size, Ctrl+F6".
- The sort keys also include the created and accessed times, kind, permissions, owner and label. A key with no data says so: "Label, none yet" or "Owner, not available here".
- Names sort in natural order, so `file2` comes before `file10`, and capitals do not matter. Folders come first, as a setting (first, mixed or last). Whatever the first key is, name is always the last, so two files that tie come in name order and the cursor never jumps.
- The status line says the result first and never the position: "Sorted by size, largest first", then "Sorted by size, smallest first" when you press it again. The cursor stays on the same entry.
- The title line says the sort when it is not the default, so a panel you reach with Tab tells you.

On macOS the system owns Ctrl+F3 to Ctrl+F8. Use the Sort list, the menu or the palette there.

## Finding and filtering

There are two ways to narrow down a long folder.

- Type to find. With single-letter keys on, typing a letter or a digit finds a row by name. The cursor jumps to the first row whose name starts with what you typed, and the status line says "3 of 12 matches". Typing more narrows the match. The `single_letters` setting chooses what typing does: find by name, run commands such as plus and minus, or nothing. Turn it off if you use dictation software, which types letters.
- Ctrl+S opens the name filter, whether single-letter keys are on or off. It hides every row that does not match. The title line says the filter in words: "filter: not *.o".

A filter is one pattern or several. The first version understands these, and a later alpha extends them without breaking them:

- A plain word matches names that contain it: `report`.
- A pattern with `*` or `?` is a glob: `*.pdf`. Separate several with commas to match any of them: `*.pdf, *.docx`.
- A `re:` prefix makes a regular expression: `re:^2026-`.
- `not` in front turns a pattern around: `not *.o`.
- A slash at the end limits it to folders: `photos/`.

Case does not matter unless you change the `filter_case` setting. A pattern that cannot be read is explained in words in one short line. To clear a filter, open it again and delete the text.

Showing and hiding hidden entries is in the View menu and the palette: "Showing hidden files, 52 entries". Its key is in [the keyboard reference](keyboard.md). It is not Ctrl+H, because terminals confuse that key with Backspace. Hidden means a name that starts with a dot on Linux and macOS, the hidden flag on macOS, and the hidden attribute on Windows. A name such as `.gitignore` is not hidden on Windows unless you ask for that.

## Marking rows

Marking picks the rows a command will act on. With no marks, a command acts on the focused row.

- Space marks the focused row and moves down: "Selected report.pdf. 1 selected." Space on a marked row unmarks it: "Not selected report.pdf. 0 selected."
- Ctrl+A marks every row. Ctrl+Shift+A clears the marks where the terminal can send it; the Select menu (Alt+S) always can.
- With single-letter keys on, plus asks for a pattern to select, minus asks for a pattern to unselect, and star inverts the marks. The pattern prompt starts with `*`, so plus and then Enter marks everything. Before it applies, diroxide says the count: "12 entries will be selected. y or n". Select by pattern is also in the Select menu and the palette.
- Ctrl+Down and Ctrl+Up go to the next and previous marked row: "report.pdf, selected, 2 of 3 selected."
- Ctrl+D says the marks with their size: "3 selected, 4.2 MB. Documents, 40 items". Folders are not sized until a later alpha.

Marks belong to rows, not to positions. If the folder is listed again, the marks stay on the same names. If a marked row has disappeared, diroxide says how many marks were lost, and if the focused row has gone it says that the focus moved. After a job, only the entries that failed stay marked, so the marks show what to try again.

### The keypad and Num Lock

In a terminal the keypad's plus, minus and star arrive as the main row's characters, so they work as plus, minus and star above. Under NVDA the bare keypad belongs to NVDA's review commands unless Num Lock is on. Keep Num Lock on when you want the keypad to reach diroxide. JAWS treats the keypad the same way in its desktop layout. This has not yet been checked by ear.

## Details and the context menu

- Alt+Enter shows the details of the focused entry as a short list: name, kind, exact size, the three times in words, attributes in words, owner, link target and the full path. A folder says "folder size not computed".
- Shift+F10 or the Menu key opens the context menu: a list of the commands that apply to the focused entry and the marked ones, each with its key.

## Refresh and watching

diroxide watches the folders on screen on a fixed local drive and merges outside changes into the list. "Folder changed: 2 added, 1 removed" is said at most as often as the progress interval, never on every change. If the folder you are in is renamed or deleted, the title says "Folder is gone" and diroxide goes up to the nearest folder that still exists. A folder that cannot be watched says so: "Folder not watched. Reload with Ctrl+R." That is always true of network drives and of removable drives, which diroxide checks again when you return to the panel and every two seconds.

## The session

When you quit, diroxide remembers each panel's folder, the focused row, the sort, the filter and the layout, and puts them back at the next start. The first thing spoken is the restored cursor's row.

## See also

- [Operations and undo](operations.md)
- [Every key](keyboard.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
