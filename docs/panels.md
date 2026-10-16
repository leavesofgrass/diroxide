# The panels

This guide covers the two panels: how they are laid out, what a row says, and how to move, sort, filter, jump and mark. Copying, renaming, deleting and undo are in [operations and undo](operations.md).

## The two panels and the three layouts

diroxide shows two panels. One is active, and the other is the target of a copy or a move. Tab switches between them. A setting called layout decides how they are drawn.

- Side by side is the default: the panels sit next to each other, as in every orthodox file manager since Norton Commander.
- Stacked puts one panel above the other. Each screen line is one row at the full width of the terminal. It is the fallback for screen reader users and for narrow terminals.
- Single shows only the active panel. Tab then switches which panel is shown and says its place.

Which layout should a screen reader user choose? Try stacked first. In the side by side layout one screen line holds a row of each panel. A screen reader reads the whole line, and a 40-cell Braille display shows its first 40 cells, so the two panels mix. When the right panel is active, its focused row starts near column 41, after the left panel's row. In the stacked layout every line is one row, and Tab changes only the cursor, a marker and a title. This advice comes from how screen readers read a line. It has not yet been confirmed by listening on this project, and this guide will change if the listening checks say otherwise. [Using diroxide with a screen reader](screen-readers.md) has more.

To change the layout, press Ctrl+P, type "layout" and press Enter, or choose Change layout in the View menu (Alt+V, then L). Each time, the layout cycles to the next one and diroxide says it: "Layout: stacked". The `layout` setting makes the choice permanent, and diroxide remembers the layout you left with between sessions.

In screen-reader mode the other panel is redrawn only when you ask, whatever the layout, so a change in the panel you are not using does not interrupt you.

## What is on the screen

- The title line comes first and puts the place first: the folder's own name, then which panel, then a filter if there is one, then the parent: "Documents, left panel, in D:\work". With a filter and a sort that is not your default: "Documents, left panel, filter *.pdf, in D:\work, sorted by size". A panel that is still reading says ", loading" after the panel. A very long parent path is cut from the left with three dots. The whole path is the starting text of Go to a path.
- The rows of the panel follow. There is no ".." row; Left, Backspace and Alt+Up go to the parent. At the top of a drive, diroxide says "At the top already".
- The status line gives the latest message with its key fact first. A move of the cursor does not change it. Between messages, Ctrl+D tells you where you are.
- The jobs line says what jobs are doing, or "No jobs".
- The key bar is a line that names the function keys in words. It is drawn only when screen-reader mode is off, and the compact setting hides it. In screen-reader mode it is never drawn, and the Say key bar command in the Help menu (Alt+H, then K) reads it to you.
- Prompts and their candidate lists appear at the bottom of the screen, in the minibuffer. In screen-reader mode a candidate list is drawn only when you move into it, and while you type one line says the count.

The hardware cursor always sits on the focused row's first cell, so a screen reader and a Braille display follow it. In a prompt it sits where the next character will go. In an empty folder it sits on the line that says "Empty folder".

## What a row says

A row puts its meaning first: the name, then the kind, the size and the modified time. "report.pdf, PDF, 2.1 MB, modified Tuesday". A folder says "folder" after its name. The position comes last when diroxide speaks the row: "3 of 40".

- A selected row says "selected" right after its name: "report.pdf, selected, PDF, 2.1 MB". When screen-reader mode is off, the screen also starts the row with an asterisk. The word is never left out, and the asterisk is never the only signal.
- A name that could cause trouble says so: "name needs care". That covers reserved characters, trailing dots and spaces, Windows device names, invisible characters, and names that differ from a sibling only by case.
- Control, private-use, direction and other invisible characters in a name are shown as a question mark. The real name is kept for operations. Any listing before a delete spells such characters out as `U+XXXX`.
- A cloud placeholder says "online only". diroxide asks before it reads such a file's contents.
- An empty cell says a word such as "not set". It is never a blank or a lone dash.
- Hidden entries are not listed until you show them.

By default the columns are type, size and modified time. The `columns` setting lists the details said after each name and their order: type, size, modified, created, accessed, changed, permissions, mode, octal, owner, group, attributes, link and extension.

Permissions are read in words first. On Linux and macOS the permissions column says who can do what: "read: everyone; write: owner". A folder says "list and enter: everyone; change: owner". The octal number is a separate column, never the first thing said. On Windows the column uses words such as read-only, hidden, system, compressed, encrypted and online only. You can see permissions in alpha.1, but you cannot change them until alpha.2.

## Moving around

- Up and Down move one row. Home and End go to the first and last row. Page Up and Page Down move a page. At the first or last row diroxide says "Top of the list" or "Bottom of the list".
- Left goes to the parent folder. Right goes into the folder under the cursor; on a file it says the kind of file and opens nothing: "Not a folder: report.pdf, PDF".
- Enter opens the focused entry. Backspace and Alt+Up go to the parent as well.
- Alt+Left and Alt+Right go back and forward through the folders you visited, saying "No folder before this one" or "No folder after this one" at the ends. Windows Terminal keeps those keys and Alt+Up; the Go menu (Alt+G) has Back, Forward and Up one folder as the sure way. Folder history in the Go menu lists the folders you visited, and Enter goes to one.
- Alt+Home goes to your home folder.
- Ctrl+R lists the folder again: "Reloaded: 124 items".

When you go to the parent with Left or Backspace, the cursor lands on the folder you just left. Back and forward return you to the row you were on, with the marks you had, folder by folder.

Enter on a folder goes in. Enter on a file hands it to your system's program for that kind of file: "Opening report.pdf". Enter on a program asks first, and the question starts on No: "Run setup.exe? No is the default. y or n". F4 hands the file to your editor instead, with the screen suspended; when you come back diroxide says "Back in diroxide, Documents". The editor is the `editor` setting, or else the VISUAL or EDITOR variable, or else the system's text editor. Archives are listed as archives, but they cannot be opened until alpha.2.

## Go to a path

Ctrl+L or Alt+D opens the path field with the current path in it. Type or paste a path and press Enter.

- Tab completes the path, and Up and Down recall the paths you typed before. [Operations and undo](operations.md) describes completion.
- Quoted paths work, such as a path copied from Explorer, and either kind of slash. A relative path is read from the folder you are in.
- A `~` or a variable such as `%TEMP%` is offered as a candidate that completes to the literal full path. Nothing is expanded when you press Enter.
- A path such as `D:` with no slash is refused as ambiguous. Type `D:\` for the root of the drive.
- If the folder does not exist, diroxide says why and stays where it was.

## Go to a place, and path jump

Ctrl+G opens Go to a place: a prompt with a list under it of your home folder, the drives on Windows or the mounts on Linux and macOS, and the folders you have visited, the ones you use most first. Each candidate says its folder first and what it is after it, such as "home folder" or "visited 12 times". A drive says its type in words ("disk", "removable drive" or "network drive") and its free space. diroxide does not wake an idle drive to read it.

Type a few words to narrow the list. This is path jump. Each word has to appear in the path, in the order you typed them, and the last word has to appear in the last folder's name. Case does not matter. Your home folder is left out of the visited list, because it is one key away. Enter goes to the chosen folder.

diroxide counts a visit each time you open a folder, and weights recent visits more. The counts are kept in the state folder.

## The other panel

- Ctrl+Right shows the focused folder in the other panel: "Other panel: Documents". Ctrl+Left shows this panel's folder in the other panel. macOS keeps these two for Mission Control; the Go menu has them.
- Ctrl+U swaps the two panels: "Panels swapped: left Downloads, right Documents".
- Alt+O says the other panel's folder and its focused row without switching to it: "Other panel: Backup, 12 items, on report.pdf, 3 of 12".
- `diro LEFT RIGHT` opens both panels on two folders from the start.

## Sorting

Sorting is quick and has many keys, because sorting is how you find things and decide what to do next.

- Ctrl+F3 sorts by name, Ctrl+F4 by extension, Ctrl+F5 by modified time, Ctrl+F6 by size and Ctrl+F7 not at all (the provider's own order). These are the Norton Commander keys that Total Commander and Double Commander also use.
- Choosing the same key again reverses the direction. Each key has a sensible first direction: names A to Z, sizes largest first, dates newest first, kinds A to Z.
- The Sort list has every key and marks the current one, saying its direction. Type a letter to jump, press Enter to apply, and choose the same key again to reverse. In the terminal you reach it from the View menu (Alt+V, then S, then Sort list), because a terminal cannot reliably send Ctrl+Shift+S, which opens it in the window. The palette has an entry for every key too: Sort by created, accessed, kind, attributes, owner and label, as well as the five above.
- A key with no data in this folder says so after the result: "No labels yet; labels come with alpha.3" for labels, and "No entry here has its owner" for a key such as owner.
- Names sort in natural order, so `file2` comes before `file10`, and capitals do not matter. Folders come first, as a setting (`sort.folders`: first, mixed or last). Whatever key you sort by, name is always the last key, so two files that tie come in name order and the cursor never jumps.
- The status line says the result first and never the position: "Sorted by size, largest first", then "Sorted by size, smallest first" when you press it again. The cursor stays on the same entry.
- The title line says the sort when it is not your default, so a panel you reach with Tab tells you.

On macOS the system owns Ctrl+F3 to Ctrl+F8. Use the Sort submenu in the View menu or the palette there.

## Finding and filtering

There are two ways to narrow down a long folder.

- Type to find. With single-letter keys on, typing a letter or a digit finds a row by name. The cursor jumps to the first row, from where you are, whose name starts with what you typed, and the status line says the row and the count: "report.pdf, 1 of 3 matches". Typing more narrows the match; the same letter again goes to the next row that starts with it. After a pause of about a second, the next letter starts a new search. When nothing starts with your text, diroxide says "No name starts with" and the text. The `single_letters` setting chooses what typing does: find by name (`type_ahead`, the default), `commands` for the key presets that come with alpha.3, or `off`. Turn it off if you use dictation software, which types letters.
- Ctrl+S opens the Filter prompt, whether single-letter keys are on or off. As you type it hides every row that does not match and says the count: "3 of 12 match". Enter keeps the filter: "Filter *.pdf: 3 of 12 shown". The title line then names the filter. To clear it, use Clear filter in the View menu or the palette (it says "Filter cleared: 40 items"), or open the prompt, delete the text and press Enter.

A filter and a pattern use the same small language. The first version understands these, and alpha.3 extends them without breaking them:

- Plain words match names that contain all of them, in any order: `report 2026`.
- A term with `*`, `?` or `[` is a glob that matches the whole name: `*.pdf`, `IMG_????.jpg`. Separate terms with commas (or semicolons) to match any of them: `*.pdf, *.docx`.
- `not` in front of a term leaves out what it matches: `*.pdf, not draft*`. A pattern of only `not` terms matches everything else.
- A slash at the end limits a term to folders: `src/`, or `*/` for every folder.
- A `re:` prefix makes the rest one regular expression, matched anywhere in the name: `re:^IMG_\d{4}`.

Case is ignored. A pattern that cannot be read is explained in one short line: "Pattern needs care: unclosed bracket".

Hidden files have their own switch: Show hidden files, on Alt+Period, in the View menu and the palette. diroxide says "Hidden files shown" or "Hidden files not shown". It is not Ctrl+H, because terminals confuse that key with Backspace. Hidden means a name that starts with a dot on Linux and macOS, the hidden flag on macOS, and the hidden attribute on Windows. A name such as `.gitignore` is not hidden on Windows.

## Marking rows

Marking picks the rows a command will act on. With no marks, a command acts on the focused row.

- Space marks the focused row and moves down: "Selected report.pdf. 1 selected." Space on a marked row unmarks it: "Not selected report.pdf. 0 selected."
- Ctrl+A marks every row, and diroxide says "40 items selected". Select none, invert, select by pattern and unselect by pattern are in the Select menu (Alt+S) and the palette. In the terminal, star inverts the marks, plus asks for a pattern to select and minus asks for a pattern to unselect, when single-letter keys are on. The prompt starts with `*`, so plus and then Enter marks everything. The result is a sentence: "Selected 12 matching *.pdf. 15 selected."
- Ctrl+Down and Ctrl+Up go to the next and previous marked row, saying "No selected row below" or "No selected row above" at the ends.
- Ctrl+D says the marks with their size: "3 selected, 4.2 MB. Documents, 40 items, on report.pdf, 3 of 40". Folders are not sized until a later alpha.
- Restore selection (in the Select menu and the palette) brings back the marks that a select all, none, invert, pattern, finished job or change of folder replaced: "Selection restored: 31 selected."

Marks belong to rows, not to positions. If the folder is listed again, the marks stay on the same names. If a marked row has disappeared, diroxide says how many marks were lost, and if the focused row has gone it says that the focus moved. After a job, only the entries that failed stay marked, so the marks show what to try again.

### The keypad and Num Lock

In a terminal the keypad's plus, minus and star arrive as the main row's characters, so they work as plus, minus and star above, and no command depends on a keypad-only key. Under NVDA the bare keypad belongs to NVDA's review commands unless Num Lock is on. Keep Num Lock on when you want the keypad to reach diroxide. JAWS treats the keypad the same way in its desktop layout. This has not yet been checked by ear.

## Details and the context menu

- Alt+Enter lists the details of the focused entry, one fact a row: name, kind, size (a folder says "folder size not computed"), the modified, created and accessed times in words, attributes, owner, group, link target, whether it is hidden, and the full path. A name that needs care adds the reason and the name spelled out. Windows Terminal keeps Alt+Enter; the File menu has Details.
- Shift+F10 or the Menu key opens the context menu: "Actions for report.pdf", a list of what the File menu offers here plus copy, cut, copy to the other panel and move to the other panel, each with its key. On Windows the Menu key never reaches a terminal program, so Shift+F10 is the way there.

## Refresh and watching

diroxide watches the folders on screen on a fixed local drive and merges outside changes into the list. "Folder changed: 2 added, 1 removed" is said when it matters, not on every change. If the folder you are in is renamed or deleted, diroxide says "Folder is gone" and goes up to the nearest folder that still exists. A folder that cannot be watched says so: "Folder not watched. Reload with Ctrl+R." That is always true of network drives and of removable drives, which diroxide checks again when you return to the panel and every two seconds.

## The session

When you quit, diroxide remembers each panel's folder, the focused row, the sort, whether hidden files are shown, the folders you visited for back and forward, the layout and which panel was active, and puts them back at the next start.

## See also

- [Operations and undo](operations.md)
- [Every key](keyboard.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
