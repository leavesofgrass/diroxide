# Compare files

This guide covers Compare files: what two files differ in, said in words before any change is shown. It describes which two files diroxide compares, what you hear and in what order, how to walk the changes, the options that leave differences out, binary and large files, and `diro diff` for the command line. It comes with alpha.3. Comparing two folders comes with alpha.8.

What this guide says comes from the design and from the tests that check each sentence. Nothing in alpha.3 had been heard on a screen reader when this guide was written, so the lines below are what diroxide is built to say. [The accessibility statement](accessibility.md) says what has been checked.

## Why it reads the way it does

A comparison that is drawn as two columns side by side is hard to hear. This one tells you the answer first: are the files the same, how many changes, and in which direction. Then it gives the changes as a list, one row each, which you can walk with the same keys as any list. Only if you ask does it show a change in full or the whole file. When two files differ in something you cannot see, such as a tab where the other has spaces, the row says so in words.

## Which two files

Compare files is in the Search menu's Compare submenu (F9, then R, then O, then F) and in the palette. It has no key of its own. The submenu's letters are f Compare files, p Compare properties, d Select the differences, n Next change and r Previous change. The Total Commander preset puts Select the differences on Shift+F2 ([customizing keys](customizing-keys.md)).

- If exactly two files are marked in the active panel, those are compared, the first in list order as "before".
- Otherwise the focused file here is compared with the file of the same name in the other panel.
- If the other panel has no file of that name, it is compared with the other panel's focused file.

Before it starts, the status line says which two. For two marked files: "Comparing old.txt with report.txt". For one file in each panel: "Comparing report.txt here with old.txt there".

The direction is from the first file to the second. For one file in each panel, that is from the other panel's file to this panel's, and the report says so: "From the other panel to this panel." For two marked files it names them: "From old.txt to report.txt." Swap turns it around and says so: "Swapped. From this panel to the other panel." Nothing is compared again when you swap.

Two folders, a folder and a file, or a file inside an archive are not compared in alpha.3. The message says what to do: "Could not compare D:/diro-demo: it is not a file. Choose two files to compare." or "Mark two files, or focus a file in each panel, to compare." or "No file to compare report.txt with in the other panel. Focus one there, or mark two here."

## What you hear, in order

1. The verdict, in one line of at most 40 cells: "5 changes between old.txt and report.txt". When the names are too long to fit, the names are cut with three dots in the line, and the full names are spoken: "1 change between bef...txt and after.txt".
2. The counts: "Changed 3, added 1, removed 1."
3. The direction: "From the other panel to this panel."
4. What was left out, if anything: "Ignoring changes in spaces." and "1 line differs only in what was ignored."
5. Facts about the whole files: "Line endings differ: LF in before.txt, CRLF in after.txt." "Byte-order mark in after.txt, none in before.txt." "Encodings differ: UTF-8 in before.txt, UTF-16 in after.txt." "Last line has no line break in after.txt."
6. If the comparison ran out of time: "Stopped after 2 seconds; the changes may be more than the least possible."
7. The change list.

The verdicts are:

- "Same: a.txt and b.txt". The two files are the same, byte for byte.
- "Same text: a.txt and b.txt". The files are not the same byte for byte, but their text is the same once what is left out is left out. The next sentences say what differs, such as a byte-order mark.
- "Same file: a.txt". Both sides are the same file.
- "5 changes between a.txt and b.txt".
- "Binary, 4 bytes differ: a.bin and b.bin", "Binary, 1 byte differs: ..." and "Binary, sizes differ: ..."
- "Too large to compare as text: ..." and "Changed while comparing: ..."
- "Not compared: ..." when you cancel. A cancelled compare never leaves a result that looks clean.

## The change list

The list has one row for each change. A row says what happened, where, and the first thing that matters. These are the rows diroxide gives for two near-identical files:

- "Removed line 12, Copyright 2025"
- "Changed line 19, was: ...in Portland opened early and the crows..., now: ...in Portland opened late and the crows..."
- "Changed line 29, trailing spaces added"
- "Changed line 39, was: total due, now: total owed"
- "Added lines 51 to 53, 3 lines"

A change inside a very long line is shown as a window around the difference, cut with three dots, so the row stays short. A change of several lines says how many: "Changed lines 40 to 41, 3 lines became 2".

The change list is a list like any other, and the position is said last: "Changed line 39, was: total due, now: total owed, 2 of 5". Its title is the verdict, at most 40 cells.

- F7 goes to the next change and Shift+F7 to the previous one. N and P do the same while single letters are on. Down and Up, Home and End work as in any list. At the ends: "No change after this one" and "No change before this one".
- Enter says one change in full. U opens the unified view. S swaps the direction.
- F3 views the file at the change. Shift+F10 lists the actions on the compare, each with its key.
- Escape closes the list.

Some terminals send Shift+F7 as another key. P and the actions list are the sure way back. The Compare submenu of the Search menu is there to reach every command, but it cannot open while a compare is open.

### One change in full

Enter on a row shows that change in full, as a short list you read line by line: its header, then the lines before and after, then what differs.

- "Change 4 of 5, line 40 before, line 39 after"
- "Before, line 40: total due"
- "After, line 39: total owed"
- "Differs: due became owed"

Escape goes back one level.

### The unified view

U opens the unified view: the changed lines with a few unchanged lines around them, in the order of the file. Its title says the file: "Unified view, report.txt". Each row starts with its kind and line number: "Same 11: Item 11: 33 widgets shipped", "Removed 12: Copyright 2025", "Added 19: ...". A header row says where the next change is: "Change 1 of 5, line 12 before, none after". Long runs of unchanged lines collapse: "9 same lines left out". The context is two lines on each side; it is the `compare.context` setting, 0 to 9.

Enter or F3 on a row opens the viewer at that line ([the viewer](viewer.md)). The unified view is a list, not a mode of the viewer.

### Sounds the same, and is not

Some differences cannot be seen on a screen or heard in speech. Compare files says what they are, so nothing stays a mystery. The nine cases:

- Case: "Changed line 3, case only".
- A tab and spaces: "Changed line 3, tab became 4 spaces".
- Trailing spaces: "Changed line 3, trailing spaces added".
- A hidden character: "Changed line 3, hidden character U+200B added".
- The same text in another Unicode form: "Changed line 1, same text in another Unicode form". It covers Latin, Greek, Cyrillic and Hangul.
- Line endings: "Changed lines 1 to 4, line ending changed from LF to CRLF", when line endings are compared.
- A byte-order mark: "Byte-order mark in after.txt, none in before.txt."
- Another text encoding: "Encodings differ: UTF-8 in before.txt, UTF-16 in after.txt."
- A missing final line break: "Last line has no line break in after.txt."

The last four are facts about the whole file. They do not make a change row. Line endings are ignored by default, so two files that differ only in them read "Same text".

## Leaving differences out

Six options leave differences out. Each one that is on is said in the report, so a hidden rule never shapes the result in silence. They are settings, in the Compare group of Settings (Alt+T, then S):

- `compare.ignore_line_endings` (on): CRLF, LF and CR compare the same. Not said unless the files differ.
- `compare.ignore_trailing_space` (off): space at the end of a line.
- `compare.ignore_space_amount` (off): any run of spaces and tabs counts as one.
- `compare.ignore_all_space` (off): every space and tab.
- `compare.ignore_blank_lines` (off): changes whose lines are all blank.
- `compare.ignore_case` (off): capital and small letters.

The line "Ignoring changes in case." and its sibling, "Ignoring changes in trailing spaces, case and blank lines.", say which are on. Lines that differ only in what was ignored are counted: "1 line differs only in what was ignored."

Five more settings shape the run:

- `compare.algorithm` (patience): how lines are lined up. Patience hears fewer false changes around repeated lines such as closing braces. `histogram` suits logs. `myers` is the classic and finds the fewest changed lines, which can split a change around a repeated line.
- `compare.deadline_ms` (2000; 100 to 60000): how long a compare may look for the fewest changes. Past it, the result is correct but may list more changes than the least possible, and the report says so.
- `compare.text_limit_mib` (16; 1 to 1024): the largest file compared as text in full.
- `compare.large_files` (ask): what to do with a file over that limit. `ask`, `head` (the first part as text) or `bytes` (byte by byte).
- `compare.context` (2; 0 to 9): the same lines shown around a change in the unified view.

[The settings reference](settings-reference.md) lists each with its help.

## Binary files, big files and changing files

A file that looks binary (the viewer's test, so a UTF-16 file with a byte-order mark counts as text) is compared byte by byte. The report says how much differs and where: "Both 27 bytes; 4 bytes differ in 3 runs." "First difference at byte 1." Each run is a row: "Bytes 1,266 to 1,269 differ, 4 bytes" or "Byte 1,266 differs", and Enter spells it in hex: "was 48 65 6c 6c, now 48 66 6c 6c". Files of different sizes say so: "Sizes: 150 bytes here, 100 bytes there." and "The first 100 bytes are the same; one file goes on past them."

A text file over 16 MiB is too large to compare as text: "Over the limit of 16 MiB. Compare the first 16 MiB as text, compare byte by byte, or cancel." With `compare.large_files` on `ask`, diroxide opens a list named Too large with three rows, the focus on the last: "Compare the first 16 MiB as text", "Compare byte by byte" and "Cancel, compare nothing". Cancel says "Nothing was compared." On the command line, `--head` compares the first 16 MiB as text. Without a choice it compares byte by byte.

If a file changes while it is compared, diroxide stops and does not trust the result: "Changed while comparing: report.txt", then "Compare again when they stop changing." If both changed, "Both files changed while comparing."

Escape cancels a running compare, which says "Not compared". A long compare says its progress at the progress interval, "Comparing, 40 percent", or "Comparing, 3.0 MB read" when the size is not known.

## Compare properties and Select the differences

Compare properties is in the Compare submenu (F9, then R, then O, then P) and the palette. It lists what differs between the two files among size, times, type, permissions and owner, one fact a line, and what is the same: "Properties of report.txt and old.txt: 2 properties differ. Size: 1.2 KB here, 1.1 KB there. Same: type, permissions". It reads the two files from the panels' listings, so both must be listed.

Select the differences (Compare submenu, D; Shift+F2 in the Total Commander preset) looks at the two folders in the panels and marks, in both, the entries only here, the entries only there, and those that differ in size or are newer: "Marked 9: 4 only here, 2 only there, 3 different". When there is nothing to mark: "No differences: both panels hold the same names, sizes and times". It does not look inside subfolders.

## diro diff

`diro diff BEFORE AFTER` is Compare files for a shell. It prints the same report: the verdict, the counts, the direction, then one row for each change.

```bash
diro diff old.txt report.txt
```

5 changes between old.txt and report.txt. Changed 3, added 1, removed 1. From old.txt to report.txt. Then the five rows above.

The direction is from the first file to the second. `--brief` or `-q` prints the verdict line only. `--format` chooses `words` (the default), `unified` or `json`.

```bash
diro diff --format unified old.txt report.txt
```

Options that leave differences out:

- `--ignore-case`, `-i`.
- `--ignore-trailing-space`.
- `--ignore-space-change`, `-b`.
- `--ignore-all-space`, `-w`.
- `--ignore-blank-lines`, `-B`.
- `--line-endings`: compare line endings too. They are ignored by default.
- `--context N`: unchanged lines around each change in the unified form, 0 to 9, 2 by default.
- `--head`: for a file over 16 MiB, compare the first 16 MiB as text.

The exit codes are 0 for the same, 1 for different, and 2 for wrong use or trouble: a file missing, unreadable or not a file, or one that changed while it was compared. A file that is the same file as the other, or whose text is the same, exits 0.

## Limits

- Only two files at a time. Folders compare with alpha.8.
- The unified view is a list, not a mode of the viewer. Enter on a row opens the viewer at its line.
- Text is compared in full up to 16 MiB for each file.
- Unicode forms are recognized for Latin, Greek, Cyrillic and Hangul. "Sounds alike" covers space characters, dashes, quotes, full-width letters and a short list of Cyrillic and Greek letters that look Latin.
- When the names do not fit 40 cells, the verdict line cuts the names with three dots. A verdict that cannot keep four cells for names leaves them out, and the next line has them.
- "Stopped early" is worked out after the run, from the clock.
- Words are compared by position in a line, not by meaning.

[Known limits](known-limits.md) keeps the list current.

## See also

- [The viewer](viewer.md)
- [Search and the filter language](search.md)
- [The panels](panels.md)
- [Customizing keys](customizing-keys.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
