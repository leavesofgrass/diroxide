# The details view

This guide covers the details view: Alt+Enter on a file, folder or archive, the facts it lists and the order they come in, the Privacy and Checksum rows, Show all with ExifTool, and the three commands that take a file's details without opening the list: Say details, Copy details and Copy all details.

The short version: Details lists every fact diroxide has about the focused entry, one fact on each row, its label first, in a fixed order. The list opens at once with what the panel already knows. The facts from inside the file arrive a moment later and join the list, and the focus stays on the row you were on.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## Opening it

Alt+Enter opens the details of the focused entry. Windows Terminal keeps Alt+Enter for full screen, so the sure way is the File menu: Details (Alt+F, then D). The palette has it too.

You hear the list's title and its first row together: "Details of photo.png, Type: PNG". The title is "Details of" and the name. Up and Down read a row at a time, and Escape closes the list.

The list already holds what the panel knows, with "Privacy: reading" and "Checksum: not computed" among the rows. When the file's own facts have been read, the status line says how many facts it found: "Read 9 more details". The focus stays on your row; the new rows come in around it, in their places.

## The order of facts

Every row is one fact, "Dimensions: 4032 by 3024", the label first. The list is in a fixed order, in ten groups. The groups are not headings, only the order the rows come in, so the most important facts are at the start.

1. What it is: "Type: PNG", "Name: photo.png", and "File group: Images". For a type that is also in a second group: "File group: Images, Screenshots".
2. What is in it: "Dimensions: 4 by 3" for a picture, the length of a song, the pages of a document, the entries of an archive: "Entries: 8 files, 2 folders".
3. Its size: "Size: 48 KB (49,152 bytes)", the words first and the exact bytes after. A file under 1,000 bytes says "512 bytes" and no more. A folder says "Size: folder size not computed".
4. Its dates: "Modified:", "Created:" and "Accessed:", each with the date and time in words, where the system has them.
5. Flags: "Attributes:", "Hidden: yes", the privacy row, and the line that says when a file's name and its contents disagree, "Name says PNG; contents say JPEG".
6. Yours: "Labels:", the labels you put on the file.
7. Place: "Path:", the full path with any hidden character spelled out, and "Link to:" for a link.
8. Technical facts, such as a codec, a bit depth or an encoding.
9. What the system keeps: "Owner:", "Group:" and "Checksum:".
10. Where it came from, and how it was read: facts about the program or device that made it, and the "Not read" rows.

A fact the file does not have is not listed. The two exceptions are Privacy and Checksum, which are always there for a file or an archive, because "nothing private found" and "not computed" are answers. A folder has neither, and has no Show all.

A name that needs care adds its reason and the name spelled out in the first group.

Rows from inside the file, such as a photo's camera, a song's tags or a document's author, join their groups as they are read. A sound or video file's details run ffprobe when you open them, if it is installed, because you asked; they never run by themselves while you move through a folder. A reader of the file itself is given two seconds and 64 MiB, and ffprobe, which is a program, is given 12 seconds. A picture's words come from its header alone and run nothing.

When a reader cannot read something, the last group says so, with the reader in words first and then why: "Not read: picture facts, the file ends too soon". The readers are named as "archive facts", "sound tags", "sound properties", "PDF properties", "document properties", "book properties", "photo facts", "text facts", "picture facts" and "sound and video facts".

### A folder

A folder lists its type, name, dates, place and the rest of what the panel knows, and "Size: folder size not computed". It has no Privacy row, no Checksum row and no Show all row.

## The Privacy row

"Privacy:" says whether the file carries something that could say who made it, where or with what.

- "Privacy: nothing private found".
- "Privacy: location, device": the kinds it found, in a fixed order. The kinds are location, device, people, comments, thumbnail, hidden data, origin and other.
- "Privacy: reading", while the file's facts are still being read.

The private rows themselves are listed in their groups, with the other facts. They are shown and never sent anywhere. Copy details leaves them out unless you ask for all.

## The Checksum row

"Checksum: not computed". The details view does not compute it, and no command in the program does yet. The checksum engine is built, and the command that asks for it is not in the program yet.

## Show all

For a file or an archive, the last row is "Show all: every tag, read by ExifTool". Press Enter on it to ask ExifTool for every tag it reads. It runs once for the file, and the status line says "Show all: reading with ExifTool" while it works.

- The tags come after the row, one a row, in ExifTool's own order, each with its group in parentheses: "Make (EXIF): Example Camera". You hear "2 more tags, from ExifTool", then the first tag.
- The row itself then reads "Show all: 2 tags below, from ExifTool". Enter on it goes back to the first tag, and ExifTool is not run again.
- Without ExifTool: "More details: ExifTool not found". Nothing runs. [Tools and diro doctor](tools.md) says how to get it.
- A file that is not on a local disk: "More details: not on a local disk".
- ExifTool found nothing more: "More details: ExifTool found no more".

## Say details

Say details says one line about the focused file and leaves the focus where it is. No list opens. It is the same sentence the quick view's preview starts with: "PNG, 4 by 3, 8-bit RGBA" and the size. A folder says "folder".

## Copy details and Copy all details

Both put the details on the clipboard, one fact a line, with the title first: "Details of notes.txt". From a panel they read the file first, so the copy holds the same facts the list would.

- Copy details leaves out the private rows: "Copied 14 details, 2 private left out". With none to leave out it says "Copied 14 details".
- Copy all details keeps them: "Copied all 16 details".
- Show all's tags are copied once Show all has read them. The private ones among them follow the same rule.

Say details, Copy details and Copy all details have no key. They are in the File menu and the palette, and you can give any of them a key in `keymap.toml` ([Customizing keys](customizing-keys.md)) or a button in a bank.

## Known limits

- A details read cannot be cancelled when you close the list. It is bounded by its own time limits, and the answer is dropped.
- Show all needs a file on a local disk.
- Folder sizes are not computed here. Nothing in the details view walks a folder.
- Nothing about the order of rows, "Read 9 more details" or how Show all pans on a Braille display has been heard yet.

## See also

- [Panels](panels.md), which lists what a row says
- [Links, permissions and times](links-permissions.md)
- [File types and groups](file-types.md), for "Type:" and "File group:"
- [Tools and diro doctor](tools.md), for ExifTool and ffprobe
- [The quick view](quick-view.md)
- [Privacy](privacy.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
