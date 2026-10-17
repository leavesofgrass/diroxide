# Known limits

This page says what diroxide does not do yet as of alpha.2, where it is limited on purpose or by the system, and what has not been tested. It is kept current with each alpha. Where a limit has a workaround, the workaround comes first.

## What is not in alpha.2

Each item says which alpha brings it.

- The window. diroxide runs in the terminal only. `diro --gui` is accepted and says "The window comes with alpha.6; the terminal opens instead." The window comes with alpha.6.
- Changing an archive. alpha.2 reads zip, 7z, tar, tar.gz, tar.xz and xz, and packs a new archive from a selection, but it cannot add to, delete from or rename inside an existing one. Copy, move, rename, delete and make folder inside an archive say "Archives are read only here. Use Pack to make a new one." Editing inside an archive comes with alpha.7.
- More archive formats. bzip2 or Zstandard compression on their own, lzip, RAR, ISO images, and a spanned zip set (`.z01`, `.z02`) are not read. diroxide says which one it met and that it is not supported. They are planned for alpha.7, through the 7z program when it is installed. diroxide does not pack to `.gz`, bzip2, Zstandard or RAR.
- The document quick view reads Markdown, plain text, code, and the formats of carta (Org, reStructuredText, MediaWiki, DokuWiki and Jira). DOCX, PDF, HTML, EPUB, spreadsheets, and images described in words come with alpha.7; until then the quick view says "No preview" and names the kind and size.
- A chooser for the viewer's text encoding. The viewer picks UTF-8, UTF-16 by its byte-order mark, or Windows-1252, and says which. A chooser for other encodings comes with alpha.7.
- Changing the owner and group of a file, and a dialog for typing a file time, come with alpha.7. <!-- to confirm with the wiring: whether alpha.2 has a Change times command (the engine is built). -->
- Full search by name and content, the full filter language, saved searches, labels and bookmarks, and the Keys list inside the program: alpha.3. `diro search ROOT PATTERN` already prints names that match a list of globs. You can already change keys by editing `keymap.toml`.
- Plugins, script commands and the downloader for optional components: alpha.4. The Plugins menu only says "Plugins come with alpha.4."
- SSH and other remote places, and the trash browser: alpha.5.
- Tools such as ImageMagick and FFmpeg, previews of images and documents, conversions, and the git commands: alpha.7.
- Tabs, the folder tree, collections, comparing and synchronizing folders, duplicates, batch rename and phones over MTP: alpha.8.
- Download files for each platform: alpha.6 for Windows x86_64 and Linux x86_64, the other platforms with alpha.9. `cargo install diro`: beta.1.
- Passwords are asked only for archives. The masked password prompt is built, and the other features that need it, such as SSH, arrive in later alphas.

## The trash

The trash is the default way to delete, and the places where it cannot be trusted are handled by asking you instead of guessing.

- Windows: the trash is used only on a fixed local drive, for a path of 259 characters or fewer, and never on a network path. The Windows trash call can remove a file for good and still report success when a drive is removable or a path is 260 characters or longer. diroxide therefore decides before the call. Everywhere else it offers the listed permanent delete, and the question starts on No: "This drive has no trash. Delete 3 items for good? No is the default. y or n".
- Windows: a folder is refused for the trash when any path inside it is 260 characters or longer.
- Windows: after every trash call diroxide checks that the original is gone and that the trash holds it. If not, it says so loudly: "Deleted but not found in the trash."
- Windows: a very large item may be bigger than the Recycle Bin's size limit. This case has not been tested yet, because testing it could push your oldest recycled items out of the bin.
- macOS: diroxide cannot restore items from the trash, so Undo cannot bring back a trashed item there. Restore it from the Trash in the Finder.
- Linux: the trash follows the freedesktop standard. A drive with no trash of its own may mean copying the item into your home trash first.
- Permanent delete cannot be undone, and it cannot erase data for good: "Deleted data may stay on the disk. diroxide cannot erase it for good."

## Long paths

- Listing, copying, renaming and removing work at any depth on every system. The extended-length prefix (`\\?\`) is never shown or spoken.
- Windows cannot start a program in a folder whose path is 259 characters or longer. diroxide says so before it tries: "Cannot start a program in a folder this deep". The same holds for the run-command prompt, shell-here, the editor, and a program stored at that depth.
- The system opener refuses a file at that depth: "Cannot open a file this deep".
- The shell hand-back (`--cwd-file`) writes the plain path. In testing, cmd and Windows PowerShell 5.1 both changed to a folder of 1,287 characters. PowerShell 7 was not installed on the test machine, so it has not been tried.

## Watching folders

- Ctrl+R lists a folder again whenever you want. On a fixed local drive diroxide also watches the folders on screen, so an outside change shows up, with your cursor and marks kept on the same names. In screen-reader mode the other panel is not redrawn behind your back.
- Removable drives are not watched. diroxide looks again when you return to the panel and every two seconds. Ejecting a USB drive while a panel shows it has not been tested yet; leave the folder first.
- Network drives are never watched. The panel says: "Folder not watched. Reload with Ctrl+R." A folder that cannot be watched for any other reason says the same.
- A burst of thousands of changes is shown as one refresh instead of one line each.

## Dates and times

- A copy keeps the modified time of files and folders, and sets a folder's time after its contents so the copy itself never replaces it.
- The created time is kept on Windows and macOS. Linux cannot set a created time, so it is not kept there.
- The accessed time is never kept.
- An unpack keeps each member's modified time. A zip stores a time in the old DOS form, which has no time zone and only two-second steps, and diroxide reads and writes it as UTC. Programs that read it as local time, such as Info-ZIP's `unzip`, show the times of a zip diroxide wrote shifted by your offset from UTC, and a zip written by such a program shows shifted in diroxide. The extended time field that zip also allows is not read or written yet.
- Keep dates is built into the job engine and is on. It is not a setting yet.

## Archives

- Limits: [the archives guide](archives.md) lists them with the questions they ask. A listing shows at most 500,000 entries of one folder and says so. An archive with more than a million members is refused unless you raise `archives.max_entries`.
- The command-line `diro archive` commands use the built-in limits, not the `[archives]` table in your settings. <!-- to confirm with the wiring: that the app maps the table and the CLI still does not. -->
- 7z passwords: diroxide wipes its own copy of a typed password when the job ends. The library that reads and writes 7z keeps a copy in memory in a form diroxide cannot wipe, for as long as it needs it: one read, or the whole time a 7z is being packed. A zip password is only borrowed by its library and does not have this limit. A locked 7z also keeps its password in the list it caches while the archive stays open. <!-- to confirm with the wiring: that leaving a locked archive makes the app forget it. -->
- 7z and tar links: a link inside a 7z is listed without its target, and unpacks as "link not followed". A hard link in a tar is left out with the line "device or special entry", which is the wrong word for it.
- Names in older zips that are not UTF-8 are not converted. A zip written in Shift-JIS, GBK or Big5 on an older system may show wrong characters. Choosing a name encoding for an archive comes with a later alpha.
- When diroxide opens a zip it compares each member's local header with the central list for up to 100,000 members. Past that, it checks each member when you read it.
- The spool limit for archives inside archives, 4 GiB by default, is shared by the whole program rather than counted for each job.
- Pack shows the free space and does not refuse when there is too little.
- An xz holds one file and no file name, so it takes its name from the archive. When you pack a file to an xz with a different name, the pack plan says what it will unpack as: "Unpacks as one: xz keeps no name".
- Debug builds of diroxide are slow on archives with a million entries. A release build opens one in a few seconds. This matters only to people who build it.
- A local file whose name holds an archive marker, such as `a.zip!b.txt`, is listed but has no place, so diroxide cannot act on it. The row is not yet flagged "name needs care".
- What diroxide writes was read back by other programs in the project's Linux container: 7-Zip 25.01, GNU tar 1.35, xz, unzip, gzip, bzip2 and zip. On Windows, bsdtar and unzip read it. Other versions and other archivers have not been tried.

## The viewer and the quick view

- A line longer than 64 KiB is cut in the viewer, and its row ends with "(line cut)".
- A text search inside one line longer than 1 MiB can miss a match that crosses an internal edge. Hex search does not have this limit.
- Wrapped rows are measured with a table of character widths that is approximate. A line of Chinese, Japanese or Korean text, or of emoji, may wrap a little early or late.
- The outline, the bracket jump, skip comment and context need the file read whole, within the highlight caps. Past them they say "Needs a file under 512 KiB".
- Colors cover nine languages: Markdown, JSON, TOML, YAML, shell, Rust, Python, HTML and CSS. YAML is colored but not checked for errors.
- A slim `diro` built without the syntax set says "Highlighting off: not in this build" and keeps the commands that need no grammar.
- The quick view's first draft was written before its own notes were in. <!-- to confirm with W2e's notes: the quick view's gaps, and whether anything is off by default. -->

## Split and join

- A read error partway through a join fails the whole join. Run it again.
- With very small parts, the split progress line can run past 40 characters. The part count stays inside the first 40.
- The checksum file has no SHA-256 line yet. It comes with alpha.7.
- Spanned zip sets (`.z01`) and RAR volumes are not byte splits and are not joined by putting the parts end to end.
- Whether Total Commander and Double Commander find the checksum file diroxide writes, `report.pdf.crc`, from `report.pdf.001` has not been tried. `cat`, `copy /b` and diroxide's own join are tested against each other.
- Parts keep the file's extension in their names, as 7-Zip's do. Total Commander's own parts may drop it. diroxide reads both.

## Links, permissions and times

- Windows needs Developer Mode or an administrator to make a symbolic link, and a FAT or exFAT drive holds no links at all. The words for the missing right are tested on the error itself; the test machine had the right, so the real refusal has not been seen yet.
- A folder link on Windows cannot be changed in one step, so changing its target uses the trash. On a drive with no trash, changing a folder link's target fails in words.
- Not built yet from the permissions plan: typing a symbolic mode such as `u=rw,g=r`, Make runnable, the list of permission presets, a list of protected places that refuse changes in administrator mode, extra checks for private-key names, changing a file with several hard links once, and a "why can't I?" explanation. <!-- to confirm with the wiring: which of these W2f's dialogs add. -->
- Access lists, flags such as immutable and locked, and extended attributes are neither shown nor edited. They come with alpha.7, shown first. Changing the owner comes with alpha.7.
- The rows alpha.2 adds to the details list (type, size with its exact bytes, a link's target) have no dates of their own, because the preview code has no time zone. The times are in the details list as before. <!-- to confirm with the wiring: how the new rows and the alpha.1 details list fit together. -->
- The Unix half of the atomic link swap is covered by the project's Linux container check. It has not been run natively on a Unix machine outside it.

## Terminals and keys

The table of keys each terminal can send waits for a key probe run by hand on each terminal, so some keys may not arrive in some terminals.

- [The keyboard page](keyboard.md) lists the keys a terminal may keep, with the sure way to each command. The ones that most often fail are the Norton sort keys Ctrl+F3 to Ctrl+F7, Alt+Left, Alt+Right, Alt+Up, Shift+F5, Shift+F10, Alt+Enter, Ctrl+C, Ctrl+V and the Ctrl+arrow keys. The menus and the palette reach every command whatever the terminal keeps.
- `diro keys test` says in words what your terminal sent and what diroxide would run.
- Windows Terminal keeps Ctrl+V for text paste. Use Paste in the Edit menu (Alt+E, then P), or unbind the key as [Terminals](terminals.md) shows.
- On Windows a text paste arrives as typed keys: letters jump to names, plus, minus and star open the pattern prompts, and a line break acts as Enter. Bracketed paste, which would let diroxide tell a paste from typing, is on for Unix terminals only. Unbind Ctrl+V as [Terminals](terminals.md) shows if you paste text by accident.
- On Windows the Menu key is never delivered to a terminal program. Shift+F10 opens the context menu instead.
- On macOS the system owns Ctrl+F3 to Ctrl+F8 and the function keys need Fn. Use the Sort submenu, the menu or the palette.
- When diroxide is ended by a signal, by Ctrl+Break or by closing the window, it does not yet give the terminal back. Type `reset` on Linux and macOS, or close the tab on Windows. Quitting, an error and a panic do give it back.
- A program that has the terminal, such as your editor or a shell from Shell here, is waited for without a time limit. Every other wait in the screen has one.
- A move of the cursor keeps the last message on the status line, so a sighted user may see a message that is no longer news. The cursor row is what a screen reader speaks.
- The inline view (`--inline`) is at most 24 rows tall, and a terminal narrower than 20 columns is drawn as 20.
- Terminals not yet tried: the classic console, Konsole, Alacritty, kitty, WezTerm, Ghostty, iTerm2 and macOS Terminal with VoiceOver.

## Screen readers and Braille

- NVDA in Windows Terminal is the main target. Nothing in alpha.2 had been heard on a screen reader when this page was written; the first listening check comes after the build is complete. [The accessibility statement](accessibility.md) records what has been checked.
- JAWS in Windows Terminal is untested.
- Orca and VoiceOver are best effort. Terminals that draw with the graphics card, such as Alacritty and kitty, give Orca nothing to read.
- The key bar is drawn on the last line in every mode, screen-reader mode included, and the `key_bar` setting turns it off. Say key bar in the Help menu reads it. diroxide does not detect a screen reader in alpha.2, so the default `auto` setting counts as on. <!-- to confirm with the wiring (W2f): the key_bar setting, its default, and that the key bar is drawn in screen-reader mode. -->
- Whether eight bytes a row is the right width for hex on a screen reader and a 40-cell display is decided by listening. The `viewer.hex_row` setting changes it.
- The by-kind code commands, the quick view's one-line announcement and the spoken sentences of the archive, split and join dialogs have been checked against 40 characters in tests, not heard. Braille cells in contracted braille take more room than characters.
- Whether NVDA or JAWS speak the terminal's window title on each folder change has not been verified.
- The side by side layout puts the cursor near column 41 when the right panel is active, so a screen reader may read the left panel's row first. The stacked layout avoids this. Which layout suits you best is still being decided by listening; see [Using diroxide with a screen reader](screen-readers.md).
- The 40-cell lines were written for computer braille, one cell per character, on a HumanWare Mantis Q40. Contracted braille and other displays are untested.

## Names and wide characters

- A file name that is not valid Unicode is flagged and refused. diroxide does not show it with changed characters.
- Windows would change some names: a trailing space or dot, a device name such as `NUL`, a reserved character. diroxide refuses to create them.
- Hidden, control and invisible characters in a name are shown as a question mark. The real name is kept for operations, and a listing before a delete spells them out as `U+XXXX`.
- A known drawing bug in the library that paints the terminal can shift the rest of a row after an emoji or a Chinese, Japanese or Korean name in some terminals. No newer release of the library fixes it, so diroxide sends a changed row that holds a wide character again from that character to the end of the row. If a row still looks shifted, note the terminal's name and version when you report it.

## Other limits

- Cut, copy and paste of files (Ctrl+X, Ctrl+C, Ctrl+V) work between diroxide's own panels. They do not use the system clipboard, so Explorer and the Finder cannot paste what diroxide copied.
- Cloud files that are only online are named "online only". diroxide asks before it reads their contents.
- The terminal frontend has no mouse.
- How many times a copy retries a lost network, how long before it calls a copy stalled, and whether it keeps dates or checks a move are built into the job engine. They are not settings.

## See also

- [The accessibility statement](accessibility.md)
- [Archives](archives.md)
- [The viewer](viewer.md)
- [The quick view](quick-view.md)
- [Split and join](split-join.md)
- [Links, permissions and times](links-permissions.md)
- [Terminals](terminals.md)
- [Operations and undo](operations.md)
- [Troubleshooting](troubleshooting.md)
- [The documentation index](README.md)
