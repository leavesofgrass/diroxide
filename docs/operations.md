# Operations and undo

This guide covers everything that changes files: copy, move, rename, make folder, duplicate, delete, the file clipboard and undo. For each one it says the question diroxide asks, what you should hear, and how to reverse it. It also covers the prompts you type into, the jobs line and the Say Status key. Packing and unpacking archives, splitting and joining files, links, permissions and times are jobs too and follow the same steps; each has its own guide, and a short entry below says where.

## How every operation works

Every operation that touches more than one entry, or can take more than a moment, is a job. A job goes through the same steps.

1. diroxide makes a plan: it counts the entries and the bytes, and lists every path the job will touch. The plan is fixed at the moment you confirm. If the focused row is not part of the action, the plan's first line says so.
2. It asks. A question names the action, the count and the target first, and ends with the answer: "Copy 3 items to Backup, 312 bytes? No is the default. y or n". Every question starts on No. Press y to go ahead; n or Escape says no. Keys that arrive within a quarter of a second of a dialog opening are ignored, so a stray Enter cannot answer for you (the `dialog_settle_ms` setting changes the time). The settings `confirm.copy`, `confirm.move` and `confirm.trash` turn the question off for those three; deleting for good always asks.
3. A delete, a move or an overwrite lists every path before it asks. Hidden, control, private-use and direction characters are spelled out as `U+XXXX`, and the count is said in words. The summary line comes first: "Delete 12 items for good, 2.3 GB, 2 read-only". A permanent delete of more than 1,000 entries, or of far more than you selected, asks you to type a number or a name instead of y, and says what to type: "Type ... to go on, or Escape". A home folder, a config or state folder, and a whole drive are never removed without that typed name, and a drive root is refused.
4. The job runs on its own thread, so you can keep moving around. The jobs line tells you what it is doing.
5. It reports, with problems first: "Copy had 8 failures: 290 done, 2 skipped."
6. Ctrl+Z undoes it, where it can.

Some rules hold for every operation, because a file manager is the one program whose whole job is to change files.

- Nothing goes through a shell. Paths are literal: nothing you type is expanded by a shell, and `~` and variables are never expanded when you press Enter.
- The usual ways file managers lose data are built out. A recursive operation never follows a symbolic link or a junction out of the folder it started in, and it stops at another drive or a mounted volume: "Skipped D:\data\usb: another drive is mounted here".
- Order matters. diroxide writes and checks the replacement before it removes anything, on every path through Skip, Retry and Cancel. A failure of the trash never falls through to a permanent delete.
- After a job, diroxide reads the results back and compares them with the plan. If something is missing it says so: "Checked 118 of 120: 2 missing".
- A path such as `C:` with no slash is refused as ambiguous. diroxide always resolves to a full path before it acts, and shows it.
- Names are checked when you create or rename. A name that could cause trouble is refused in words, and a name Windows would change is refused (the `portable_names` setting, on by default).

## Copy and move: F5 and F6

F5 copies the marked rows, or the focused row, to the other panel's folder. F6 moves them. The question names the target by its folder's own name: "Copy 3 items to Backup, 312 bytes? No is the default. y or n". If both panels show the same folder, diroxide says so instead: "Copy needs another folder: both panels show Backup. Duplicate makes copies here."

To pick a place other than the other panel, use Copy to a place or Move to a place (in the Edit menu and the palette). They open a prompt, "Copy 3 items to", that completes from your recent destinations first, then the other panel, your home folder, the drives and the folders you use most.

Before it starts, diroxide checks what it can: the free space, the largest file against the drive's limit (a FAT drive cannot hold a file of 4 GiB or more), that the source and the target are not the same file, and that you are not moving a folder into itself. Items that cannot be copied are left out and listed in the plan.

How a copy works:

- Each file is written under a temporary name in the target folder and renamed when it is complete, so a cancelled or failed copy leaves no half-written file. The temporary name starts with `.diro-part-`.
- The modified time of files and folders is kept (a folder's is set after its contents), and the created time too where the system can set one (Windows and macOS). The accessed time is not kept.
- A folder is copied as a whole. A copy that is cancelled halfway keeps the files already finished.

How a move works: on one drive a move is a rename. Across drives, diroxide copies, reads the copy back and compares it with the original, and only then removes the original. The jobs line says "Verifying 60 percent" for that second phase. If the check fails, the original stays where it was. A moved file's labels do not follow it, because a label belongs to the file's path; label it again at its new place.

### When a name is taken

When a file with the same name is already in the target folder, a list opens. Its lines, in order:

- the situation and the name: "Name taken: report.pdf". When the target is read-only, the situation says so;
- the difference: "New is newer and larger", "New is older, the same size", or "New has the same size and time";
- two fact lines: "Existing: 2.1 MB, October 9" and "New: 2.3 MB, today, 9:02 PM";
- then the choices, each starting with its verb: "Skip this one", "Skip all that exist", "Replace this one", "Replace all that exist", "Keep both: report (2).pdf", "Keep both for all", "Replace only if newer, for all", "Rename this copy..." and "Cancel the job".

The focus starts on "Skip this one", which changes nothing, and Escape answers it too, so an accidental Escape never ends a long copy. "Cancel the job" is the last row. Each choice is read with its position: "Skip this one, 1 of 9".

- A folder over a folder is merged without asking: "Folder exists: photos. Its contents are merged." The questions come for the files inside.
- A file over a folder, or a folder over a file, is skipped and reported: "Cannot merge: photos is a folder here". Nothing is removed to make room.
- Copying a file onto itself says "Same file: report.pdf. Nothing to do."
- "Replace" sends the old file to the trash first when the place has one, so Undo can bring it back. Where there is no trash, the result says "replaced; this cannot be undone".
- "Keep both" numbers the copy: `report (2).pdf`. Double extensions stay together: `archive (2).tar.gz`.

### When something goes wrong

A failure names the file and the cause, then says what to do: "Could not copy report.pdf: another program is using it. Close it in the other program, then retry." The choices are never more than four: Retry, Skip, Skip all like this, and Cancel the job. The list starts on Skip when retrying cannot help, and on Retry when it can, such as a file in use by another program.

If a network share goes away, diroxide retries three times, five seconds apart, and then asks. If nothing moves for 20 seconds, the jobs line says "Copying stalled at 30 percent". Escape gives up on the job. A folder that could not be emptied is reported once, not once for every parent. These retry and stall values are built in; they are not settings.

## The jobs line and progress

The jobs line says "No jobs" when idle. A change of state is announced at once: "Copy started: 3 items to Backup", "Copying paused at 30 percent", "Copying waits for an answer", "Copy finished: 3 done."

While a job runs, the line gives the percent first and the estimate after it: "Copying 30 percent, 2 minutes left". The estimate waits until five seconds have passed and five percent is done, and it never says "about". Before a big job starts, diroxide counts first: "Copying: counting, 120 so far". Say Status (Ctrl+D) adds the detail: "12 of 300 items, 1.2 of 4.0 GB, 48 MB a second".

How often the line changes depends on screen-reader mode. In screen-reader mode it changes at once for a change of state, and otherwise at most every ten seconds (the `progress_interval_seconds` setting). Otherwise it changes at most twice a second. A screen reader that speaks new text would speak every change, so the slow rate is what keeps a long copy from talking all the time.

Ctrl+D gives the full sentence whenever you ask. Escape cancels the job in front: "Copying: cancelling", then "Copy cancelled: 2 done." The files already finished stay, and nothing half-written is left. Job history, in the Tools menu (Alt+T, then J) and the palette, lists the jobs that ran; Enter on one lists what failed or was skipped, and why.

The end of a job says the count first, with problems before it: "Copy finished: 3 done, 1 skipped.", "Copy finished with 1 warning: 3 done.", "Copy cancelled: 2 done." The entries that failed stay selected so you can see what to try again.

If diroxide is killed in the middle of a job, a temporary file may be left in the target folder, and the next start says so in words: "Last session ended during a copy: 3 of 5 steps done." Leftover `.diro-part-` files are never deleted automatically; you can delete them yourself like any other file.

## Rename: F2

F2 asks for a new name for the focused entry: "Rename report.pdf to". The name is in the field with the cursor before the extension, so you type over the stem. Press Enter: "Renamed report.pdf to report-2026.pdf".

- With several rows marked, F2 steps through them one at a time, then renames them as one job.
- A name that could cause trouble is refused and the prompt comes back: "Name needs care: ends in a dot or space. Nothing renamed."
- A rename never replaces an existing name. If the name is taken, you are told.
- A rename that only changes the case, such as `Readme` to `README`, works even on Windows and macOS, where the two names count as the same.
- Pressing Enter on an unchanged name says "Name unchanged. Nothing renamed."
- A file's labels belong to its full path, so they do not follow a rename. Label it again ([labels and bookmarks](labels-bookmarks.md)).

## Make a folder: F7

F7 asks for a name: "New folder in work". The cursor lands on the new folder: "Created folder Reports". A name that could cause trouble is refused, and the result says "Nothing was made."

## Duplicate: Shift+F5

Shift+F5 makes a copy of the focused or marked entries in the same folder. It asks "Duplicate report.pdf as" with the free name filled in: `report (2).pdf`. With several rows marked, it steps through them one at a time.

## Archives: pack, unpack and test

Pack makes a new archive from the marked rows, Unpack writes an archive's members to a folder, and Test archive reads every member and checks it. They are jobs, with a plan, a question that starts on No, progress, a result list and Undo. In a terminal they are in the File menu under Archive (Alt+F, then A) and in the palette; in the desktop window they are Alt+F5, Alt+F9 and Alt+Shift+F9. F5 from inside an open archive unpacks the marked members, reading the archive once, with the default settings. Copying, moving, renaming or deleting inside an archive is not possible, and says "Archives are read only here. Use Pack to make a new one." [Archives](archives.md) has the dialogs, the passwords and the limits.

## Split and join

Split file cuts a file into numbered parts (`report.pdf.001`, `report.pdf.002`) in the other panel's folder, with a `.crc` checksum file written last. Join files puts the parts back together from any part, names a gap before it writes anything, checks the result against the checksum file and says the verdict, then offers to move the parts to the trash. Both are jobs with Undo: Ctrl+Z on a split moves the parts to the trash, and on a join moves the whole to the trash. [Split and join](split-join.md) has the sizes and the messages.

## Links

Make symbolic link makes a link to each marked entry in the other panel's folder, absolute or relative, symbolic, hard or junction (File menu, Alt+F then L). Change link target swaps in a new target as one step that Undo reverses (Alt+F then I). Neither has a key of its own. On Windows a link needs Developer Mode or an administrator, and diroxide says so in words: "Cannot link: needs developer mode." [Links, permissions and times](links-permissions.md) has the rest.

## Permissions and times

Change permissions (File menu, Alt+F then W; no key of its own) says the result in words first and the octal number second: "Owner can read and write, group can read, others can read (644)". Its dialog steps owner, group and others through read and write, read only, read, write and run, read and run, and no access. On Windows it changes the attributes. A change can go inside folders, with "Apply to:" as the plan's first row. The plan lists what will change and asks first, and Ctrl+Z reverses it. There is no Change times command in alpha.2. A copy keeps the modified time of files and folders; see [links, permissions and times](links-permissions.md).

## Delete to the trash: F8 and Delete

F8 and Delete move the marked rows, or the focused row, to the trash. The word is "trash" on every system, including the Recycle Bin on Windows. diroxide lists what it will move and asks: "Move 1 item to the trash, 2.1 KB? No is the default. y or n". A folder is moved as one step. The result says "Moved 3 items to the trash. They can be restored."

### Where the trash cannot be used

On Windows the trash call can remove a file for good and still report success, so diroxide decides before it calls. In plain words:

- The trash is used only on a fixed drive inside the computer (the `trash_drives` setting lists the drive kinds; its default is fixed).
- It is not used on a removable drive such as a USB stick, or on a network drive.
- It is not used for a path of 260 characters or more, and a folder is refused if any path inside it is that long.

In those cases diroxide does not guess. It says why and offers the listed permanent delete instead, and the question starts on No: "This drive has no trash. Delete 3 items for good? No is the default. y or n". A path that is too long asks "Path is too long for the trash. Delete 1 item for good? No is the default. y or n".

After a trash call on Windows, diroxide checks that the original is gone and that the trash holds it. If the check fails, it says so loudly: "Deleted but not found in the trash."

On Linux the trash follows the freedesktop standard. On macOS the trash works, but diroxide cannot restore from it, so Undo cannot bring a trashed item back there; restore it from the Trash in the Finder.

### In a list of search results

A search's results are real files, so F8 or Delete in that list moves the real files to the trash, with the usual question starting on No, and Shift+Delete deletes them for good. F5 and F6 copy and move them to the other panel. Rename and labels do not work from the list; show the result in its folder first (Backspace). Removing a row from the list without touching the file is a separate command, Remove from results. A row whose file is gone since the list was made says "gone since this list was made", and the action is refused in words. In a label's results the trash key takes the label off the item and does not touch the file. [Search and the filter language](search.md) has the rest.

## Delete for good: Shift+Delete

Shift+Delete deletes without the trash. It is its own command with its own key and its own listing. The listing comes first, deepest paths first, with hidden characters spelled out and the count and size in words: "Delete 12 items for good, 2.3 GB, 2 read-only". A read-only file is part of the same question, and diroxide clears the attribute as part of the delete and says so in the listing. The question starts on No: "Delete 12 items for good? No is the default. y or n". The last line of the listing is honest about what deleting does:

"Deleted data may stay on the disk. diroxide cannot erase it for good."

Deleting removes the name, not the data. The data stays on the disk until something overwrites it, and on flash drives and modern file systems it may stay even after an overwrite. diroxide does not offer a secure erase, because it cannot know whether the file is on a journaled file system, a flash drive, a network share, a synced folder or a snapshot, and it cannot see your backups. What works is to keep the disk encrypted from the start and destroy the key (BitLocker, FileVault or LUKS), to use the drive maker's secure erase, or to destroy the drive. Backups and cloud copies need their own deletion.

## Undo: Ctrl+Z

Ctrl+Z undoes the last job. What it can reverse:

- a rename, and a move within one drive: the entry goes back: "Undid rename: 1 item is back";
- a move to the trash: the entry is restored (Windows and Linux): "Undid trash: 1 item is back";
- a made folder: removed, only if it is empty: "Undid make folder: 1 folder removed";
- a copy or a duplicate: the copies that diroxide made go to the trash, if they have not changed: "Undid copy: 3 copies are in the trash";
- an overwrite: the replaced file is restored from the trash, if the job trashed it first;
- a pack, an unpack, a split and a join: the files the job made go to the trash;
- a link: the link goes to the trash, and a changed link target goes back to the old link from the trash;
- a change of permissions, attributes or times: the old values are put back, for each entry that is still as the job left it;
- a label step (set, toggle or clear): the labels go back: "Undid label urgent on 3 items". This lasts for the session only. A job's undo survives a restart; a label's does not.

What it cannot reverse: a permanent delete, and a move across drives. Before it reverses each step, diroxide checks that the entry is as the job left it. If it changed since, that step is skipped and the undo says why. If there is nothing to undo, diroxide says "Nothing to undo".

An undo is not itself undone: press Ctrl+Z again and it undoes the job before. The list of recent jobs is kept in the state folder after each job, so Undo still works after a restart.

## The file clipboard: Ctrl+C, Ctrl+X, Ctrl+V

Ctrl+C and Ctrl+X put the marked rows, or the focused row, on diroxide's own clipboard, and Ctrl+V pastes them into the active panel's folder through the same job engine as F5 and F6. Ctrl+C then Ctrl+V is a copy, and Ctrl+X then Ctrl+V is a move. diroxide says it once: "Copied 3 items. Paste with Ctrl+V." or "Cut 3 items. Paste with Ctrl+V." Ctrl+D adds "3 items on the clipboard, to move".

- A cut leaves the items where they are until you paste. Nothing is lost if you never paste.
- A copy stays on the clipboard after you paste, so you can paste it again. A cut is used up by its paste.
- Pasting into the folder the items are already in says so: "These items are already here. Duplicate makes copies here."
- The clipboard is internal. It does not use the system clipboard, so Explorer and the Finder cannot paste what diroxide copied.
- Windows Terminal keeps Ctrl+V for pasting text, and Ctrl+C when text is selected. Use Paste in the Edit menu (Alt+E, then P), or unbind the key as [Terminals](terminals.md) shows.

## Copy a path or a name

Copy path and Copy name, in the palette, put the focused entry's full path or its name on the system clipboard, and say it: "Path copied: D:\docs\report.pdf", "Name copied: report.pdf". This is text for other programs, not the file clipboard of Ctrl+C. In a terminal diroxide asks the terminal to set its clipboard (OSC 52), so a terminal that does not allow that will not receive the text. The viewer has the same two commands for the file it shows.

## Open, edit and run

- Enter on a file opens it with your system's own program for it: "Opening report.pdf". Enter on a program asks first, starting on No: "Run setup.exe? No is the default. y or n".
- F3 opens the file in the viewer, which is for reading; Escape closes it. [The viewer](viewer.md) has the rest. Shift+F3 turns the other panel into a live preview of the focused file, and Open in textweaver in the View menu hands that file to textweaver ([the quick view](quick-view.md)).
- F4 opens the file in your editor with the screen suspended. When you come back, diroxide says "Back in diroxide, Documents". The `editor` setting names the editor; if it is empty, diroxide tries the VISUAL variable, then EDITOR, then the system's text editor.
- Run a command and Shell here are in the File menu and the palette. Run a command asks "Run in work" and runs your line in your shell, with the active folder as its working folder. Shell here opens your shell in the active folder: "Shell here: exit the shell to come back." Unlike every built-in operation, a command you type is a shell command, so you are responsible for what it does. Nothing from the panels is pasted into your line. Information arrives in environment variables instead: `DIRO_ACTIVE` and `DIRO_OTHER` hold the two panels' folders, `DIRO_CURSOR` the focused entry, and `DIRO_SELECTION_FILE` names a file that lists the selected paths separated by NUL characters. `DIRO_LEVEL` counts how deep diroxide is nested, so a script can tell it runs under diroxide. The shell is the one in `SHELL`, or else PowerShell 7, then Windows PowerShell 5.1, then cmd. Windows cannot start a program in a folder of 259 characters or more, and diroxide says so first: "Cannot start a program in a folder this deep". The same limit holds for opening a file: "Cannot open a file this deep".

## Typing in prompts: completion and history

Every prompt you type into has completion, with the same keys.

- The prompts are Go to a path, Go to a place, rename, make folder, duplicate, the run-command prompt, the pattern and filter prompts, Set label, the copy and move pickers, and the palette. Each completes from what suits it: paths and folders from the place you are in, places, command names, label names, and the prompt's own history.
- Candidates appear as a list under the field, in the minibuffer region. Each row puts the candidate first and its kind after it: "Downloads, folder". Typing narrows the list, with names that start with your text before names that merely contain it.
- Tab completes the single match, or the longest part that all the matches share, and says what it did: "completed to Downloads", "2 matches start Download, Down arrow lists them" or "No match". A second Tab, or Down, moves into the list. Enter accepts the highlighted candidate. Escape closes the list and keeps your text.
- In screen-reader mode nothing is announced while you type, except the count when you press Tab. The `completion_list` setting says whether the list is shown as you type or only on Tab; in the terminal the default is on Tab.
- With the list closed, Up and Down recall what you typed in that prompt before. History is kept for each prompt in the state folder, and never for a prompt that asks for a secret.
- A `~` or a variable you type in a path is offered as a candidate that completes to the full literal path. Nothing is expanded when you press Enter.

`diro completions` gives you the same Tab completion for diroxide's own subcommands and flags at your shell prompt.

## Limits that touch these operations

[Known limits](known-limits.md) has the full list. Three matter most here: Linux cannot set a created time on a copy; on Windows the trash is used only on fixed drives; and macOS cannot restore from the trash.

## See also

- [The panels](panels.md)
- [Archives](archives.md)
- [Split and join](split-join.md)
- [Links, permissions and times](links-permissions.md)
- [Search and the filter language](search.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Watching a folder](watching.md)
- [Terminals](terminals.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
