# Operations and undo

This guide covers everything that changes files: copy, move, rename, make folder, delete, the file clipboard and undo. For each one it says the question diroxide asks, what you should hear, and how to reverse it. It also covers the prompts you type into, the jobs line and the Say Status key.

## How every operation works

Every operation that touches more than one entry, or can take more than a moment, is a job. A job goes through the same steps.

1. diroxide makes a plan: it counts the entries and the bytes, and lists every path the job will touch. The plan is fixed at the moment you confirm. If the focused row is not part of the action, the plan's first line says so.
2. It asks. A question names the action, the count and the target first, and ends with the answer keys: "Copy 3 files to Backup? y or n". Every question starts on No. Keys that arrive within a quarter of a second of a dialog opening are ignored, so a stray Enter cannot answer for you (the `dialog_settle_ms` setting changes the time).
3. A delete, a move or an overwrite lists every path before it asks. Hidden, control, private-use and direction characters are spelled out as `U+XXXX`, and the count is said in words. For a very large plan you are asked to type the number of items to confirm.
4. The job runs on its own thread, so you can keep moving around. The jobs line tells you what it is doing.
5. It reports, with problems first: "Copy had 8 failures: 290 done, 2 skipped".
6. Ctrl+Z undoes it, where it can.

Some rules hold for every operation, because a file manager is the one program whose whole job is to change files.

- Nothing goes through a shell. Paths are literal: nothing you type is expanded by a shell, and `~` and variables are never expanded when you press Enter.
- The usual ways file managers lose data are built out. A recursive operation never follows a symbolic link or a junction out of the folder it started in, and it stops at another drive or a mounted volume: "Skipped D:\data\usb: another drive is mounted here". A root, a home folder or a whole drive is never the target of a recursive delete.
- Order matters. diroxide writes and checks the replacement before it removes anything, on every path through Skip, Retry and Cancel. A failure of the trash never falls through to a permanent delete.
- A path such as `C:` with no slash is refused as ambiguous. diroxide always resolves to a full path before it acts, and shows it.
- Names are checked when you create or rename. A name that could cause trouble is flagged in words, and a name Windows would change is refused.

## Copy and move: F5 and F6

F5 copies the marked rows, or the focused row, to the other panel's folder. F6 moves them. The question names the target: "Copy 3 files to Backup? y or n" and "Move 3 files to Backup? y or n".

Before it starts, diroxide checks what it can: the free space, the largest file against the drive's limit (a FAT drive cannot hold a file of 4 GiB or more), that the source and the target are not the same file, and that you are not moving a folder into itself. A copy of a file onto itself says "Same file: report.pdf. Nothing to do."

How a copy works:

- Each file is written under a temporary name in the target folder and renamed when it is complete, so a cancelled or failed copy leaves no half-written file. The temporary name starts with `.diro-part-`.
- The modified time of files and folders is kept (a folder's is set after its contents), and the created time too where the system can set one (Windows and macOS). The accessed time is not kept. The `keep_dates` setting is on by default.
- A folder is copied as a whole. A copy that is cancelled halfway keeps the files already finished and says so: "Copy cancelled: 120 done. Undo trashes the copies."

How a move works: on one drive a move is a rename. Across drives, diroxide copies, checks the copy against the original, and only then removes the original. The summary says so: "Move finished: 40 done, verified". If the check fails, the original stays where it was.

### When a name is taken

When a file with the same name is already in the target folder, a list opens. Its lines, in order:

- the situation and the name: "Name taken: report.pdf". Other situations are "Folder exists: photos", "Read-only file exists: report.pdf" and "Same file: report.pdf";
- the difference: "New is newer and larger";
- where you are: "In Backup, other panel, file 3 of 12";
- two fact lines: "Existing: 2.1 MB, Oct 9, 2026 10:14" and "New: 2.3 MB, Oct 10, 2026 21:02";
- then the choices, each starting with its verb: "Skip this one", "Skip all that exist", "Replace this one", "Replace all that exist", "Keep both: report (2).pdf", "Keep both for all", "Replace only if newer, for all", "Rename this copy" and "Cancel the job".

The focus starts on "Skip this one", which changes nothing, and Escape answers it too, so an accidental Escape never ends a long copy. "Cancel the job" is the last row. Each choice is read with its position: "Skip this one, 1 of 9".

- A folder over a folder always merges, with file questions inside. There is no "replace the whole folder".
- A file over a folder, or a folder over a file, is skipped and reported. Nothing is deleted to make room.
- Replacing a read-only or hidden file is its own question, and needs an explicit answer.
- "Replace" sends the old file to the trash first when the place has one, so Undo can bring it back. Where there is no trash, the dialog says "Replacing cannot be undone."
- "Keep both" numbers the copy: `report (2).pdf`. Double extensions stay together: `archive (2).tar.gz`.

### When something goes wrong

A failure names the file and the cause, then says what to do: "Could not copy report.pdf: the drive is full. Free some space, then retry." The choices are never more than four: Retry, Skip, Skip all like this, and Cancel the job. The list starts on Skip when retrying cannot help, and on Retry when it can, such as a file in use by another program.

If a network share goes away, diroxide retries three times, five seconds apart, and then asks. If no data moves for 20 seconds, the jobs line says "Copy stalled: no data for 20 seconds" and offers "Keep waiting", "Give up on this file" and "Cancel the job". A folder that could not be emptied is reported once, not once for every parent.

## The jobs line and progress

The jobs line says "No jobs" when idle. A change of state is announced at once: "Copy started: 300 files to Backup", "Copy paused at 30 percent", "Copy needs an answer: name taken", "Copy finished: 297 done, 3 skipped", "Copy cancelled: 120 done".

While a job runs, the line gives the percent first and the estimate after it: "Copying 30 percent, 2 minutes left". The estimate waits until five seconds have passed and five percent is done. The detail line has "12 of 300 files, 1.2 of 4.0 GB" and "Now: report.pdf". Before a big job starts, diroxide counts first and says so: "Counting, 12,000 items so far". The second phase of a verified move says "Verifying 60 percent".

How often the line changes depends on your screen reader setting. In screen-reader mode it changes at once for a change of state, and otherwise at most every ten seconds (the `progress_interval_seconds` setting). Otherwise it changes at most twice a second. A screen reader that speaks new text would speak every change, so the slow rate is what keeps a long copy from talking all the time.

Ctrl+D (Say Status) gives the full sentence whenever you ask. Escape cancels the job that is running; the files already finished stay. You can pause and resume from the palette.

If diroxide is killed in the middle of a job, a temporary file may be left in the target folder. The next start tells you in words that a job was interrupted. Leftover `.diro-part-` files are never deleted automatically; you can delete them yourself like any other file.

## Rename: F2

F2 asks for a new name for the focused entry: "Rename report.pdf to:". The name is in the field with the extension left alone, so you type over the stem. Press Enter: "Renamed report.pdf to report-2026.pdf".

- With several rows marked, F2 steps through them one at a time.
- A name that could cause trouble is flagged and nothing is renamed: "Name needs care: ends with a space".
- A rename never replaces an existing name. If the name is taken, you are told.
- A rename that only changes the case, such as `Readme` to `README`, works even on Windows and macOS, where the two names count as the same.

## Make a folder: F7

F7 asks for a name. The cursor lands on the new folder: "Created folder Reports". The name may contain slashes to make several levels at once, such as `2026/reports`: "Made 3 folders; the last is c". A level that already exists is not an error. A level that exists as a file is: "Cannot make a: a file has that name".

## Duplicate: Shift+F5

Shift+F5 makes a copy of the focused or marked entries in the same folder. It asks for the name first, with the keep-both name filled in: `report (2).pdf`. With several rows marked, it steps through them one at a time.

## Delete to the trash: F8 and Delete

F8 and Delete move the marked rows, or the focused row, to the trash. The word is "trash" on every system, including the Recycle Bin on Windows. diroxide lists what it will move and asks: "Move report-2026.pdf to the trash? y or n" and "Move 3 items to the trash? y or n". A folder is moved as one step, with its item count. The result says "Moved 3 items to the trash." and they can be restored.

### Where the trash cannot be used

On Windows the trash call can remove a file for good and still report success, so diroxide decides before it calls. In plain words:

- The trash is used only on a fixed drive inside the computer.
- It is not used on a removable drive such as a USB stick, or on a network drive.
- It is not used for a path of 260 characters or more, and a folder is refused if any path inside it is that long.

In those cases diroxide does not guess. It says why and offers the listed permanent delete instead, and the question starts on No: "This drive has no trash. Delete 3 items for good? y or n". A path that is too long says "Path is too long for the trash" first.

After every trash call on Windows, diroxide checks that the original is gone and that the trash holds it. If the check fails, it says so loudly: "Deleted but not found in the trash."

On Linux the trash follows the freedesktop standard. On macOS the trash works, but diroxide cannot restore from it, so Undo cannot bring a trashed item back there; restore it from the Trash in the Finder.

## Delete for good: Shift+Delete

Shift+Delete deletes without the trash. It is its own command with its own key and its own listing. The listing comes first, deepest paths first, with hidden characters spelled out and the count and size in words: "Delete 12 items for good, 2.3 GB, 2 read-only". A read-only file is part of the same question, and diroxide clears the attribute as part of the delete and says so in the listing. The question starts on No. The sentence under it is honest about what deleting does:

"Deleted data may stay on the disk. diroxide cannot erase it for good."

Deleting removes the name, not the data. The data stays on the disk until something overwrites it, and on flash drives and modern file systems it may stay even after an overwrite. diroxide does not offer a secure erase, because it cannot know whether the file is on a journaled file system, a flash drive, a network share, a synced folder or a snapshot, and it cannot see your backups. What works is to keep the disk encrypted from the start and destroy the key (BitLocker, FileVault or LUKS), to use the drive maker's secure erase, or to destroy the drive. Backups and cloud copies need their own deletion.

## Undo: Ctrl+Z

Ctrl+Z undoes the last job. What it can reverse:

- a rename, and a move within one drive: the entry goes back;
- a move to the trash: the entry is restored (Windows and Linux);
- a made folder: removed, deepest level first, only if it is empty;
- a copy: the copies that diroxide made go to the trash, if they have not changed;
- an overwrite: the replaced file is restored from the trash, if the job trashed it first.

What it cannot reverse: a permanent delete, and a move across drives, which diroxide reports as not undoable. Before it reverses each step, diroxide checks that the entry still has the size and time the job left, and that the old name is free. If the world has changed, it skips that step and says why: "Undid move: 2 items are back. 1 could not be undone: it was changed since."

An undo is a job too. A single rename or move undoes at once and says what came back: "Undid rename: report.pdf is back". An undo that trashes or restores several items lists them first and asks. Undoing an undo puts things back. If there is nothing to undo, diroxide says "Nothing to undo".

The list of recent jobs is kept in the state folder after each job, so Undo still works after a restart.

## The file clipboard: Ctrl+C, Ctrl+X, Ctrl+V

Ctrl+C and Ctrl+X put the marked rows, or the focused row, on diroxide's own clipboard, and Ctrl+V pastes them into the active panel's folder through the same job engine as F5 and F6. Ctrl+C then Ctrl+V is a copy, and Ctrl+X then Ctrl+V is a move. diroxide says it once: "Cut 3 items. Paste with Ctrl+V." Ctrl+D adds "3 items on the clipboard, to move".

- A cut leaves the items where they are until you paste. Nothing is lost if you never paste.
- A copy stays on the clipboard after you paste, so you can paste it again.
- The clipboard is internal. It does not use the system clipboard, so Explorer and the Finder cannot paste what diroxide copied.
- Windows Terminal keeps Ctrl+V for pasting text. Use Paste in the Edit menu (Alt+E, then P), or unbind the key as [Terminals](terminals.md) shows.

## Open, edit and run

- Enter on a file opens it with your system's own program for it. Enter on a program asks first, starting on No: "Run setup.exe? y or n".
- F4 opens the file in your editor with the screen suspended. When you come back, diroxide says "Back in diroxide, Documents." The `editor` setting names the editor; if it is not set, diroxide tries the VISUAL variable, then EDITOR, then the system's text editor.
- The palette has Run command and Shell here. A command you type runs in your shell, with the active folder as its working folder. Unlike every built-in operation, it is a shell command, so you are responsible for what it does. The focused file and the marked rows are not pasted into the line. They arrive in environment variables instead: `DIRO_FILE` holds the focused file, and `DIRO_FILES` names a file that holds the marked paths separated by NUL characters. `DIRO_LEVEL` is set so a script can tell it runs under diroxide. The shell is the one in `SHELL`, or else PowerShell 7, then Windows PowerShell 5.1, then cmd. Windows cannot start a program in a folder of 259 characters or more, and diroxide says so first: "Cannot start a program in a folder this deep".

## Typing in prompts: completion and history

Every prompt you type into has completion, with the same keys.

- The prompts are the path field, Go to a place, rename, make folder, the run-command prompt, the pattern and filter prompts, and the palette. Each completes from what suits it: paths and folders from the place you are in, places, command names, and the prompt's own history.
- Candidates appear as a list under the field, in the minibuffer region. Each row puts the candidate first and its kind after it: "Downloads, folder". Typing narrows the list, with names that start with your text before names that merely contain it.
- Tab completes the single match, or the longest part that all the matches share, and says what it did: "completed to Downloads", or "4 matches, Down arrow lists them". A second Tab, or Down, moves into the list. Enter accepts the highlighted candidate. Escape closes the list and keeps your text.
- In screen-reader mode nothing is announced while you type, except the count when you press Tab. The `completion_list` setting says whether the list is shown as you type or only on Tab; in the terminal the default is on Tab.
- With the list closed, Up and Down recall what you typed in that prompt before. History is kept for each prompt in the state folder, and never for a prompt that asks for a secret. The Forget history command clears it.
- A `~` or a variable you type in a path is offered as a candidate that completes to the full literal path. Nothing is expanded when you press Enter.

`diro completions` gives you the same Tab completion for diroxide's own subcommands and flags at your shell prompt.

## Limits that touch these operations

[Known limits](known-limits.md) has the full list. Three matter most here: Linux cannot set a created time on a copy; on Windows the trash is used only on fixed drives; and macOS cannot restore from the trash.

## See also

- [The panels](panels.md)
- [Terminals](terminals.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
