# Privacy before sharing

This guide covers three commands that help you hand a file to someone else without handing over more than you meant to: Check before sharing, which says what a file carries that could say who made it, where or with what; Strip metadata into a copy, which makes a picture's copy without those details; and Remove the internet mark, which takes off the note a system keeps about where a download came from.

The short version: run Check before sharing on the files first. If a picture holds a place or a device's serial number, run Strip metadata into a copy and share the copy. Nothing here changes an original, and nothing is sent anywhere.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

All three commands are in the File menu and the palette. None has a key; you can give one a key in `keymap.toml` ([Customizing keys](customizing-keys.md)). Each acts on the marked files, or on the focused file when nothing is marked.

## What a file can give away

diroxide sorts the private details it finds into eight kinds, and says them in this order:

- location, such as where a photo was taken;
- device, such as a camera's or lens's serial number;
- people, such as a document's author, the person who last saved it, a company or a manager;
- comments, in a picture, a document or a sound file;
- thumbnail, a small copy of a picture stored inside it;
- hidden data, such as XMP and IPTC records, or a second picture riding along inside a JPEG;
- origin, where a file was downloaded from;
- other.

The details view shows the same details, and its Privacy row names the kinds: "Privacy: location, device". Copy details leaves them out. [The details view](details.md) has the rest.

## Check before sharing

Check before sharing reads each file and says what it holds. It only reads. It writes nothing, in the folder or anywhere else.

"Checking 2 files before sharing" says it has started. A list opens when it is done, and the title is the summary: "1 of 2 files hold private details". The other forms are "1 file holds private details" and "No private details in 5 files".

The rows name each file first, then what it holds, then each detail, then anything the check could not read:

- "beach.jpg: comments, hidden data", the file's name and then its kinds.
- Each private detail after it, label first: "Comment: from the beach". A place is said to two decimals, "Location: 45.50 north, 122.69 west", which is about a kilometer. Nothing read here is sent anywhere.
- "Not read: WHY" for a reader that could not read the file.
- "notes.txt: nothing private found" for a file with nothing.

After a file's details come the cautions that fit its kind, the things a strip cannot reach:

- "Its stored thumbnail may show the uncropped picture: crop first, then strip."
- "A strip leaves the picture itself and the file's name."
- "A PDF can keep earlier versions; a strip here does not reach them."
- "Comments and tracked changes inside a document stay in it."

The check reads photos, sound tags, PDF, Word, OpenDocument and EPUB properties, zip contents and the file's internet mark. A file the readers do not take says "nothing private found", which means nothing was found, not that nothing exists.

## Strip metadata into a copy

Strip works on JPEG, PNG and WebP pictures. It makes a copy beside the original and takes the private details out of the copy.

With a non-picture it says "Strip works on JPEG, PNG and WebP pictures; none is chosen". Otherwise it asks first, and the question starts on No: "Strip beach.jpg into a copy? No is the default. y or n", and "Strip 3 photos into copies?" for several, with the names listed under it. No says "Nothing stripped." and writes nothing.

On yes, the job starts: "Stripping beach.jpg into a copy", or "Stripping 3 photos into copies". It is a job in the jobs line and Cancel stops it. When it is done, a list opens, titled with the copy's name: "Stripped into beach-clean.jpg". Its rows say what changed:

- "Removed: XMP data, comments". The words for what can go are Exif data, XMP data, IPTC data, comments, text notes, time of change, other program data and pictures riding along.
- "Left in the copy: image size". The copy is read again, so this says what is really there. What a picture needs to look right stays: the color profile and the orientation, so a portrait photo does not turn sideways.

Several pictures end with "Stripped 3 photos into copies", or "Stripped 2 of 3 photos into copies" when one could not be done.

### Where the copy goes

- The copy is named `NAME-clean` with the original's extension: `beach-clean.jpg`.
- It never writes over a file. If that name is taken, the copy is numbered the way a copy is: "Stripped into beach-clean (2).jpg".
- The original is not touched, byte for byte. The picture's coded data is not decoded or changed either.
- Ctrl+Z puts the newest copy in the trash and says "Undid strip: 1 copy is in the trash".
- A picture of more than 256 MiB is not stripped.

### What a strip removes and keeps

Removed: the Exif block, with its place, camera, serial numbers, author, comments, maker's note and stored thumbnail; XMP; IPTC; JPEG comments and other program data; PNG text notes and the time of change; and in a JPEG the multi-picture segment and anything after the end of the picture, where a phone may keep a second picture with its own details.

Kept: the color profile, the JFIF record and Adobe's record (which says how a CMYK picture decodes), the coded picture itself, and the orientation.

A strip does not crop and does not rename. A thumbnail that shows the uncropped picture is removed with the rest, but a picture you meant to crop still has to be cropped first, which is what the caution says.

## Remove the internet mark

A system notes where a download came from, in a stream or attribute beside the file: Windows keeps a `Zone.Identifier` stream, macOS keeps the quarantine attribute, and on Linux a program such as curl may write the origin address. A system may warn when you open a file that has the mark.

Remove the internet mark takes it off. It asks first, and the question says what changes on this system and starts on No:

- Windows: "Remove the internet mark from setup.exe? Windows stops warning when you open it. No is the default. y or n".
- macOS: the same question with "macOS stops warning when you open it."
- Linux: the same question with "It will no longer say where it is from."

A line under the question says where the file came from. No says "The internet mark stays." Yes says "Removing the internet mark from setup.exe", then "Removed the internet mark from setup.exe". A file with none says "No internet mark on notes.txt". The file itself is not changed.

Ctrl+Z puts the mark back, exactly as it was: "Putting back the internet mark on setup.exe", then "Put back the internet mark on setup.exe". It works while removing the mark is the last thing you did. A job or a label change after it comes first in Undo, and the mark's undo is kept for the session only.

Remove the mark only from a file you trust. The warning is the system's protection, and taking it off removes it for that file.

## Known limits

- Check before sharing is not a job you can cancel. It is bounded by the readers' own time limits.
- Strip takes JPEG, PNG and WebP pictures only. A PDF, a document or a video is checked and not stripped.
- Strip needs the picture's container to be whole. A damaged picture is refused: "the picture is damaged".
- A strip does not crop. Crop a picture first when its thumbnail would show the part you cut.
- A removed internet mark comes back with Ctrl+Z only in the same session, and only while no job or label change has come after it.
- Nothing about how the check's rows read on a Braille display has been heard yet.

## See also

- [The details view](details.md)
- [Columns, notes and fields](columns.md)
- [Checksums and verifying](checksums.md)
- [Previews and the quick view's formats](previews.md)
- [Privacy](privacy.md)
- [Operations and undo](operations.md)
- [Customizing keys](customizing-keys.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
