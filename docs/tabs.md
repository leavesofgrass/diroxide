# Tabs

This guide covers tabs: several folders in one panel, each kept as you left it. It says how to open, close and switch tabs, what a tab remembers, how to lock or pin one, how to bring a closed tab back, the tab list, tab sets you save by name, what is restored at the next start, and the keys.

The short version: each panel holds a row of tabs, and one tab is shown. A tab is a whole folder view with its own folder, sort, filter, marks, cursor and history. Open a tab with Ctrl+T, close it with Ctrl+W, and move between tabs with Ctrl+Tab, Ctrl+Shift+Tab or Alt and a digit in the window, and with the Tabs menu in a terminal. diroxide says a tab in words, the folder first: "Documents, tab 2 of 3". It says nothing about tabs while a panel has only one.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader or read on a Braille display when it was written; if your experience differs, that is a finding.

## What a tab is

- A tab is a folder view in one panel. Each panel has its own tabs, and each panel starts with one.
- Every tab keeps its own folder, sort, filter, columns, marks, cursor row and history of folders visited. Back and forward in one tab never move another.
- Only the tab you are in is watched. A tab you are not in does not follow changes to its folder. When you come back to it, diroxide lists its folder again, quietly when the tab had been listed before, so what you find is fresh.
- A job you started in one tab goes on when you switch to another. When it ends it speaks its result, and it does not change the tab you are in.
- No tab can be closed when it is the last one in its panel.

## The words of a tab

A tab is said with its folder first, then its place in the row, as a list says its position last: "Documents, tab 2 of 3". A lock is said after the number, and only when one holds: "Documents, tab 2 of 3, locked".

- With one tab in a panel, nothing about tabs is said. The title line and each arrival read as they always have.
- With more than one tab, the title line says the tab after the folder: "diroxide-app, tab 2 of 2, left panel, in ...". A locked tab adds the word: "diroxide-app, tab 1 of 2, locked, left panel".
- Going into a folder in a tab says the tab too: "src, tab 2 of 2, " and then the count of items and where the folder is, as an arrival does.
- diroxide draws no row of tab headers in this build. A tab's state is always in words, so a screen reader and a Braille display have all of it. [Known limits](known-limits.md#tabs) says what that leaves out.

## Open a tab

- New tab, Ctrl+T, opens a copy of the tab you are in, right after it, and makes the copy the one you are in: "New tab: diroxide-app, tab 2 of 2". The copy has the same folder, sort, filter, history, marks and cursor, and from then on each tab goes its own way.
- New tab for the focused folder opens the folder under the cursor in a new tab. On a file it says so and opens nothing.
- Move tab to other panel and Copy tab to other panel send a tab across. [Moving and copying a tab](#moving-and-copying-a-tab) has them.

These commands are in the View menu's Tabs submenu (Alt+V, then N) and in the palette.

## Switch tabs

- Next tab and Previous tab go round the row, past the ends: "diroxide-app, tab 1 of 2", then "src, tab 2 of 2". With one tab they say so and stay.
- Go to tab 1 to Go to tab 9 go straight to a tab by its number. A number past the last says: "No tab 9: this panel has 2 tabs."
- The tab list is a chooser of this panel's tabs. Its title says the panel and the count, "Tabs, left panel, 2 tabs". Each row is a tab in words, the folder first, and the tab you are in has the cursor. Up and Down move, Enter goes to the tab, and Escape closes the list. The tab list is in the View menu (Alt+V, then J) and in the palette.
- Move tab left and Move tab right change a tab's place in the row by one. At the end they say "Already the last tab." and the same for the first.

A tab that you come back to is listed again, so the first words you hear can be its arrival: "src, tab 2 of 2" and what its folder holds now.

## Close a tab

- Close tab, Ctrl+W, closes the tab you are in. The tab after it becomes the one you are in, or the one before when you closed the last. It says what closed and where you landed: "Closed src. diroxide-app."
- The last tab in a panel never closes: "Last tab stays open: diroxide-app."
- Close other tabs closes every other tab in this panel except the locked ones: "Closed 1 tab. Kept 2 locked tabs."
- A locked tab asks first, and the question starts on No: "Close locked tab NAME? No is the default. y or n". No says "Nothing closed; the tab stays."

## Undo close tab

Undo close tab brings back the tab you closed last, in the panel you are in, at the place it had in the row, and makes it the one you are in: "Reopened src, tab 2 of 2". With nothing to bring back it says "No closed tab to bring back."

- Each use brings back one more tab, newest first. Close other tabs puts every tab it closed on the same list, so it takes one Undo per tab.
- The list holds the last 16 tabs and lasts for this session only. When you quit it is gone.
- Loading a tab set puts the tabs it replaced on the list too, so a load can be taken back one tab at a time.
- The brought-back tab keeps its folder, sort, filter, marks, cursor, history and lock. Its folder is listed again when it returns.
- This is not Ctrl+Z. Ctrl+Z undoes file jobs; Undo close tab has no key of its own and is in the Tabs submenu and the palette.

## Lock and pin

A tab has one of three states: free, locked or pinned. Lock tab and Pin tab each switch their state on and off, and say the result: "diroxide-app, locked", then "diroxide-app, free" when you switch it off. With more than one tab the number follows.

- A locked tab keeps its folder. Closing it asks first, and a change of folder does not happen in it. Going into a folder, going up, a typed path, back and forward all open the new folder in a new tab after it, and the locked tab stays where it was: "Tab locked: opening in tab 2." The new tab is free.
- A pinned tab changes folder freely. It remembers the folder it was pinned in as its base. Go to base folder returns to it. In the base folder already it says "Already in the base folder, diroxide-app." On a tab that is not pinned: "Not a pinned tab: it has no base folder."
- Pinning a locked tab makes it pinned, and locking a pinned tab makes it locked. A tab never holds both.
- Reload and the changes diroxide merges into a folder do not count as a change of folder, so they never open a new tab.

## Moving and copying a tab

Move tab to other panel takes the tab out of this panel and makes it the one you are in on the other side, after that panel's current tab: "Moved src to the right panel, tab 2 of 2". The other panel becomes the active one. A panel's only tab cannot move: "Only one tab here: it cannot move."

Copy tab to other panel does the same with a copy and leaves the original: "Copied src to the left panel, tab 2 of 2". Either way the tab keeps its lock.

## Saved tab sets

A tab set is both panels' tabs saved under a name, to open again whenever you like, for work you do the same way each time.

- Save tab set asks "Tab set name". The first saved name is already in the field. A name is at most 24 characters and has no control characters. An empty name says "Type a name for the tab set." and a long one says "Name too long: 24 characters at most." The result says what it saved: "Saved tab set Work, 3 tabs."
- A name already used, whatever its case, is replaced with no question, and the result says it replaced a set.
- A set holds each panel's tabs, the tab each panel was in, and which panel was active. For each tab it holds the folder, the row under the cursor, the sort, whether hidden files show, the history, and the lock with a pinned tab's base folder. Marks, the filter and the columns are not saved.
- Load tab set lists the sets, one row each, the name first: "Work, 3 tabs". With none saved it says "No tab sets saved yet."
- Enter loads the set in place of the tabs that are open: "Tab set Work loaded, 3 tabs". The tabs it replaced go on the list for Undo close tab.
- The sets are in `tab-sets.toml` in the state folder, and the file holds folder paths. [Settings](settings.md#where-the-files-live) and [privacy](privacy.md) say where it is and what that means. A file that cannot be read is kept as `tab-sets.toml.bad`, and diroxide says so: "tab-sets.toml could not be read. A copy is kept as tab-sets.toml.bad; no tab sets are in use."

There is no command yet to delete a saved set or to rename one. Save under the same name to replace a set. 
## At the next start

When you quit, diroxide remembers every tab of both panels, the one you were in, and each tab's folder, cursor row, sort, hidden files, history and lock, and puts them back at the next start. A tab you were not in is listed when you first go to it.

- Start diroxide on a folder, as in `diro DIR`, and the tabs come back beside it. When that folder is not the saved tab's, it opens as a new tab after the saved current one, and the others are kept.
- The session file is `session.toml` in the state folder. Alpha 5 writes its version 2, which adds the tabs. It reads an older version 1 file as one tab in each panel, so nothing is lost on the way up.
- Going back down loses the tabs. Alpha 4 opens a version 2 file and shows the current folder of each panel, which is what it always read. When alpha 4 writes the file again at its quit, the other tabs are gone. [Known limits](known-limits.md#tabs) says this too.

## Keys

In the window:

- Ctrl+T: New tab.
- Ctrl+W: Close tab.
- Ctrl+Tab: Next tab.
- Ctrl+Shift+Tab: Previous tab.
- Alt+1 to Alt+9: Go to tab 1 to tab 9.

In a terminal only Ctrl+T and Ctrl+W are bound. Terminals send Ctrl+Tab as plain Tab, Windows Terminal switches its own tabs on it, and GNOME Terminal does the same on Alt with a digit, so diroxide does not use them there: [terminals](terminals.md) says which terminals keep which keys. In a terminal, reach the rest through the View menu's Tabs submenu (Alt+V, then N), which holds every tab command, or through the palette.

Every tab command is in the palette, and a key you prefer is set in the Keys list. [Every key](keyboard.md) lists all 26 tab commands with their ids, from `tab.new` to `tab.load-set`, and [customizing keys](customizing-keys.md) says how to change a key.

## What tabs do not do yet

- No tab bar is drawn, in the window or the terminal. Tabs are words and the tab list.
- A locked tab alone in its panel shows no lock in the title line, because nothing about tabs is said with one tab. Lock tab still says it when you press it.
- Changing the column setting updates only the tab that is shown.
- Undo close tab keeps no tab past the end of the session.
- No command deletes or renames a saved set.

[Known limits](known-limits.md#tabs) has the same list with what has not been tried.

## See also

- [The panels](panels.md)
- [Settings and where the files live](settings.md)
- [Privacy](privacy.md)
- [Every key](keyboard.md)
- [Customizing keys](customizing-keys.md)
- [Terminals](terminals.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
