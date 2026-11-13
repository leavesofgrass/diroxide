# Duplicates

This guide covers Find duplicates: finding files with the same contents, choosing which copies to keep, and sending the others to the trash. It says where to look, the options, what you hear while it works, how the groups read, the five rules for keeping a copy, what is checked again before the trash, and `diro dups` for a shell.

The short version: duplicates go to the trash and nowhere else. Every group keeps at least one copy, and diroxide never marks a whole group. Nothing is marked until you choose, and nothing leaves a folder until you answer a question that starts on No.

What this guide says comes from the design and from the tests that check each sentence. Nothing here had been heard on a screen reader or read on a Braille display when it was written; if your experience differs, that is a finding.

## How a duplicate is found

Two files are duplicates when their contents are the same. diroxide does not trust names, sizes or times for that. It works in steps, each cheaper than the next:

1. It groups files by size. A file with a size no other file has cannot be a duplicate and is never read.
2. It reads the first and last blocks of the files that are left, and drops the ones that differ.
3. It reads the rest in full and takes a BLAKE3 hash, the same one the Checksum command uses. Files with the same hash are a group.
4. With the byte check on, it compares the bytes of whole files too, so a hash that matched by chance cannot cost you a file.

Left out on purpose, and said: empty files ("1 empty file left out.") and several names of one file, a hard link, which are one file and not a copy. Hidden files follow the Hidden files option, and when they are left out they are left out before anything is hashed or counted, so they cost no reading and are not in the estimate: "3 hidden files left out".

## Find duplicates

Find duplicates is in the Tools menu (Alt+T, then U) and in the palette. It has no key of its own. It asks two things first, each a list you walk like any other.

### Where to look

The list is named "Find duplicates where". Its rows:

- "Marked: 3 entries", when entries are marked. Files are read, and folders are searched whole.
- "This folder: PATH", the active panel's folder and every folder below it.
- "Both panels: a and b", both folders as one search, so a copy in one panel is found against a file in the other.

Enter on a row chooses it.

### The options

The next list is named "Find duplicates options". The first row runs the search, and the others change an option each. Enter on an option changes it and says the new row.

- "Find now: PATH" starts the search.
- "Least size: any size", or "Least size: 1.0 KB or more", and so on up to 100 MB. Enter steps to the next size.
- "Byte check: off, hashes only" or "Byte check: on, slower".
- "Hidden files: left out" or "Hidden files: read". The setting `duplicates.hidden` decides the start: `panel` (the default) follows what the panel shows, `include` always reads them and `leave_out` never does.

Reading many big files takes time, so a search that would read more than `duplicates.ask_above_gb` asks first, 10 GB by default: "Read 400 KB to find duplicates? 2 files. No is the default. y or n". With Yes, it runs the listing and the quick check again and then reads.

## While it works

The search is a job. The jobs line starts "Finding duplicates in PATH", with no closing period. When the line is said aloud it ends with one, "Finding duplicates in work.", so it is not run into the count line that follows; "Checking 1 copy again." does the same. Progress is said at the progress interval: "Listing, 1,200 files found", then "Hashing 120 of 450 files". The engine's own line for the same stage reads "Duplicates: hashing, 38 percent". Cancel in the Jobs list, or Escape, stops it: "Finding duplicates was cancelled." Pause does not reach the search; Cancel does.

## The groups

When it finishes, the groups open as a list, the largest waste first. The count line is said first: "1 group, 2 files, 11 bytes to free". Notes follow it: "1 empty file left out.", "3 hidden files left out".

Each group is a row with its meaning first: "Group 1 of 1, 3 copies, 2.0 KB each, 4.0 KB to free", with "none marked" or "2 marked" after it. The two rows at the end of the list are "Choose copies to keep" and "Trash the marked: none marked yet", which becomes "Trash the marked: 1 file, 11 bytes" as you mark.

Enter on a group lists its copies. The list is named "Group 1 of 3, 3 copies". Each row says what will happen to the copy and then where it is:

- "Kept: a.txt, in PATH"
- "Marked: b.txt, in PATH"

Space or Enter on a copy changes its mark and says it once: "Marked b.txt" or "Kept b.txt". A group must keep one copy: when you try to mark the last kept one, "One copy must stay." The last row, "Show these in the results list", opens the group's files in the general list, each with "kept" or "marked" after its name.

Escape goes back a level. The marks live in the open dialog: when you leave the groups list with Escape they are gone, so choose and trash in one visit.

## Choose copies to keep

"Choose copies to keep" is a list of five rules. Enter applies one to every group at once, and says what it did: "Marked 2 files, 4.0 KB. Kept the oldest in 1 group."

- "Keep the oldest": the default. Where a copy's time is not known, that group is left alone.
- "Keep the newest".
- "Keep the shortest path".
- "Keep the copies in FOLDER": the copies directly in the folder you named stay. A group with none there, or with every copy there, is left alone.
- "Keep the first by name".

A rule never marks a whole group. A group it leaves alone is said, with the reason. A tie is settled the same way each time: the shortest path, then the first found. "No rule chosen; the marks stay." is what Escape says. You can change any mark by hand afterward.

## The trash

"Trash the marked" asks first, with the count and the size, starting on No: "Trash 1 copy, 11 bytes? No is the default. y or n". n or Escape says "Nothing moved to the trash."

Before it asks, every kept and every marked copy is read again. The listing begins "Checked again: 1 file can go to the trash". A group whose kept copy changed since the search is refused whole, so a copy cannot go while its last twin has changed: "Group 1 refused: the kept copy, new.txt, changed".

When the job ends, the report starts with the bytes freed: "Freed 4.0 KB: moved 2 items to the trash. They can be restored."

The trash is the only way out. Duplicates are never deleted for good, and Undo brings the files back from the trash: [volumes and the trash](volumes-trash.md) says how the trash works on each system.

## From a shell

`diro dups PATH...` finds duplicates and prints them in the same words. The count line comes first ("3 groups, 7 files, 44 KB to free"), then the notes on what was left out, then each group's row and its files, name first.

```bash
diro dups photos backup
```

- `--keep RULE` says what would be marked, keeping one copy in each group, one line for each file ("Marked: NAME, in FOLDER"), and the groups it left alone. The rules are `oldest`, `newest`, `shortest`, `name` and `first`.
- `--keep-in FOLDER` keeps the copies in FOLDER and marks the others.
- `--format FORMAT`: `text` (the default), `tsv` (group, size, path) or `json`, one object for each group.
- `-0` or `--null` prints only paths, separated by NUL, every member or with `--keep` the marked ones, for a pipeline.
- `--empty` groups empty files too, and `--bytes` compares bytes after the hash.
- `--yes` or `-y` reads without asking. A search of more than 10 GB asks first, starting on No.

Nothing is removed from a shell in this alpha. The marked files are a list to feed to another tool or to the trash in the app: "Nothing was removed." Ctrl+C stops the search. The exit codes are 0 for done, 1 for could not run, stopped, or a large read not confirmed, 2 for wrong use and 3 for done but with some files that could not be read.

## Limits

- No search inside archives, and no cache of hashes: a second search reads the files again.
- A collection is not a scope yet. The scopes are marked entries, this folder and both panels.
- "Keep the copies in FOLDER" matches the files directly in that folder, not in its subfolders.
- The marks are gone after Escape from the groups list.
- [Known limits](known-limits.md#duplicates) has the same list.

## See also

- [Checksums and verifying](checksums.md)
- [Synchronize](synchronize.md)
- [Volumes and the trash](volumes-trash.md)
- [Settings](settings.md)
- [Every key](keyboard.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
