# Watching a folder

This guide covers Watch this folder and `diro watch`. Both tell you, in one polite line, when a new file has finished arriving in a folder: a scan that a scanner is writing, a download, a file another program is saving.

What this guide says comes from the design and from the tests that check each sentence. Nothing in alpha 1 had been heard on a screen reader when this guide was written, because the listening tests come after the build, so the lines below are what diroxide is built to say. [The accessibility statement](accessibility.md) says what has been checked.

## Why it waits

A file does not arrive all at once. A scanner writes a page at a time, a browser fills a download, a copy runs for minutes. A program that says "new file" at the first byte tells you before the file is ready. This one waits until the file has stopped changing, and says "Arrived" only then.

A file counts as arrived when its size, its modified time and its identity have stayed the same for 2 seconds. On a place that cannot be watched, which diroxide looks at every 5 seconds instead, the wait is 10 seconds. On Windows it also waits while a program holds the file open for writing.

## Watch this folder

Watch this folder is in the palette and in the View menu (Alt+V, then A). It has no key of its own. It watches the folder in the active panel. diroxide says what it is doing:

- "Watching in for new files. Escape stops." (The folder is called in here.)

Then, for each file that settles:

- "Arrived: scan-004.pdf, 1.2 MB". The name first, then the size.
- "Arrived again: scan-004.pdf, 1.2 MB". A file that arrived, changed and settled again, or one that was still being copied when the watch began.

In screen-reader mode diroxide says at most one line in each progress interval, ten seconds by default (`progress_interval_seconds`). If several files arrive within it, it says the count instead of each name: "Watching in: 3 arrived".

The status line shows what it is waiting for, "Waiting for scan-004.pdf to settle" or "Waiting for 3 files to settle", but never says it aloud. It is there when you look.

To stop, press Escape (with no dialog, job or search in front) or run Watch this folder again. diroxide says "Stopped watching in: 3 arrived". If the folder is removed: "Stopped watching in: the folder is gone". A place that is checked and not watched says so when it starts: "Checking in every 5 seconds: it cannot be watched".

Other lines:

- "Skipped scan-004.pdf: online only". A cloud file that is not on this computer is skipped, and said, because looking inside it would download it.
- "Gave up waiting for app.log". A file that kept changing for an hour, such as a log. The watch goes on with the others.
- "Close this dialog first." The watch needs the panel, not a dialog.

Watching lasts for the session. Nothing is saved, moved or run. A rule that does something with each arrival comes in a later alpha.

### What is never an arrival

- Folders and links.
- Names that begin with a dot, with `~$` (Office's lock files) or with `~syncthing~`.
- Names that end in `.part`, `.partial`, `.partial~`, `.crdownload`, `.download`, `.tmp`, `.temp`, `.swp`, `.swo` or `.swx`, or in a final `~`. These are unfinished downloads, scratch files and editors' backups. When a download finishes it is renamed, and the new name arrives.
- Emacs's auto-save names, `#notes.txt#`.
- The names `4913` (vim's write test), `desktop.ini` and `Thumbs.db`.
- Files that were already in the folder when the watch began. One that is still being copied then is said once as "Arrived again" when it finishes.

## diro watch

`diro watch [PATH]` is the same for a shell. It watches the current folder, or the one you name:

```bash
diro watch in
```

It prints "Watching in for new files. Ctrl+C stops." and then one line for each arrival, as the app does. Ctrl+C stops: "Stopped watching in: 1 arrived".

- `--settle SECONDS`: wait this long with no change. The default is 2. Use a longer time for a slow producer.
- `--print0`, `--null` or `-0`: write only the paths, separated by NUL characters, for `xargs -0`. The words go to standard error. Unfinished names such as `.part` and `.crdownload` are never written.

```bash
diro watch --print0 in | xargs -0 -n1 echo
```

The exit codes are 0 when watched until stopped, 1 when the folder could not be watched, 2 for wrong use, such as a bad option, and 3 when the watch ended early because the folder went away. A folder that is not there says "Could not watch nofolder: it does not exist."

## Limits

- On Linux and macOS there is no check for a program that still holds a file open. A producer that pauses for longer than 2 seconds is said early, and then said again as "Arrived again" with the final size. `--settle SECONDS` in the shell command is the remedy. The app has no quiet-period setting in alpha 1.
- A file renamed into the folder waits only half a second, because a producer that writes a file elsewhere and renames it in has finished. On Windows only a rename inside the watched folder gets this; a file moved in from another folder is seen as new and waits the full 2 seconds.
- Escape pressed to clear letters you typed to find a row also stops the watch, because the typed letters are cleared before the watch sees the key.
- A network share is checked, not watched, and its wait is long.

[Known limits](known-limits.md) keeps the list current.

## See also

- [Operations and undo](operations.md)
- [The panels](panels.md)
- [Search and the filter language](search.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Terminals](terminals.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
