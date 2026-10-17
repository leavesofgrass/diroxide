# Labels and bookmarks

This guide covers the ways diroxide remembers things for you: labels on files, bookmarks on folders, the History list of places you visit, and a view saved for one folder. It also says where saved searches fit. It comes with alpha.3.

What this guide says comes from the design and from the tests that check each sentence. Nothing in alpha.3 had been heard on a screen reader when this guide was written, so the lines below are what diroxide is built to say. [The accessibility statement](accessibility.md) says what has been checked.

## Labels

A label is a name you put on a file or folder, such as Urgent or Review. A label has a name first and a color second. The color is always said as a word after the name, never shown alone, and it is never red or green. The colors are blue, orange, purple, yellow, gray and brown.

A row that has a label says so right after its name: "report.pdf, label: urgent, 2.1 MB". With several, "labels: urgent, review". A label name is at most 30 characters, so a row keeps its meaning in the first 40 cells.

The starter labels are Urgent (orange), Waiting (yellow), Review (purple), Done (blue), Keep (brown) and Skip in searches (gray). Skip in searches is special: a place with that label is left out of every search, and the summary says so.

### Set a label

Alt+L is Set label. It acts on the marked rows, or on the focused row when nothing is marked.

1. Press Alt+L. A field opens with the labels listed under it: "Set label on 3 items, empty". Type to narrow the list, and one line says how many match. Down goes into the list.
2. Press Enter on a label, or type a new name and press Enter. diroxide says "Label urgent added to 3 items". If some already had it: "Label urgent added to 1 item; 1 already had it".
3. A new name makes a new label. It takes a color no label has yet, or the next in turn when all six are used, and says it: "Label taxes added to report.pdf; a new label, blue".

Toggle label adds a label, or takes it off when every item has it: "Label urgent removed from report.pdf". Clear labels takes every label off: "Labels cleared from 2 items". Both are in the palette and in the Edit menu under Labels (Alt+E, then B): s Set, t Toggle, c Clear and l Labels list.

Ctrl+Z undoes the last label step: "Undid label urgent on 3 items". Label undo lasts for the session. A job's undo survives a restart, but a label's does not.

### The favorite labels

Ctrl+1 to Ctrl+7 toggle the first seven labels of the Labels list, in its order. The Labels list shows each label's key at the end of its row. Many terminals do not send Ctrl with a digit, so the sure way is the Edit menu: Alt+E, then B, then 1 to 7. "No label 4 yet. The Labels list has 3." says there is none to toggle.

### The Labels list

Labels list, in the palette and in the Edit menu, lists the labels: "Labels, 6 labels". A row puts the name first, then the kind, the count and the color: "Urgent, label, 14 items, 2 missing, orange, Ctrl+1". diroxide counts the missing items in the background, so a row first says "checking". When the count is ready the status line says "Checked 14 labeled items: 2 missing". The count is not spoken again by the row, so look at the row after a moment.

- Enter on a label lists its items: "Urgent, 14 items". Enter on an item shows it in its folder, with the item focused. Delete takes the label off that item. There is no question, and Ctrl+Z puts the label back.
- An item that was moved or removed says "missing" in its row. Enter on it says "Missing: report.pdf. It was moved or removed; Delete takes its label off."
- With no labels at all: "No labels. Set label makes one."

### What a label is tied to

A label belongs to a place, the full path of the file. diroxide keeps the labels in `labels.toml` in the state folder.

- A file that is renamed or moved loses its label, even when diroxide itself did it. Label it again. The Labels list shows the old place as missing until you take it off.
- A label works in a filter: `label urgent`, `label urgent, waiting` for either, or `labeled` for anything with a label. [Search and the filter language](search.md) has the rest. Sort by label is in the Sort list.
- A label never changes the file itself.

There is no command yet to rename a label, change its color, put the labels in another order or delete one. You can do each by editing `labels.toml` by hand. A label is a `[[label]]` table with a `name` and a `color`:

```toml
[[label]]
name = "Taxes"
color = "blue"
```

diroxide reads a color word in lowercase from the six above. A color it does not offer, such as red or green, is reported in words at start: "Label Hot: red is not offered; labels never use red or green." Order matters, because the favorite keys follow it.

## Bookmarks

A bookmark is a folder you want to reach in one step, or a single file or folder you want to open on the spot.

Ctrl+B or Alt+B opens the Bookmarks list. The first row is the action for this folder: "Add this folder", or "Remove this folder" when it is already bookmarked. Then come the bookmarks, each with its name first: "Reports, bookmark, in work, used 12 times", or "sample, bookmark, in Drive D:\, not used yet". A bookmark with an alias says it: "Reports, alias r, in work".

- Enter on a bookmark goes there. Enter on the first row adds or removes this folder: "Bookmarked Reports, in work".
- An alias is one letter or digit. Press it in the list and diroxide jumps at once. A letter that is no alias moves by name instead.
- F2 opens the bookmark's settings, one a row: Name, Place, Category, Alias, Sticky and Opens. A sticky bookmark stays at the top. Enter on a row changes it.
- Delete asks first, and the question starts on No: "Remove bookmark Reports? No is the default. y or n". Then "Removed bookmark Reports".
- Typing does not narrow the list. It jumps, as above.
- Messages: "Alias r is taken by Reports." "An alias is one letter or digit." "Already bookmarked: Reports, in work." "That bookmark is gone."

Without opening the list, the Go menu has Bookmark this folder (Alt+G, then M) and Bookmark the focused item (Alt+G, then E). An item bookmark opens its folder and puts the cursor on the item, so it works for a single file. Bookmarks also appear in the Go to a place list (Ctrl+G), with their kind first, "Bookmark, Reports", beside the recent places. The `bookmarked` word in a filter reads the same store.

<!-- to confirm: the exact wording of a bookmark row in the Go to a place list; the wiring lane (W3a, commit 39) may change it. -->

Bookmarks live in `bookmarks.toml` in the state folder. A bookmark holds a name, a place, a category, an alias, whether it is sticky, whether it is an item, how many times it was used, and when it was last used. Nothing is removed on its own.

## The History list

diroxide counts a visit each time you open a folder. Folder history in the Go menu lists the folders of this session for Back and Forward. The History list is longer: every folder you visited in any session, most used first.

- History list is in the Go menu (Alt+G, then R) and the palette. A row says the folder first, then how much it is used: "reports, in work, used 12 times, today". The title says the count: "History, 40 places".
- Enter goes to the folder.
- Delete forgets the place: "Forgot Backup. 2 places left."
- Forget this place, in the Go menu (Alt+G, then G) and the palette, takes the active panel's folder out of the History list and out of path jump. Use it for a folder you visited once and do not want offered again.
- With nothing yet: "No places in the history yet".

## A view saved for one folder

A folder can keep its own look: its sort, its filter, whether hidden files show, and its columns. Save view for this folder, in the View menu (Alt+V, then W) and the palette, saves the panel's look for the folder it shows: "View saved for work: sorted by size, largest first".

From then on, each time you enter the folder, diroxide applies it and says so after the folder's line: "Sorted by size, largest first; saved for this folder." When you leave, the usual view comes back. If the saved view differs from your usual hidden setting, it adds "hidden files shown".

Forget this folder's view, in the View menu (Alt+V, then G) and the palette, stops it: "View forgotten for work. The usual view returns on leaving." In a folder with none: "work has no saved view."

The views are in `folder-views.toml` in the state folder. A view keeps the columns that were in use when you saved it, because no command sets a panel's columns yet. Nothing is removed on its own.

## Saved searches

A saved search is a different kind of memory: a filter and a scope, run again when you open it. [Search and the filter language](search.md) describes it. A saved search is a file in the `saved` folder of the state folder, in the plain shape that collections will use with alpha.8.

## Settings and files

- `labels.toml`, `bookmarks.toml`, `folder-views.toml` and the `saved` folder are in the state folder. [Settings and where the files live](settings.md) says where that is.
- A file diroxide cannot read is kept with `.bad` added to its name, and the program starts without it, saying so in words.
- The history of visited folders is in the `history` folder of the state folder, the same one path jump uses.

## See also

- [Search and the filter language](search.md)
- [The panels](panels.md)
- [Operations and undo](operations.md)
- [Customizing keys](customizing-keys.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
