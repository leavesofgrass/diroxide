# The hot-folder engine

This page is for the person who works on watched folders in the code: how the engine is laid out, what it guarantees and why, and what the spikes found about files that are still being written. [Watched folders](../watched-folders.md) is the guide for the person who uses them, and it holds the task file's keys.

The engine is in `crates/diroxide-ops/src/hot/`, with the task file's reader in `crates/diroxide-config/src/task_files.rs`. The runner is stepped by its caller with the time, so the whole machine runs in tests against a fixed clock. The app and the command line give it what ops cannot reach through one trait, `TaskActions`.

## The modules

- `task`: a task file's keys checked into a `Task`. Every problem is a sentence that names the task first. Unknown keys are refused with their line.
- `refuse`: the literal-path rules and the folders a task may never name. A path is written out in full: no wildcard, no variable, no `~`, no bare drive letter, no `.` or `..`, no device prefix, no hidden character. A drive root, the home folder itself, diroxide's own settings and state folders, a trash, a system folder and the folder the program lives in are refused.
- `check`: the tasks together. A shared `in` folder and a loop between two tasks are refused, and at most 50 tasks are read.
- `words`: the task as one sentence, the same in the list, the approval question and `diro watch --list`. `lines`: what a running task says, each line within 40 cells.
- `approval`: the approval key, a SHA-256 of the task as checked together with what its action resolves to: the recipe's tool, arguments, time limit and the tool's version, the script file's hash, or the plugin's id and version. A comment or a blank line in the task file changes nothing.
- `store`: a task's state in its own folder under the state folder's `hot`, never in the task file. Small TOML files, written whole to a new name and renamed into place, so a crash leaves the old file or the new one.
- `ledger` and `log`: one line a handled file, and the log with the outcome first and the time last, rotated at 1 MiB with five kept.
- `runner`: one task's queue around the arrival tracker. `retry`: what a failure means. `probe`: the look again before the original moves. `actions`: what the frontend gives. `spawn`: a task on its own thread over a real watch. `once`: `diro watch --task NAME --once` and the dry run.

## One file, start to end

A file that has settled and matches waits in a first-in, first-out queue, ties broken by natural name order. Its turn:

1. Its facts are read and the ledger asked. A file already handled is passed over, and one whose output was committed before a crash has only its original moved.
2. Free space is checked where the result goes. Below 1 GB, or a tenth of the drive, the task pauses.
3. The action runs through the job engine: a conversion through the Convert engine, a move or copy as an ordinary job, a script or plugin command through `TaskActions`. These jobs carry `JobOrigin::HotFolder` and continue and list, so nothing ever asks and nothing is ever replaced.
4. A made line goes to the ledger and is flushed to the disk.
5. The look again: the file's facts are read again and compared with the ones taken at the start, and the writer probe is asked. A change says "Changed during the run" and the file settles and runs again.
6. The original moves to `processed`, or the task's own folder, never over a file. Then a done line, the log line and the summary.

Step 4 comes before step 6 on purpose. A crash between them leaves the input in `in` with a made line holding its identity, so the next start finishes the move and converts nothing again. Spike H3 aborted a child between the output's commit and the input's move, on NTFS and on ext4, and the restart moved the input, ran the tool 0 times and said "Done: scan.pdf, finished after a restart".

## When a file fails

The cause decides, in `retry`:

- Again: in use, timed out, stopped by a signal, or changed during the run. The file is tried after 10 seconds, then 1 minute, then 10 minutes, as many times as the task's `retries` says, at most 5.
- The file: unreadable, damaged, refused, a tool's error for this file, a password needed. The original goes to `failed` with its reason in the log, and the next file goes on.
- The task: a tool not installed or not starting, a drive full or read only, the network lost, a folder missing. The task pauses and says why once, and every file stays in `in`. A missing folder resumes by itself when it comes back.

The circuit breaker: ten files in a row that fail with one cause pause the task, since that is almost always a wrong recipe and not ten bad files. A file counts once, after its last try, and a success or a different cause starts the count again.

## Files that are still being written

The look again must say "still being written" without harming the writer. Spike H1 (Friday, November 6, 2026, Rust 1.96.1) decided how, and the finding is written in `probe.rs`.

- On NTFS, std's `File::try_lock` and `try_lock_shared` took the lock on every look at a file another process was writing without a lock, so they cannot tell a half-written file from a finished one. Worse, holding that lock for 40 ms made 11 of the writer's 40 writes fail with error 33 and lost 704 KiB. The share-mode open, which the arrival tracker's writer probe already does, said "a writer holds it" on every look while the writer ran, and "no writer" after. So on Windows the look again is the share-mode probe, and no lock is ever taken on a file in `in`.
- On ext4 and overlayfs, a try-lock sees only a writer that locks: 36 of 36 looks said "would block" for such a writer and 36 of 36 "got it" for one that does not, and a writer that does not lock was not harmed. So on Linux and macOS the look again takes an exclusive lock on a read handle and lets it go at once. That is a bonus for producers that lock, never a promise.
- Either way the facts compared before the move, which are size, modified time and identity, are the rule. The probe only adds "still being written".

Where a folder cannot be watched, `spawn` lists it every 30 seconds instead and says so once ("Scans: checking every 30 seconds"). APFS, a network share, macOS, Linux natively and Windows long paths were not tried.

## What the spikes also found

From the engine's lane report, not run again for this page:

- H2: the arrival and failure cases as 35 runner tests on a fixed clock: arrivals in parts, rename in, in use, timed out, damaged, tool missing, the breaker and its resets, catch-up asked and remembered, two runners, approval changed, a folder gone and back, a file grown or swapped in the middle of a run, the queue cap, low space, leave, skip, dry run, once, tell, move with keep-both, script, filter, partial files, put back, make folders, parallel, interrupt, and the real thread. A thousand arrivals in one rescan ran in natural order in 12.2 seconds alone, about 12 ms a file in a debug build on NTFS, and 36 seconds beside other tests.
- H4, in a container only: an 8 MiB original moved from ext4 to overlayfs by copy and removal in 0.96 seconds, with "moved across drives" in the log.
- H5, in the tools image: a hostile Downloads-style folder through the real recipes. The pictures took 10.1 seconds and exited 3, with 2 done and 6 filed with their causes. The papers took 1.0 second and exited 3. Nothing hung, the breaker never tripped, and an existing `out/good.jpg` was kept beside "good (2).jpg". A loop of two folders was refused before anything ran. Natively, with no ImageMagick, the task paused at once with "Pictures paused: ImageMagick is not installed" and exited 1, with nothing touched.

## The exit codes

`diro watch --task NAME --once` uses the family's codes. 0 is everything waiting done. 1 is a task that could not run: not approved, invalid, locked, its tool missing or its folder missing. 2 is wrong use, which the command line itself says. 3 is partly done, a file having failed or been left for the next run. The first design had 2 for "some failed", and the family's 3 replaced it.

## Rules to keep

- Nothing ever replaces a file, and nothing ever removes one. A task that asks to overwrite, to ask or to delete is refused with its reason.
- The runner reaches a tool only through the engine's `ToolRunner`, and a conversion only through the Convert engine. What comes through `TaskActions` is a recipe, a type answer, a yes or no, or the outcome of a command the host ran with its own limits, never a shell string.
- A ledger line spells every hidden character `\u{XXXX}` and escapes a backslash, a tab and a line break, so a name reads back exactly. A log line spells them `[U+XXXX]`.
- The ledger and the log hold no secret, and nothing in them leaves the computer.

## See also

- [Watched folders](../watched-folders.md)
- [Conversions](../conversions.md)
- [Architecture](architecture.md)
- [Fuzzing](fuzzing.md)
