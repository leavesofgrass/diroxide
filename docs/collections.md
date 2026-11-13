# Flat view, collections and recent files

This guide covers lists that are not a folder: the flat view, which lists every file below a folder at once; collections, which are lists of places you keep by name; saved searches, which are collections that run again when you open them; recent files; and `diro -`, which makes a list from paths another program prints.

The short version: these are all lists, and they read like any list in diroxide, with the meaning first and the position last. A row in a list says where its file lives after the name. Taking an item out of a list never touches its file.

What this guide says comes from the design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader or read on a Braille display when it was written; if your experience differs, that is a finding.

## Flat view

The flat view lists every file below this panel's folder in one list, without walking folder by folder. It is in the View menu (Flat view) and in the palette. It has no key of its own.

Flat view first opens a short form, named for the folder: "Flat view of work". Its rows:

- "Start the flat view".
- "Depth: every level", "Depth: 1 level" or "Depth: 3 levels". Enter asks for a number: 1 is this folder's files only. Anything that is not a number says "Depth is a number of levels: 1 for this folder's files."
- "Files only: yes" or "Files only: no, folders too".
- "Hidden files: listed" or "Hidden files: left out". The start follows what the panel shows.

Enter on Start walks the folder. The list opens at once, while the walk goes on, so the first rows are there to read. It says "Flat view of work: listing files only", then "Flat view on, 3 files in 3 folders." when it ends. Escape stops the walk, and in a large folder says where: "Stopped at 4,200 entries".

Each row names the file first, then where it is, then its size: "report.pdf, in work/2026/q3, 2.1 KB". The row's folder is its path below the folder you started from. The refining filter you type in the list matches names.

Choose Flat view again, in the View menu or the palette, to turn it off: "Flat view off. work."

The refining filter you type in the list matches the folder words too, so typing `q3` finds files in a folder named q3.

### Copy and move out of a flat view

Copy or Move from a flat view asks a question first, and it starts on the safer answer: "Keep folders, or all in one folder?". Its two rows are "Keep folders, as below the flat view" and "All in one folder; taken names asked".

- Keep folders puts each file in the same folder below the destination as it had below the flat view's folder. It works for a Move one folder level down. Copy says "Keep folders is for Move only, for now." and a file deeper than one level says "Keep folders: one folder level, for now."
- All in one folder puts every file in the destination folder. A name that is taken is asked about, never overwritten in silence.
- Escape says "Nothing moved." or "Nothing copied."

## Collections

A collection is a list of places you keep under a name: the files for one trip, the pages you are reading this week, what you want to look at again. It keeps places, not copies. A file can be in any number of collections and stays where it is.

Collections are kept as files in the `saved` folder of the state folder, one TOML file each, next to your saved searches. They hold paths, which [privacy](privacy.md) says. A collection you cannot read is not deleted.

### Make one

- Save as collection saves the marked entries of the panel as a collection. It asks for a name, "Collection name for 2 items", with the field empty. An empty name says "A collection needs a name." With nothing marked: "Nothing to save: mark entries first."
- Collect adds the marked entries, or the focused one when none are marked, to the collection named "Collected". "Added 1 to Collected." A place that was already there is counted apart: "Added 0 to Collected. 1 was there already."

Both are in the File menu and in the palette. They work from a list too: with a results list open, the palette, the menus and the list's Actions reach the places of the list on the screen, and the list stays open after you name a collection. Neither has a key by default. [Customizing keys](customizing-keys.md) says how to give them one.

### Open one

Collections is in the Go menu (Alt+G, then J) and in the palette. Its title says how many lists there are, "Collections, 2 lists", and it holds your collections and your saved searches. Each row says which it is: "Trip photos, collection, 1 item", with ", 1 missing" or ", 1 not checked" after it, and "Large images, saved search, runs on open". Enter on a collection opens its list. Enter on a saved search runs it again, so the list is fresh. With none: "No collections yet. Save as collection makes one."

The last two rows are "Rename a collection" and "Remove a collection".

### Inside a collection

A collection's list has one row for each item, the name first and the folder after it. All the items are checked when it opens. A file that is gone says so after its name: "report.pdf, gone since this list was made". The title says how many are gone: "1 of 2 items gone from Trip photos."

- Delete, the trash key, takes the item out of the collection and never touches the file: "Removed 1 from Trip photos. The file was not touched." For several: "Removed 3 from Trip photos. The files were not touched."
- Remove missing items takes out every item whose file is gone. "Nothing missing in Trip photos." when there are none. It is in the palette, the menus and the list's Actions, and no file is touched.
- Open a collection first: when you use Remove missing items outside one, "Open a collection first: Collections, in the Go menu."

### Rename and remove

"Rename a collection" asks which one, then for the new name, and says "Renamed Trip photos to Trip 2026." A name is at most 100 characters.

"Remove a collection" asks first and starts on No: "Remove collection Trip 2026? No is the default". It says what will happen: its file goes to the trash and the items' files stay. No, or Escape: "Nothing removed; Trip 2026 stays." Yes: "Removing Trip 2026. Its items' files stay."

## Saved searches

A saved search is the live kind of collection. It stores what you typed and runs it again each time you open it from Collections, so the list is never old. [Search](search.md) says how to save one. The row reads "Large videos, saved search, runs on open".
## Recent files

Recent files is in the Go menu (Alt+G, then Q) and in the palette. It lists the files you opened or edited from diroxide, newest first, at most 50. Its line comes first: "Recent: 2 files, newest one.txt, today". Each row names the file first, then what you did and when, then where it is: "one.txt, opened today, 10:00 AM, in ...".

- A file is kept when you open it with Enter, or edit it with F4, in a panel or from inside a results list. F3, which views, never keeps a file.
- Only the state folder keeps them, in `recent.toml`, on this computer. The setting Remember recent files, `recent_files`, turns it off. Then nothing is kept and the list is emptied: "Recent files are off in Settings."
- Clear recent files forgets every one, in the palette: "Cleared 20 recent files." No file is touched.
- With none: "Recent: no files".

A file you open from inside a results list is kept too.

## diro -

`diro -` makes a list from paths another program prints. It reads all of standard input first, and the list is what that input names.

```bash
find . -name "*.log" -print0 | diro -
```

- Paths are separated by NUL by default, the way `find -print0` and `fd -0` print them. With `--lines` they are one on a line.
- A relative path is read against the current folder, and a path given twice is kept once. A name that is not Unicode is said by its position.
- It reads at most 16,384 paths or 64 MiB. The settings `lists.stdin_paths` and `lists.stdin_mib` set them.
- The opening options work as they do for `diro [PATH]`, before or after the dash: `--portable`, `--tui`, `--gui`, `--no-plugins`, `--theme`.
- It says first how many it read: "3 paths from standard input". Then it says each path it could not read, the word first: "Missing: gone.txt, in D:\old\place".

On a terminal, `diro -` opens the list in the app. The list is named "Standard input", and its first line is "3 paths from standard input, 1 missing". When its output is redirected, so there is no terminal to open, it prints the lines instead, one for each entry as `diro ls` says them. `--cwd-file` is not handled with `diro -` yet. `diro ls -` reads the same way.

The exit codes are 0 when every path was read, 1 when none could be, or the list is too long, 2 for wrong use or separators that are not the ones asked for, and 3 when some could not be read.

## Limits

- Keep folders works for a Move one folder level down. Copy and a deeper folder say "for now" (see [Copy and move out of a flat view](#copy-and-move-out-of-a-flat-view)).
- A flat view's rows read "a.txt, in work/b/deep", with the folder as the list gives it.
- The flat view's settings are not remembered from one time to the next.
- A collection's items are all checked when it opens, which is slow for a very long one.
- No command deletes a collection's file except Remove a collection, which sends it to the trash.
- `diro -` does not handle `--cwd-file`. After it opens, the panel's arrival line may be said after the list's start line.

[Known limits](known-limits.md#lists-that-are-not-folders) keeps the list current.

## See also

- [Search and the filter language](search.md)
- [The panels](panels.md)
- [Tabs](tabs.md)
- [Privacy](privacy.md)
- [Settings and where the files live](settings.md)
- [Every key](keyboard.md)
- [Customizing keys](customizing-keys.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
