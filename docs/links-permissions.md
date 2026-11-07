# Links, permissions and times

This guide covers making links and changing where a link points, changing permissions and Windows attributes, reading who can do what with a file, setting times by hand, changing the owner, what happens to times when you copy, and reading the details of an entry.

What this guide says about the dialogs and the spoken lines comes from their design and from the tests that check each sentence. Nothing here had been heard on a screen reader when it was written, because the listening tests come after the build.

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

Typing an octal number or a symbolic mode, a one-key Make runnable and a list of named presets are not built yet.

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

## Who may do what

Alt+Enter on a file or folder also says who can do what with it. These rows are read for the focused entry only, a moment after the list opens, with the file's own facts. They join the Flags group of [the details view](details.md), and a few, the owner on Windows and a Linux security label, join the group of what the system keeps. The panel's own Owner, Group and attributes rows give way to them where they say the same thing in full.

The first row says what you can do. The rows after it say who else can, in the words of the system you are on.

### Linux, macOS and other Unix systems

- "You can: read and write". This is what this account may do with the file, as the system answers now. A folder says it in its own verbs, "You can: list, enter and change contents", and with no access at all the row says "You have no access".
- "Owner can read and write (ada)", "Group can read (staff)" and "Others can read". The owner and the group have their names in parentheses. A class with nothing says "Others have no access".
- "Special: runs as its owner" when a special mark is set. On a folder it says "Special: new items get its group, only owners delete inside".
- "Mode: 644, rw-r--r--", the number and the letters, after the words.

On Linux a file may also have an access list: extra permissions for named users and groups.

- "Access list: 2 extra entries", then a row for each named user or group: "User ada can read and write" and "Group dev can read". Where the account has a full name, the row says it, "Ada Example (ada) can read and write", and Copy details leaves that row out as private.
- "Mask limits them to read", when the list's mask takes some of their rights away.
- "Mode's group bits cap named users". With an access list, the group digit of the mode is the mask. The Group row then shows the owning group's own rights, and this note says why the number may look smaller. Changing the group's permissions changes the mask, which limits every named user and group too.
- A folder may have a default access list, for what is made in it: "Default access list: 1 extra entry", with rows that start "Default:".

On macOS the list is not read in this alpha: "Access list: not read on this system". When a list cannot be read, "Access list: could not be read".

Where the system keeps file flags, rows say them: "Locked: no" or "Locked: yes, nobody can change it", "Append only: yes, it can only grow", and "Flags: no backup" for the rest (no backup, compressed, encrypted, no copy on write). A Linux file with an SELinux label says "Label: user_home_t (SELinux)", in the group of what the system keeps, and any other security attribute is a row of its own, such as "security.ima: 34 bytes".

### Windows

- "You can: read and write", then "Owner: BUILTIN\Administrators". The owner is in the group of what the system keeps.
- A row for each entry of the access list, in the order Windows checks them: explicit denials, explicit grants, inherited denials, inherited grants. "Users can read and run (inherited)", "Administrators have full control" and "Denied: Guests cannot write (inherited)".
- The rights are said as Explorer's boxes: read, write, run and delete. A set that is none of those lists its rights one by one: "Users have special permissions: read data, write attributes".
- A folder's entry says where it applies when that is not the usual: "Users can read (subfolders and files only)" and "Users can read (this folder only)". An inherited entry says where from: "Users can read and run (inherited from D:\data)". Windows often keeps one grant as two entries on a folder, and the two read as one.
- "Access list: none, everyone has full control", "Access list: empty, nobody has access", "Access list: you cannot read it" and "Access list: could not be read".
- "Access list order: not the usual one", when the entries are in an order Windows would not write, and "Inherits from its folder: no", when inheritance is turned off.
- The attributes row stays: read-only, hidden, system and archive.

### Other places

- An SFTP site and an archive give the numbers their listing holds, with no "You can" row, because diroxide cannot ask the server or the archive what you may do: "Owner can read and write (1000)" and "Group has no access (1000)".
- A place a plugin serves says "This place has no permissions".
- A link says "Permissions: a link has none of its own".
- When the facts cannot be read, the row says why: "Permissions: not read, permission denied".

### "You can" is advice

The row is not a promise. Another program that is using the file, or a disk that turns read only, can still stop a change. F1 on the list says so, in the list's help: "You can: what this account may do, as the system answers now. It is advice: another program using the file, or a disk turning read only, can still stop a change." With an access list, the help adds what the mask does.

macOS access lists and SELinux with the label switched on were not tried, and nothing here has been heard on a screen reader.

## Times

Change time sets the modified, accessed or created time of the marked files, or of the focused one. It is in the File menu and the palette. It has no key of its own; you can give it one in `keymap.toml` ([Customizing keys](customizing-keys.md)). The menu shows it only where the place keeps permissions or attributes, and where a place keeps no times, the command says "Could not change the times of main.rs: this place does not keep them."

### Change time

The command opens a list titled "Change time", in the shape of the permissions dialog. The first row says what will happen and plans it on Enter: "Change the times of main.rs", or "Change the times of 3 items". The rows after it:

- "Change: modified only". Enter steps through "modified and accessed" and "accessed only", and where the place can set a created time, "all three" and "created only". Windows and macOS can set a created time. Linux cannot.
- "Set to: now". Enter steps to the other panel's focused file, "Set to: the times of notes.txt", which copies that file's times, and back to now. With no file there it says "No file in the other panel to copy times from".
- "Type a date and time". Enter opens a field, "Set to, as 2026-10-10 22:09:", with a time in it to edit. Type the year, month and day with dashes, then the hour and minute with a colon: `2026-10-10 22:09`. Seconds are optional, a T may stand in for the space, and a date alone means its midnight. The time is your local time. "now" means now. Anything else says "Not a date: type it as 2026-10-10 22:09", and the field stays so you can fix it. After a good date the row says it with its weekday, computed from the date: "Set to: Saturday, 2026-10-10 22:09".
- The times the entry has now, each with its weekday: "Modified: Saturday, 2026-10-10 22:09", then "Accessed:" and "Created:". Where a place cannot set one it says "Created: not settable on this system", and a time it does not know says "not known".
- On Linux and macOS, "Changed: set by the system". The change time is when the entry's details last changed, and the system sets it, never a program. Enter on the row says "Could not set the change time: the system sets it. Set the modified time instead."
- "Apply to: this item only", when a folder is chosen. Enter steps through the same four choices as in permissions.

Enter on the first row plans the change, and the question starts on No: "Change the times of 1 item? No is the default. y or n". The plan lists each change: "Modified time to Saturday, 2026-10-10 22:09". If the chosen source has no time for the chosen kind, it says "Nothing to change: no time to set".

It is a job, with a result and Undo. When it ends, diroxide reads the times back and says when the disk kept something else: "Modified time rounded by the disk", or "Modified time not kept by the disk". A FAT disk keeps the modified time to two seconds and the accessed time to the day. Ctrl+Z puts the old times back: "Undid time change: 1 item is back".

### Copying keeps dates

Copies, moves and unpacks keep the modified time of files and folders. A folder's time is set after its contents, so the copy itself never replaces it. The created time is kept on Windows and macOS, where the system can set one. Linux cannot set a created time. The accessed time is never kept. A change time is set by the system alone and cannot be copied. There is no setting to turn this off.

## Owner

Change owner changes who owns a file or a folder, and which group it belongs to. It is in the File menu and the palette. It has no key of its own. The menu shows it where the place keeps permissions or attributes, and a place that keeps no owners says "Change owner: this place keeps no owners". Change owner never starts as an administrator and never asks for an administrator's password. Where the system wants more rights than you have, it says so and stops.

### On Linux and macOS

The command opens a list titled "Change owner", in the shape of the permissions dialog. The first row plans the change on Enter: "Change owner of main.rs". The rows after it:

- "Now: owner ada, group staff". What the entry has today.
- "Owner: ada, as now". Enter opens a field, "New owner of main.rs:", with the user names this computer knows, which Tab completes. A number works too. The row then says the new owner: "Owner: root".
- "Group: staff, as now". Enter opens "New group of main.rs:" the same way. The row then says whether the group is one of yours, "Group: staff (one of your groups)" or "Group: wheel (not one of yours)".
- "Apply to: this item only", when a folder is chosen, with the same four choices as in permissions.

Leave a field as it is and that part does not change. With nothing chosen, "Nothing to change: choose a new owner or group."

The plan's question starts on No: "Change owner of 3 items to root? No is the default. y or n". A group alone reads "Change group of 1 item to staff?", and both together "Change owner of 3 items to root, group to wheel?". It is a job, and Ctrl+Z puts the owners back.

What it refuses, each in words with what to do:

- Giving a file to another owner takes an administrator. Anyone else hears "Could not change the owner of notes.txt: only an administrator can. Start diro as administrator, then try again."
- A group change takes a group you are in: "Could not change the group of a.txt: you are not in group staff. Choose one of your groups."
- A name this computer does not know: "Could not change the owner of a.txt: there is no user named bob here. Type a user name or a number."
- A link: "Could not change the owner of latest: links are changed through their targets."
- The system's folders, and everything in them, are never changed: "Could not change the owner of /usr/bin: system folders are protected. Use the system's own tools." They are `/bin`, `/boot`, `/dev`, `/etc`, `/lib`, `/lib32`, `/lib64`, `/proc`, `/root`, `/run`, `/sbin`, `/sys`, `/usr` and `/var`, and on macOS `/Applications`, `/System`, `/Library` and `/private`. The folders `/home` and `/Users` are protected as themselves, but what is inside them is not. The top of a drive is protected too.
- Putting an owner back can take an administrator too. When it would, and you are not one, the change ends with "Undo needs an administrator: it puts the owners back."

### On Windows

Windows keeps owners in its own Security dialog, so Change owner hands the focused item over to it and does not change anything itself. The details view says the owner: "Owner: BUILTIN\Administrators". The command says "Opening the Security dialog for notes.txt". In this build nothing can open that dialog yet, so what you hear next is "Could not open the Security dialog: Windows did not open it. In File Explorer, open Properties, then Security." Use File Explorer for now.

## Details: the information list

Alt+Enter lists the facts about the focused entry, one fact on each row, with its name first. Windows Terminal keeps Alt+Enter; the File menu has Details as the sure way ([terminals](terminals.md)). The list is titled "Details of course.zip", and its first rows are:

- "Type: zip archive" or "Type: PDF document". The type comes first.
- "Entries: 8 files, 2 folders", for an archive, before the size.
- "Size: 48 KB (49,152 bytes)". The size in words, then the exact bytes.

The other rows follow in a fixed order, the times, attributes, labels, the path, "Link to: ../data/x.txt" for a link (the target, with any hidden character in it spelled out), who can do what, owner, group and the rest. [Who may do what](#who-may-do-what) lists those rows for each system. [The details view](details.md) has the whole order, and the rows that are read from inside the file.

## See also

- [Operations and undo](operations.md)
- [The panels](panels.md)
- [Archives](archives.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Terminals](terminals.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
