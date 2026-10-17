# Links, permissions and times

This guide covers making links and changing where a link points, changing permissions and Windows attributes, what happens to times, and reading the details of an entry. These commands come with alpha.2. Changing the owner and setting a time by hand come with alpha.7.

What this guide says about the dialogs and the spoken lines comes from their design and from the tests that check each sentence. Nothing in alpha.2 had been heard on a screen reader when this guide was written.

## Links

A link is a second name that points at a file or a folder. Make symbolic link makes a link to each marked entry, or to the focused one, in the other panel's folder. It has no key of its own. It is in the File menu (Alt+F, then L) and in the palette, and the menu shows it only where the place can hold links. There are three kinds:

- Symbolic: a name that holds the path of its target. The target can be a file or a folder, on this drive or another.
- Junction: a folder link on Windows that needs no special right. It holds a folder only, and always points by the full path.
- Hard: a second name for the same file, on the same drive.

The command opens a list titled "Make link". Its first row says what will happen and runs it on Enter: "Make a link to main.rs in out". The rows after it:

- "Kind: symbolic, a pointer to the place". Enter steps through symbolic, hard ("a second name, one drive") and junction ("folders on Windows").
- "Target written: absolute, the full path". Enter switches to "relative, from the link".
- "Where: the other panel, D:\out".

Enter on the first row plans the link and asks, with the question starting on No. The job says "Link started: 1 item to out" and "Link finished: 1 done." Ctrl+Z undoes it: the link goes to the trash.

### Relative or absolute

A symbolic link writes its target in one of two ways, and the plan says which:

- Absolute, the default: the full path. The link keeps working wherever you move the link, as long as the target stays put.
- Relative: the way from the link's own folder, such as `../data/x.txt`, or `..\data\x.txt` on Windows. The link keeps working when you move a folder that holds both the link and its target. A relative link to another drive cannot be written, so the plan makes it absolute and says so: "Absolute link: the target is on another drive."

A junction and a hard link point by the full path, and the plan's note says so.

### Changing where a link points

Change link target is in the File menu (Alt+F, then I) and in the palette. On a link it asks "Point latest to", with the current target in the field and path completion, and writes the new target as a full path. On anything that is not a link it says "Not a link: report.pdf". The change is one step. diroxide makes the new link beside the old one under a temporary name, then swaps it into place, so there is no moment when the name points nowhere, where the system allows a swap. Otherwise it sends the old link to the trash first, and Ctrl+Z brings the old link back from the trash. On a drive without a trash, that fallback refuses and says so. On Windows the system will not swap a folder link in one step, so changing a folder link there always uses the trash.

### Where links cannot be made

diroxide checks before it plans. On a FAT or exFAT drive on Windows, inside an archive, or in a read-only place, it says so: "Cannot link here: this place holds no links".

### Windows: Developer Mode, and the junction

On Windows a symbolic link needs Developer Mode turned on in the system settings, or an administrator's rights. Without them diroxide says, in words and not as a system code:

"Cannot link: needs developer mode."

For a folder, diroxide then offers the way around: "Make a junction instead? No is the default. y or n". A junction needs no right at all, holds a folder only, and points by the full path.

## Permissions

On Linux and macOS, Change permissions changes who can read, write and run a file, or list, change and enter a folder. On Windows the same command changes the attributes. Both are a job: a plan, a question, a result, and Undo. The command has no key of its own. It is in the File menu (Alt+F, then W) and in the palette, and the menu shows it only where the place keeps permissions.

### Said in words first

The first row of the dialog and the plan say the result in words, with the octal number after it:

"Owner can read and write, group can read, others can read (644)"

The octal number is never the first thing said. For several entries the plan groups them by the change: "12 files: group gains read", "3 folders: group gains list and enter". The words for a change are short: "group gains write", "others lose list and enter", "everyone gains run", "turns on run as owner (setuid)".

A folder reads in its own verbs: list, change and enter instead of read, write and run.

### The dialog

The list is titled "Permissions". Its first row says what will happen and plans it on Enter: "Change permissions of main.rs", or "Change permissions of 3 items". The rows after it, on Linux and macOS:

- "Now: Owner can read and write, group can read, others can read (644)". What the entry has today.
- "Owner: read and write". Enter steps the owner through read and write, read only, read, write and run, read and run, and no access.
- "Group: read only" and "Others: read only", the same way.
- "Apply to: this item only", when a folder is selected. Enter steps through the choices below.

On Windows the rows are "Read only: off", "Hidden: off" and "Marked for backup: off". Enter turns one on or off. The attributes are read-only, hidden, system and archive.

If nothing would change the dialog says "Nothing to change: permissions already match."

Typing an octal number or a symbolic mode, a one-key Make runnable and a list of named presets are not in this alpha.

### Apply to: the recursive option

The "Apply to:" row has four choices:

- this item only
- files and folders inside
- only files inside
- only folders inside

When the change goes inside folders, the plan adds two rows: "Files keep their run setting." and "Folders stay enterable." A file does not gain the right to run unless someone could already run it, and a folder that gains read also gains enter, so a recursive change cannot make a tree unusable.

Other rules for any change:

- A link is never changed. A selected link is refused: "Could not change permissions of link.txt: links have none. Change the target instead." A link inside a folder is skipped.
- A change never goes into another drive or follows a link out of its folder.
- No file ever gains setuid in a change that goes inside folders.
- A folder whose new mode would take away your own right to enter it is changed after its contents, so the walk is never cut off.
- Each step checks that the entry is as the plan saw it. One that changed since is skipped and listed with the reason "it changed after the plan", never overwritten.

### Questions, and refusals

A question starts on No. These are the ones permissions ask:

- "Change permissions of 12 items? No is the default. y or n", the summary.
- "You would lose read access to 3 items. Continue? No is the default. y or n".
- "Let everyone change report.pdf? No is the default. y or n", when others would gain write.
- "Let report.sh run as its owner (setuid)? No is the default. y or n".

A refusal says the fact, the cause and what to do:

- "Could not change permissions of a.txt: only its owner (ada) can. Ask the owner, or start diro as administrator."
- "Could not change permissions of a.txt: the disk is read only."
- "Could not change permissions of a.txt: this disk does not keep them."
- "Permissions did not change: the disk ignored the request." diroxide reads the entry back after a change, and says this when a drive pretends to accept a change.

The result is "Changed permissions of 12 items." and Ctrl+Z says "Undid permission change of 12 items." The end-of-job line has a rough wording for now; [known limits](known-limits.md) says what it is.

## Times

There is no Change times command in alpha.2. The engine can set the modified, created and accessed times, but the date entry dialog comes with alpha.7, along with changing the owner.

### Copying keeps dates

Copies, moves and unpacks keep the modified time of files and folders. A folder's time is set after its contents, so the copy itself never replaces it. The created time is kept on Windows and macOS, where the system can set one. Linux cannot set a created time. The accessed time is never kept. A change time is set by the system alone and cannot be copied. There is no setting to turn this off in this alpha.

## Details: the information list

Alt+Enter lists the facts about the focused entry, one fact on each row, with its name first. Windows Terminal keeps Alt+Enter; the File menu has Details as the sure way ([terminals](terminals.md)). The list is titled "Details of course.zip", and alpha.2 puts the rows the viewer and the archives need at its start:

- "Type: zip archive" or "Type: PDF document". The type comes first.
- "Size: 48 KB (49,152 bytes)". The size in words, then the exact bytes.
- "Entries: 10 items", for an archive, after the size.
- "Link to: ../data/x.txt", for a link: the target, with any hidden character in it spelled out.

The other rows are the ones [the panels guide](panels.md) lists: the times, attributes, owner, group and the full path.

## See also

- [Operations and undo](operations.md)
- [The panels](panels.md)
- [Archives](archives.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Terminals](terminals.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
