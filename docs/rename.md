# Batch rename

This guide covers Batch rename: renaming many entries by rules, with a preview of every new name before anything changes. It says how to open it, what the rules can do, how the preview reads, how to fix a problem, how to edit the names in your own text editor, how presets and passes work, what is checked again before the run, how to undo it, and `diro rename` for a shell.

The short version: Batch rename never changes a name until you answer a question that starts on No. The preview is the plan, one row for each entry, and a row that cannot be renamed says why in its first words. Nothing is ever overwritten, deleted or moved to another folder. A batch is one job with one undo.

What this guide says comes from the design and from the tests that check each sentence. Nothing here had been heard on a screen reader or read on a Braille display when it was written; if your experience differs, that is a finding.

## Open Batch rename

Mark the entries to rename, then choose Batch rename. With nothing marked, the focused entry is the one. It is in the Tools menu (Alt+T, then B) and in the palette.

- F2 with several entries marked opens Batch rename. F2 with one entry asks for its new name, as it always did.
- Rename one by one is the old stepping, a new name for each marked entry in turn. It is a command of its own, in the File menu and the palette.

A folder in the batch is renamed after what is inside it, so a name inside never goes stale. Entries inside an archive are problem rows, since renaming there rewrites the archive and batch rename does not.

## The rules list

Batch rename opens a list. Its first row is the count line, and Enter on it opens the preview. Then come, in order:

- "Rename now".
- The rules, each as a sentence, such as "Rule 1: Replace IMG_ with trip-, in the name". A later pass puts its number first: "Pass 2: ".
- "Add a rule".
- "Counter: from 1, step 1, 1 digit".
- "Preview shows: old and new names", which Enter turns to "new names only" and "changes only".
- "Make names unique".
- "Skip problems".
- "Edit names in editor".
- "Next pass: more rules after these".
- "Presets: load, save or delete".

After each change the count line is said once, and the rows are not read. When a batch is opened, the first thing said is "Reading the folder first" while diroxide looks at the folders.

### The count line

The count line is the first row of the list and the name of the preview. It says problems first, because they are what stops you:

- "2 problems. 46 of 50 files will change."
- "No problems, 3 need care. 47 of 50 change."
- "3 of 3 files will change. No problems."
- "No file would change."

### The kinds of rule

"Add a rule" lists seven kinds. Each rule edits a part of the name: "the name" (without its extension), "the extension" or "the whole name". A name's extension is what follows its last dot, and `tar.gz`, `tar.xz`, `tar.bz2` and `tar.zst` count as one extension.

- "Replace text": find a text and put another in its place. The text is literal. It replaces the first match, or every match when you choose every. Case can be ignored.
- "Replace a pattern": the same with a regular expression. [The limits](#regular-expressions) are below.
- "Insert text": put text at a position: "Insert x after character 3, in the whole name". A position counts characters as you read them, so a letter with its accents is one. It counts from the start or from the end.
- "Remove characters": take out some from a position: "Remove 4 characters from the end, in the name".
- "Make the name from a template": build the part anew from placeholders: "Make the name from {stem}-{counter}".
- "Change case": "Change case to lowercase, in the extension". The forms are lowercase, uppercase, title case and sentence case.
- "Tidy spaces and characters": trim spaces at both ends, make runs of spaces one, swap spaces for a character and back, or remove every digit, space, punctuation mark or character outside ASCII.

A rule is made by prompts over the list. Escape goes back one step. To change a rule, Enter on it in the rules list.

### Templates and the counter

A template is text with placeholders in braces. The list of placeholders is read in words, "Template, put in:", and Enter on one puts it in. Doubling a brace, `{{`, writes one.

- `{stem}`, `{ext}` and `{name}`: the name, the extension without its dot and the whole name, as they are so far in this pass.
- `{old-stem}`, `{old-ext}` and `{old-name}`: the same as on disk.
- `{parent}`: the name of the folder the entry is in.
- `{counter}`: the counter, which has a start, a step and a width.
- `{modified}`, `{created}` and `{accessed}`: a date. With no format it is `YYYY-MM-DD`, with no colon because Windows refuses one in a name. A format uses `YYYY`, `YY`, `MM`, `DD`, `HH`, `mm` and `ss`: `{modified:YYYY-MM}`.
- `{size}` in bytes and `{size:human}` in words, such as 2.1 MB.

The counter is one row, "Counter: from 1, step 1, 4 digits". A step can be negative. The width is at most 20 digits, and the numbers pad with zeros. Counters always number in the panel's order, whatever order the preview is in.

### Regular expressions

A pattern uses the syntax of Rust's regex crate. In a replacement, `$1` and `${1}` stand for a group, `${name}` for a named group and `$$` for a dollar sign. These are the limits, each said in words before the rule is kept:

- A pattern is at most 1,000 characters.
- Look-around is refused: "Pattern needs care: look-around". The way out is said with it: "Match the text around it too: for a v before a digit, write v\d."
- A backreference is refused: "Pattern needs care: backreference". The way out: "List the cases instead, such as (aa|bb|cc)."
- A replacement that names a group the pattern lacks is refused: "no group 2; the pattern has 1".
- `$1_` reads as a group named `1_`, so write `${1}_`. diroxide says so.
- An empty text to find is refused: "nothing to find".

## The preview

The preview is a list named by the count line. By default the problems come first, then the rows that need care, then the changes, then the rows that stay as they are. The setting `rename.preview_order` can put them in the panel's order instead.

Each row starts with its state, so the first 40 characters hold the state and the name that identifies the row:

- "IMG_0001.jpg becomes trip-0001.jpg": a plain change has no state word.
- "Same name: notes.txt".
- "Skipped by you: notes.txt".
- "Edited by you: IMG_0001.jpg becomes first.jpg", for a name you typed.
- "Swap with b.txt: a.txt becomes b.txt" and "Ring of 3: ...": the names pass around, and diroxide takes a temporary name to do it safely.
- "Case only: a.txt becomes A.txt".
- "Problem, name taken: c.txt becomes taken.txt": a file of that name is there already.
- "Problem, same new name as b.txt: ...": two rows would get one name.
- "Problem, ..." also for a reserved character in the name, an empty name, a name too long for the place, a counter below zero and a placeholder with no value.
- "Care, ...: ..." for a name that works but you should read twice.

A row with more than one problem adds ", and 2 more". Hidden characters in a name are spelled out, such as `[U+200B]`, so you hear them.

Enter on a row lists what can be done to it:

- Edit its new name.
- Skip it, or take it back.
- Say both names, with hidden characters spelled out: "Old: IMG_0001.jpg. New: ...".
- Go to the next problem, or the previous one.

Space skips the focused row or takes it back, and F2 edits its new name, in the preview. A skipped row is left out and does not count as a problem.

### Fixing problems

- "Make names unique" numbers the new names that clash, so every one is different. It says "Numbered the clashes. " and then the count line.
- "Skip problems" leaves out every row with a problem and renames the rest. It says "Skipped 2 problems. " and then the count line.
- Edit a row's name by hand, or change a rule, and the plan is made again.

## Rename now

"Rename now", the first row of the rules list, does one of two things.

With a problem left, it refuses and moves the focus to the first one: "Cannot rename: 3 problems. First: name taken, a.txt becomes b.txt." Nothing is asked.

Without one, it asks, and the question starts on No: "Rename 3 files in FOLDER? No is the default. y or n". No or Escape says "Nothing renamed."

On Yes, diroxide says "Checking the folder again." and reads the folders once more. If a name changed since the preview, it refuses in words and makes you look again: "The names changed since the preview. Preview again." Otherwise the batch runs as one job, and a finished batch says "Renamed 3 files." Cancel in the Jobs list stops it. The job is one entry for Undo, and Ctrl+Z puts every name back in one step, even after the dialog is long closed.

The plugins' `before_rename` and `after_rename` hooks hear the batch as they hear any rename. [Hooks](hooks.md) says how.

## Edit names in an editor

"Edit names in editor" opens every new name in your text editor, one a line, so you can edit them with the tools you know.

- It writes a file named `rename-NUMBER.txt` in the state folder and starts your editor on it.
- Each line is keyed: the new name, then the old one in brackets, "trip-01.jpg (was IMG_0001.jpg)". A line you move or delete still finds its file by the key. The setting `rename.editor_lines` switches to `plain`, one new name a line, in the order of the rows.
- When you close the editor, diroxide asks "Done editing? No is the default. y or n". It asks whether or not the editor's program waited, since some do not. Answer Yes when you have saved and closed.
- On Yes it reads the names back: "Names read. 1 of 3 files will change. No problems." Names you typed show as "Edited by you".
- A buffer it cannot read back is refused in words, with its line, and kept for another try: "Names not read. Edit names in editor again when done." A changed number of lines is refused.

## Presets and passes

A preset is a set of rules saved under a name, for a rename you do the same way each time.

- "Presets: load, save or delete" lists them. Enter on one loads it: "Loaded Trip: ..." and the rules are in the list. Save asks for a name, then "Saved preset Trip." Delete asks first, starting on No: "Delete preset Trip? No is the default. y or n", and then "Deleted preset Trip."
- Presets are in `rename-presets.toml` in the config folder, a file you can edit by hand. `diro rename --preset NAME` reads the same file.
- The rules you used last are kept in `rename-last.toml` in the state folder and offered to the next batch.
- "Next pass: more rules after these" applies this pass's rules in memory and starts another set of rules on the result. A pass has its own counter. Use it when one step depends on the one before: remove digits, then change case.

## Recipes

Each recipe is a list of rules in the list above. These are the ones people look for:

- Replace a prefix: one rule, "Replace IMG_ with trip-, in the name".
- Number files in order: "Make the name from trip-{counter}", with "Counter: from 1, step 1, 3 digits".
- Lowercase the extensions: "Change case to lowercase, in the extension".
- Tidy a download: "Trim spaces at both ends of the name", a tidy that makes runs of spaces one, and a tidy that turns spaces into hyphens.
- Put the date in front: "Make the name from {modified:YYYY-MM-DD} {old-name}".
- Strip the last four characters: "Remove 4 characters from the end, in the name".
- Move a year to the front with a pattern: replace `(.*)_(\d{4})` with `${2}_${1}`.

## From a shell

`diro rename` is the same engine for a shell. The preview comes first, and the batch runs only when you say so.

```bash
diro rename --in photos --find IMG_ --replace trip- --dry-run
```

- What to rename: `PATH...` as typed. `--in FOLDER`, with `--filter EXPRESSION` in the filter language, for shells that expand no wildcards, such as PowerShell and cmd.exe; `--hidden` adds hidden entries. `--stdin` reads paths from standard input, one a line, or NUL-separated with `--null`. `--from-pairs` reads an old path, a tab and a new name on each line.
- The rules, applied in this order: `--find TEXT --replace TEXT` (with `--all` for every match and `--ignore-case`), `--regex PATTERN --replace TEXT`, `--template TEXT` with `--start`, `--step` and `--width` for `{counter}`, and `--case FORM` (lower, upper, title or sentence). `--apply-to` says the part: `stem` (the default), `ext` or `name`.
- `--preset NAME` loads a preset from `rename-presets.toml`, `--recipe FILE` a file of the same shape, and `--portable` reads the presets beside the program.
- `--dry-run` or `-n` prints the preview and renames nothing. The preview is a heading, the count line and one row a name, problems first. It ends "Nothing was renamed."
- Without it the question starts on No, and `--yes` or `-y` renames without asking. A finished run says "Renamed 50 files."

Nothing is ever overwritten. A name that is taken is a problem row, and the batch does not run. The exit codes are 0 for renamed, or a dry run with no problems; 1 for problems, a path that could not be read, a No, or nothing that could run; 2 for wrong use or a rule that cannot be read; and 3 for partly renamed.

## Limits

- Details placeholders such as `{meta:exif.date}` are in the list of placeholders, but no values are fed to them in this build. A rule that uses one makes every row a problem: "Problem, no value for {meta:exif.date}". [Known limits](known-limits.md#batch-rename) says it too.
- The commands inside a batch, Next problem, Say both names, Make names unique and the rest, have no keys. Each is reached by Enter on a row, since the palette cannot open over a dialog.
- The plan is made again after every change, on the app's thread. That is fast for hundreds of files and has not been timed for 10,000.
- Back from an editor in a terminal, diroxide says "Back in diroxide, photos", and the open "Done editing?" question is not said again.
- No Windows editor other than the default has been tried.
- The rules that need a Unicode table, such as turning accented letters into plain ones or changing a Unicode form, are not built. `diro rename` has no `--format`, `--accept-care`, `--skip-problems`, `--sort`, `--undo` or `--edit` option yet.

## See also

- [Operations and undo](operations.md)
- [The panels](panels.md)
- [Search and the filter language](search.md)
- [Hooks](hooks.md)
- [Settings and where the files live](settings.md)
- [Every key](keyboard.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
