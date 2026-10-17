# Search and the filter language

This guide covers finding things in diroxide. It describes the filter language that every place that narrows a list speaks, the panel filter, select by pattern, Search and its results list, saved searches, the Space list, the newest files, and `diro search` for the command line. It comes with alpha.3.

What this guide says comes from the design and from the tests that check each sentence. Nothing in alpha.3 had been heard on a screen reader when this guide was written, so the lines below are what diroxide is built to say, not a report of what was heard. If your experience differs, that is a finding. [The accessibility statement](accessibility.md) says what has been checked.

## One language in five places

diroxide has one way to say what you are looking for. The same text works in all of these:

- The panel filter (Ctrl+S) narrows the folder you are in as you type.
- Select by pattern (Plus) and Unselect by pattern (Minus) mark the rows that match.
- Search looks in the folders below the one you are in.
- A saved search keeps the text and runs it again each time you open it.
- `diro search` takes the same text in a shell.

Learn the language once. A filter that finds your files in a panel finds them in a search.

## The shape of a filter

A filter is made of tests. A test says one thing about an entry: `report`, `*.pdf`, `size over 1 MB`, `modified last week` or `hidden`.

- Join tests with `and`, `or` and `not`, and group them with brackets. A space between two tests means and, so `ext pdf size over 1 MB` is the same as `ext pdf and size over 1 MB`.
- The words of the language ignore case. `AND` works.
- `not` applies to the next test or the next bracket: `not (ext md or ext txt)`. An exclamation mark right before a test means not, so `!draft` reads back as "not name contains draft".
- Mixing and with or in one group, without brackets, is a problem and not a guess. Other programs disagree about which binds tighter, so diroxide asks you to say it: "Filter problem: and mixed with or", then "Put brackets around one of them." Write `(ext jpg or ext png) and not name thumb*`.
- The pipe and the ampersand are refused with the word to use: "Filter problem: write or, not |" and "Filter problem: write and, not &". Quote the character if it is part of a name.
- A bracket has to be closed. "Filter problem: unclosed bracket" is followed by "Add ) to close it." Brackets nest at most 32 deep, and a filter is at most 10,000 characters.

## Names and patterns

A word that is not a field name is a name test.

- A bare word matches names that contain it: `report`. Several bare words must all be in the name, so `report 2026` finds names that hold both.
- A word with `*`, `?` or `[` is a glob and must match the whole name: `*.pdf`, `IMG_????.jpg`, `[abc].txt`. The star and the question mark never cross a folder separator. In a `path` test, two stars cross folders: `path **/src/*.rs`.
- A comma joins values of one test and means either: `*.txt, *.md`, or `ext pdf, docx`. A space after the comma is fine, and a semicolon works as a comma. A comma binds tighter than a space, so `report 2026, *.pdf` means the name holds report, and also holds 2026 or matches `*.pdf`.
- A slash at either end of a name limits it to folders, as a gitignore file does: `build/`. The read-back says it: "name contains build/, folders only".
- `re:` before a word reads the rest as a regular expression, searched anywhere in the name: `re:^IMG_\d{4}`. Use quotes if the pattern holds a space. diroxide uses Rust's regular expressions, so look-around and backreferences are refused with a sentence that says how to write it another way: "Filter problem: look-around not allowed", then "Match the text around it too: for a v before a digit, write v\d." A pattern is at most 1,000 characters.
- `like` is a flexible match, for when you remember a name loosely: `name like "rprt"`.

### Quotes

Double quotes hold spaces and operators. A doubled quote inside quotes is one quote. A backslash is never an escape, so a Windows path inside quotes is itself: `path "D:\work"`.

Some words belong to the language. To search for one of them as a name, quote it: `"or"`. The words are `and`, `or`, `not`, `case`, `anycase`, `state`, `regex`, every field name (with `ext`, `timemod` and `timeaccess`) and every yes-or-no word. A name that starts with a bracket or an exclamation mark has to be quoted too.

### Case

Case is smart. A word matches without regard to case, unless it has a capital letter. Then it matches case exactly, the habit of ripgrep, fd and vim. When a capital forces an exact match, the read-back says "match case", so you know why nothing matched. `case` before a test forces an exact match, and `anycase` forces none.

The `filter_case` setting changes the default: `smart`, `ignore` or `match`.

## Fields

A field name, an optional colon, then a value. `ext pdf`, `ext:pdf` and `ext is pdf` are the same filter. The fields:

- `name`: the name. `path`: the whole path.
- `ext` (or `extension`): the extension without its dot. A comma list means either.
- `kind`: `folder`, `file`, `link`, `archive`, `special` or `item`. A file kind also matches archives, because an archive is a file. An item is a row that is not a file, such as one a plugin lists.
- `type`: the detected type's short name, such as `PDF`. It matches only entries whose type is already known.
- `group`: a group of file types by extension: `images`, `music`, `video`, `documents`, `archives`, `code` and `text`. The full type model comes with alpha.7.
- `label`: a label name, or several with commas. [Labels and bookmarks](labels-bookmarks.md) describes labels.
- `owner`: the name of the file's owner. `usergroup`: the Unix group. (The word `group` always means the group of file types.) Owner and group are compared without regard to case. A Windows listing does not carry owners.
- `size`: a number with a unit. `depth`: how many folders below the start; direct children are 1.
- `modified`, `created`, `accessed` and `changed`: dates.
- `content`: text or a regular expression inside the file: `content:"total due"` or `content:re:"due \d+"`. See "Searching file contents".

Worker's long names `timemod` and `timeaccess` work as `modified` and `accessed`, and `state=selected` works as `selected`.

### Yes-or-no words

These stand alone, or after `not`: `hidden`, `read-only`, `executable`, `system`, `empty`, `broken` (a link whose target is missing), `online-only`, `needs-care` (a name that could cause trouble), `selected`, `bookmarked` and `labeled`.

A positive `hidden` makes hidden entries visible for that filter, and the read-back ends ", including hidden".

### Words for comparing

For text: `is`, `is not`, `contains`, `starts with`, `ends with`, `matches` (a glob) and `like`. For numbers: `is`, `is not`, `over`, `under`, `at least`, `at most` and `between X and Y`. For dates: `on`, `before`, `since`, `after`, `between X and Y`, `within 3 days` and `older than 1 year`.

A range may also be written with two dots, `size 1 KiB..1 MiB`, and it reads back the same as `size between 1 KiB and 1 MiB`. The signs `=`, `!=`, `>`, `<`, `>=`, `<=` and `:` are accepted and read back as words, so `size>1mb` reads back as "size over 1 MB". Words are better, because a shell treats `>` and `<` as redirects. See "diro search" below.

### Sizes

A number and a unit. Digits and one decimal point only, with no thousands separator, because the comma separates items. Bytes are the default. KB, MB, GB and TB are powers of 1,000. KiB, MiB, GiB and TiB are powers of 1,024. A bare `k`, `m` or `g` is refused with the units to write: "Filter problem: write MB or MiB", then "MB is 1,000,000 bytes; MiB is 1,048,576." The read-back gives the exact byte count on a details line: "Details: 100 MB is 100,000,000 bytes."

A folder has no size until it is counted. A test on a size that is not known matches neither the test nor its opposite. The summary says how many entries were left undecided and why: "1 unknown, folder size not counted."

### Dates

Dates are written year first: `2026-10-11`, or `2026-10`, or `2026`, or with a time, `2026-10-11T14:30`. A date such as `10/11/2026` is refused, because it means different days in different countries: "Filter problem: write the year first", then "Dates are written year, month, day, such as 2026-10-11." A date may also be spelled with its year: `October 1, 2026`. Without the year it is not understood.

Date words: `today`, `yesterday`, `this week`, `last week`, `this month`, `last month`, `this year` and `last year`. A duration is a number and a unit: second, minute, hour, day, week, month or year, in the plural, or `s`, `min`, `h`, `d`, `w`, `mo` and `y`. The first day of the week is the `week_starts_on` setting: Sunday by default, or Monday or Saturday.

With a duration, `within 3 days` means newer than three days ago and `older than 1 year` means older. A number of days with neither, such as `modified 2026 days`, is a problem: "Filter problem: say within or older than", then "Word 3. Try modified within 3 days." The signs follow the same idea: `modified > 3 days` reads back as "modified older than 3 days".

The read-back always spells the real days, so you hear what a word meant today: "modified last week, October 4 to 10".

Dates use this computer's current offset from UTC. A file changed in the hour a clock changed may land on the wrong side of midnight.

## What is not in the filter

Where to start, whether to follow links, whether to read ignore files, whether to stay on one drive and whether to look inside archives are the scope. The scope sits beside the filter in the Search dialog, in a saved search and on the command line.

## Twenty examples

Each example gives the text you type and what diroxide reads back. These are the read-backs diroxide really gives, on a computer clock of Saturday, October 17, 2026, with weeks that start on Sunday.

- `report`: "name contains report".
- `*.log`: "name matches *.log".
- `*.txt, *.md`: "name matches *.txt or *.md".
- `re:^IMG_\d{4}`: "name regex ^IMG_\d{4}, match case". The capital letters forced the exact match.
- `size over 100 MB`: "size over 100 MB", and the details line "100 MB is 100,000,000 bytes."
- `size between 1 KiB and 1 MiB`: "size from 1 KiB to 1 MiB".
- `modified today`: "modified today, October 17, 2026".
- `modified last week`: "modified last week, October 4 to 10".
- `ext pdf and modified within 3 days`: "extension is pdf and modified within 3 days", with the details "3 days ago: October 14, 2026, 12:48 PM."
- `modified older than 1 year and size over 1 GB`: a clean-up finder for old, large files.
- `kind folder and empty`: "kind is folder and empty". Empty folders.
- `kind link and broken`: "kind is link and broken". Links whose target is gone.
- `needs-care`: names that need care, such as hidden characters, trailing dots or spaces and device names.
- `label urgent and not label done`: "label is urgent and not label is done".
- `group images and size over 5 MB`: "group is images and size over 5 MB". Large pictures, by extension.
- `content:"total due"`: files that hold the text total due. It reads back as content contains, then the text in quotes.
- `content:re:"due \d+" and ext txt,md`: regular-expression content in two kinds of file.
- `hidden and kind file`: "hidden and kind is file, including hidden".
- `depth at most 1 and name *.rs`: direct children only. Depth works in searches, not in a panel.
- `(ext jpg or ext png) and not name thumb*`: "(extension is jpg or extension is png) and not name matches thumb*".

More forms that help: `path "/work/" and draft` (the path holds work and the name holds draft), `bookmarked or label urgent`, `selected and size under 1 KB` (narrow a selection), `owner Alice` (Unix) and `modified between 2026-10-01 and 2026-10-07`.

## Reading it back

The read-back is one sentence in a fixed form: field words, comparison words, units spelled out, dates with their real days, and "match case" or "including hidden" when those apply. Typed forms never matter. You hear the same sentence for `size>1mb` and `size over 1 MB`.

The read-back is the title of the panel's filter, the name of a saved search that has none, and what `diro search --explain` prints. Say filter, in the View menu (Alt+V, then Y) and the palette, says it on request. Parsing the read-back gives the same filter again; a test checks that.

## When a filter has a problem

Every problem starts "Filter problem: " and puts the meaning next, so the first 40 cells carry it. A second sentence says which word and what to try. The second sentence is said once, after the typing pause, never on every key.

- "Filter problem: size needs a number". Then "Word 3. Try size over 10 MB."
- "Filter problem: unknown field 'sise'". Then "Word 1. Did you mean size?"
- "Filter problem: unknown unit 'mbb'". Then "Units: B, KB, MB, GB, TB, KiB, MiB, GiB, TiB."
- "Filter problem: nothing after and". Then "Add a test, such as *.txt."
- "Filter problem: date not understood". Then "Try today, last week or 2026-10-01."
- "Filter problem: bad regular expression". Then the reason, for example "The pattern is not valid: unclosed group."
- "Filter problem: content needs text". Then an example to copy, `content:"total due"`.
- "Filter problem: depth works in searches". Then "A panel shows one folder; search to go deeper."
- "Filter problem: duration needed". Then "Try within 3 days or older than 1 year."
- "Filter problem: unknown kind 'thing'". Then the kinds. "Filter problem: unknown group 'pictures'" names the groups.

diroxide also notices filters that were probably not meant and says so once. Two globs joined by a space cannot both match one name (`*.txt *.md`): "Both patterns must match one name. Use a comma or or." A label that does not exist: "No label named urgnt. Did you mean urgent?" A range that runs backward: "The range runs from high to low." These are hints. The filter still runs.

## The panel filter: Ctrl+S

Ctrl+S opens the Filter prompt in the active panel. Type any filter from this guide. A bare word still works, so `report` filters by name as before. Tab completes field names, label names, groups and units.

- While you type, the list narrows silently. Nothing is spoken for each key.
- After you stop for 400 milliseconds, one line says the count and the first match: "37 matches, first report.pdf", or "1 match, report.pdf". With none: "No matches". The pause is the `filter_pause_ms` setting, 0 to 5,000. The focus is on the first match.
- If some entries could not be decided, the line adds how many and why: "1 match, report.pdf. 2 unknown, folder size not counted".
- A problem is said in two parts. After the pause, the first sentence: "Filter problem: size needs a number". After one more pause, the second: "Word 3. Try size over 10 MB." Each is said once. The panel stays as it was while the filter cannot be read.
- A hint about a filter that was probably not meant, such as two globs joined by a space, is added to the count line once in each prompt.
- Enter keeps the filter: "Filter on, 4 of 9 shown". The title line reads it back: "filter: size over 1 KB and name matches *.log".
- Escape inside the prompt puts back the filter the prompt opened with: "Filter as it was, 4 of 9 shown", or "No filter, 9 shown".
- Escape in the panel, with nothing else to stop, takes the filter off: "Filter off, 9 shown". Clear filter, in the View menu and the palette, says the same.
- Hidden entries follow the panel's setting, unless the filter says `hidden`.
- A depth test is refused, because a panel shows one folder: "Filter problem: depth works in searches".
- With single-letter keys on, typing a bare word starts the same prompt.

Ctrl+D (Say status) inside the prompt says the text, how it reads, and the count: "Filter: size over 1 KB. Reads as size over 1 KB. 4 of 9 shown".

### A filter that reads files

A test on `content` has to open the files. While you type, the pause says "Content test: Enter asks to read files". Enter then asks, starting on No: "Read the contents of 6 files to filter? No is the default. y or n". Answer y and the files are read: "Reading the files for the filter". Answer n and "Nothing was read. The filter is as it was."

### Say filter, stash and the quick toggles

These are in the View menu and the palette. They have no keys of their own.

- Say filter (Alt+V, then Y) reads the panel's filter back in one sentence, with the real days of date words and the exact byte counts: "modified last week, October 4 to 10". In the filter prompt it reads the text you are typing.
- Stash filter (Alt+V, then K) keeps the filter aside and shows everything: "Filter stashed, 9 shown. 1 stashed". Restore filter (Alt+V, then E) brings the newest one back: "Filter restored, 4 of 9 shown", then its sentence.
- Folders only (Alt+V, then D) shows only the folders: "Folders only, 3 of 9 shown". Files only (Alt+V, then I) shows only the files and archives. Choose it again to go back to the filter before it.

## Select by pattern, and the other ways to mark

Plus (in a terminal, with single-letter keys on, and in the Select menu) asks for a filter, starting as `*`: "Select by pattern: *". Before it marks anything it counts and asks, starting on No: "4 entries will be selected. No is the default. y or n". Then "4 selected". Minus unselects the rows that match. When nothing is left to change: "No more entries match. Nothing changed." or "No selected entry matches. Nothing changed." The word `selected` lets a second pass narrow the first.

The Select menu (Alt+S) has more ways to mark, all on the filter language:

- Select by kind (K) lists the kinds and file-type groups: Folders, Files, Links, Archives, and each group. Choose one: "Selected 3 folders. 3 selected."
- Select by size (Z) opens the pattern prompt on `size over `, to finish with a size such as 1 MB. Select by date (D) opens it on `modified within `, to finish with a time such as 3 days.
- Select same extension (E) marks every file with the focused file's extension: "Selected 4 with .md. 5 selected."
- Mark range (G) starts a range at the focused row: "Range started at report.pdf. Mark range again ends it." Again, at another row: "Marked 5, report.pdf to notes.txt. 5 selected."
- Select next rows (W) asks "Select the next rows, how many" and marks that many rows from the focus.
- Restore selection brings back the marks a select replaced.

Selecting by file contents is not built yet. It says "Selecting by contents is not built yet. Filter by contents, then select all."

## Find files and the results list

Find files searches names and contents in the folders below the one you are in, or in the entries you marked. The results are a list of real files, and every command that makes sense works on them.

Press Ctrl+F, or choose Find files from the Search menu (F9, then R, then F) or the palette. Ctrl+F opens the Search dialog.

### The Search dialog

The dialog is a list named Search. The first row says what will happen, and Enter on it starts the search: "Search work for *.txt". Then comes one setting a row, with its meaning first:

- "Name or filter: *.txt". The filter. Empty means every entry.
- "Contains text: none". Text to look for inside files.
- "Look in: work". The folder, or "3 marked items".
- "Hidden files: left out" or "searched".
- "Ignore files: not read" or "read, ignored files left out".
- "Links to folders: not followed" or "followed".
- "Inside archives: not searched" or "searched".
- "Depth: any" or "2 levels down".
- "Rows: one per file" or "one per matching line".
- "Online-only files: left out" or "read after asking".

Enter on a row changes it. Typing a letter on a text row opens its field with that letter in it. Enter in the name or text field starts the search.

### The scope

- It starts in the active panel's folder, or in the marked entries.
- Hidden entries follow the panel's setting. The `search_respect_ignore` setting sets whether ignore files are read.
- Links to folders are not followed, a search stays on one drive, archives are not entered, and the depth is not limited, until you change those rows.
- The folders that a label called Skip in searches marks are left out.

Whatever a search skipped is named in the summary.

### While it runs

Results arrive in the order they are found and never move under your cursor. The cursor sits on the first row once there is one, and stays.

- diroxide says what it is doing once: "Searching work for *.txt", or "Searching work for *.txt, containing total due".
- The status line counts: "Searching: 214 found, 3,120 folders". In screen-reader mode it is spoken at the progress interval, every ten seconds by default (`progress_interval_seconds`), not on every change.
- Escape stops the search: "Stopped at 214 found." F2 pauses: "Paused at 214 found." F2 again: "Searching again, 214 found so far." In another list, F2 says "No search is running."
- A folder blocked on a dead share cannot be interrupted. The stop message then says "1 folder is still being read." and the search lets go of it.
- When it ends: "Done: 4 found in 3 folders." The skips follow in words: "Hidden files skipped."
- The list holds at most 100,000 rows (the `search_row_cap` setting, 1,000 to 10,000,000). Past it: "Showing the first 100,000. Narrow the search." The count keeps counting.

Skips are said as sentences, meaning first: "Hidden files skipped.", "Files in ignore files skipped.", "2 links to folders not followed.", "1 folder on another drive skipped.", "1 folder unreadable.", "6 binary files skipped.", "3 online-only files skipped, 1.4 GB.", and "3 unknown, folder size not counted."

On Windows a leading dot does not make a file hidden, so "Hidden files skipped." appears there only for files with the hidden attribute.

### A result row

A row starts with the name, then what the hit is, then where it is, with the position last: "notes.txt, in work, 4 KB". A folder: "q3, folder, in Documents". A content hit says the line: "invoice.txt, line 3: total due now". With more matches in the file: "report.txt, line 42: total due, 3 more, in Documents/q3".

The keys in the results:

- Enter opens a file with its program ("Opening report.txt") and a folder in the panel.
- F3 views the file at the match: "At line 3 of 4". Escape from the viewer: "Back in the results, invoice.txt". F4 edits at the line. The line number goes to vi and its family, nano, Emacs, micro, Kakoune, joe, ne and mcedit.
- F5 and F6 copy and move the marked results, or the focused one, to the other panel. F8 or Delete moves them to the trash, with the usual question, starting on No. Shift+Delete deletes for good.
- Space marks a result: "Selected notes.txt. 1 selected."
- Backspace shows the result in its folder, with the entry focused. The Search menu has Show in its folder too.
- Ctrl+S refines the results with a filter, without searching again: "Refined: 4 of 9 shown". Refining by file contents is not built yet: "Refining by contents is not built yet. Search again with Contains text."
- Remove from results, in the Search menu, takes the focused row out of the list and leaves the file: "Removed notes.txt from the results. The file stays."
- Shift+F10 lists the actions on the results, each with its key.
- Escape closes the list once the search has ended: "Results closed. Search results opens them again." Search results, in the Search menu, opens them again. Find files in the list starts a new search.

Rename and labels do not work from the results list. Show the result in its folder first.

A row's file is checked when the focus has rested on it for 300 milliseconds, and before an action. A file that is not there says "notes.txt, gone since this list was made", and the action is refused in words. A changed file says "changed since this list was made".

### Searching file contents

A content search looks inside files. Type text in the "Contains text" row, or use `content:"total due"` in the filter.

- Smart case applies, as in names. Capitals match exactly.
- A file that holds a NUL byte looks binary. It is skipped and counted: "6 binary files skipped." A UTF-16 file with a byte-order mark is searched. One without a mark holds NUL bytes and counts as binary.
- One row per file by default, with the first matching line and a count of the others. "Rows: one per matching line" gives a row for each match, and `--lines` does on the command line.
- `first 64 KB` after a content test reads only the start of each file.
- Online-only files are left out by default, because reading one downloads it. Set the row to "read after asking" and diroxide asks once for the search, starting on No: "report.pdf is online only, 1.4 MB. Download online-only files to search them? No is the default. y or n". The ones left out are counted in the summary: "3 online-only files skipped, 1.4 GB."

### Over a network

A search needs no index, so a network place costs a round trip for every folder. diroxide reads such a place with two threads, says the rate at the progress interval, "Searching over the network: 400 items a second", and does not search file contents there without your yes. Escape stops it within a second. For a very large share, the server's own search is faster.

## Saved searches

Save this search, in the Search menu and the palette, keeps the last search under a name: "Save the search as", then "Search saved as Text files. Saved searches runs it." It keeps the filter as you typed it, the contents, the scope and the sort, in a file in the state folder.

Saved searches, in the Search menu and the palette, lists them: "Saved searches, 1 search, Text files, saved search, runs on open, 1 of 1". Enter runs the one you choose, and date words are read again each time: "Running Text files: Searching work for *.txt", then "Done: 4 found in 3 folders." Delete asks first, starting on No: "Delete saved search Text files? No is the default. y or n", then "Deleted saved search Text files".

A saved search can be run and deleted. It cannot be renamed or edited yet: save it again under a new name.

## Where the space went

The Space list answers "what is taking my disk". Space list, in the Search menu and the palette, counts everything below the folder: "Counting the space in work". It counts every file, hidden files included, links not followed, and a hard link once on Unix. It starts with one sentence, at most 40 cells: "Total 66.4 GB. Largest 3 hold 81 percent". Then one row for each of the largest items: "Videos, folder, 41.2 GB, 62 percent". It ends with the rest: "And 340 smaller items, 2.1 GB, 3 percent". At most 30 rows are listed.

- Enter on a folder row goes into it. Backspace goes up. On anything else: "Not a folder: only folders open here." The Sort list works.
- Space by kind groups the files by their file-type group: images, music, video and the rest, by extension. Space by age groups them by when they last changed: "changed in the last year", "not changed in a year", "not changed in two years".
- Largest items below here lists the 50 biggest files below a folder, as search results, so every key of a result works on them. It says "Looking for the largest items below work" and then "Done: the largest 50 below work."
- Folder sizes counts the marked folders, or the focused one, with everything inside: "Sizing Photos", then "Photos: 5.0 KB in 2 files and no folders". Where it could not read everything: "At least 12.3 GB, 2 folders unreadable". On Windows hard links may be counted more than once, and the line says so. Cloud folders give "total" and "on this computer".
- Size every folder counts each folder in the panel, then offers the size sort: "Sized 2 folders, 5.0 KB in all. Sort by size? No is the default. y or n". Until then the Size column says the folder's size is not counted.
- Nothing is drawn as a treemap. It is words.

## Newest files

Newest files, in the Search menu and the palette, lists the files changed on the latest day that has any, as search results. The title says the day: "Newest files: 12 changed on October 8, 2026". Earlier day steps back to the day before that has files. With nothing found: "Newest files: none found". Earlier day anywhere else says "Earlier day works in Newest files' results."

## The Search menu

F9, then R, opens the Search menu. Its letters: f Find files, r Search results, p Pause search, i Show in its folder, m Remove from results, s Save this search, v Saved searches, c Space list, k Space by kind, a Space by age, l Largest items below here, z Folder sizes, e Size every folder, n Newest files, d Earlier day, and o the Compare submenu ([Compare files](compare.md)). Only Find files has a key of its own, Ctrl+F. The Total Commander preset also has Alt+F7, and the Explorer and Finder preset Ctrl+E ([customizing keys](customizing-keys.md)).

## diro search

`diro search` is Search for a shell. It prints one row a line, then a summary, and works in any terminal, over any connection and in a script.

```bash
diro search docs size over 1 MB and ext pdf
```

The place comes first, then the filter. The words after the place are joined into one filter, so quotes are not needed unless the shell wants them. A shell treats `*`, `(`, `)`, `;`, `&`, `|`, `>` and `<` as its own. Put a filter that holds one of them in single quotes, and prefer words to signs: `size over 1 MB`, not `size > 1 MB`, which would redirect the output to a file named `1`.

```bash
diro search docs '(ext jpg or ext png) and not name thumb*'
```

The first line says what is being searched: "Searching diro-demo for ext md". Each row is the name, where it is and the size: "report.md, in diro-demo, 649 bytes". A folder says "folder". The last line says the count and what was skipped: "1 found." or "1 found. 1 unknown, folder size not counted."

A filter that is wrong stops with the same two sentences the panel says, and exit code 2:

```bash
diro search docs size over
```

Filter problem: size needs a number. Word 3. Try size over 10 MB.

### Options

Several of fd's habits are accepted. Each one turns into a part of the filter, and `--explain` shows it.

- `--hidden`, `-H`: search hidden entries too.
- `--ignore-files`: skip what `.gitignore` and `.ignore` files list. `--no-ignore`, `-I`: read no ignore files (the default).
- `--follow`, `-L`: follow links to folders. `--archives`: search inside archives.
- `--max-depth N`, `-d N`: at most N folders deep, as `depth at most N`.
- `--type T`, `-t T`: `f` file, `d` folder, `l` link, `x` executable, `e` empty.
- `--extension E`, `-e E`: this extension.
- `--size S`, `-S S`: `+10M` is "size over 10 MB" and `-1ki` is "size under 1 KiB".
- `--changed-within T`: a duration such as `2d`, or a date such as `2026-10-01`. `--changed-before T`: a date, or a duration such as `1y`.
- `--exclude GLOB`, `-E GLOB`: leave out names that match, as `and not name GLOB`.
- `--content TEXT`: files that hold the text, smart case. `--content-regex RE`: files that hold a match. `--lines`: one row for each matching line.
- `--glob`, `-g`: read the filter as one glob. `--regex`: read it as one regular expression. `--full-path`, `-p`: match against the whole path.
- `--ignore-case`, `-i`, and `--case-sensitive`, `-s`.
- `--null`, `-0`: write NUL-separated paths only, for `xargs -0`. `--json`: one JSON object a line.
- `--explain`: read the filter back and search nothing.
- `--threads N` and `--max-results N`: the count goes on past the limit.

```bash
diro search docs --content "total due" --extension txt
```

`--explain` prints the read-back and the scope:

```bash
diro search docs modified last week --explain
```

Reads as: modified last week, October 4 to 10. The next lines show the tree, then the scope: the folder, "hidden entries skipped; ignore files not read; links not followed; one drive; archives not entered; any depth." Then "Searched nothing: --explain only reads the expression back."

### Exit codes

- 0: found something.
- 1: found nothing.
- 2: wrong use, or a bad filter.
- 3: partly done. Some folders or files could not be read, and the output says which.

`diro diff` follows the same idea: 0 for the same, 1 for different, 2 for trouble.

## Limits, said in words

- Patterns are Rust regular expressions: no look-around and no backreferences.
- Dates use the computer's current offset from UTC, with no daylight-saving rules.
- Types that have not been checked do not match. Groups go by extension until alpha.7.
- Owner and user group are read where the listing has them. Windows owners are not read in a listing.
- A folder has no size until it is counted.
- Names are compared as stored. Accents are not folded.
- Content search reads files, and on Windows the cost is opening each one.
- A search has no index. It walks the folders. Instant answers over a whole drive, from an index kept up to date, are not what it does.
- Typing in a panel narrows the listed folder only. A recursive search starts when you press Enter, never on a key.
- A panel's filter that reads file contents runs the search engine over that one folder. `selected` and `bookmarked` stay undecided inside it, and the undecided reasons are counted, not kept for each file.
- Selecting by contents, and refining results by contents, are not built.
- The results list acts on real files with view, edit, copy, move, trash, delete for good, marking, refine and show in folder. Rename and labels do not work from it.
- A saved search can be run and deleted, not renamed or edited.

[Known limits](known-limits.md) keeps the list current.

## See also

- [The panels](panels.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Compare files](compare.md)
- [Watching a folder](watching.md)
- [Customizing keys](customizing-keys.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
