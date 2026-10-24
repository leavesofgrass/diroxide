# Volumes and the trash

This guide covers two lists and the free space diroxide tells you about. The Volumes list shows every drive, mount, connected storage and device, each with its state first, and lets you open, mount, unmount, eject or hide one. The trash browser shows what is in the trash and lets you restore it, delete it for good, or empty the trash. The last section covers free space: in the plan before a copy, and in Say Status.

What this guide quotes is what the program says. Each sentence is checked in tests against the 40 cells of a Braille line, and each is written so its meaning comes first. None of it has been heard yet on NVDA, JAWS or a Braille display. The two lists have no terminal tests yet: their tests run the program's logic, not the drawn screen. Unmount, Eject and Mount have been tried against stand-ins for the system's tools and never against a real drive. If your experience differs, that is a finding.

[Operations and undo](operations.md) says how F8 sends things to the trash. [Optional components](components.md) says how to get the Volume actions plugin. [Known limits](known-limits.md) lists what is not built or not tested.

## The Volumes list

Open it from the Go menu: Alt+G, then V. It is in the command palette (Ctrl+P) as Volumes. It has no key of its own. To give it one, add a line for `go.volumes` to `keymap.toml`; [customizing keys](customizing-keys.md) says how.

You hear "Reading the volumes", and then the list opens. Its title gives the count first:

```text
Volumes, 4 volumes
```

With nothing to list, it says "No volumes to list".

### What a row says

Every row puts its state first, then the name, the kind, and the free space or the size:

```text
Mounted: Backup, removable, 412 GB free
Not mounted: Backup, removable, 931 GB
Connected: build, sftp, 1.2 TB free
```

- Mounted is a drive or a mount you can open now.
- Not mounted is a device the system knows about that has no mount. These rows come from the Volume actions plugin, so they appear only on Linux and macOS, and only when that plugin is turned on.
- Connected is a storage that a plugin reaches, such as a server you opened with the SFTP places plugin.
- The kind is one word: "disk", "removable", "network" or "drive" when the kind is not known. A plugin's storage says its scheme, such as "sftp".

A drive on Windows is named by its label and its letter, "Backup E:", or "Drive E:" when it has no label. A mount on Linux and macOS is named by its label, or by its folder when it has none.

After the row come its details: how much is free of the whole, and where it is. For a device that is not mounted they name the device.

```text
412 GB free of 931 GB, /media/ada/Backup
Device /dev/sdb1
```

### Free space fills in

The list opens at once. Each mounted row first says "checking free space", and the answer replaces it as it arrives. When the answer for the row under your cursor arrives, that row is said again once, quietly.

A volume that has not answered in 2 seconds says "free space not known". That is never the same as zero, and the list does not wait for it, so a sleeping network share cannot hold the list.

diroxide asks each volume for its label, its size and its free space, and nothing else. It does not list a drive or read a file on it. A device that is not mounted is not asked at all.

### The keys

- Up and Down move between rows.
- Enter offers the row's actions, below.
- Ctrl+R reads the volumes again. You hear "Reading the volumes again". Use it after you plug in a stick.
- Escape closes the list.

The list also reads the volumes again by itself when an Unmount, Eject or Mount you started ends.

### The actions

Enter on a row opens a list named for that volume, with what you can do. It opens on the first action: "Backup, Open, 1 of 4" for a removable volume on Linux or macOS, and "1 of 3" on Windows.

- Open goes to the volume in the active panel. A device that is not mounted has no Open.
- Mount mounts a device that is not mounted.
- Unmount unmounts a mounted volume.
- Eject unmounts a removable drive and powers it off.
- Hide leaves the row out of the list.

Which of these a row has depends on the row:

- A removable volume that is mounted: Open, Unmount, Eject, Hide.
- A device that is not mounted: Mount, Eject, Hide. Eject appears only if it is removable.
- A fixed disk, a network drive, the root folder, and a plugin's storage: Open and Hide. A fixed disk also gets Unmount when the Volume actions plugin has reported its device. The root folder and network drives never get Unmount or Eject.
- On Windows there is no Mount or Unmount, because Windows mounts and unmounts removable drives itself. A removable drive has Open, Eject and Hide.

To leave a storage a plugin gave you, such as an SFTP site, use that plugin's own Disconnect. [Remote places over SFTP](remote-places.md) has it.

### When the Volume actions plugin is not here

Unmount, Eject and Mount are done by the Volume actions plugin, one of the example plugins. Without it, the actions are still listed, but marked unavailable, with the reason. The word "unavailable" comes last:

```text
Eject, needs the Volume actions plugin, unavailable
```

Enter on one says why, and then offers the example plugins:

```text
Eject needs the Volume actions plugin.
```

If the example plugins pack is not on this computer, the question that follows starts on No: "Getting example plugins needs Example plugins 0.1.0. Download it now? No is the default. y or n". If the pack is already here, you hear "Already installed: Example plugins. Manage plugins lists them." Either way, the plugin still has to be turned on. To do that:

1. Get the pack, if you have not: Alt+P, then G.
2. Open Manage plugins: Alt+P, then M. Choose Volume actions, press Enter, and choose Enable. The question starts on No. [Plugins](plugins.md) shows what it asks.
3. Open the Volumes list again.

The actions stay unavailable while the plugin is there but not turned on, and again if you turn it off. A plugin whose files changed since you turned it on asks again before it runs.

### What the plugin needs

The plugin is a small Python program. It uses the system's own tool and nothing else, by an argument list and never through a shell. It never uses a force option. It makes no network call.

- Linux: Python 3, `udisksctl` and `lsblk`. It tells `udisksctl` not to ask questions, so a volume that needs a password says so instead of opening a password dialog.
- macOS: Python 3 and `diskutil`.
- Windows: Python 3 through the `py` launcher, and PowerShell. Windows has Eject only. The plugin asks the shell to eject the drive with no console window, and waits up to 15 seconds for the drive to go.

### What you hear

Unmount, Eject and Mount do not ask a question of their own. You answered the plugin's question when you turned it on, and a plugin that changed asks again. Each says its result in one sentence with the volume's name first, and the plugin's name last:

```text
Backup is safe to remove. By Volume actions.
Backup is unmounted. By Volume actions.
Backup is mounted at /media/ada/Backup. By Volume actions.
```

A volume that is in use is never forced. It stays where it is and you hear why, then what was not done, then where the plugin's log is:

```text
Could not eject Backup: files are open. By Volume actions.
Nothing was forced: close the files, then try again. By Volume actions.
Next: diro plugin log volume-actions
```

Close what is using the volume, and eject again.

The other reasons are in the same shape: "the system asked for a password", "it did not answer in time", "the system refused", and "udisksctl was not found" (or `diskutil`, or PowerShell). A busy unmount on macOS can take most of a minute before it gives up.

The plugin's commands also declare a preview that says what would run and changes nothing. The program offers no way to ask for that preview yet.

### Hide, and Hidden volumes

Hide leaves a volume out of the Volumes list. You hear which one, and where to undo it:

```text
Hidden: Backup. Settings, Hidden volumes has it.
```

Nothing happens to the drive. Hide keeps the volume's device when the row has one, and its place when it does not, so a stick you hid stays hidden wherever it mounts. Hide changes only the Volumes list. Go to a place still lists the drive.

The setting is Hidden volumes (`hidden_volumes`, in the Panels group, empty by default). The Settings list (Alt+T, then S) shows what is hidden. To list a volume again, take its entry out of `hidden_volumes` in `config.toml`, leave the others as they are, and start diroxide again.

```toml
hidden_volumes = ["/dev/sdb1"]
```

[Settings and where the files live](settings.md) says where the file is.

### Volumes in Go to a place

Go to a place (Ctrl+G) lists the volumes too, each row with its kind first, then the name and the state:

```text
Volume, Backup, mounted, removable
Volume, build, connected, sftp, 1.2 TB free
Trash, what is in the trash
```

A local volume says its free space there only after the Volumes list has asked for it in this session. The place list reads the volumes once a session, quietly, without asking any drive for its space. A stick plugged in later shows in the list in its older words, such as "Drive E:, removable drive", until you open the Volumes list, which reads them again. Enter on a volume goes to it. Enter on the Trash row opens the trash browser. [The panels](panels.md) describes the rest of the list.

## The trash browser

Open it from the Go menu: Alt+G, then C. It is in the command palette as Trash, and it is the Trash row of Go to a place (Ctrl+G, type "trash", Enter). It has no key of its own; the command is `go.trash`.

You hear "Reading the trash", and then the list opens, newest first. Its title gives the count:

```text
Trash, 2 items
```

An empty trash does not open a list. You hear "The trash is empty."

### What a row says

Each row puts the name first, then when the item was deleted, then the folder it came from:

```text
report.pdf, deleted Tuesday, D:\work
notes.txt, deleted yesterday, 9:30 AM, D:\work
```

The time reads the way the panels read a modified time: "today, 2:05 PM", "yesterday, 9:30 AM", a weekday within the last week, then "October 3", then "October 3, 2025". A trash that does not say when says "deleted at a time not known". A name with hidden characters shows a question mark where they are.

### The keys

- Up and Down move between rows.
- Space marks a row and moves down. You hear "Marked" or "Unmarked".
- Enter offers the actions. They apply to the marked rows, or to the row you are on when none is marked.
- Escape closes the list: "Trash closed."

The list follows each restore and delete, and keeps your place and your marks.

### The actions

Enter opens a list named for the item, or "Trash, 3 items marked" when several are marked. It has three rows, and the count is in the words:

```text
Restore 3 items
Delete 3 items for good
Empty the trash
```

It opens on the first: "notes.txt, Restore 1 item, 1 of 3". Every question that follows starts on No.

### Restore

Restore puts each item back where it was. The question puts its point first:

```text
Restore 3 items? No is the default. y or n
```

Say n or press Escape, and you hear "Nothing changed." Say y, and when it ends: "Restored 3 items."

Restore never writes over a name that is taken. The plan leaves that item out and says so in the list above the question:

```text
Left out: notes.txt, its name is taken
```

and the result counts it:

```text
Restored 2 items. 1 name was taken.
```

If every item you chose has its name taken, nothing is asked: "Nothing to restore: notes.txt, its name is taken." If the trash holds two items from one place, only the newest comes back, and the older one is left out with the reason "a newer item from the trash comes back to this name". diroxide checks the name again just before each item comes back. Ctrl+Z after a restore sends the restored items back to the trash.

### Delete for good

Delete for good deletes the items from the trash. Nothing goes before it is listed: the summary, then one line for each item, then the honest sentence about deleting. The question starts on No:

```text
Delete 2 items for good? No is the default. y or n
```

```text
Deleted data may stay on the disk. diroxide cannot erase it for good.
```

The result is "Deleted 2 items from the trash for good." It cannot be undone.

### Empty the trash

Empty the trash deletes everything in it, not only the marked rows. It is the third row of the actions, and it is in the command palette as Empty the trash, so you can run it without opening the browser. It lists every item first, like Delete for good, and the question starts on No:

```text
Empty the trash: delete 12 items for good? No is the default. y or n
```

An empty trash says "The trash is empty."

### More than 1,000 items

A delete for good, or an empty, of more than 1,000 items does not ask y or n. It asks you to type the count, after the listing:

```text
Type 1001 to go on, or Escape
```

Typing anything else says "That is not 1001. Nothing was done; type it, or Cancel." Escape stops, and nothing is deleted. The count has no comma in it.

### On macOS

diroxide cannot list the macOS trash, because the Finder keeps its own records of it. The browser opens nothing and says:

```text
The trash cannot be listed on this system.
```

F8 still moves things to the trash on macOS. Restore them with the Finder's Put Back, as [operations and undo](operations.md) says.

## Free space

### In the plan, before a copy or a move

A copy or a move checks the room before it asks. When the destination has too little, the question says so and starts on No, and the list under it names the place and the sizes:

```text
Not enough room on build:
Needs 4.0 GB, 1.2 GB free.
Go on anyway? y or n
```

A place on a server is named by its host. A folder on this computer is named by its drive.

A server's free space comes from the server, and a slow server must never stall the plan. diroxide waits 2 seconds. A server that cannot say, or does not answer in time, is not treated as full and not treated as empty. You hear this before the question:

```text
Free space on build is not known.
```

The copy then goes on if you say y, and the server may still refuse it at the end: "The server's disk or quota is full."

### In Say Status

Ctrl+D ends its first sentence with the free space of the place you are in, once the place has answered:

```text
Documents, 40 items, no selection, on report.pdf, 3 of 40, 412 GB free of 931 GB
```

A place that cannot say, or is too slow, says "free space not known". A place that is still being asked says nothing yet, and Say Status never waits for it.

## See also

- [Operations and undo](operations.md)
- [The panels](panels.md)
- [Optional components](components.md)
- [Plugins](plugins.md)
- [Remote places over SFTP](remote-places.md)
- [The transfer queue](transfer-queue.md)
- [Settings and where the files live](settings.md)
- [Customizing keys](customizing-keys.md)
- [Every key](keyboard.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
