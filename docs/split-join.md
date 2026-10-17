# Split and join

This guide covers splitting a big file into numbered parts and putting the parts back together: the sizes diroxide offers and where each comes from, what the plan says before anything is written, the checksum file that travels with the parts, how a join finds its parts and names a gap, the verdict, undo, the `diro split` and `diro join` commands, and how other programs read the same parts. Split and join come with alpha.2.

What this guide says about the dialogs and the spoken lines comes from their design and from the tests that check each sentence. Nothing in alpha.2 had been heard on a screen reader when this guide was written.

## What split does

Split file has no key of its own. It is in the File menu (Alt+F, then P) and in the palette. It cuts the focused file, or each marked file in turn, into numbered parts, and writes them to the other panel. With no file chosen it says "Split works on files. No file is chosen." A file called `report.pdf` becomes:

- `report.pdf.001`, `report.pdf.002`, and so on: the parts. The name keeps its extension, so a lost checksum file costs nothing.
- `report.pdf.crc`: the checksum file, written last. When it is there, the split finished.

These are the names every joiner understands. `cat` on Linux and macOS, `copy /b` in a Windows command prompt, 7-Zip and Total Commander all put the parts back together without diroxide.

## The sizes, and where each comes from

Split asks for a size in a list. Each row says the use first and the size after it, and its details line gives the exact byte count. These are the built-in presets:

- Email attachment, 18 MB. Parts of 18,000,000 bytes. Mail turns every three bytes into four characters in lines of at most 76 (RFC 2045, section 6.8), about 37 percent more. A 25 MB mail limit counts the message after that, so a file part of 18 MB is about 25 MB of mail.
- GitHub file limit, 100 MiB. Parts of 104,857,600 bytes. GitHub blocks a file over 100 MiB.
- CD, 700 MB. Capacity 737,280,000 bytes, which is an 80-minute CD-R (360,000 sectors of 2,048 bytes). Parts of 729,907,200 bytes, with 1 percent kept free for the file system.
- FAT32's largest file, 4 GiB less one byte. Parts of 4,294,967,295 bytes, the most the 32-bit size field of a FAT32 file holds.
- DVD, 4.7 GB. Capacity 4,700,372,992 bytes, a single-layer DVD+R, the smaller of the two figures in use. Parts of 4,653,369,262 bytes, with 1 percent kept free.
- Blu-ray, 25 GB. Capacity 25,025,314,816 bytes, a single layer. Parts of 24,775,061,667 bytes, with 1 percent kept free.
- Floppy, 1.44 MB. Parts of 1,457,664 bytes, the data area of a formatted 1.44 MB disk: 2,847 clusters of 512 bytes. The raw size is 1,474,560 bytes, but the boot sector, the two copies of the file table and the root folder take the rest. Nothing more is kept free.

Each count is tied to its source by a test, so a number cannot drift from the page it came from.

The list is titled "Split into", and each row is read with its details and its position: "Split into, Email attachment, 18 MB, Parts of 18,000,000 bytes, 1 of 9". The seven presets come first, in the order above. Two more rows follow:

- "Equal parts, a number you choose": a prompt, "Number of parts", then diroxide divides the file. Type a whole number, 2 or more.
- "Custom size, with its unit": a prompt, "Part size, such as 20MB or 100MiB".

Enter on a row plans the split. Something that cannot be read is explained, and nothing is split: "Type a whole number of parts, 2 or more. Nothing was split." and "Type a size with its unit, such as 20MB. Nothing was split."

A custom size takes a unit, in the prompt and on the command line: KB, MB, GB and TB are powers of 1000, and K, M, G, KiB, MiB and GiB are powers of 1024. `20MB` is 20,000,000 bytes and `100MiB` is 104,857,600.

### Adding your own size

Add your own preset in `config.toml`. Each `[[split_presets]]` table has a name, a size in bytes and, if you like, the percent to keep free:

```toml
[[split_presets]]
name = "Zip disk"
bytes = 100663296
```

```toml
[[split_presets]]
name = "Scratch DVD"
bytes = 4700372992
headroom_percent = 2
```

Your presets follow the built-in ones. One with a built-in name replaces that preset, in any letter case, so you do not have to copy the whole table. `headroom_percent` is from 0 to 50. The setting is `split_presets` in [the settings reference](settings-reference.md). In the Settings list the row opens a list that you can read, "Split presets, 7 presets, Email attachment, 18 MB, ...", and it says to add your own to `config.toml`; the row does not take typed text.

## What the plan says first

Before anything is written, the plan says the outcome, the use first:

- "12 parts of 20 MB, the last 3.1 MB"
- "223 MB in all"
- "Enough space: 12 GB free on D:"

Then the question, starting on No: "Split report.pdf into 12 parts, 223 MB? No is the default. y or n". Answer y and the job says "Split started: 1 item to out". The details line gives the exact bytes: "Parts of 20,000,000 bytes, the last 3,100,000; 223,100,000 bytes in all". A file that divides exactly has no empty last part.

If the other panel's drive does not have room, the plan says so first: "Not enough room on D:", then the numbers, then "Go on anyway? No is the default. y or n".

- More than 999 parts asks first: "This makes 1,500 parts. Go on? No is the default. y or n", and a second line says "Splitting report.pdf: past 999, parts are 1000 and up." Past 999 the numbers just grow: `.1000`, `.1001`. Answer no and the file is left out.
- More than 99,999 parts is refused, with the size to use instead: "Too many parts: 150,000. Use parts of 6.1 MB or more."
- An empty file has nothing to split: "it is empty: nothing to split".

## Running it, cancelling it, undoing it

- Progress is by part, in the job engine's words: "Splitting 4 of 12 parts, 1.4 of 4.0 GB", and for a join "Joining 4 of 12 parts, 1.4 of 4.0 GB". In screen-reader mode it changes at most every ten seconds. The estimate of time left is added only where the line still fits in 40 characters.
- Each part is written under an unfinished name and given its name when it is whole. Cancel with Escape and no part and no checksum file is left behind.
- If the file changes while it is split, the split fails: "report.pdf changed while it was split". Nothing is trashed.
- The end counts the checksum file with the parts: "Split finished: 5 done." for four parts and their `.crc` file.
- Ctrl+Z undoes a split: the parts go to the trash.

## The checksum file

The `.crc` file records what the whole file should be, so any program can check a join. It is three lines, in this order, in UTF-8 with a carriage return and a line feed at the end of each:

```
filename=report.pdf
size=223100000
crc32=21EB719A
```

The file name has no folder. The size is the byte count in decimal. The checksum is a CRC-32 of the whole file in eight uppercase hexadecimal digits. The format is the one Total Commander and Krusader write.

diroxide reads checksum files from other programs tolerantly: the key names in any letter case, one line or several, either kind of line ending, either letter case in the checksum, and UTF-8 first, then Windows-1252. A key it does not know is ignored and said. It looks for `report.pdf.crc` and then `report.crc`, and accepts the file only if the `filename` in it matches the set. A SHA-256 line comes with alpha.7.

Whether Total Commander finds `report.pdf.crc` from `report.pdf.001` has not been tried. [Known limits](known-limits.md) says so.

## What join does

Join files has no key of its own. It is in the File menu (Alt+F, then J) and in the palette. It starts from any part. Focus part 5 of 12 and run it: diroxide finds the rest by name and number, in numeric order, never as text, so `.1000` comes after `.999`. It writes the whole to the other panel. It has no dialog: there is only the question, with the listing under it. On a file that is not a numbered part it says "Not a part: report.pdf. Join starts from a numbered part, such as report.pdf.001."

Before it writes anything, it lists what it found, so you can check:

- The question, first, starting on No: "Join 12 parts into report.pdf, 223 MB? No is the default. y or n".
- Each part in order, with its size: "Part 001, 20 MB".
- Whether it can check the result: "Checked against report.pdf.crc", or "No checksum file: the join is not checked."
- The room: "Enough space: 12 GB free on D:".

### Gaps, odd sizes and other problems

A problem is named first, and a join that would be wrong does not start.

- A gap: "Part 004 is missing. Nothing was joined." With more than one: "Part 003 is missing, and 1 more. Nothing was joined." Nothing partial is ever offered.
- An empty part: "Part 002 is empty. Nothing was joined."
- A part of the wrong size: "Part 007 is 18 KB, expected 20 MB." Every part but the last should be as long as the first. diroxide asks "Join anyway? No is the default. y or n", starting on No, and No cancels the whole join. It never leaves a part out to make the rest fit.
- Parts that start at 000, as Krusader numbers them: "Parts start at 000." If `.000` and `.001` are both there: "Part 000 is there too. Join from 000? No is the default. y or n". Yes joins the set that starts at 000.
- Two widths of number, such as `.001` and `.0001`: "Parts are numbered two ways, 001 and 0001. Join the 001 set? No is the default. y or n".
- A file that is not a numbered part: "Not a numbered part: report.pdf. Nothing was joined."
- A spanned zip (`.z01`, `.z02`, then `.zip`) or a RAR volume is not a byte split and cannot be joined by putting the parts end to end: "Spanned zip sets need the 7z tool. Nothing was joined."
- A part that changes while it is read stops the join and leaves nothing: "Part 004 changed while it was joined".

At most two parts are open at once, so ten thousand parts do not run out of file handles.

### The verdict, and the trash

When the join is done, diroxide says whether the whole is right, the fact first:

- "Joined report.pdf: checksum matches."
- "Checksum differs: report.pdf may be damaged."
- "Joined report.pdf: size matches, no checksum to check." The checksum file holds no CRC.
- "Joined report.pdf. No checksum file found."

After a join whose checksum matches, diroxide offers to move the parts to the trash: "Move the 12 parts to the trash? No is the default. y or n". It starts on No. Answer n and it says "The parts stay where they are." After any other verdict the parts stay without a question.

Ctrl+Z undoes a join: the whole goes to the trash and the parts are where they were.

## Split archives

A set whose name ends in an archive extension and a part number, such as `backup.7z.001` with its parts beside it, opens as one archive without being joined: diroxide reads the parts in order as if they were one file. Enter on `backup.7z.001` opens it as an archive, and the title says "backup.7z.001, archive of 12 parts". A numbered part of anything else, such as `photo.jpg.001`, is not an archive; join it with Join files. [Archives](archives.md) has the rest.

A part that changes while the archive is open stops the read with the words "part changed", and no other bytes are given.

## From a shell

```bash
diro split --presets
```

`diro split --presets` lists the sizes, use first, with the exact bytes after:

```text
Email attachment, 18 MB. Parts of 18,000,000 bytes.
GitHub file limit, 100 MiB. Parts of 104,857,600 bytes.
CD, 700 MB. Capacity 737,280,000 bytes, parts of 729,907,200, 1 percent kept free.
FAT32's largest file, 4 GiB less one byte. Parts of 4,294,967,295 bytes.
DVD, 4.7 GB. Capacity 4,700,372,992 bytes, parts of 4,653,369,262, 1 percent kept free.
Blu-ray, 25 GB. Capacity 25,025,314,816 bytes, parts of 24,775,061,667, 1 percent kept free.
Floppy, 1.44 MB. Parts of 1,457,664 bytes.
```

The size comes from a preset, a size or a number of parts:

```bash
diro split big.bin --preset email --into out
```

```bash
diro split big.bin --size 20MB
```

```bash
diro split big.bin --parts 4
```

`--into FOLDER` writes the parts there instead of the current folder, and `--replace` replaces parts of the same names and sends the old ones to the trash. A question is asked in the terminal, and answered No when there is no terminal.

```bash
diro join out/big.bin.002 --into out
```

`diro join PART` finds the set from any part and joins it into the current folder, or into `--into FOLDER`. It says the same lines as the panel: "Joined big.bin: checksum matches." and then the offer about the trash, which starts on No. `--replace` replaces a file of the same name and sends the old one to the trash.

Both commands exit with 0 when they are done, 1 when nothing was done or the checksum differs, 2 for wrong use and 3 when some parts failed, with the output saying which.

## Working with other programs

- Unix and macOS: `cat report.pdf.001 report.pdf.002 report.pdf.003 > report.pdf` joins the parts. Put the parts in order on the line, or use a pattern that sorts them.
- Windows: in a command prompt, `copy /b report.pdf.001+report.pdf.002+report.pdf.003 report.pdf` joins them. Use backslashes in paths there, because the command prompt reads a forward slash as a switch.
- 7-Zip: open `report.pdf.001`, or an archive such as `backup.7z.001`, and it reads the whole set.
- Total Commander and Double Commander: their Split and Combine commands work with this naming, and the `.crc` file carries the same three fields. Whether they find diroxide's checksum file by name has not been tried.

The parts diroxide writes joined with `cat` and with `copy /b` are byte for byte what diroxide's own join makes, which the tests check.

## See also

- [Archives](archives.md)
- [Operations and undo](operations.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings reference](settings-reference.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
