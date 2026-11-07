# Watched folders

This guide covers a folder that does something with each file that arrives in it, on its own: scans turned into text, pictures made into JPEG, downloads moved or copied, or files only noted. It covers the task file that describes one, the approval you give first, the Watched folders list in the program, and `diro watch --task` for the shell.

The short version: a task is one small TOML file. You turn it on from the Watched folders list, hear what it will do in plain words, and say yes. From then on it waits for each file to stop changing, runs its action on it, and files the original away. It never replaces a file and never removes one. It says what it did in one polite line at a time, and Ctrl+Z never undoes what it did.

[Watching a folder](watching.md) is different: it only tells you when a file has arrived. A watched folder acts on the file.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## A task is a file

Each task is one file in the `tasks` folder of the config folder: `tasks/NAME.toml`. The name is plain, with letters, digits, `-` and `_`, and it matches the file's name. The file holds only what you wrote. Whether the task is on, whether you approved it, and its ledger and log are kept apart, in the `hot` folder of the state folder, so editing the file never loses them.

A task that turns scanned PDFs into text:

```toml
name = "scans"
about = "Makes a text file from each scanned PDF."
kind = "convert"
recipe = "pdf-to-text"

[trigger]
kind = "folder"
home = 'D:\scans'
match = ["*.pdf"]
settle = "2s"
catch-up = "ask"

[run]
after = "move"
conflicts = "keep-both"
parallel = 1
timeout = "10m"
retries = 3
online-only = "skip"

[say]
announce = "summary"
```

Most keys have a default, so a short task works: a name, the recipe it needs, and a `home` with a `match`. A word that a key does not take is refused, and the problem names the words it does take. An unknown key is refused with its line.

### The folders

`home` holds four folders that you make: `in`, `out`, `processed` and `failed`. diroxide does not make them for you yet. A folder that is missing pauses the task, "Scans paused: folder is missing", and the task goes on by itself when the folder comes back. Put files in `in`. The results go to `out`. The original goes to `processed` when the action worked and to `failed` when it did not. Nothing but the files that arrive goes into `in`.

- Every folder is written out in full, as the system writes it: no wildcard, no variable such as `$HOME` or `%TEMP%`, no `~`, no `.` or `..`, no bare drive letter such as `C:`, and no address. Use single quotes in TOML so a backslash stays as typed.
- A task refuses a drive root, your home folder itself, a folder that holds diroxide's settings or state or lies inside one, a trash folder, a system folder and the folder the program lives in. Downloads is allowed, because it is not a drive root.
- A task's `to` and `after` folders may not be inside `in`, and may not be the home folder itself.
- Two tasks may not share an `in` folder, and no chain of tasks may come back to where it started: one task's results or originals landing in another's `in`. A task on such a chain never runs: "Task scans: it loops into itself."

### What a task does

`kind` says what happens to each file:

- `convert` runs a conversion recipe, named by `recipe`. [Conversions](conversions.md) lists the recipes and says how to write your own.
- `move` and `copy` put the file in the folder named by `to`.
- `script` runs a script command, named by `script`. Only a script that promises it never asks and always ends, with `unattended = "ok"`, may run with nobody there. [Script commands](script-commands.md) says what that promise means.
- `plugin` runs a plugin's command, named by `plugin`.
- `tell` only notes the arrival in the log and says it.

A key that does not belong to the kind is refused: "recipe is not used by a move task". One that the kind needs and lacks is refused too: "no recipe; a convert task needs one".

### `[trigger]`

- `kind`: `folder`, the only trigger.
- `home`: the folder described above.
- `match`: name patterns a file must match, such as `["*.pdf"]`. With none, every file matches. A pattern names files and not paths. `re:` starts a regular expression.
- `filter`: a filter expression a file must pass, in [the filter language](search.md), such as `size over 100 KB`. It is read by the program, so it can use a file's type and size.
- `settle`: how long a file must stop changing before it counts as arrived. The default is 2 seconds, the least is half a second and the most is an hour.
- `catch-up`: what the first turn-on does with files already waiting: `ask` (the default), `all` or `none`.

### `[run]`

- `after`: where the original goes. `move` (the default) sends it to `processed`, or to `failed` when the action failed. `leave` keeps it in `in`. A folder, written out in full, moves it there. A removal is refused: "delete is refused; a task never removes a file".
- `conflicts`: what a taken output name gets. `keep-both` (the default) numbers the new one, `report (2).pdf`, `skip` leaves the file, and `newer` makes the output only when the input is newer. `ask` is refused because a task never asks, and `overwrite` because a task never replaces a file.
- `parallel`: how many files run at once, 1 to 4. The default is 1.
- `timeout`: how long one file may take, such as `10m`.
- `retries`: how many times a file is tried again when it might pass later, 0 to 5. The default is 3.
- `online-only`: `skip` (the default) leaves a cloud placeholder alone, and `fetch` lets the system download it first.

### `[say]`

`announce` says how much the task speaks. `summary` (the default) is one polite line for everything since the last, at most every ten seconds. `each` is a line for each file, with at least 2 seconds between lines, and the files that come faster are counted into the summary. `failures` says only the failures, counted into the summary, and `off` says nothing. The log and the Watched folders list still say it.

### When a task has a problem

Every problem is said at start, with the task first, and a task with any problem never runs. The fault comes in the first 40 cells where it can, and a longer sentence runs on when it must: "Task scans: home has a wildcard." The Watched folders list and `diro watch --list` show them.

## The approval

A task runs only after you have heard what it does in words and said yes. The sentence is the same in the list, the question, the log and `diro watch --list`, so a task reads the same everywhere. Say rule, below, says it again. For the scans task above:

"Scans. Watches D:\scans\in. When a file named like *.pdf arrives and stops changing for 2 seconds, it runs the recipe pdf-to-text on it and puts the result in D:\scans\out. It then moves the original to D:\scans\processed; if the recipe fails, to D:\scans\failed. A taken name gets a number. Nothing is overwritten or removed. One file at a time."

The question follows it and starts on No: "Turn on Scans? No is the default. y or n".

You are asked once. You are asked again when anything that matters changes: the task itself, its recipe or the version of the tool the recipe runs, the hash of its script, or the version of its plugin. The task turns off until you say yes: "Scans changed. Turn it on again? y or n". A comment or a blank line in the task file changes nothing.

## The Watched folders list

The Watched folders list is in the Tools menu (Alt+T, then W) and the palette, where typing "hot" finds it. It has no key. You can give it one in `keymap.toml`.

It lists the tasks, one row each, the name first and then its state and what it did today: "Notes, off", "Scans, on, 3 done today, 1 failed". The title says how many: "Watched folders: 1 task". Under each row the rule is said as its sentence, and a paused task adds why it paused. A task file with a problem comes after the tasks as a row that says its problem first. It cannot be chosen, and its task never runs.

With none, it says where they go: "No watched folders. Each task is a file in" followed by the tasks folder.

Enter on a task opens its actions, titled "Notes actions":

- Turn on or Turn off. Turn on asks first, as below.
- Pause or Resume, for a task that is running.
- Preview. It says what the task would do right now and touches nothing, not even the ledger. It opens a list titled "Notes: 2 would convert, 0 skipped", with a line a file, "Would convert a.md", and last "Dry run: 2 would convert, 0 skipped".
- Show log. It lists the newest lines of the task's log. With none: "Notes log is empty".
- Put failed back. It asks first and starts on No, with the names under it: "Put back 1 file from failed? No is the default. y or n". Yes moves them back to `in`, says "Put back: c.md", and the task goes on. With none: "Nothing failed in Notes".
- Say rule. It says the task's sentence.
- Edit. It opens the task file in your editor and says "Editing Notes". When it cannot, it says where the file is: "The task file is" and the path.

### Turning a task on

Turn on asks the approval question, starting on No: "Turn on Notes? No is the default. y or n", with the task's sentence under it. No says "Nothing changed."

Yes asks about the files already waiting in `in`, when there are any, the first time only, unless the task's `catch-up` is `all` or `none`: "Process the 2 files in Notes? No is the default. y or n". Yes processes them. No leaves them where they are, and the task watches for new ones. Your answer is kept. Then diroxide says "Hot folder Notes is on".

Turn off says "Hot folder Notes is off". A task keeps its on or off state across restarts, and an approved task that is on starts with the program. A headless command such as `diro run` starts none.

### What a task says

A running task never opens a dialog and never interrupts. Its lines go to the live region by importance:

- The polite summary, at most every ten seconds: "Scans: 3 done, 1 failed".
- A pause at once, with the reason: "Scans paused: pdftotext is not installed".
- A change of state: "Hot folder Notes is on", "Hot folder Notes is paused".
- With the default `announce`, a file's own line is not said. It is in the log. The status line holds what waits, and is never spoken: "Scans: 3 waiting". With `announce = "each"` a line is said for each file, and with `failures` only the failures.

Say Status adds each running task's tally to its line: ". Notes: 2 done, 1 failed".

Ctrl+Z never undoes a task's work. Its jobs do not reach the undo log. To take back a converted file, delete it, or put the original back from `processed`.

## What a task does with each file

1. A file arrives in `in`. It must stop changing for the settle time. Size, modified time and identity all have to stay the same, and on Windows a program holding it open for writing makes it wait.
2. It must match the task's `match` and `filter`. Hidden files, `.part` and other unfinished names are never arrivals, as in [watching a folder](watching.md#what-is-never-an-arrival).
3. The action runs. A conversion writes under a temporary name in `out` and renames it only when the tool succeeded.
4. The task writes a line in the ledger, then moves the original. If diroxide stops between the two, the next start moves the original and runs nothing twice: "Done: scan.pdf, finished after a restart".

A file's own line puts the outcome first. `diro watch --task NAME --once` prints one for each file, and `announce = "each"` says them as they come: "Done: scan-004.pdf, 3 seconds", "Failed: scan-004.pdf, unreadable", "Skipped scan-004.pdf: output exists", "Moved across drives: scan-004.pdf" (a move from one drive to another copies and then removes the original), and "Changed during the run: scan-004.pdf", after which the file settles and runs again.

### When something goes wrong

What a failure means depends on its cause:

- The same file might succeed later: another program is using it, it timed out or was stopped, or it changed during the run. The task waits 10 seconds, then 1 minute, then 10 minutes, as many times as `retries` allows: "Retrying scan-004.pdf in 1 minute".
- Something is wrong with this file: it is unreadable, damaged or refused. The original goes to `failed` with its reason in the log, and the next file goes on: "Failed: scan-004.pdf, unreadable".
- Something is wrong with the surroundings: a tool is not installed, the drive is full, the network is lost, or a folder is missing. The task pauses, says why once, and waits for you: "Scans paused: pdftotext is not installed". A task whose folder goes missing resumes by itself when it comes back.
- Ten files in a row failing for the same cause pause the task: "Scans paused: 10 failures in a row". That is almost always a wrong recipe for the files and not ten bad files.
- Free space where the results go below 1 GB, or below a tenth of the drive, pauses it: "Scans paused: only 200 MB free".

A paused task keeps its files in `in`. Resume it from its actions when you have fixed the cause.

### The ledger and the log

Each task has a folder under `hot` in the state folder, with `state.toml`, a ledger, a log and a lock. Nothing in them leaves the computer.

- The ledger has one line for each file handled, written after its output is made and before its original moves, so nothing is done twice. It is capped at 1 MiB.
- The log is `log.txt`, one line a file. The outcome word comes first, the fields follow with two spaces between them, and the time comes last, so the first 40 cells of a Braille line say what happened. Hidden characters in a name are spelled out as `[U+202E]`. It rotates at 1 MiB and keeps five, `log.1.txt` the newest and `log.5.txt` the oldest, and diroxide removes the oldest when it rotates. A failure carries the tool's own words after the name. Show log in the Watched folders list reads the newest lines.
- The lock means one diro runs a task at a time. A second says "Scans is run by another diro".

A folder that cannot be watched, such as a network share, is checked every 30 seconds: "Scans: checking every 30 seconds".

## Scripts and plugins in a task

A script task runs with nobody to ask. diroxide sets `DIRO_UNATTENDED` to `1`, and also `DIRO_TRIGGER` to `hot-folder` and `DIRO_HOT_RULE` to the task's name, so a script can tell a file that arrived from a file you picked. A script's `ui/ask` answers "no user present".

## From a shell

`diro watch --task NAME` runs a task from the terminal, which is also how a scheduler can run one. `--rule` is another name for `--task`. A task needs your approval, so the first run asks in the terminal, starting on No. With nobody at the terminal it says so and does not run: "Answered no: there is no terminal to ask." Approve it once from the Watched folders list or in a terminal.

```bash
diro watch --task scans --once
```

- With neither option, the task runs until Ctrl+C: "Watching Scans. Ctrl+C stops." then its polite lines, and "Stopped watching Scans." at the end. The first start asks about the files already waiting, as the list does. With nobody at the terminal the answer is No.
- `--once` catches up and ends. It handles the waiting files and the queue, then exits with the family's codes, so a scheduler gets catch-up for free. It gives up on a file that is still changing after 2 minutes. Its lines are one a file, "Done: scan-004.pdf, 3 seconds", and the summary "Scans: 3 done, 1 failed". Progress words go to standard error.
- `--dry-run` says one line a file in `in`, "Would convert a.md", and ends with "Dry run: 3 would convert, 1 skipped". It writes nothing, not even the ledger.
- `--list` lists the tasks without starting any, the name first: "Notes, off, 2 done today", then each task that cannot be used, its problem first. With none: "No tasks. Each is a file in" and the folder.

The exit codes are the family's: 0 when everything was done, 1 when the task could not run (it is not approved, or it is not found: "Could not run nope: no task is named nope."), 2 for wrong use, and 3 when some files were done and some failed.

`diro watch PATH`, with no `--task`, stays what [watching a folder](watching.md) describes.

## Known limits

- A script task and a plugin task are built, but have not been run through the real script host or a real plugin. A plugin's command in a task pauses it for now: "plugin commands are not available here".
- A task's filter is read by the program, and the count in the catch-up question ignores it, so the question may count files that the filter will leave.
- On Linux and macOS a writer that does not lock its file is seen only by the quiet period. On Windows diroxide looks again through a share-mode open and never locks a file in `in`.
- `online-only = "fetch"` downloads a placeholder to run on it.
- The Watched folders list's "done today" counts come from each task's log, while Say Status counts this session's notes.
- `diro watch --task` answers the first start's catch-up question No when nobody is at the terminal.
- A form for making a task without writing the file is planned for the final alpha. The file and `--list` serve until then.
- A hot folder has been run for seconds in tests, never for hours. APFS, a network share, macOS and Linux natively have not been tried.
- Nothing about how the Watched folders list or its questions read on a Braille display has been heard yet.

## See also

- [Watching a folder](watching.md)
- [Conversions](conversions.md)
- [Search and the filter language](search.md)
- [Script commands](script-commands.md)
- [Plugins](plugins.md)
- [Tools and diro doctor](tools.md)
- [Operations and undo](operations.md)
- [Privacy](privacy.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
