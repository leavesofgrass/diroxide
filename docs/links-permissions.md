# Links, permissions and times

This guide covers making links and changing where a link points, changing permissions and Windows attributes, changing times, and reading the details of an entry. These commands come with alpha.2. Changing the owner comes with alpha.7.

What this guide says about the dialogs and the spoken lines comes from their design and from the tests that check each sentence. Nothing in alpha.2 had been heard on a screen reader when this guide was written.

<!-- to confirm with the wiring (W2f): the names of the commands in the palette and the menus, their keys, the order of the rows in each dialog, how a time and an octal mode are typed, and whether Make runnable and the permission presets are in the dialog. The engine builds none of the last two. -->

## Links

A link is a second name that points at a file or a folder. Make symbolic link, from the palette, makes a link to the focused entry, and the dialog says where the link goes. There are three kinds:

- Symbolic: a name that holds the path of its target. The target can be a file or a folder, on this drive or another.
- Junction: a folder link on Windows that needs no special right. It holds a folder only, and always points by the full path.
- Hard: a second name for the same file, on the same drive.

### Relative or absolute

A symbolic link writes its target in one of two ways, and the plan says which:

- Absolute, the default: the full path. The link keeps working wherever you move the link, as long as the target stays put.
- Relative: the way from the link's own folder, such as `../data/x.txt`, or `..\data\x.txt` on Windows. The link keeps working when you move a folder that holds both the link and its target. A relative link to another drive cannot be written, so the plan makes it absolute and says so: "Absolute link: the target is on another drive."

A junction and a hard link point by the full path, and the plan's note says so.

### Changing where a link points

Change link target, from the palette, asks for a new target and changes the link in one step. diroxide makes the new link beside the old one under a temporary name, then swaps it into place, so there is no moment when the name points nowhere, where the system allows a swap. Otherwise it sends the old link to the trash first, and Ctrl+Z brings the old link back from the trash. On a drive without a trash, that fallback refuses and says so. On Windows the system will not swap a folder link in one step, so changing a folder link there always uses the trash.

### Where links cannot be made

diroxide checks before it plans. On a FAT or exFAT drive on Windows, inside an archive, or in a read-only place, it says so: "Cannot link here: this place holds no links".

### Windows: Developer Mode, and the junction

On Windows a symbolic link needs Developer Mode turned on in the system settings, or an administrator's rights. Without them diroxide says, in words and not as a system code:

"Cannot link: needs developer mode."

For a folder, diroxide then offers the way around: "Make a junction instead? y or n". It starts on No. A junction needs no right at all, holds a folder only, and points by the full path.

<!-- to confirm with the wiring: that the junction question is asked in the dialog right after the refusal, and how Developer Mode is described. -->

## Permissions

On Linux and macOS, Change permissions changes who can read, write and run a file, or list, change and enter a folder. On Windows the same command changes the attributes. Both are a job: a plan, a question, a result, and Undo.

### Said in words first

The plan says the result in words, with the octal number after it:

"Owner can read and write, group can read, others can read (644)"

The octal number is never the first thing said. For several entries the plan groups them by the change: "12 files: group gains read", "3 folders: group gains list and enter". The words for a change are short: "group gains write", "others lose list and enter", "everyone gains run", "turns on run as owner (setuid)".

A folder reads in its own verbs: list, change and enter instead of read, write and run.

On Windows the rows are the attributes, each turned on or off: "Turns on read-only", "Turns off hidden". The attributes are read-only, hidden, system and archive.

### The dialog

The dialog is a list of rows, each "label: value" with its meaning first, such as "Group can write: no". Space cycles a row through unchanged, yes and no, and a row says both its value and its state: "Group can write: yes, changed". A mixed selection reads "Group can write: mixed, unchanged". There are rows for the octal number ("Octal: 644", typed, and kept in step with the other rows) and for what the change applies to.

<!-- to confirm with the wiring: the rows' exact labels and order, how the octal number is typed, and the title (Permissions for report.pdf, Permissions for 3 items). Make runnable, the permission presets and a typed symbolic mode such as u=rw,g=r are not built in alpha.2. -->

### Apply to: the recursive option

The first row of the plan is "Apply to:", and it has four choices:

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

- "Change permissions of 12 items? y or n", the summary.
- "You would lose read access to 3 items. Continue? y or n".
- "Let everyone change report.pdf? y or n", when others would gain write.
- "Let report.sh run as its owner (setuid)? y or n".

A refusal says the fact, the cause and what to do:

- "Could not change permissions of a.txt: only its owner (ada) can. Ask the owner, or start diro as administrator."
- "Could not change permissions of a.txt: the disk is read only."
- "Could not change permissions of a.txt: this disk does not keep them."
- "Permissions did not change: the disk ignored the request." diroxide reads the entry back after a change, and says this when a drive pretends to accept a change.

The result is "Changed permissions of 12 items." and, when nothing needed changing, "Nothing to change: permissions already match." Ctrl+Z says "Undid permission change of 12 items."

## Times

<!-- to confirm with the wiring: whether alpha.2 has a Change times command. The research report places the engine in alpha.2 and the date-entry dialog in alpha.7. If there is no dialog, this section keeps its plan lines and the first sentence says the dialog comes with alpha.7. -->

Change times sets the modified time and, where the system allows, the created and accessed times, as a job with a plan and Undo. The plan says each one in words: "Modified time to 2026-10-10 22:09". Times are typed and said in your local time. Done recursively, a folder's time is set after the times of what is inside it, so the change does not move it again.

- Linux cannot set a created time. The plan says so: "Created time not set: this drive cannot keep one."
- A change time (ctime) is set by the system alone, and cannot be set: "Could not set the change time: the system sets it. Set the modified time instead."

### Copying keeps dates

Copy, move and unpack keep the modified time of files and folders. This is the job engine's Keep dates behavior, and it is on. A folder's time is set after its contents, so the copy itself never replaces it. The created time is kept on Windows and macOS, where the system can set one. The accessed time is never kept. Keep dates is not a setting in alpha.2; it is on for every copy.

## Details: the information list

Alt+Enter lists the facts about the focused entry, one fact on each row, with its name first. Windows Terminal keeps Alt+Enter; the File menu has Details as the sure way ([terminals](terminals.md)). In alpha.2 the list gains the rows that the viewer and the archives need:

- "Type: zip archive" or "Type: PDF document". The type comes first.
- "Size: 48 KB (49,152 bytes)". The size in words, then the exact bytes.
- "Link to: ../data/x.txt", for a link: the target, with any hidden character in it spelled out.
- "Entries: 10 items", for an archive.
- For permissions and times, the same words as the plan: "Owner can read and write, group can read, others can read (644)".

Ctrl+D says the same facts in one line, as it does for a panel. [The panels guide](panels.md) describes the rest of the list.

<!-- to confirm with the wiring: the rows an archive and a link add, their order, and that Entries is a row of the information list. -->

## See also

- [Operations and undo](operations.md)
- [The panels](panels.md)
- [Archives](archives.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Terminals](terminals.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
