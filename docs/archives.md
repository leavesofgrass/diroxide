# Archives

This guide covers archives: opening one like a folder, copying files out of it, unpacking, packing a selection into a new archive, testing, passwords, split archives, the limits that protect you, and the `diro archive` command.

What this guide says about the dialogs and the spoken lines comes from their design and from the tests that check each sentence. Nothing in alpha 1 had been heard on a screen reader when this guide was written, because the listening tests come after the build, so every "you should hear" line is what diroxide is built to say. If your experience differs, that is a finding.

## What diroxide reads and writes

- Read and write: zip, 7z, tar, tar.gz (also `.tgz`), tar.xz (also `.txz`) and xz, which holds one file.
- Read only: a single file compressed with gzip, such as `notes.txt.gz`, opens as an archive of one member. diroxide does not pack to `.gz`.
- Zip: zip64 archives are read. Passwords are AES-256 when diroxide writes one. Archives that use the old ZipCrypto password are read, with a warning.
- 7z: solid archives, and archives with encrypted names, are read. diroxide writes 7z with AES-256 and encrypted names.
- A split archive, such as `backup.7z.001` with its parts beside it, opens as one archive. See [split and join](split-join.md).
- Not read: bzip2 or Zstandard compression on its own, lzip, RAR, ISO images and spanned zip sets (`.z01`, `.z02`). diroxide says why in words, for example "it uses the RAR format, which is not supported". These come later, through the 7z program when it is installed. [Known limits](known-limits.md) lists them.

The format comes from the file's first bytes, not only its name. A zip named `data.dat` opens as a zip. When the bytes and the name disagree, the title says which format it found: "a.zip, 7z archive, in D:\docs". Open as archive opens a file by its bytes even when its name says nothing. It is in the File menu under Archive (Alt+F, then A, then O) and in the palette, and it has no key of its own.

## Opening and leaving an archive

Enter or Right on an archive opens it in the panel. Nothing is extracted to browse: diroxide reads the list of members and opens each file only when you ask.

- The title puts the kind first: "course.zip, archive, in D:\docs". Inside a folder of the archive it is "week1, in course.zip, left panel", the folder, where it is, then which panel. The window title says "week1, in course.zip - diroxide". For an archive inside an archive: "inner.zip, archive, in course.zip". For a split set: "backup.7z.001, archive of 12 parts".
- The status line says what opened, with the archive's size: "Opened course.zip: 10 items, 1.8 KB." Entering a folder inside says "week1, 3 items, in course.zip".
- Left, Backspace and Alt+Up go up one folder. At the top of the archive they go back to the folder that holds it, with the cursor on the archive, so Right enters it again.
- The path of a member is written with an exclamation mark: `course.zip!week1/a.txt`. `diro ls`, `diro archive list` and the other subcommands take it.

A tar.gz, a tar.xz and a solid 7z can only be read from the start, so listing a big one takes time, and diroxide keeps the list for the session. It does not yet say how far it has read. Opening one member of such an archive on its own may mean reading a lot first, so diroxide asks: "Read 3.2 GB first to open note.txt? No is the default. y or n". The size that triggers it is the `archives.ask_read_ahead_mib` setting.

## What a row says

A member's row says its name first, then what it is, then its size, and its position last, like any row: "notes.txt, text file, 3 KB, 5 of 10". A folder says "week1, folder, 3 items, 1 of 10". Anything unusual comes right after the name, so a long name cannot push it past the 40th character:

- "secret.docx, locked, Word, 40 KB": the member needs a password.
- "report.txt, name needs care, text, 1 KB": the name has a control character, ends in a space, is a Windows device name such as `CON`, or holds `.zip!`. The details line says which.
- "latest, link to v2, not followed": a link inside the archive. diroxide lists it and never follows it.
- "README, case clash with readme, text": two names that differ only by case. On a drive that ignores case they would collide.
- "a.txt, appears twice, last copy shown": the archive holds two members with one name. The last one wins, as it does for most programs.

Junk that archive tools add, such as `__MACOSX` folders and `._` files, is marked hidden. A name that would leave the archive, such as `../x`, is never listed as a member. When you unpack, it is named and skipped: "Skipped ../x: name leaves the archive."

## Archives are read only here

Nothing can be changed inside an archive in alpha 1. Copy into it, move into it, rename, delete and make folder all give the same answer: "Archives are read only here. Use Pack to make a new one."

To get a changed archive, pack a new one. Adding to an archive, deleting from it and renaming inside it come in a later alpha.

## Copy a file out

Mark members and press F5 with the other panel on a folder. diroxide unpacks them, reading the archive once, so copying five thousand files out of a tar.gz is one pass. The question is the unpack question: "Unpack 4 members into out? No is the default. y or n". The job says "Unpack finished: 4 done." F5 out of an archive always uses the default unpack settings, which are the ones the Unpack dialog starts with below. There is no "copy with options" yet. To choose the settings, use Unpack.

Ctrl+Z undoes it: the files diroxide made go to the trash.

To view a member without copying it, press F3 on it. The title names both: "notes.txt, in course.zip". [The viewer](viewer.md) has the rest.

## Unpack

Unpack writes the members of the focused archive to the other panel's folder. It is Alt+F9 in the graphical interface (GUI). In a terminal it is File, Archive, Unpack (F9 or Alt+F, then A, then U), or the palette. On an archive that is not open it unpacks all of it. Inside an open archive, mark members first and it unpacks those.

The dialog is a list. Its first row says what will happen and runs it on Enter: "Unpack hostile.zip into out". Every other row is "label: value", and Enter changes the row:

- "Where:" the other panel's folder.
- "Folder:" whether diroxide makes a folder named after the archive: "made when several items are at the top", "always made" or "never made". The default keeps anything from scattering into the folder you chose, and does not make `name/name/` when there is one top folder.
- "Paths:" "kept", or "names only, folders left out". Names only makes clashes likely, so it asks before it replaces anything.
- "When a name is taken:" ask, skip it, replace it (the old one to the trash), keep both, rename the new one, or replace it if newer. Undo can bring a replaced file back.
- "Links:" "left out, each one said" or "safe ones restored".

Enter on the first row makes the plan ("Planning the unpack"). The plan lists its rows under the question, which starts on No: "Unpack 4 members into out? No is the default. y or n". The rows say the room: "Writes 118 files, 40 MB. 12 GB free on D:." If the job needs more room than there is, the row becomes a refusal, "Needs 40 GB, only 12 GB free on D:.", and nothing is written. When members are locked: "Password: asked for 3 locked members".

An unpack is one pass over the archive, in the archive's own order, so a solid 7z is read block by block and a tar.xz of five thousand files is read once. The result puts problems first: "Unpack finished: 2 done, 1 skipped." Each skipped member has a line, the name first. Ctrl+Z undoes an unpack: the files it made go to the trash.

- "Skipped ../escape.txt: name leaves the archive."
- "Skipped latest: link not followed." A link in an archive is not made unless "Links: safe ones restored" is chosen. Then it is made only if it stays inside the folder you unpack to and the drive can hold links; otherwise the line says "link leaves the archive" or "this drive holds no links".
- "Skipped, link not followed: a-very-long-name.txt." A long name moves to the end so the reason stays in the first 40 characters.
- "Renamed 3 names: unsafe characters." The drive could not hold the name, so diroxide changed it.
- A device or other special entry is not unpacked: "device or special entry, not unpacked".
- "file and folder share the name, folder kept", when two members need one name.

diroxide never writes outside the destination. An archive that holds a link and then a file written through that link cannot send the file anywhere else, because links are made last and only when they stay inside. A member's modified time is kept, and a folder's is set after its contents. Permissions are made safe: no set-id or sticky bit, no write for the group or others, and run only where the owner has it. Owners recorded in the archive are never applied.

Not an archive: on a file that is not one, Unpack says "Not an archive. Unpack needs an archive file."

## Pack

Pack makes a new archive from the marked rows, or the focused row, in the other panel's folder. It is Alt+F5 in the GUI. In a terminal it is File, Archive, Pack (F9 or Alt+F, then A, then P), or the palette. It never changes an existing archive: it writes under a temporary name, tests the result, and only then gives it its name. A cancel, or a failed test, leaves any older file of that name untouched.

The dialog is a list. The first row says what will happen and runs it on Enter: "Pack 2 items into demo.zip". The other rows, in order:

- "Name: demo.zip". Enter opens a field. The name starts as the folder's name, or the single item's. Changing the extension changes the format.
- "Format: zip". Enter opens the choices: zip (the default, because everyone can open it), 7z, tar, tar.gz, tar.xz and xz, which takes one file only.
- "Level: normal (6 of 9)". The levels are fastest, fast, normal, maximum and ultra, each said with its number. A tar has none.
- "Where: the other panel, D:\backup".
- "Password: none (zip names stay visible)". Enter asks for one. A zip password is always AES-256 ("Password: AES-256 (zip names visible)"). For a 7z it says "Password: AES-256". For a tar or an xz: "Password: not for tar; use 7z".
- "Hide names: not for zip". For a 7z with a password, "Hide names: on" is the default, so the member names are encrypted too. Without a password: "Hide names: needs a password".
- "Solid: not for zip". For a 7z, "Solid: on" is how it gets small, at the price of reading a block to open one file. "Solid: off" reads each file alone.
- "Test after packing: on". The new archive is read back and every checksum is checked.

A row that does not apply stays in the list and says why, so you never have to guess what is missing.

The password is asked in a masked prompt, twice: "New password for demo.7z (hidden):", then "Same password again for demo.7z". Two that differ get "The two passwords differ. Nothing was changed. Try again." and an empty one "A password cannot be empty." When they match: "Password set. It is kept only for this pack." Escape in the prompt says "No password set." There is no way to recover a forgotten password.

Enter on the first row makes the plan ("Planning the pack") and asks the question, starting on No: "Pack 2 items into demo.zip? No is the default. y or n". The plan lists its rows under it: the format and level ("Format: zip, level normal (6 of 9)"), where it goes, the password, the solid choice, "Reads 300 files, 48 MB. 12 GB free on D:." (the room is shown and does not stop you), "Paths: relative to their folder", "Owner names: left out" for a tar (a tar usually records who owns each file, and diroxide leaves that out), and "Test after packing: on". Packing one file to an xz under a name that is not the file's name plus `.xz` adds "Unpacks as one: xz keeps no name", because an xz holds no file name and takes it from the archive.

The job says "Pack started: 2 items into demo.zip". The progress line is the job engine's: "Packing: counting, 120 so far" first, then "Packing 30 percent, 2 minutes left". Say Status (Ctrl+D) adds the detail: "12 of 300 items, 1.2 of 4.0 GB, 48 MB a second". The end puts the result and the check in one line: "Pack finished: 2 done. Checked 2 of 2 entries, all good." Ctrl+Z puts the new archive in the trash.

Packing to an existing name asks the usual question about a taken name. The old archive goes to the trash if you choose to replace it.

## Test an archive

Test archive reads every member and checks every checksum, and writes nothing. It is Alt+Shift+F9 in the GUI; in a terminal File, Archive, Test archive (Alt+F, then A, then T), or the palette. It has no dialog. On a file that is not an archive it says "Not an archive. Test archive needs an archive file."

- "Test started: demo.zip", then "Archive is sound: 2 members."
- "2 members damaged: first is week1/a.txt". The result list names every damaged member.
- A locked archive asks for its password. With none, the test says what it could not check: "Not checked: 3 of 4 members."
- A 7z cannot tell a wrong password from damaged data, so it says both.

Pack runs the same test on its own result, which is where "Checked 2 of 2 entries, all good." comes from.

## Passwords

- When the prompt comes depends on the archive. A zip lists without asking, because zip does not hide names. The prompt comes when you press F3 or F5 on a locked member. A 7z with encrypted names cannot be listed at all without the password, so it asks when you open it: "Could not open a.7z: a password is needed. Enter the password and try again."
- The prompt names its purpose and hides what you type: "Password for course.zip (hidden):". It never reads your typing back. By default it shows an asterisk for each key, which proves a key registered; the `secret_echo` setting can make it silent, and `secret_show_toggle` starts the field with the text shown, which anyone near can read.
- A wrong password gets up to three tries. Each retry is a question starting on No: "Wrong password for course.zip. Try again? No is the default. y or n". For a 7z: "Wrong password or damaged data. Try again? No is the default. y or n". Escape or no skips the locked members and says so: "No password given. Skipped 10 members of course.zip." After three wrong tries: "Gave up on course.zip after 3 wrong passwords. Skipped."
- An archive with ZipCrypto, the old zip password, is read, with a warning: "Weak encryption in a.zip (ZipCrypto)." diroxide never writes it.
- One password serves a whole job, and diroxide forgets it when the job ends. Leaving a locked 7z makes diroxide forget its password, and entering it again asks again. A password is never written to the log, the jobs list, a script or the undo record, and it is never taken on a command line. [Privacy](privacy.md) has more.

## Archives inside archives

A zip inside a zip opens like a folder, to four levels. A fifth level is refused in words. diroxide keeps an inner archive in memory up to 64 MiB, and past that in a `spool` folder in the state folder, never in the system's temporary folder. The most one operation may write there is 4 GiB, after a free-space check. The settings are `archives.max_nesting`, `archives.spool_memory_mib`, `archives.spool_disk_gib` and `archives.spool_folder`.

## Limits, and the questions they ask

A hostile or broken archive can claim to be small and expand to fill a drive, or ask a decoder for more memory than the computer has. diroxide checks each of these against a limit, and a limit reached is one of two things. A note on sizes: the settings count in GiB and MiB, which are powers of 1024, and the messages say sizes in KB, MB and GB, powers of 1000. So the default of 32 GiB is said as 34.4 GB.

A question: something large but possibly real. The question puts the fact first, ends in "y or n", and starts on No:

- "Expands to 40 GB from 40 KB. Read huge.bin? No is the default. y or n". One member grows far more than any normal file would (the ratio and the amount are `archives.max_ratio` and `archives.ratio_floor_mib`).
- "Expands to more than 34.4 GB. Go on? No is the default. y or n". The whole job passes the total it asks at (`archives.ask_total_gib`).
- "big.bin is 9.1 GB. Read it? No is the default. y or n". A single member is larger than `archives.max_member_gib`.
- "Needs 400 MB of memory for a.7z. Go on? No is the default. y or n". A 7z or xz asks for a large dictionary (`archives.ask_dictionary_mib`).
- "Read 3.2 GB first to open note.txt? No is the default. y or n", described above.

A refusal: something past any real use. It says what, in the usual shape: "Could not open bomb.zip: it passes the limit for bytes unpacked: 1 TB." The hard limits are the total (`archives.max_total_gib`), the dictionary (`archives.max_dictionary_mib`), the header and the list of members (`archives.max_header_mib`, `archives.max_index_mib`), the number of members (`archives.max_entries`, a million), the length of a name (`archives.max_name_bytes` and `archives.max_component_bytes`) and the memory of a decoder (`archives.max_memory_mib`).

A folder in an archive lists at most 500,000 entries, and says "Over 500,000 entries: listing the first 500,000".

An archive that is damaged says where: "it is damaged: its data ends early". A member that lies about its size stops at the size it claimed. Every limit has a setting; [the settings reference](settings-reference.md) lists them under Archives and split, each with its default and range. Raising a limit is your choice. The defaults are set high enough that ordinary archives rarely meet them.

## From a shell

These subcommands do the same jobs without the panels, and each prints one plain line a member, the path first. They read passwords only in a terminal, never from the command line. They use the built-in default limits. Only the program in the terminal or the GUI reads the `[archives]` table of your settings.

```bash
diro archive list course.zip
```

`diro archive list PLACE` lists every member. PLACE may be inside an archive, as `course.zip!week1`, or the first part of a split one. `--long` adds exact sizes, the packed size, the checksum and permissions in words. `--json` writes one JSON object a line with the real names kept, `--hidden` includes junk such as `__MACOSX`, and `--null` or `-0` writes only the paths, each ended by a NUL character, which is safe for any name. A list ends with a summary line: "10 items in course.zip, 934 bytes."

```bash
diro archive pack notes.7z notes.txt todo.txt --level maximum --password
```

`diro archive pack ARCHIVE ITEM...` packs the items, and the archive's name gives the format. `ARCHIVE -` reads NUL-separated paths from standard input, or one a line with `--lines`. The options are `--level` (fastest, fast, normal, maximum or ultra), `--password` (asked twice), `--hide-names` for a 7z, `--not-solid` for a 7z, and `--replace`, which sends an old archive of that name to the trash. Without `--replace`, a taken name is kept and the new archive is numbered.

```bash
diro archive unpack course.zip --into out
```

`diro archive unpack ARCHIVE` unpacks into the current folder, or into `--into FOLDER`. ARCHIVE may name a member or a folder inside, as `course.zip!week2/d.txt`. `--replace` replaces files of the same name and sends the old ones to the trash.

```bash
diro archive test course.zip
```

`diro archive test ARCHIVE` reads every member and checks it: "Archive is sound: 10 members."

The exit codes are the family's: 0 when done, 1 when nothing was done because the archive could not be read or written, 2 for wrong use and 3 when some members were skipped or failed, with the output saying which. A script can tell "nothing happened" from "some of it did". `diro archive --help` shows the usage.

## Settings

The Archives and split group of [the settings reference](settings-reference.md) holds nineteen settings: the seventeen limits above plus `archives.ask_read_ahead_mib` and `archives.spool_folder`. The `split_presets` setting is in the same group and is described in [split and join](split-join.md). An unpack keeps each member's modified time, as every copy and move keeps dates; there is no setting to turn that off.

## Known gaps

[Known limits](known-limits.md) lists everything alpha 1 does not do with archives. The ones you are most likely to meet: no adding to or changing an existing archive, no RAR or ISO, no progress line while a big tar.xz is listed, zip times read as UTC, and the names of older zips in Shift-JIS and other legacy encodings are not chosen yet.

## See also

- [Split and join](split-join.md)
- [The viewer](viewer.md)
- [Operations and undo](operations.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [Settings reference](settings-reference.md)
- [The documentation index](README.md)
