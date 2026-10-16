# Known limits

This page says what diroxide alpha.1 does not do yet, where it is limited on purpose or by the system, and what has not been tested. It is kept current with each alpha. Where a limit has a workaround, the workaround comes first.

## What is not in alpha.1

Each item says which alpha brings it.

- The window. alpha.1 runs in the terminal only. `diro --gui` is accepted and says in words that the window is not built yet. The window comes with alpha.6.
- Archives. Zip, 7z, tar and the other formats opened as folders, packing a selection, and the passwords they need: alpha.2. Until then an archive is a file, and Enter opens it with your system's program.
- The file viewer and the document quick view (the preview of a document in the other panel): alpha.2. Splitting and joining big files: alpha.2.
- Creating symbolic links and changing permissions. The panels show permissions in words from alpha.1, but you cannot change them until alpha.2. Changing the owner and the file times come with alpha.7.
- Search by name and content, the full filter language, saved searches, labels and bookmarks, and the Keys list inside the program: alpha.3. You can already change keys by editing `keymap.toml`.
- Plugins, script commands and the downloader for optional components: alpha.4.
- SSH and other remote places, and the trash browser: alpha.5.
- Tools such as ImageMagick and FFmpeg, previews of images and documents, conversions, and the git commands: alpha.7.
- Tabs, the folder tree, collections, comparing and synchronizing folders, duplicates, batch rename and phones over MTP: alpha.8.
- Download files for each platform: alpha.6 for Windows x86_64 and Linux x86_64, the other platforms with alpha.9. `cargo install diro`: beta.1.
- Nothing in alpha.1 asks for a password yet. The masked password prompt is built, and the features that need it arrive in later alphas.

## The trash

The trash is the default way to delete, and the places where it cannot be trusted are handled by asking you instead of guessing.

- Windows: the trash is used only on a fixed local drive, for a path of 259 characters or fewer, and never on a network path. The Windows trash call can remove a file for good and still report success when a drive is removable or a path is 260 characters or longer. diroxide therefore decides before the call. Everywhere else it offers the listed permanent delete, and the question starts on No: "This drive has no trash. Delete 3 items for good? y or n".
- Windows: a folder is refused for the trash when any path inside it is 260 characters or longer.
- Windows: after every trash call diroxide checks that the original is gone and that the trash holds it. If not, it says so loudly: "Deleted but not found in the trash."
- Windows: a very large item may be bigger than the Recycle Bin's size limit. This case has not been tested yet, because testing it could push your oldest recycled items out of the bin.
- macOS: diroxide cannot restore items from the trash, so Undo cannot bring back a trashed item there. Restore it from the Trash in the Finder.
- Linux: the trash follows the freedesktop standard. A drive with no trash of its own may mean copying the item into your home trash first.
- Permanent delete cannot be undone, and it cannot erase data for good: "Deleted data may stay on the disk. diroxide cannot erase it for good."

## Long paths

- Listing, copying, renaming and removing work at any depth on every system. The extended-length prefix (`\\?\`) is never shown or spoken.
- Windows cannot start a program in a folder whose path is 259 characters or longer. diroxide says so before it tries: "Cannot start a program in a folder this deep". The same holds for the run-command prompt, shell-here, and a program stored at that depth.
- The system opener refuses a file at that depth: "Cannot open a file this deep".
- The shell hand-back (`--cwd-file`) writes the plain path. In testing, cmd and Windows PowerShell 5.1 both changed to a folder of 1,287 characters. PowerShell 7 was not installed on the test machine, so it has not been tried.

## Watching folders

- Ctrl+R lists a folder again whenever you want. On a fixed local drive diroxide also watches the folders on screen, so an outside change shows up, with your cursor and marks kept on the same names. In screen-reader mode the other panel is not redrawn behind your back.
- Removable drives are not watched. diroxide looks again when you return to the panel and every two seconds. Ejecting a USB drive while a panel shows it has not been tested yet; leave the folder first.
- Network drives are never watched. The panel says: "Folder not watched. Reload with Ctrl+R." A folder that cannot be watched for any other reason says the same.
- A burst of thousands of changes is shown as one refresh instead of one line each.

## Dates and times

- A copy keeps the modified time of files and folders, and sets a folder's time after its contents so the copy itself never replaces it.
- The created time is kept on Windows and macOS. Linux cannot set a created time, so it is not kept there. diroxide says so only when you asked for created times to be kept.
- The accessed time is never kept.

## Terminals and keys

The table of keys each terminal can send waits for a key probe run by hand on each terminal, so some keys may not arrive in some terminals.

- The keys that most often fail are the Norton sort keys Ctrl+F3 to Ctrl+F7, Alt+Home, Alt+Left, Alt+Right, Alt+Up, Shift+F5, Alt+Enter, Ctrl+V and Ctrl+Tab. The menus, the palette and the Sort list reach every command whatever the terminal keeps.
- `diro keys test` says in words what your terminal sent and what diroxide would run.
- Windows Terminal keeps Ctrl+V for text paste. Use Paste in the Edit menu (Alt+E, then P), or unbind the key as [Terminals](terminals.md) shows.
- On macOS the system owns Ctrl+F3 to Ctrl+F8 and the function keys need Fn. Use the Sort list, the menu or the palette.
- Terminals not yet tried: the classic console, Konsole, Alacritty, kitty, WezTerm, Ghostty, iTerm2 and macOS Terminal with VoiceOver.

## Screen readers and Braille

- NVDA in Windows Terminal is the main target. Nothing in alpha.1 had been heard on a screen reader when this page was written; the first listening check comes after the build is complete. [The accessibility statement](accessibility.md) records what has been checked.
- JAWS in Windows Terminal is untested.
- Orca and VoiceOver are best effort. Terminals that draw with the graphics card, such as Alacritty and kitty, give Orca nothing to read.
- The side by side layout puts the cursor near column 41 when the right panel is active, so a screen reader may read the left panel's row first. The stacked layout avoids this. Which layout suits you best is still being decided by listening; see [Using diroxide with a screen reader](screen-readers.md).
- The 40-cell lines were written for computer braille, one cell per character, on a HumanWare Mantis Q40. Contracted braille and other displays are untested.

## Names and wide characters

- A file name that is not valid Unicode is flagged and refused. diroxide does not show it with changed characters.
- Windows would change some names: a trailing space or dot, a device name such as `NUL`, a reserved character. diroxide refuses to create them.
- Hidden, control and invisible characters in a name are shown as a question mark. The real name is kept for operations, and a listing before a delete spells them out as `U+XXXX`.
- A known drawing bug in the library that paints the terminal can shift the rest of a row after an emoji or a Chinese, Japanese or Korean name in some terminals. diroxide works around it where it can. If a row looks shifted, note the terminal's name and version when you report it.

## Other limits

- Cut, copy and paste of files (Ctrl+X, Ctrl+C, Ctrl+V) work between diroxide's own panels. They do not use the system clipboard, so Explorer and the Finder cannot paste what diroxide copied.
- Cloud files that are only online are named "online only". diroxide asks before it reads their contents.
- The terminal frontend has no mouse.

## See also

- [The accessibility statement](accessibility.md)
- [Terminals](terminals.md)
- [Operations and undo](operations.md)
- [Troubleshooting](troubleshooting.md)
- [The documentation index](README.md)
