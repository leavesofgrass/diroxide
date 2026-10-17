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

- A zip says "Wrong password for course.zip. Try again? y or n". A 7z cannot tell a wrong password from damaged data, so it says "Wrong password or damaged data. Try again? y or n". Answer y to get the prompt again. You have three tries. Answer n or press Escape to skip the locked members.
- The prompt hides what you type. If you are not sure a key registered, the default shows one asterisk for each key (`secret_echo`). Check Caps Lock and the keyboard layout. Passwords are case sensitive.
- An archive with a locked header, a 7z with encrypted names, cannot be listed at all without the password: "Could not open a.7z: a password is needed. Enter the password and try again."
- Test archive says what it could not check: "Not checked: 3 of 4 members."
- diroxide never takes a password from the command line, and `diro archive` asks only in a terminal. With no terminal it says "No password taken: passwords are typed in a terminal."
- A zip may use the old ZipCrypto password. diroxide reads it and warns: "Weak encryption in a.zip (ZipCrypto)." It never writes it.

## An archive asks a question about its size

A question such as "Expands to 40 GB from 40 KB. Unpack? y or n" means a member or the whole job would write far more than the archive's size suggests. Real archives that squeeze well, such as a log or a disk image, can do this, and so can an archive made to fill your drive. The question starts on No. Answer n unless you trust where the archive came from. Test archive reads every member without writing anything, so it is a safe way to look. Other questions are "big.bin is 4.3 GB. Read it? y or n", "Needs 33.6 MB of memory for a.7z. Go on? y or n" and "Read 3.2 GB first to open note.txt? y or n". [Archives](archives.md) explains each one and the settings that move its limit, such as `archives.ask_total_gib` and `archives.max_ratio`.

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

Windows lets only a program with the right to make symbolic links do so. Turn on Developer Mode in Windows Settings (System, then For developers), or start diroxide as an administrator. For a folder, diroxide offers "Make a junction instead? y or n". A junction needs no right and points by the full path. On a FAT or exFAT drive no link is possible: "Cannot link here: this place holds no links".

## The viewer or the quick view does not start

- F3 or Shift+F3 may not reach diroxide. Run the key test, or use the palette: View, and Quick view. [Terminals](terminals.md) lists the keys terminals keep.
- Windows Terminal keeps Alt+Enter, which is the details list in diroxide. Use Details in the File menu. It also keeps Ctrl+V and Alt with the arrow keys; Paste, Back and Forward are in the Edit and Go menus.
- A terminal built into another program, such as a desktop app's terminal pane or an editor's, may keep other keys for itself, for example Alt+F for its own menu or Ctrl+Q. Run `diro keys test` in it and see what arrives. If Ctrl+Q does nothing, File, then Quit, in the menu bar (F9) quits. diroxide's key tables do not list such terminals yet.

<!-- to confirm with the wiring and the owner's probe: the keys that the desktop app's built-in terminal keeps, the menu path to Quit, and the exact palette names for View and Quick view (the View menu may also hold Quick view). -->

## Code has no colors in the viewer

The viewer says why, once, when you open the file: "Highlighting off: over 512 KiB" (the file is larger than `viewer.highlight_max_kib`), "over 10,000 lines", "a line is too long", "Highlighting stopped: took too long", or "not in this build" for a slim `diro`. In screen-reader mode colors are off by default; the `highlighting` setting turns them on, and the commands that say each word's kind, in the palette, work either way. [The viewer](viewer.md) has the details.

## diroxide asks before it quits

It asks only when a job is running or a prompt still holds text you typed: "Quit? 1 job is running. No is the default. y or n". Answer y to quit anyway, and the job is cancelled. Answer n and press Escape to cancel the job first.

## A settings or keymap file has a problem

diroxide never throws a file away. It reports the problem at start in words, uses the defaults for that file, and keeps the old one with `.bad` added to its name. Config problems, in the Help menu (Alt+H, then C) and the palette, lists every problem with its file. [Settings](settings.md) has the details.

## `diro --gui` opens the terminal

That is true until alpha.6. `diro --gui` says "The window comes with alpha.6; the terminal opens instead." Use the terminal for now.

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
- [Known limits](known-limits.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
