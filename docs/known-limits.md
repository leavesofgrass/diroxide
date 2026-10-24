# Known limits

This page says what diroxide does not do yet, where it is limited on purpose or by the system, and what has not been tested. It is kept current with each alpha: the sections on plugins, script commands and optional components arrived with alpha 2. Where a limit has a workaround, the workaround comes first.

## What is not in alpha 1

Each item below is still to come. None of them is dated: a feature that is not in alpha 1 arrives in a later alpha.

- The graphical interface (GUI). diroxide runs in the terminal only. `diro --gui` is accepted, says that the GUI comes later, and opens the terminal.
- Changing an archive. Alpha 1 reads zip, 7z, tar, tar.gz, tar.xz and xz, and packs a new archive from a selection, but it cannot add to, delete from or rename inside an existing one. Copy, move, rename, delete and make folder inside an archive say "Archives are read only here. Use Pack to make a new one." Editing inside an archive comes in a later alpha.
- More archive formats. bzip2 or Zstandard compression on their own, lzip, RAR, ISO images, and a spanned zip set (`.z01`, `.z02`) are not read. diroxide says which one it met and that it is not supported. They are planned for a later alpha, through the 7z program when it is installed. diroxide does not pack to `.gz`, bzip2, Zstandard or RAR.
- The document quick view reads Markdown, plain text, code, and the formats of carta (Org, reStructuredText, MediaWiki, DokuWiki and Jira). DOCX, PDF, HTML, EPUB, spreadsheets, and images described in words come in a later alpha; until then the quick view says "No preview" and names the kind and size.
- A chooser for the viewer's text encoding. The viewer picks UTF-8, UTF-16 by its byte-order mark, or Windows-1252, and says which. A chooser for other encodings comes in a later alpha.
- Changing the owner and group of a file, and a dialog for typing a file time, come in a later alpha. There is no Change times command in alpha 1, although copies keep dates.
- Remote places other than SFTP, and the trash browser, come in a later alpha. SFTP places are in this build; "Remote places over SFTP" below says what they do not do.
- Tools such as ImageMagick and FFmpeg, previews of images and documents, conversions, and the git commands come in a later alpha.
- Tabs, the folder tree, collections, comparing and synchronizing folders, duplicates, batch rename and phones over MTP come in a later alpha. Compare files works on two files; two folders come in a later alpha.
- Download files for each platform come in a later alpha, first for Windows x86_64 and Linux x86_64 and then for the other platforms. `cargo install diro` comes with the first beta.
- Passwords are asked for archives and for signing in to an SFTP server. The masked password prompt serves both.

## Plugins and script commands

[Plugins](plugins.md), [writing a plugin](writing-a-plugin.md) and [script commands](script-commands.md) describe what they do. These are the limits.

- A plugin is a program that runs with your rights. diroxide limits what a plugin may ask of it, and checks that. It cannot limit what the program does on its own. [Security](security.md) says plainly what a plugin could still do. Turn on only what you have read or trust.
- No limits are set on a plugin's processor use, memory, disk use or number of processes. A plugin can use all it is given.
- Ending a plugin whole. On Windows, a job object ends a plugin and everything it started, even when diroxide is ended by force. On Linux and macOS, a normal quit ends the plugin's whole group. When diroxide is killed outright, a plugin that watches its parent, as the examples do, exits on its own, and one that does not can keep running. The code that ends a plugin's group on Linux and macOS has been run in the project's Linux container only.
- A cancel gives a script three seconds to look at its cancel file, and then ends it and everything it started. There is no gentler signal first on Linux and macOS.
- Scripts and folder trust. On Linux and macOS a script whose file or folder others can change is refused. On Windows the access lists are not read yet, so nothing is refused there. Keep the scripts folder writable by you alone.
- A script can start slowly: about a tenth of a second for bash and Python, and a third of a second to over a second and a half for PowerShell 5.1, on the test machine. Plugins stay running between uses for five minutes, so the cost is paid once.
- PowerShell 7 has not been tried. Windows PowerShell 5.1 has, with the policy override. A `.cmd` or `.bat` script runs on Windows only.
- On Windows, bash is Git for Windows' bash. The `bash.exe` in the system folder is WSL's launcher, and diroxide refuses it. Without Git for Windows, bash scripts do not run, and diroxide says so.
- Hooks run for copy, move, delete, rename and new folder (before and after), and for entering a folder and changing the selection. The manifest also accepts `folder_changed`, and nothing sends it yet. [Hooks](hooks.md) has the rest. Shape only for now: `column/format`, `column/changed`, `place/describe`, `ui/form`, `ui/show_diff`, `fs/stage` and `fs/commit`. [The protocol reference](dev/plugin-protocol.md) marks each. A plugin can declare them and will be called when the feature that needs them arrives.
- Watching a folder through a plugin's place is a shape without a working end-to-end test. SFTP cannot watch a folder, so a remote panel does not refresh by itself; open the folder again to see changes.
- The preview region and the conversions dialog come in a later alpha. Until then `diro plugin preview` and `diro plugin convert` run a previewer or a converter on one file.
- Saved tasks and hot folders that would run scripts with nobody there come in a later alpha. Until then `unattended` is checked and shown in the consent question, and nothing uses it.
- A plugin's columns are declared and not shown in the panels yet.
- A list a plugin opens always opens over the panels, like search results, whichever panel the plugin names.
- A plugin's keys are lines in `keymap.toml` and rows in the Keys list, and the Keys list cannot change them yet: Enter on a plugin's row says to edit `keymap.toml`. `diro keys check` does not know plugin commands and calls such a line unknown; the program does not. Manage plugins, on a plugin's Commands and keys row, also says what happened to each proposed key.
- A script's `dry_run` promise is read and checked, and the program offers no preview with it yet.
- The `terminal`, `terminal-wait` and `output-column` modes of a script command are not shown by the program yet.
- `diro serve` has no `fs/*` or `ui/ask` services for the program that connects to it. It sends `ui/announce` and `panel/changed` events.
- A plugin cannot yet tell diroxide that a lost connection is final. Only a plugin turned off for good stops the jobs that wait for its place.
- Scripts in the example pack run from the components folder, where they are pinned and still need your yes. Scripts you write run only from the `plugins/scripts` folder.
- The conformance checks do not yet try a line over the length limit, or check that a plugin exits when diroxide is gone.
- The lines a plugin causes were written to carry their meaning in the first 40 cells and are checked in tests. They have not been heard on a screen reader or read on a Braille display yet.

## Optional components

[Optional components](components.md) describes the feature. These are the limits.

- The list in this build has one component, the example plugins. The pack is not published with a release yet, so until it is, `diro components download diro-examples` needs `--source FOLDER`, or use `diro components install`. A download from the public address says "Try again later, or install from a file."
- Downloads use a built-in client, then `curl`, then `wget` from your PATH. A build of diroxide made without the built-in client needs `curl` or `wget`, and says "Not downloaded: curl was not found." when it has neither.
- A private repository as your own source, reached through the GitHub CLI's sign-in, is not built yet. A folder works, and a repository written `owner/name` is read from its public release downloads.
- Where a helper has no build for your computer, the message says "No build for this computer." and names a generic next step, not your system's own install command.
- The list's `min_diro`, `needs_tools` and `protocol` fields are read and not enforced yet.
- After Escape, a stalled transfer in the built-in client can take a moment to stop, until the client's own time limits end it. `curl` and `wget` are ended at once.
- A component that would run an installer is refused: "installers are not supported yet".
- The downloaded programs are not code-signed, and neither are the builds of diroxide. Windows' SmartScreen and virus scanners may react.
- The first-run list of components is built and is not shown at the first start yet, because the list holds only the examples. Ask first-run choices again, in the Tools menu, opens it, and it says "No optional components to choose from yet".
- A no to a download is not remembered. Get example plugins asks again each time.
- Installing from a zip file or a folder says "Not downloaded" when it finds none of the files, which is the wrong word for an install.

## Remote places over SFTP

[Remote places over SFTP](remote-places.md) describes what they do. These are the limits.

- Editing a remote file in place is not built. F4, Shell here and Run a command start programs only in folders on this computer. Copy the file down, edit it, and copy it back.
- Synchronized browsing, and importing sites from PuTTY, WinSCP or FileZilla, are not built.
- Only SFTP, through the OpenSSH program on your computer. There is no FTP, and no built-in SSH for a computer without OpenSSH.
- Passwords need OpenSSH 8.4 or newer. With an older ssh the plugin works with keys and the agent only, and says so.
- If several ssh programs are installed, the newest wins. On a Windows computer with Git installed that is often Git's, not the one in Windows. The `ssh_program` setting chooses.
- On Windows, ssh's password questions reach the plugin through a named pipe that only your account can open. diroxide reads the pipe's permissions back from the live pipe to check this, and a proof that only the real helper can make stands behind it as a second layer. It has been tried on one Windows build.
- The tests use a keyring held in memory. The round trip through the real Windows Credential Manager, macOS keychain or Secret Service is a test that runs only when the owner asks, and it has not been run. Windows Credential Manager entries can be read by other programs of the same user, and the store limits the length of a secret.
- A site's `bookmarks` list is read and kept in `sites.toml`. Nothing offers those folders yet.
- A background connect, such as an automatic reconnect, never uses the keyring.
- The tests ran against a fake server and against OpenSSH 10.0's server in a Linux container. A server across a network, and the Windows and macOS builds of ssh against a real server, have not been tried by hand. The password prompts were captured from OpenSSH 10.0 on Linux, and the new-key and changed-key texts also from the ssh in Windows. The questions of other versions are expected to read the same.
- A server that does not report free space is not checked for room before a copy. A copy that does not fit ends with "The server's disk or quota is full."
- A frozen server takes about a minute to be noticed, because ssh gives up on the fourth missed keepalive. A server that closes the connection is noticed in a fraction of a second.
- A server-side copy that was cancelled still finishes on the server. The cleanup runs after it.
- Changing the permissions of a link changes what the link points to.
- The plugin is built from the source tree. A download for each system comes with a later build.
- A list the plugin opens, the Sites list and the doctor report, comes over the panels, not inside one.

## The transfer queue

[The transfer queue](transfer-queue.md) describes the Jobs list. These are the limits.

- The queue is not saved when you quit. Quitting asks first when any job runs or waits.
- Pause is not offered for a job that is waiting its turn. Cancel it or move it instead.
- A job's error policy can be changed only while the job waits.
- Retry the failed items covers copy, move, trash and delete. It runs again the original selections that held failures and skips what is already at the destination.
- Resume applies to copies of 4 MiB or more. The head and tail check compares the first 64 KiB and the last 1 MiB of the part file, so a bad byte in the middle is not seen by a plain copy. A move reads everything back before it removes the source.
- Split and join never resume. A network share counts as a local place: it has no gate for a lost connection, only the per-item retries.
- A copy cut by a lost network keeps its part file for a resume. On a network share, answering Skip can leave a `.diro-part-` file, listed in `partials.json` in the state folder and never deleted unasked.
- Several connections for one big file are not built. The speed limit applies to remote jobs only, unless the scope setting says all.
- A `select` hook also fires when the focus moves, after a pause of a third of a second.
- A crash in the middle of a run is reported at the next start, but the queue is not rebuilt, and a retry goes on from a part file only if the ledger still holds it.

## Search and the filter language

- Selecting by file contents, and refining search results by file contents, are not built. Each says so in words: "Selecting by contents is not built yet. Filter by contents, then select all." and "Refining by contents is not built yet. Search again with Contains text."
- A panel's filter that reads file contents runs the search engine over that one folder, at depth 1. `selected` and `bookmarked` stay undecided inside it, and the undecided reasons are counted, not kept for each file. A saved view does not apply a filter that reads contents.
- A result's file is checked after the focus has rested on it for 300 milliseconds. A row already known to be gone is refused. A row not yet checked relies on the error the action meets.
- The question that offers to download online-only files, shown when a search's Online-only files row is "read after asking", goes through the runtime and has not been tried against a real online-only file.
- The results list is a list in front of the panels, not a panel. Only these panel keys work on its rows: view, edit, copy, move, trash, delete for good, marking, refine and show in folder, with the actions list on Shift+F10. Rename and labels do not.
- A saved search can be run and deleted. It cannot be renamed or edited. Save it again under a new name.
- Pause search is F2 in a list. In another list F2 says "No search is running."
- Tab in the Go to a place list completes bookmarks only, by name or place. The list under the field still narrows over every place.
- On Windows a leading dot does not make a file hidden, so "Hidden files skipped." is said there only for files with the hidden attribute.
- Go to a path on Windows keeps the forward slashes of a place with a scheme, such as `sftp://`.
- Content search on Windows is bound by the cost of opening each file: reading every file of a 1 GB tree whole takes about 1.5 seconds on the test machine, and a search needs about 2. Memory maps are off.
- Search over a network has been tested against a fake place that answers in 5 milliseconds, not against a real share. The thread counts and the rates in [the search guide](search.md) come from that fake place.
- A search that follows links or looks inside archives always uses the slower walk, one folder at a time through the provider.
- In a search, `selected` and `bookmarked` are left undecided, because a search has no panel. `empty` on a folder reads that folder when asked.
- A depth test works in a search and not in a panel, which shows one folder.
- `type` matches only entries whose type is already known, and `group` goes by extension, with seven groups. The full type model comes in a later alpha.
- Dates use the computer's current offset from UTC, with no daylight-saving rules. A file changed in the hour a clock changed may land on the wrong side of midnight.
- Patterns are Rust regular expressions: no look-around and no backreferences. A pattern is at most 1,000 characters.
- Owner and user group are read where the listing has them. Windows owners are not read in a listing.
- Names are compared as stored. Accents are not folded.
- Search has no index. It walks the folders, so it is not instant over a whole drive.
- The Space list shows at most 30 rows. Hard links are counted once on Unix only; on Windows they may be counted more than once, and the list says so.

## Labels, bookmarks and saved views

- A label step can be undone only in the same session. A job's undo survives a restart.
- There is no command to rename, recolor, reorder or delete a label. Edit `labels.toml` by hand. The favorite keys follow the order in the file.
- A file that is renamed or moved loses its label, even when diroxide did it, because a label belongs to the file's path. A label's results show the old place as gone until you take the label off.
- Ctrl+1 to Ctrl+7, the favorite labels, do not arrive in many Unix terminals. The Edit menu's Labels submenu is the sure way.
- No command sets a panel's columns yet, so a saved view keeps the columns in use when you saved it.
- Typing in the Bookmarks list does not narrow it. An alias letter jumps, and any other letter moves by name.
- Ctrl+B is tmux's prefix key, so inside tmux it arrives only when pressed twice. Alt+B and the Go menu open the bookmarks.

## Compare files

- Two files only. Two folders come in a later alpha, and so does `--names -0`.
- A file inside an archive is not compared. Copy it out first.
- The unified view is a list, not a mode of the viewer. Enter on a row opens the viewer at its line.
- Next and previous change are F7 and Shift+F7 in the compare list. Some terminals send Shift+F7 as another key. P, or Shift+F10 and its actions list, is the sure way. The Compare submenu in the Search menu cannot open while a compare is open.
- Unicode forms are recognized for Latin, Greek, Cyrillic and Hangul. "Sounds alike" covers space characters, dashes, quotes, full-width letters and a short list of Cyrillic and Greek letters that look like Latin ones. Other lookalikes are listed as ordinary changes.
- Text is compared in full up to 16 MiB for each file. Past that, you choose the first 16 MiB as text or a byte-by-byte compare.
- "Stopped early" is worked out from the clock after the run, not reported by the comparing code.
- When names cannot keep four cells in the 40-cell verdict line, the line leaves them out, and the next line names the files.
- The deadline, the text limit and how the three line algorithms compare have starting values that have not been tuned on a quiet machine.

## Keys and presets

- The presets are modeled on written sources, not on the programs. Total Commander's own Help was not read, and its keys come from published lists and forum pages that disagree in places. Midnight Commander's come from its manual, Worker's from the project's own notes, and the vim-style keys from the manuals of ranger, lf, yazi and vifm. A key may differ from the version you know.
- A preset holds no key sequences. Midnight Commander's Ctrl+X family, Worker's Ctrl-x family and the doubled letters of vim-style managers (gg, dd, yy, pp) are left out or turned into single letters.
- The keypad keys that some presets use need Num Lock on under NVDA and JAWS, whose review keys use the bare keypad.
- The terminal column of every preset keeps off the keys a terminal never sends. A key a source manager uses that fails this check is in the GUI column only.
- A key press in a terminal can change only the terminal column. The GUI column is changed by typing the key's name.
- The delivery table, which says what each terminal does with each key, has most of its rows marked "to probe". `diro keys probe` fills in a terminal's column, but only when someone runs it there and sends the answer back.
- A command that Change key has no sentence for is described as "runs" and its name.
- On Windows a text paste arrives as typed keys, so during Change key its first character is taken as the key. The questions that follow start on No. The capture says nothing about the terminal on Windows, because whether Windows reports keys exactly is not probed.
- The Explorer and Finder preset has no Command+D, because on a Mac Command+D is Say status. In the Worker preset Alt+T sorts in a panel, so the Tools menu is F9, then T.
- In the Explorer and Finder preset, Control with 1 to 7 sets the favorite labels, and macOS may keep those keys for Mission Control. That is checked when the GUI arrives on a Mac. The Labels submenu of the Edit menu is the sure way.
- A key that a plugin or a script proposes binds only when it is free and has no concern. The Keys list and Manage plugins both say what happened to it.

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
- Watch this folder, which says when a new file has finished arriving, has its own limits: [watching a folder](watching.md) lists them. On Linux and macOS it cannot tell that a program still holds a file open, so a producer that pauses for more than 2 seconds is said early and then said again as "Arrived again". On Windows only a rename inside the watched folder gets the half-second wait; a file moved in from another folder waits the full 2 seconds.

## Dates and times

- A copy keeps the modified time of files and folders, and sets a folder's time after its contents so the copy itself never replaces it.
- The created time is kept on Windows and macOS. Linux cannot set a created time, so it is not kept there.
- The accessed time is never kept.
- An unpack keeps each member's modified time. A zip stores a time in the old DOS form, which has no time zone and only two-second steps, and diroxide reads and writes it as UTC. Programs that read it as local time, such as Info-ZIP's `unzip`, show the times of a zip diroxide wrote shifted by your offset from UTC, and a zip written by such a program shows shifted in diroxide. The extended time field that zip also allows is not read or written yet.
- Copies and moves keep dates, and there is no setting to turn that off.

## Archives

- Limits: [the archives guide](archives.md) lists them with the questions they ask. A listing shows at most 500,000 entries of one folder and says so. An archive with more than a million members is refused unless you raise `archives.max_entries`.
- The command-line `diro archive` commands use the built-in limits, not the `[archives]` table in your settings. Only the program in the terminal or the GUI reads the table.
- 7z passwords: diroxide wipes its own copy of a typed password when the job ends, and leaving a locked 7z makes it forget the password. The library that reads and writes 7z keeps a copy in memory in a form diroxide cannot wipe, for as long as it needs it: one read, or the whole time a 7z is being packed. A zip password is only borrowed by its library and does not have this limit.
- A tar.gz, tar.xz or solid 7z is listed with no progress line. A "Reading logs.tar.xz: 3.2 of 14.0 GB" line is planned and not wired, so a big one shows nothing until the list is ready.
- Junk entries such as `__MACOSX` are marked hidden by the archive reader. Whether Show hidden files lists them in the panel has not been checked.
- F5 out of an archive always unpacks with the default settings. There is no "copy with options" yet. To choose the folder rule, the name policy or links, use Unpack from the File menu. Shift+F5 is Duplicate.
- Leftover temporary files from packing (a pack that was killed) are cleaned up only in folders diroxide remembers, and only when they are older than a day.
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
- The quick view does not draw code colors. It shows code as plain lines; the viewer (F3) colors it.
- In the single layout the quick view is drawn only while the focus is in it. The one-line announcement still arrives as you move.
- Some terminals send Shift+F3 as F15. The View menu (Alt+V, then Q) and the palette turn the quick view on.
- The key test, `diro keys test`, reports panel and viewer keys. It does not know the quick view's keys yet.
- An Org file's row in the panel says "file", because the file type table has no Org entry. The preview line says "Org".
- The quick view wraps by counting characters, so a wide character counts as one.
- A reader that runs past its deadline cannot be stopped, so it is left to finish, and at most three run at once. A deeply nested document can be cut off at the deadline: "Could not read big.org: took too long." carta 0.0.11 keeps only 32 levels of a nested DokuWiki list, and content nested past 256 elements reads as one row.
- Raw HTML in Markdown keeps its text and drops its tags. A front-matter title or an Org `#+TITLE` becomes the document's title, not a heading row.
- Open in textweaver hands over files on this computer only, not a file inside an archive.

## Split and join

- A read error partway through a join fails the whole join. Run it again.
- With very small parts, the split progress line can run past 40 characters. The part count stays inside the first 40.
- The checksum file has no SHA-256 line yet. It comes in a later alpha.
- Spanned zip sets (`.z01`) and RAR volumes are not byte splits and are not joined by putting the parts end to end.
- Whether Total Commander and Double Commander find the checksum file diroxide writes, `report.pdf.crc`, from `report.pdf.001` has not been tried. `cat`, `copy /b` and diroxide's own join are tested against each other.
- Parts keep the file's extension in their names, as 7-Zip's do. Total Commander's own parts may drop it. diroxide reads both.

## Links, permissions and times

- Windows needs Developer Mode or an administrator to make a symbolic link, and a FAT or exFAT drive holds no links at all. The words for the missing right are tested on the error itself; the test machine had the right, so the real refusal has not been seen yet.
- A folder link on Windows cannot be changed in one step, so changing its target uses the trash. On a drive with no trash, changing a folder link's target fails in words.
- Not built yet from the permissions plan: typing an octal number or a symbolic mode such as `u=rw,g=r`, Make runnable, the list of permission presets, a list of protected places that refuse changes in administrator mode, extra checks for private-key names, changing a file with several hard links once, and a "why can't I?" explanation. The permissions dialog steps each of owner, group and others through six choices instead.
- Access lists, flags such as immutable and locked, and extended attributes are neither shown nor edited. They come in a later alpha, shown first. Changing the owner comes in a later alpha too.
- The first rows of the details list (type, size with its exact bytes, a link's target) have no dates of their own, because the preview code has no time zone. The times are in the details list as before.
- The Unix half of the atomic link swap is covered by the project's Linux container check. It has not been run natively on a Unix machine outside it.

## Terminals and keys

The table of keys each terminal can send is built from a key probe run by hand on each terminal, and few terminals have been probed, so some keys may not arrive in some terminals.

- [The keyboard page](keyboard.md) lists the keys a terminal may keep, with the sure way to each command. The ones that most often fail are the Norton sort keys Ctrl+F3 to Ctrl+F7, Alt+Left, Alt+Right, Alt+Up, Shift+F5, Shift+F10, Alt+Enter, Ctrl+C, Ctrl+V and the Ctrl+arrow keys. The menus and the palette reach every command whatever the terminal keeps.
- `diro keys test` says in words what your terminal sent and what diroxide would run. `diro keys probe` asks for each key in turn and prints what arrived.
- The Keys list says "check terminal" on a row whose key many terminals keep or merge, and on every Shift plus function key, because some terminals send those as F13 to F24.
- Windows Terminal keeps Ctrl+V for text paste. Use Paste in the Edit menu (Alt+E, then P), or unbind the key as [Terminals](terminals.md) shows.
- On Windows a text paste arrives as typed keys: letters jump to names, plus, minus and star open the pattern prompts, and a line break acts as Enter. Bracketed paste, which would let diroxide tell a paste from typing, is on for Unix terminals only. Unbind Ctrl+V as [Terminals](terminals.md) shows if you paste text by accident.
- On Windows the Menu key is never delivered to a terminal program. Shift+F10 opens the context menu instead.
- On macOS the system owns Ctrl+F3 to Ctrl+F8 and the function keys need Fn. Use the Sort submenu, the menu or the palette.
- When diroxide is ended by a signal, by Ctrl+Break or by closing the window, it gives the terminal back and exits with 130. While a program has the terminal, such as your editor, the signal is that program's. Quitting, an error and a panic give the terminal back too.
- A program that has the terminal, such as your editor or a shell from Shell here, is waited for without a time limit. Every other wait in the screen has one.
- A move of the cursor keeps the last message on the status line, so a sighted user may see a message that is no longer news. The cursor row is what a screen reader speaks.
- The inline view (`--inline`) is at most 24 rows tall, and a terminal narrower than 20 columns is drawn as 20.
- In the terminal built into some desktop apps, Alt+F and Ctrl+Q never arrive. F9 opens the menus, and Quit is in the File menu and the palette. Other terminals built into programs may keep keys too; `diro keys test` says what arrives.
- Terminals not yet tried: the classic console, Konsole, Alacritty, kitty, WezTerm, Ghostty, iTerm2 and macOS Terminal with VoiceOver.

## Screen readers and Braille

- NVDA in Windows Terminal is the main target. Nothing in alpha 1 had been heard on a screen reader when this page was written; the listening checks come after the build is complete. [The accessibility statement](accessibility.md) records what has been checked.
- JAWS in Windows Terminal is untested.
- Orca and VoiceOver are best effort. Terminals that draw with the graphics card, such as Alacritty and kitty, give Orca nothing to read.
- The key bar is drawn on the last line in every mode, screen-reader mode included, and the `key_bar` setting turns it off. Say key bar, in the Help menu (Alt+H, then K) and the palette, reads it. At 80 columns the bar uses one space between pairs so that all nine fit. diroxide does not detect a screen reader in alpha 1, so the default `auto` setting counts as on.
- Whether eight bytes a row is the right width for hex on a screen reader and a 40-cell display is decided by listening. The `viewer.hex_row` setting changes it.
- The by-kind code commands, the quick view's one-line announcement and the spoken sentences of the archive, split and join dialogs have been checked against 40 characters in tests, not heard. Braille cells in contracted braille take more room than characters.
- Whether NVDA or JAWS speak the terminal's window title on each folder change has not been verified.
- The side by side layout puts the cursor near column 41 when the right panel is active, so a screen reader may read the left panel's row first. The stacked layout avoids this. Which layout suits you best is still being decided by listening; see [Using diroxide with a screen reader](screen-readers.md).
- The lines of search, the filter, labels, bookmarks, compare, the Keys list and watching were written to carry their meaning in the first 40 cells and are checked in tests, not heard. Whether the count after the filter pause helps or interrupts, whether a result row's first 26 cells are enough, and whether the status line's "Waiting for ... to settle" is read aloud as dynamic content are for the listening check to decide.
- The 40-cell lines were written for computer braille, one cell per character, on a HumanWare Mantis Q40. Contracted braille and other displays are untested.
- The lines of remote places, the Jobs list, the connection question, resume and hooks were written to carry their meaning in the first 40 cells and are checked in tests, not heard. That includes the new server key dialog, the password prompts, the changed-key refusal, the Sites list rows, and "Delete stopped: folder is protected. By Folder guard."
- The lines of plugins, script commands and optional components (the turn-on question, the jobs line, the failure lines, the ask-first question) were written to carry their meaning in the first 40 cells and are checked in tests, not heard. A plugin's own text is cleaned and given its name last, but its wording is the plugin's, so a plugin's lines may not fit 40 cells.

## Names and wide characters

- A file name that is not valid Unicode is flagged and refused. diroxide does not show it with changed characters.
- Windows would change some names: a trailing space or dot, a device name such as `NUL`, a reserved character. diroxide refuses to create them.
- When a name is taken on a copy, Keep both and Duplicate follow the desktop convention: `course (2).zip`, with a space and parentheses. That is the owner's choice, so the names look like the ones Explorer and the Finder make. Rename the file afterward if you prefer a plain name.
- Hidden, control and invisible characters in a name are shown as a question mark. The real name is kept for operations, and a listing before a delete spells them out as `U+XXXX`.
- A known drawing bug in the library that paints the terminal can shift the rest of a row after an emoji or a Chinese, Japanese or Korean name in some terminals. No newer release of the library fixes it, so diroxide sends a changed row that holds a wide character again from that character to the end of the row. If a row still looks shifted, note the terminal's name and version when you report it.

## Other limits

- Cut, copy and paste of files (Ctrl+X, Ctrl+C, Ctrl+V) work between diroxide's own panels. They do not use the system clipboard, so Explorer and the Finder cannot paste what diroxide copied.
- Cloud files that are only online are named "online only". diroxide asks before it reads their contents.
- The terminal frontend has no mouse.
- How long before a copy is called stalled (20 seconds), and whether it keeps dates or checks a move, are built into the job engine. They are not settings. The number of tries on a lost network and the first wait are settings, in the Jobs and transfers group.

## See also

- [The accessibility statement](accessibility.md)
- [Plugins](plugins.md)
- [Hooks](hooks.md)
- [Remote places over SFTP](remote-places.md)
- [The transfer queue](transfer-queue.md)
- [Writing a plugin](writing-a-plugin.md)
- [Script commands](script-commands.md)
- [Optional components](components.md)
- [Security](security.md)
- [Archives](archives.md)
- [The viewer](viewer.md)
- [The quick view](quick-view.md)
- [Split and join](split-join.md)
- [Links, permissions and times](links-permissions.md)
- [Search and the filter language](search.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Compare files](compare.md)
- [Customizing keys](customizing-keys.md)
- [Watching a folder](watching.md)
- [Terminals](terminals.md)
- [Operations and undo](operations.md)
- [Troubleshooting](troubleshooting.md)
- [The documentation index](README.md)
