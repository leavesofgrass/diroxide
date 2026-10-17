# Troubleshooting

This guide lists common problems, one heading each, with what to try. It ends with how to report a bug. Where the cause is a known limit, the entry links to it.

## A key does nothing

The terminal may have kept the key. Run the key test, press the key, and read what diroxide says:

```bash
diro keys test
```

If the terminal sent nothing, use the palette (Ctrl+P, then type the command's name) or the menus (F9). Windows Terminal keeps Ctrl+V, the Alt arrows and Alt+Enter; [Terminals](terminals.md) lists them with the sure way around each, and a snippet that gives the keys back. On Windows the Menu key never reaches a terminal program; Shift+F10 opens the context menu instead.

If the key arrives but runs another command, check your own `keymap.toml`:

```bash
diro keys check
```

The Keys list (Settings, then Keys) shows each command's key and marks a key that many terminals keep with the words "check terminal". If you chose a key preset, your own lines in `keymap.toml` win over the preset, and the list says "changed" for yours and "preset" for the preset's. [Customizing keys](customizing-keys.md) describes the list.

## Typing moves the cursor when I did not mean it

With single-letter keys on, typing a letter finds a row by name. That also happens when text is pasted into a panel, or when dictation software types. On Windows a paste in Windows Terminal arrives as typed keys: letters jump to names, plus, minus and star open the pattern prompts, and a line break acts as Enter. Turn the typing off in Settings (`single_letters`), or unbind Ctrl+V in Windows Terminal as [Terminals](terminals.md) shows, so that the key reaches diroxide as a file paste instead of pasting text. Every command still has a key that is not a letter, a menu entry and a palette entry.

## Nothing is spoken, or it is spoken twice

- Check that your screen reader reads new text in the terminal. NVDA: Report dynamic content changes (NVDA+5) on. JAWS: Screen echo set to all (Insert+S). [Using diroxide with a screen reader](screen-readers.md) has the full list.
- Check `screen_reader_mode` in Settings (Alt+T, then S). The default `auto` behaves as on, because diroxide does not detect a screen reader yet.
- For NVDA in Windows Terminal, try both choices of "Speak new text in Windows Terminal via".
- If your screen reader speaks the cursor move and diroxide's message, you hear it twice. The `cursor_fallback` setting parks the cursor on the status line.
- The same message twice is blanked for 150 milliseconds so the second one is heard. If a message is still missed, say which.

## Tab or a refresh speaks a lot

A screen reader that reads new text speaks everything that changes on screen. The stacked layout changes less on Tab than side by side. Press Ctrl+P, type "layout" and press Enter. The `layout` setting keeps the choice. The `quiet_screen` setting draws as little as possible.

## The Braille display does not follow the row

- In NVDA, set Tether braille to automatically or to focus. In JAWS, turn on Braille follows active cursor.
- The display follows the hardware cursor, which diroxide keeps on the first cell of the focused row. If it does not follow, report your screen reader, its version and the terminal.
- NVDA and JAWS messages can cover the line for a few seconds. Shorten the message timeout in your screen reader's Braille settings.

## Rows look shifted or characters are missing

A known bug in the library that paints the terminal can shift the rest of a row after an emoji or a Chinese, Japanese or Korean name in some terminals. diroxide works around it by sending such a row again. Try the stacked layout, or `diro --inline`. If it still happens, report the terminal's name and version. [Known limits](known-limits.md) has the details.

## There is no color, or the colors are wrong

- If `NO_COLOR` is set in your environment, diroxide uses bold, underline and reverse only. Unset it to get colors.
- Choose another theme in Settings, or start with `diro --theme NAME`. Each theme says whether it meets contrast in words when you choose it.
- Nothing in diroxide depends on color alone, so every state is also in words.

## The terminal is in a strange state after diroxide ended

diroxide gives the terminal back when it quits, meets an error or panics. It does not yet do so when it is ended by a signal, by Ctrl+Break or by closing the window, so the terminal may be left in raw mode, with no echo. On Linux and macOS, type `reset` and press Enter. On Windows, close that tab and open a new one.

## The shell stays in the old folder after I quit

The folder hand-back needs the small function that `diro init` prints, named `dcd`, and you must start diroxide with `dcd` instead of `diro`. The function must be in your shell's startup file. See [the quick start](quickstart.md). If you quit with "Quit, shell stays where it was", the shell is meant to stay.

## "Folder not watched. Reload with Ctrl+R."

diroxide cannot watch a network drive, and it checks a removable drive only when you return to it and every two seconds. Press Ctrl+R to list the folder again. [Known limits](known-limits.md) has the details.

## "This drive has no trash" or "Path is too long for the trash"

On Windows the trash is used only on a fixed drive inside the computer, for a path of 259 characters or fewer. Anywhere else diroxide offers the listed permanent delete, starting on No. If you do not want that, copy the files somewhere with a trash first. [Operations and undo](operations.md) explains why.

## "Deleted but not found in the trash."

diroxide checked after a trash call and could not find the item in the trash. The item may be gone for good. Do not trust it to be recoverable. Please report what drive and what path length were involved.

## "Cannot start a program in a folder this deep"

On Windows a program cannot start in a folder whose path is 259 characters or longer. Go to a shallower folder, or move the files up. The same limit applies to opening a file: "Cannot open a file this deep".

## A copy did not finish

- The jobs line says what happened: "Copy had 8 failures: 290 done, 2 skipped." The entries that failed stay selected, so you can see what to try again. Job history, in the Tools menu (Alt+T, then J), lists each job, and Enter on one lists what failed or was skipped, and why.
- A failure names the file and the cause, and says what to do. "Could not copy report.pdf: another program is using it. Close it in the other program, then retry."
- A copy that stops moving says "Copying stalled at 30 percent". Press Escape to cancel the job.
- If diroxide was killed during a job, the next start says "Last session ended during a copy: 3 of 5 steps done." A leftover file whose name begins with `.diro-part-` is a partly written copy. Delete it yourself; diroxide never does.

## Undo says it could not undo something

Undo checks that an entry is as the job left it. If you changed the entry since, or another file took its name, that step is skipped and the undo says why. A permanent delete and a move across drives cannot be undone. [Operations and undo](operations.md) lists what can.

## An archive refuses my password

- A zip says "Wrong password for course.zip. Try again? No is the default. y or n". A 7z cannot tell a wrong password from damaged data, so it says "Wrong password or damaged data. Try again? No is the default. y or n". Answer y to get the prompt again. You have three tries. Answer n or press Escape to skip the locked members.
- The prompt hides what you type. If you are not sure a key registered, the default shows one asterisk for each key (`secret_echo`). Check Caps Lock and the keyboard layout. Passwords are case sensitive.
- An archive with a locked header, a 7z with encrypted names, cannot be listed at all without the password: "Could not open a.7z: a password is needed. Enter the password and try again."
- Test archive says what it could not check: "Not checked: 3 of 4 members."
- diroxide never takes a password from the command line, and `diro archive` asks only in a terminal. With no terminal it says "No password taken: passwords are typed in a terminal."
- A zip may use the old ZipCrypto password. diroxide reads it and warns: "Weak encryption in a.zip (ZipCrypto)." It never writes it.

## An archive asks a question about its size

A question such as "Expands to 40 GB from 40 KB. Read huge.bin? No is the default. y or n" means a member or the whole job would write far more than the archive's size suggests. Real archives that squeeze well, such as a log or a disk image, can do this, and so can an archive made to fill your drive. The question starts on No. Answer n unless you trust where the archive came from. Test archive reads every member without writing anything, so it is a safe way to look. Other questions are "big.bin is 9.1 GB. Read it? No is the default. y or n", "Needs 400 MB of memory for a.7z. Go on? No is the default. y or n" and "Read 3.2 GB first to open note.txt? No is the default. y or n". [Archives](archives.md) explains each one and the settings that move its limit, such as `archives.ask_total_gib` and `archives.max_ratio`.

A refusal that says "it passes the limit for" something has no question: the archive is past what diroxide will read. Raise the setting in [the reference](settings-reference.md) if you are sure.

## "Skipped ../x: name leaves the archive."

The archive holds a member whose name would write outside the folder you chose, such as `../x` or a path that starts at the root of a drive. diroxide skipped that one member and unpacked the rest. This is the protection working, not a failure. A well-made archive does not have such names, so if the archive came from someone you trust, tell them.

## "Archives are read only here."

You tried to change something inside an archive: copy into it, move, rename, delete, or make a folder. alpha.2 cannot do that, on purpose. Use Pack to make a new archive, or unpack the archive, change the files, and pack again.

## An archive will not open

The message says why, in the cause after the name: "Could not open a.rar: it uses the RAR format, which is not supported." The formats alpha.2 reads are zip, 7z, tar, tar.gz, tar.xz and xz. [Known limits](known-limits.md) lists the rest. If you think the file is an archive with the wrong name, try Open as archive in the palette, which goes by the file's bytes.

## "Part 004 is missing. Nothing was joined."

Join found the part numbers, and one is not there. Nothing was written. Put the part in the same folder, or check that its name is the same as the others up to the number, and join again. "Part 007 is 18 KB, expected 20 MB." means a part is the wrong size, often a copy that did not finish. The question that follows starts on No, and No cancels the join. [Split and join](split-join.md) has the other messages.

## "Part 004 changed while it was joined", or "part changed"

A part was written to, replaced or moved while diroxide read it. diroxide stops, gives no half-read data and leaves nothing behind. Wait until whatever is changing the parts has finished, then run the join again. The same sentence for a split, "report.pdf changed while it was split", means the file changed while it was cut.

## "Checksum differs: report.pdf may be damaged."

The joined file's checksum is not the one in the `.crc` file. A part was damaged, in a download or a copy, or the checksum file belongs to another version of the file. diroxide keeps the parts, and does not offer to trash them. Download or copy the parts again, then join again.

## "Cannot link: needs developer mode."

Windows lets only a program with the right to make symbolic links do so. Turn on Developer Mode in Windows Settings (System, then For developers), or start diroxide as an administrator. For a folder, diroxide offers "Make a junction instead? No is the default. y or n". A junction needs no right and points by the full path. On a FAT or exFAT drive no link is possible: "Cannot link here: this place holds no links".

## F3, or another new key, does nothing

- F3 may not reach diroxide. Run the key test, or use View in the File menu (Alt+F, then V) or the palette. The viewer's own keys are in the View menu's Viewer submenu (Alt+V, then V) and in the palette. [Terminals](terminals.md) lists the keys terminals keep. If Shift+F3 does nothing, your terminal may send it as F15. Quick view in the View menu (Alt+V, then Q) and in the palette turns the quick view on. [The quick view](quick-view.md) has the rest.
- Windows Terminal keeps Alt+Enter, which is the details list in diroxide. Use Details in the File menu (Alt+F, then D). It also keeps Ctrl+V and Alt with the arrow keys; Paste, Back and Forward are in the Edit and Go menus. Pack, Unpack and Test archive are Alt+F5, Alt+F9 and Alt+Shift+F9 in the desktop window only; in a terminal they are in the File menu under Archive (Alt+F, then A).
- A terminal built into another program, such as the terminal pane of a desktop app or an editor, may keep other keys for itself. In the one in some desktop apps, Alt+F and Ctrl+Q never arrive. Press F9 to open the menus, and quit from the File menu or the palette. Run `diro keys test` in such a terminal and see what arrives. diroxide's key tables do not list these terminals yet.

## Code has no colors in the viewer

The viewer says why, once, when you open the file: "Highlighting off: over 512 KiB" (the file is larger than `viewer.highlight_max_kib`), "over 10,000 lines", "a line is too long", "Highlighting stopped: took too long", or "not in this build" for a slim `diro`. In screen-reader mode colors are off by default; the `highlighting` setting turns them on, and the commands that say each word's kind, in the palette, work either way. [The viewer](viewer.md) has the details.

## diroxide asks before it quits

It asks only when a job is running or a prompt still holds text you typed: "Quit? 1 job is running. No is the default. y or n". Answer y to quit anyway, and the job is cancelled. Answer n and press Escape to cancel the job first.

## A settings or keymap file has a problem

diroxide never throws a file away. It reports the problem at start in words, uses the defaults for that file, and keeps the old one with `.bad` added to its name. Config problems, in the Help menu (Alt+H, then C) and the palette, lists every problem with its file. [Settings](settings.md) has the details.

## `diro --gui` opens the terminal

That is true until alpha.6. `diro --gui` says "The window comes with alpha.6; the terminal opens instead." Use the terminal for now.

## A filter says "Filter problem"

The line after "Filter problem:" says what is wrong, and a second line names the word and what to try: "Filter problem: size needs a number", then "Word 3. Try size over 10 MB." The second line is said once, after you stop typing.

- "and mixed with or": put brackets around one of them. diroxide never guesses which binds tighter.
- "write or, not |" or "write and, not &": use the word. Quote the character if it is part of a name.
- "unknown field": the word before a value is not a field name. The next line says "Did you mean size?" when it is close. To search for a field's name as a name, quote it.
- "write the year first": write 2026-10-11, not 10/11/2026.
- "write MB or MiB": a bare k, m or g is not a unit. KB, MB and GB are powers of 1,000, and KiB, MiB and GiB powers of 1,024.
- "depth works in searches": a panel shows one folder. Use Search.

In a shell, quote a filter that holds `*`, `(`, `)`, `|`, `&`, `>` or `<`, and prefer words to signs. `diro search docs size > 1 MB` redirects its output to a file named `1`. [Search and the filter language](search.md) lists the rest.

## A filter or search finds nothing, and I expected something

- Check the read-back. Say filter, in the palette, or `--explain` in a shell, says what diroxide understood: "name contains Report, match case". A capital letter makes a word match case exactly. Type it in lowercase, or put `anycase` before it.
- Hidden files are left out unless the panel shows them or the filter says `hidden`. The summary says "Hidden files skipped."
- A test on something not known matches neither the test nor its opposite. A folder has no size until it is counted: "1 unknown, folder size not counted." `type` matches only a type that is already known.
- A file with a NUL byte is skipped by a content search: "6 binary files skipped."
- Files named in `.gitignore` are left out only when you ask: `search_respect_ignore` or `--ignore-files`.
- A link to a folder is not followed, and another drive is not entered. The summary says so.

## A search takes a long time

Escape stops it: "Stopped at 214 found." Search reads each folder in turn and has no index.

- A network drive costs a round trip for every folder, so it is slower. diroxide says the rate: "Searching over the network: 400 items a second". It does not read file contents there without your yes.
- A content search opens every file. On Windows that is the slow part.
- A very large result stops listing at 100,000 rows: "Showing the first 100,000. Narrow the search." The count goes on. Narrow the filter.
- A folder blocked on a dead share cannot be interrupted: "1 folder is still being read." diroxide lets go of it. Restart the share, or leave it out of the search.
- Leave a big folder out of every search with the label Skip in searches.

## A label is gone, or Ctrl+1 does nothing

- A file that was renamed or moved loses its label, because a label belongs to the file's path. The Labels list shows the item as missing, and Delete in the label's items takes the label off. Label the file again at its new place.
- A label step can be undone only in the same session.
- Many Unix terminals do not send Ctrl with a digit. Use the Edit menu, Labels submenu (Alt+E, then B), then 1 to 7. [Labels and bookmarks](labels-bookmarks.md) has the rest.
- A label with a color diroxide does not offer, such as red or green, is reported at start and left out. Use blue, orange, purple, yellow, gray or brown.

## Compare says "Same text", and I can see a difference

Same text means the text is the same once what is left out is left out. Line endings are left out by default. The next lines of the report say what else differs: a byte-order mark, another encoding, a missing final line break, or "Ignoring changes in case." Turn the ignore options off to see every difference. A hidden character, a tab for spaces, and the same letters in another Unicode form are listed as changes with the reason: "Changed line 3, hidden character U+200B added". [Compare files](compare.md) lists the nine cases.

"Changed while comparing" means a file was written to during the run. Compare again when it stops changing. "Too large to compare as text" asks you to choose: the first 16 MiB as text, byte by byte, or cancel.

## Watch this folder does not say a file arrived

- A file counts as arrived only after its size, time and identity stay the same for 2 seconds, or 10 seconds on a place that is checked and not watched. The status line shows "Waiting for scan-004.pdf to settle" but diroxide does not say it.
- Unfinished names are never said: names that end in `.part`, `.crdownload`, `.tmp` and the like, and names that start with a dot. When the download finishes and is renamed, the new name arrives.
- Files already in the folder when the watch began are not arrivals.
- A producer that pauses for more than 2 seconds is said early on Linux and macOS, then said again as "Arrived again". In a shell, `diro watch --settle 10` waits longer.
- In screen-reader mode, at most one line is said in each progress interval (ten seconds). Several arrivals become "Watching in: 3 arrived".

[Watching a folder](watching.md) has the rest.

## A saved view keeps applying

A folder with a saved view gets it each time you enter: "Sorted by size, largest first; saved for this folder." To stop, use Forget this folder's view in the View menu (Alt+V, then G). The usual view returns when you leave.

## Starting clean

To start with factory settings, move the config folder aside by renaming it and start diroxide. [Settings](settings.md) says where the folder is. Your old files stay where you put them.

## Report a bug

When you report a problem, say:

- what you did, what you expected and what happened, in words;
- the output of `diro --version`;
- your system, the terminal's name and version, your screen reader's name and version, and whether a Braille display was in use;
- the lines of `terminal.log` in the state folder, which hold only what the terminal reported about itself and no file names. [Privacy](privacy.md) says what the log holds.

If a key is involved, add what `diro keys test` said. If an accessibility problem is involved, say so first. It is a bug, not a preference.

## See also

- [Terminals](terminals.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Archives](archives.md)
- [Split and join](split-join.md)
- [The viewer](viewer.md)
- [Links, permissions and times](links-permissions.md)
- [Search and the filter language](search.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Compare files](compare.md)
- [Customizing keys](customizing-keys.md)
- [Watching a folder](watching.md)
- [Known limits](known-limits.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
