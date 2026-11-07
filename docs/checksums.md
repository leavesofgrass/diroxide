# Checksums and verifying

This guide covers the two commands that check a file's integrity, Checksum and Verify checksum, and the two shell commands that do the same without the panels, `diro sum` and `diro sum --check`.

The short version: Checksum computes a file's SHA-256 and lists it to read, copy, compare with a value you paste, or save as a `SHA256SUMS` file. Verify checksum finds the checksum list beside a download and checks the file against it, the verdict first. A match tells you the file is not damaged. It never tells you who made the file, and diroxide never calls a file safe or genuine because of one.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## Checksum

Checksum is in the File menu and the palette, and has no key. You can give it one in `keymap.toml` ([Customizing keys](customizing-keys.md)). It acts on the marked files, or on the focused file when nothing is marked.

- "Computing the SHA-256 of example-tool.zip" says it has started. It is a job in the jobs line, so Cancel stops it, and it ends in the jobs history as "Checksum finished: 1 done."
- A file is read once. SHA-256 is what publishers post, so it is the one shown. BLAKE3 is computed in the same read, so a pasted BLAKE3 value can be told apart without reading the file again.
- Folders in the selection are left out. A folder alone says "Checksum works on files; no file is chosen".

When the job ends, a list opens. Its title puts the meaning first and the start of the value after it: "SHA-256 of example-tool.zip, ba78 16bf". The rows are:

- The whole value, in lines of eight groups of four digits, so each line is 39 cells: "ba78 16bf 8f01 cfea 4141 40de 5dae 2223", then the second half.
- "Copy the value" puts the value on the clipboard, with no spaces, and says "Copied the SHA-256 of example-tool.zip".
- "Compare with a pasted value", below. It is there when one file was computed.
- "Save as SHA256SUMS", below.

Enter on a line of the value copies it, and Enter on an action does it. The help line says so: "Enter on a value copies it; Enter on an action does it. Escape closes."

For several files the title is "SHA-256 of 2 files", and each file has a row that says its name, then the start and the end of its value: "example-tool.zip: ba78 16bf, ending 15ad". Enter on a file's row copies that file's value. "Copy all as SHA256SUMS lines" copies the whole list and says "Copied the SHA-256 of 2 files". A file that could not be read has its own row, "Unreadable: NAME, WHY", and the title adds how many. Comparing with a pasted value is for one file at a time.

### Compare with a pasted value

Copy the value the publisher gives, choose "Compare with a pasted value", and paste it. The field is titled "Compare with the SHA-256 of example-tool.zip, paste the value". The verdict comes first, then one line more if it helps:

- "Match: example-tool.zip, SHA-256".
- "Does not match: setup.exe, SHA-256", then "The first difference is in group 6". The groups are the four-digit groups of the lines above, so a person who mistyped one digit can find it.

Case, spaces, dashes and colons in the pasted value are ignored. A value that is not the right length for the algorithm says so before it judges anything: "Not a SHA-256 value: 63 digits, needs 64". A value with a character that is not a hexadecimal digit says "Not a checksum: use 0 to 9 and a to f".

The length of the pasted value picks the algorithm:

- 64 digits are tried as SHA-256 and as BLAKE3, both already computed. If it is BLAKE3's, you hear "Match: example-tool.zip, BLAKE3" and then "Matches BLAKE3, not SHA-256".
- 128 digits are SHA-512, 32 are MD5 and 8 are CRC-32. diroxide computes that one now, says "Computing the SHA-512 of example-tool.zip", and then judges.
- A match for MD5 adds "MD5 is old, not safe against tampering." A match for CRC-32 adds "CRC-32 is for damage, not tampering."

### Save as SHA256SUMS

"Save as SHA256SUMS" writes a checksum list into the folder of the files, one line a file in the common form that `sha256sum` writes and reads: the value, two spaces and the name. diroxide says "Saving SHA256SUMS", then "Saved SHA256SUMS in work".

It never writes over a file. If `SHA256SUMS` exists, the new list takes the first free numbered name and says so: "Saved SHA256SUMS (2) in work". The panel lists the new file.

### In the details view

The details view's Checksum row keeps what you computed this session, while the file is as it was: "Checksum: SHA-256: ba78 16bf, ending 15ad". Until you compute one it says "Checksum: not computed". [The details view](details.md) has the rest.

## Verify checksum

Verify checksum is in the File menu and the palette, and has no key. Use it on a downloaded file, or on the checksum list itself.

### On a file

Verify checksum looks beside the file for a list that may hold its value, in this order:

1. The file's own list: `NAME.sha256`, `NAME.sha512`, `NAME.md5` and `NAME.blake3`.
2. A shared list: `SHA256SUMS`, `SHA512SUMS`, `MD5SUMS` and `B3SUMS`.
3. The same four shared names ending in `.txt`.

If one list is found, it is used. If several are found, a list opens to choose from, titled "Checksum lists beside example-tool.zip", each row the list's name and its number of files: "example-tool.zip.sha256, 1 file", "SHA256SUMS, 1 file". With none: "No checksum list beside setup.exe". If the list has no line for the file: "SHA256SUMS has no line for setup.exe".

"Verifying example-tool.zip" says it has started. Only the focused file's line is checked, and the file is read again, because no stored value ever stands in for a check.

### On a list

With the focus on a checksum list itself, such as `SHA256SUMS`, `files.md5`, `report.pdf.sha256` or `checksums.sfv`, Verify checksum reads every file the list names again. The names are relative to the list's own folder. A name that is absolute or leaves the folder is skipped and said: "Skipped a name that leaves the folder: ../x".

The forms diroxide reads are the ones in common use:

- The GNU form, `VALUE  NAME`, and `VALUE *NAME` with the binary marker. This is `SHA256SUMS`.
- The BSD form, `SHA256 (NAME) = VALUE`.
- One file's list, such as `report.pdf.sha256`, holding the value alone or one GNU line.
- `.sfv`, `NAME VALUE`, which holds a CRC-32.

Lines that start with `#` and blank lines are passed over. A line that no form reads is counted and named: "A line cannot be read: line 4". A list that names no algorithm and holds 64-digit values is tried as SHA-256 and as BLAKE3 in one read. A SHA-1 value is read from an old list and said, not checked: "Not checked: NAME, SHA-1".

### What you hear

A list opens with the verdict as its title, and the verdict is also what the status line says first.

- "All 12 files match SHA256SUMS", or "1 file matches SHA256SUMS" for one.
- "2 of 12 files do not match" when only differences stand against matches, and "1 file named in the list is missing" when only missing files do.
- "1 differs, 1 missing, 1 match" when they mix. The counts say what is wrong first and the matches last.

The rows come in this order:

1. The failures, each with the name first: "Does not match: notes.txt, SHA-256", "Missing: gone.txt", and "Unreadable: NAME, WHY" for a file that could not be read.
2. "Matches BLAKE3, not SHA-256" for a value that named no algorithm and matched the second of the two.
3. The line about the list's signature, below.
4. "This shows the files are not damaged. It does not show who made them." This line comes only when something matched.
5. "Show the 1 match", which lists each match when you ask: "Match: example-tool.zip, SHA-256".

### The signature line

A checksum list proves nothing about who made it unless it is signed, and a match against an unsigned list means only that the file is the one the list describes.

- "List not signed." says no signature file sits beside the list.
- "Signature not checked: SHA256SUMS.asc" says one does, and names it. diroxide looks for `NAME.asc`, `NAME.sig` and `NAME.minisig` beside the list. Checking it comes in a later pass of this alpha, so today the line only names the file.

## From a shell

`diro sum` prints a file's checksum as one plain line, the name first, for scripts and for a Braille display.

```bash
diro sum example-tool.zip
```

prints "example-tool.zip, SHA-256, ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad". A file that cannot be read says "notes.txt: could not be read: WHY" and the command goes on with the others.

- `--algorithm NAME` (or `-a`) picks `sha256` (the default), `sha512`, `blake3`, `md5` or `crc32`. Any case works. SHA-1 is not computed.
- `--list` prints the lines a list holds instead, `VALUE  NAME`, as `sha256sum` writes them. Redirect them into `SHA256SUMS`.
- `--check LIST` (or `-c`) reads every file the list names again and prints the verdict first, then the failures, the same lines as the program. It takes one list, and it reads the algorithm from the list, so `--list` and `--algorithm` are refused with it.

```bash
diro sum --check SHA256SUMS
```

The exit codes are those of `sha256sum --check`: 0 when done, and with `--check` when every file matches; 1 when a file could not be read, and with `--check` for any difference, missing file, unreadable file or line it could not read; 2 for wrong use. `diro sum --help` shows the usage.

## Known limits

- The signature beside a list is named and not checked yet. The words "List is signed" are not said anywhere in this build.
- The details view's Checksum row holds only the values computed in this session.
- SHA-1 lines are read from old lists and not checked, because diroxide computes no SHA-1.
- Nothing about how the two lists read on a Braille display has been heard yet.

## See also

- [The details view](details.md)
- [Split and join](split-join.md), which writes a checksum file for its parts
- [Tools and diro doctor](tools.md)
- [Operations and undo](operations.md)
- [Security](security.md)
- [Known limits](known-limits.md)
- [Customizing keys](customizing-keys.md)
- [The documentation index](README.md)
