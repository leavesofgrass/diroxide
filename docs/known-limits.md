# Known limits

This page says what diroxide does not do yet, where it is limited on purpose or by the system, and what has not been tested. It is kept current with each alpha: the sections on plugins, script commands, optional components, remote places, the transfer queue, and volumes and the trash arrived with alpha 2, and the section on the GUI arrived with alpha 3. Alpha 4's sections on tools, file types, previews, the details view and access facts, columns, checksums, encryption and signatures, privacy before sharing, conversions, watched folders, disk images, git, photos, and links, permissions and times are here as the alpha stands at its release. Alpha 5, which has not been released, adds the sections on tabs, compare folders and synchronize, duplicates, batch rename, and lists that are not folders, and changes the lines that alpha 4 held back for it. What alpha 4 held back for the next alpha is listed once, near the top. Where a limit has a workaround, the workaround comes first.

## What is not in alpha 5 yet

Each item below is still to come. None of them is dated: a feature that is not in alpha 5 yet arrives in a later pass or a later alpha.

- Parts of the graphical interface (GUI). On a desktop `diro` opens the window, and `diro --tui` opens the terminal. The window shows the panels, takes every key and says what happens, and it leaves out an icon on macOS, a native menu bar on macOS and a few smaller things. [The GUI](#the-gui) below lists what it does not do yet, and what has not been tried.
- Changing an archive further. A zip can be edited: Delete, Rename and Make folder inside it rewrite it. Adding files to an existing archive is not built, and a 7z, a tar and an archive read through 7-Zip are never changed. Copy into one, rename, delete and make folder say "Archives are read only here. Use Pack to make a new one."
- More archive formats. bzip2 or Zstandard compression on their own, and lzip, are not read. diroxide says which one it met and that it is not supported. RAR, CAB, ARJ, LZH, RPM, cpio, ar and a spanned zip set (`.z01`, `.z02`) are read, read only, through 7-Zip when it is installed. An ISO image and the virtual disk formats open through the disk image plugin, an optional component, or through 7-Zip; [Disk images](disk-images.md) says what they do not do yet. diroxide does not pack to `.gz`, bzip2, Zstandard or RAR.
- The quick view reads Markdown, plain text, code, the formats of carta (Org, reStructuredText, MediaWiki, DokuWiki and Jira), pictures, sound and video, archives, PDF, Word and other documents, and spreadsheets. [Previews](#previews) below says what it does not do.
- Remote places other than SFTP come in a later alpha. SFTP places, the Volumes list and the trash browser are in this build; "Remote places over SFTP" and "Volumes and the trash" below say what they do not do.
- Tools, previews, the file type model, columns, checksums, the privacy commands, conversions, watched folders, disk images, the git plugin, encryption and signatures, the access rows with Change time and Change owner, and the photo helpers are in alpha 4, and the sections below say what they do not do.
- The folder tree and phones over MTP come in a later alpha. Tabs, collections, comparing and synchronizing folders, duplicates and batch rename are in alpha 5: [Tabs](#tabs), [Compare folders and synchronize](#compare-folders-and-synchronize), [Duplicates](#duplicates), [Batch rename](#batch-rename) and [Lists that are not folders](#lists-that-are-not-folders) below say what they do not do yet.
- A grid of thumbnails for the photo helpers, an editor for extended attributes, and tasks other than watched folders, such as a synchronize or a rename that runs by itself, come in a later alpha. Extended attributes are read for the details view and are not edited.
- A Lua tier for plugins, saved synchronize tasks, an rsync engine and Save as script are not in this alpha.
- Download files for Windows on ARM and Linux on ARM come in a later alpha. The releases have the files for Windows x86_64, macOS (one universal file) and Linux x86_64. `cargo install diro` comes with the first beta.
- Passwords are asked for archives and for signing in to an SFTP server. The masked password prompt serves both.

## Held back from alpha 4

Each item below was planned for alpha 4, is known, and was left for a later alpha. A person may look for each, so it is named here, and it is also in the section of its feature.

- Conversions and watched folders: a plugin's converters are not in the Convert list, and a plugin's command in a watched folder's task still pauses the task. The program has no Save report command. `diro convert --report FILE` writes one.
- Columns: a field can give a sort value in alpha 5, and the duration and dimensions columns sort as numbers; other fields still sort by their words. Field columns have no header or grid. A cell says "reading" for a kind of file that no reader takes, until the readers have been through it.
- Encryption: the Encryption plugin writes its files itself, so Undo for an age command lasts for the session only. In alpha 5, gpg's ASCII armor is offered as two commands, "Encrypt with gpg, as text" and "Sign with gpg, as text".
- Photos: the Taken column reads the date to the minute. Alpha 5 reads it to the second as a field of its own, `image.taken_second`, which the details list leaves out. Bursts already read the date to the second. Move into month folders is planned by the engine in alpha 5, so a plugin's before-move hook runs for it. A row of the details list has no help of its own.
- Speed: listing a folder and reading each file's details cost more than they did in the build of October 16, 2026. Alpha 5 removes the two causes, and a quiet machine has not measured it yet. [Other limits](#other-limits) has the numbers.
- The window: a paste under Wayland with no XWayland still reads nothing, and no real Wayland compositor has been tried.

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
- Hooks run for copy, move, delete, rename and new folder (before and after), and for entering a folder and changing the selection. The manifest also accepts `folder_changed`, and nothing sends it yet. [Hooks](hooks.md) has the rest. Shape only for now: `column/changed` (nothing tells the panels that a plugin's cells are stale) and `completion/query`. `column/format`, `place/describe`, `ui/form`, `ui/show_diff`, `fs/stage` and `fs/commit` are served since alpha 4. A plugin can declare the shapes and will be called when the feature that needs them arrives.
- A plugin's form or diff has few limits checked yet: nothing stops 5,000 fields, a default of 1 MB, a label of 90 cells or a title of 120 cells, and an empty id or label is not refused. Unknown keys are dropped in silence, and a title that already names the plugin gets the name twice. Binary content cannot travel in JSON, so a diff side with NUL bytes is read as text and compared byte by byte, and `encoding` and `place` on a diff side are dropped.
- Watching a folder through a plugin's place is a shape without a working end-to-end test. SFTP cannot watch a folder, so a remote panel does not refresh by itself; open the folder again to see changes.
- The Convert command does not list a plugin's converters yet, and the quick view does not call a plugin's previewer. `diro plugin preview` and `diro plugin convert` run a previewer or a converter on one file.
- A plugin's command run by a watched folder's task pauses the task: "plugin commands are not available here".
- A watched folder's script task runs only a script that promises `unattended = "ok"`, and has been built and not run through the real script host. [Watched folders](#watched-folders) says more. Saved tasks other than watched folders come in a later alpha.
- A plugin list opens inside a panel. These lists still open over the panels, as dialogs: the Volumes list, the trash browser, the Jobs list, search results, and the results of a script in `lister` mode.
- A plugin list is not saved when you quit, and it stays in its panel when its plugin stops or is turned off: leave it with Backspace.
- The window draws a plugin list in its panel. A click on a row of a plugin list does not move the focus to it yet, and a double click presses Enter on the row that has the focus. Use the keys.
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

- The list in a build made from source has the example plugins and the helper programs. The release files add the SFTP places, disk image, git and Encryption plugins, each with a program for Windows on x86-64, Linux on x86-64 and macOS. Windows on ARM and Linux on ARM have none of the four yet. The example pack is not published with a release yet, so until it is, `diro components download diro-examples` needs `--source FOLDER`, or use `diro components install`. A download from the public address says "Try again later, or install from a file."
- Downloads use a built-in client, then `curl`, then `wget` from your PATH. A build of diroxide made without the built-in client needs `curl` or `wget`, and says "Not downloaded: curl was not found." when it has neither.
- A private repository as your own source, reached through the GitHub CLI's sign-in, is not built yet. A folder works, and a repository written `owner/name` is read from its public release downloads.
- Where a helper has no build for your computer, the message says "No build for this computer." and names the install command for your system where diroxide knows it, such as "Install: sudo apt install ffmpeg". For a plugin of the project, such as the SFTP places plugin, it names the `cargo install` line instead, which needs a Rust toolchain, and the program it builds still has to be put where [remote places](remote-places.md) says.
- The list's `min_diro`, `needs_tools` and `protocol` fields are read and not enforced yet.
- After Escape, a stalled transfer in the built-in client can take a moment to stop, until the client's own time limits end it. `curl` and `wget` are ended at once.
- A component that would run an installer is refused: "installers are not supported yet".
- The downloaded programs are not code-signed, and neither are the builds of diroxide. Windows' SmartScreen and virus scanners may react.
- The first-run list of components opens once, at the first start, and only when the list holds something besides the example plugins: that is, the helper programs in every build, and the four plugins in the release files. `diro run`, `diro serve` and a plain listing on a pipe never open it. The list never covers a dialog that is open at the start; Ask first-run choices again, in the Tools menu, opens it later, and says "No optional components to choose from yet" when there is nothing to choose.
- A no to a download is not remembered. Get example plugins asks again each time.
- Installing from a zip file or a folder says "Not downloaded" when it finds none of the files, which is the wrong word for an install.
- After a helper is installed, diroxide runs it once to see that it starts. The line that says so, "Installed pandoc, but it did not start.", is built and is not said yet by the program or by `diro components`.

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
- A server that does not report free space, or does not answer within 2 seconds, is not checked for room before a copy. You hear "Free space on build is not known." before the question, and the copy goes on. A copy that does not fit ends with "The server's disk or quota is full."
- A frozen server takes about a minute to be noticed, because ssh gives up on the fourth missed keepalive. A server that closes the connection is noticed in a fraction of a second.
- A server-side copy that was cancelled still finishes on the server. The cleanup runs after it.
- Changing the permissions of a link changes what the link points to.
- The plugin is a component with a program for Windows on x86-64, Linux on x86-64 and macOS, in the release files. On any other computer, and in a build made from source, it is built from the source tree.
- The doctor report comes over the panels, as a document. The Sites list opens inside a panel.

## The transfer queue

[The transfer queue](transfer-queue.md) describes the Jobs list. These are the limits.

- The queue is kept in `queue.json` in the state folder, and the next start lists what a quit or a crash stopped as interrupted jobs. With two diroxide windows on one state folder, only the first keeps the queue: the second neither lists nor saves it, and its jobs are lost when it quits. Quit the second window when its jobs are done.
- A quit stops a running copy the way a lost connection does, so a copy of 4 MiB or more that a quit cuts keeps its part file, and Retry on its Interrupted row goes on from it. A smaller copy starts again. Cancel still removes the part file. Only this computer's own disks keep part files so far: an SFTP place has the calls in its protocol and the program does not use them yet, so a copy to a server that is cut starts again.
- Retry on an interrupted job needs the request it was planned from. An undo is saved without one, and offers Retry dimmed. Forget it, and undo again yourself.
- Pack, unpack and archive tests, and a plugin's own commands, do not go through the queue, so they are not kept across a restart either.
- An interrupted job that had started skips what is already at the destination on Retry, as Retry the failed items does, even where the first run would have asked or replaced.
- A plain listing on a pipe (`diro` with its output not a terminal) reads the saved queue like any start, so its one line about interrupted jobs can be said there and not in the next window.
- A job's error policy can be changed only while the job waits or is held.
- Retry the failed items covers copy, move, trash and delete. It runs again the original selections that held failures and skips what is already at the destination.
- Resume applies to copies of 4 MiB or more. The head and tail check compares the first 64 KiB and the last 1 MiB of the part file, so a bad byte in the middle is not seen by a plain copy. A move reads everything back before it removes the source.
- Split and join never resume. A network share counts as a local place: it has no gate for a lost connection, only the per-item retries.
- A copy cut by a lost network keeps its part file for a resume. On a network share, answering Skip can leave a `.diro-part-` file, listed in `partials.json` in the state folder and never deleted unasked.
- Several connections for one big file are not built. The speed limit applies to remote jobs only, unless the scope setting says all.
- A `select` hook also fires when the focus moves, after a pause of a third of a second.
- A crash in the middle of a run is said twice at the next start: once by the undo journal, "Last session ended during a copy: 3 of 5 steps done.", and once as interrupted jobs. A retry goes on from a part file only if the ledger still holds it.

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
- `type` matches only entries whose type is already known. `group` goes by the type model in a panel, with eight groups and yours, and a file can be in two. In a search, and in Space by kind, it goes by extension, with seven groups.
- Dates use the computer's current offset from UTC, with no daylight-saving rules. A file changed in the hour a clock changed may land on the wrong side of midnight.
- Patterns are Rust regular expressions: no look-around and no backreferences. A pattern is at most 1,000 characters.
- Owner and user group are read where the listing has them. Windows owners are not read in a listing.
- Names are compared as stored. Accents are not folded.
- Search has no index. It walks the folders, so it is not instant over a whole drive.
- The Space list shows at most 30 rows. Hard links are counted once on Unix only; on Windows they may be counted more than once, and the list says so.

## Labels, bookmarks and saved views

- A label step can be undone only in the same session. A job's undo survives a restart.
- There is no command to rename, recolor, reorder or delete a label. Edit `labels.toml` by hand. The favorite keys follow the order in the file.
- A label, a note and a field belong to the file's path. They follow a file that diroxide moves or renames, and an undo of that, but a copy does not take them, and a file moved by another program loses them. A label's results show the old place as gone until you take the label off.
- A note is one line of at most 300 characters. A field name is at most 24 characters, in small letters, digits, `-` and `_`.
- Ctrl+1 to Ctrl+7, the favorite labels, do not arrive in many Unix terminals. The Edit menu's Labels submenu is the sure way.
- No command sets one panel's columns yet, so a saved view keeps the columns in use when you saved it. The Columns setting changes both panels.
- Typing in the Bookmarks list does not narrow it. An alias letter jumps, and any other letter moves by name.
- Ctrl+B is tmux's prefix key, so inside tmux it arrives only when pressed twice. Alt+B and the Go menu open the bookmarks.

## Compare files

- Compare files takes two files. Two folders are compared by Compare folders, and [that section](#compare-folders-and-synchronize) has its limits.
- A file inside an archive is not compared. Copy it out first.
- The unified view is a list, not a mode of the viewer. Enter on a row opens the viewer at its line.
- Next and previous change are F7 and Shift+F7 in the compare list. Some terminals send Shift+F7 as another key. P, or Shift+F10 and its actions list, is the sure way. The Compare submenu in the Search menu cannot open while a compare is open.
- Unicode forms are recognized for Latin, Greek, Cyrillic and Hangul. "Sounds alike" covers space characters, dashes, quotes, full-width letters and a short list of Cyrillic and Greek letters that look like Latin ones. Other lookalikes are listed as ordinary changes.
- Text is compared in full up to 16 MiB for each file. Past that, you choose the first 16 MiB as text or a byte-by-byte compare.
- "Stopped early" is worked out from the clock after the run, not reported by the comparing code.
- When names cannot keep four cells in the 40-cell verdict line, the line leaves them out, and the next line names the files.
- The deadline, the text limit and how the three line algorithms compare have starting values that have not been tuned on a quiet machine.

## Tabs

This section is for alpha 5, which has not been released. [Tabs](tabs.md) is the guide.

- No tab bar is drawn, in the window or in the terminal. A tab is said in words ("Documents, tab 2 of 3") and the tab list shows them all. Nothing about tabs is said while a panel has one tab.
- A locked tab that is alone in its panel shows no lock in the title line, since nothing about tabs is said with one tab. Lock tab says the state when you press it.
- A plugin enabled or disabled while tabs are parked updates only the shown tabs' plugin columns, to keep a tab switch cheap. Other column changes reach every tab, parked and closed ones too.
- A tab you are not in does not watch its folder. It lists the folder again when you come back to it.
- Undo close tab holds the last 16 closed tabs, for this session only. No command deletes or renames a saved tab set; a set is replaced by saving under its name again.
- A tab set and a session keep each tab's folder, cursor row, sort, hidden files, history and lock. Marks, the filter and the columns are not kept.
- Going back to alpha 4 loses the tabs. The session file's version is 2 in alpha 5, with each panel's tabs beside the two panels. Alpha 4 reads the panels as before and shows one folder in each, and when it writes the file at its quit, the other tabs are gone. Saved tab sets in `tab-sets.toml` are a file alpha 4 does not read.
- The tests this page was written from open folders on the local disk. A tab on an SFTP place, inside an archive or on a plugin's list has not been tried. Nothing about tabs has been heard on NVDA, JAWS or Orca, or read on a Braille display.

## Compare folders and synchronize

This section is for alpha 5, which has not been released. [Compare files](compare.md#compare-two-folders) and [Synchronize](synchronize.md) are the guides.

- The compare and the run start at once. They do not wait in the transfer queue behind other jobs. Compare progress is spoken but does not update the Jobs list's line.
- "By contents" is covered by the engine's tests and has not been tried by hand on a large tree.
- Copy toward this panel and Copy toward the other panel have no default keys, since no pair is free in all five presets. They are in the row's list of changes and the palette.
- The stale result ("Result is old: folders changed") is noticed only for changes the panels' own folder watchers report. A step whose file changed since the compare is skipped in any case, and says so: "changed since the compare; compare again".
- Nothing is replaced or removed for good from a shell: a place with no trash is refused.
- No rsync engine, no saved synchronize tasks, no Save as script and no compare of an archive by its stored checksums.
- A compare on an SFTP place, inside an archive or on a plugin's list has not been tried. Nothing here has been heard on NVDA, JAWS or Orca, or read on a Braille display.

## Duplicates

This section is for alpha 5, which has not been released. [Duplicates](duplicates.md) is the guide.

- Duplicates go to the trash only. Nothing is deleted for good, and a group always keeps one copy.
- No search inside archives, and no cache of hashes: a second search reads the files again.
- "Keep the copies in FOLDER" matches the files directly in that folder, not in its subfolders.
- A collection is not a place to look in yet. The scopes are the marked entries, this folder and both panels.
- Pause in the Jobs list does not reach the search. Cancel does.
- The marks are kept in the open dialog and are gone after Escape from the groups list.
- Yes to the large-read question runs the listing and the quick check again.
- Nothing here has been tried on a real trash, with a hidden file on Windows, or on a remote place.

## Batch rename

This section is for alpha 5, which has not been released. [Batch rename](rename.md) is the guide.

- Details placeholders, such as `{meta:exif.date}`, were cut from this alpha. They are not in the template's list of placeholders, and the app feeds them no values, so a template typed by hand with one makes each row a problem: "Problem, no value for {meta:exif.date}". Dates and sizes from the file itself work.
- The rules that need a Unicode table are not built: turning accented letters into plain ones, and changing a Unicode form.
- The commands in a batch, Next problem, Previous problem, Say both names, Make names unique and the rest, have no keys. Each is reached by Enter on a row.
- The question "Rename 3 files in photos? No is the default. y or n" has its meaning in the first 25 cells, but "No is the default" still ends at cell 43, past a 40-cell line. The window asks "Done editing?" before the editor starts, and does not say "Waiting for your editor. Escape cancels."
- The plan is made again after every change, on the app's thread. It is fine for hundreds of files and has not been timed for 10,000.
- Back from an editor in a terminal, diroxide says "Back in diroxide, photos", and an open "Done editing?" question is not said again. No Windows editor other than the default has been tried.
- `diro rename` has no `--format`, `--accept-care`, `--skip-problems`, `--sort`, `--undo` or `--edit` yet.
- Nothing here has been heard on NVDA, JAWS or Orca, or read on a Braille display.

## Lists that are not folders

This section is for alpha 5, which has not been released. [Flat view, collections and recent files](collections.md) is the guide.

- Keep folders, in the question for Copy and Move out of a flat view, works for a Move one folder level down. Copy says "Keep folders is for Move only, for now." and deeper folders say "Keep folders: one folder level, for now." Both need an engine planner that copies into several folders and makes missing parent folders more than one level deep. A Keep-folders move goes through the job's own question as well.
- A flat view's rows read "a.txt, in work/b/deep", the way the lister gives them, and its settings are not remembered from one time to the next.
- A collection's items are all checked when it opens, which is slow for a very long one.
- `diro -` opens its list in the app on a terminal, and prints it as lines when its output is redirected. It does not handle `--cwd-file`. After it opens, the panel's arrival line may be said after the list's start line.
- On macOS, `diro -` needs crossterm's `use-dev-tty` feature to read keys with a pipe on standard input; that is a manifest line that is not set yet.
- Nothing here has been heard on NVDA, JAWS or Orca, or read on a Braille display.

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
- In the Explorer and Finder preset, Control with 1 to 7 sets the favorite labels, and macOS may keep those keys for Mission Control. That has not been checked, because the GUI has not been tried on a Mac. The Labels submenu of the Edit menu is the sure way.
- A key that a plugin or a script proposes binds only when it is free and has no concern. The Keys list and Manage plugins both say what happened to it.

## The trash

The trash is the default way to delete, and the places where it cannot be trusted are handled by asking you instead of guessing.

- Windows: the trash is used only on a fixed local drive, for a path of 259 characters or fewer, and never on a network path. The Windows trash call can remove a file for good and still report success when a drive is removable or a path is 260 characters or longer. diroxide therefore decides before the call. Everywhere else it offers the listed permanent delete, and the question starts on No: "This drive has no trash. Delete 3 items for good? No is the default. y or n".
- Windows: a folder is refused for the trash when any path inside it is 260 characters or longer.
- Windows: after every trash call diroxide checks that the original is gone and that the trash holds it. If not, it says so loudly: "Deleted but not found in the trash."
- Windows: a very large item may be bigger than the Recycle Bin's size limit. This case has not been tested yet, because testing it could push your oldest recycled items out of the bin.
- macOS: diroxide cannot restore items from the trash, so Undo cannot bring back a trashed item there. Restore it from the Trash in the Finder. The trash browser cannot list it either.
- Linux: the trash follows the freedesktop standard. A drive with no trash of its own may mean copying the item into your home trash first.
- Permanent delete cannot be undone, and it cannot erase data for good: "Deleted data may stay on the disk. diroxide cannot erase it for good."

## Volumes and the trash browser

[Volumes and the trash](volumes-trash.md) describes both lists. These are the limits.

- Both lists open over the panels, as dialogs, not inside one, unlike a plugin's list.
- Neither list has terminal tests yet. Their tests run the program's logic over made-up volumes and a test-only trash, and never drew the screen. No test touches your real trash or your real drives.
- Unmount, Eject and Mount, done by the Volume actions plugin, were tried only against stand-ins for `udisksctl`, `diskutil` and PowerShell. The words those tools use for a busy volume were written from typical text and not captured, so a real busy volume may say "the system refused" where the plugin means "files are open". Nothing is forced either way.
- On Windows, Eject was run only as far as PowerShell starting with no console window and saying that it does not list a drive that is not there. Ejecting a real stick, idle and with a file open, has not been tried. The name of the shell's Eject verb has not been tried on Windows in other languages.
- The Volume actions plugin needs Python 3, and on Windows the `py` launcher. It declares a preview that says what it would run, and the program offers no preview yet.
- Devices that are not mounted are listed only on Linux and macOS, and only while the Volume actions plugin is turned on.
- A volume has 2 seconds to say its free space. A network share that answers in 3 seconds still says "free space not known".
- Hide changes only the Volumes list. Go to a place still lists a hidden drive in its older words. To list a hidden volume again, take its entry out of `hidden_volumes` in `config.toml`; editing that list in the Settings field has not been tried.
- Go to a place reads the volumes once a session, so a local volume has no free space there until you open the Volumes list, and a stick plugged in later shows in its older words until you do.
- Windows: a restore checks that the name is free just before it puts an item back. A file made in that same instant could be replaced. On Linux and the other freedesktop systems that cannot happen.
- A restore puts an item back only where it came from. A taken name is left out and said, and is never given another name.
- A trash with tens of thousands of items has not been tried. The listing comes in batches of 500 and gives up after 30 seconds.

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
- An xz of more than 100,000 streams is refused, with no setting to raise it. Each stream is a whole compressed file, so such an xz is 100,000 files joined end to end, which no archiver writes. Before each read, diroxide checks the list of blocks at the end of every stream, and past 100,000 it says so rather than read the rest unchecked.
- An xz holds one file and no file name, so it takes its name from the archive. When you pack a file to an xz with a different name, the pack plan says what it will unpack as: "Unpacks as one: xz keeps no name".
- Debug builds of diroxide are slow on archives with a million entries. A release build opens one in a few seconds. This matters only to people who build it.
- A local file whose name holds an archive marker, such as `a.zip!b.txt`, is listed but has no place, so diroxide cannot act on it. The row is not yet flagged "name needs care".
- What diroxide writes was read back by other programs in the project's Linux container: 7-Zip 25.01, GNU tar 1.35, xz, unzip, gzip, bzip2 and zip. On Windows, bsdtar and unzip read it. Other versions and other archivers have not been tried.
- Archives read through 7-Zip: a listing runs without `-ba`, so a member's "Read 3.2 GB first" question counts the bytes ahead only when the archive is solid. A hostile member name with a line break could add a fake row to the listing, though a name that climbs out of the archive is still refused. diroxide's 7-Zip runs set `LANG=C.UTF-8`, and a test lists a cpio member named with accented and Japanese letters under a C language in the project's Linux container with 7-Zip 25.01, so such a name is found there. Members over 64 MiB are unpacked alone into the spool first.
- Archives read through 7-Zip, tried: 7-Zip 25.01 in the project's Linux container listed, read and unpacked every format. Not tried: macOS, p7zip, a network place, 7-Zip 26.05, a RAR 5 with a password, a real compressed RAR file from WinRAR, and a separate program holding an archive open. Debian's 7-Zip 25.01 read a RAR made for the test, though a Debian package is said to leave RAR out.
- Editing inside a zip rewrites the whole zip, so a big one takes time: 1.2 GB of made content took about 52 seconds. A rewrite loses zip comments and extra fields and redoes the compression, with stored members kept stored. An encrypted zip needs its password to read and to write, and ZipCrypto members become AES-256.
- A zip opened by its bytes with Open as archive, such as a `.docx`, is not a zip by its name, so the File menu hides Make folder, Rename, Move to trash and Delete for good there. Their keys and the palette reach them.
- The old copy goes to the trash after the new zip passes its test. The trash step was tested on a test trash, not on the real Windows Recycle Bin. Where there is no trash the old copy is not kept, and the line says so.

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
- An Org file's row in the panel says "Org", and a reStructuredText file's says "RST", because the file type table has entries for them. A MediaWiki, DokuWiki or Jira file's row says "file", because it has none. The quick view reads all five by their extensions.
- The quick view wraps by counting characters, so a wide character counts as one.
- A reader that runs past its deadline cannot be stopped, so it is left to finish, and at most three run at once. A deeply nested document can be cut off at the deadline: "Could not read big.org: took too long." An Org file nested 3,000 deep still takes 39 seconds to read under carta 0.0.13, in a release build, so the deadline guard stays. Content nested past 256 elements reads as one row.
- Raw HTML in Markdown keeps its text and drops its tags. A front-matter title or an Org `#+TITLE` becomes the document's title, not a heading row.
- Open in textweaver hands over files on this computer only, not a file inside an archive.

## Tools and diro doctor

[Tools and diro doctor](tools.md) describes the registry, the Tools list and `diro doctor`, and [optional components](components.md) the helper programs. These are the limits.

- Nothing here has been heard on a screen reader or read on a Braille display. The "Install:" lines and the "Built in:" line pass 40 cells, so they pan on a Braille display, the name first.
- Several install commands were written from each package manager's usual names and not checked against a live one: pacman's `pandoc-cli`, several of the dnf names, the Gpg4win folders on Windows, and winget's Poppler id, which is a community build. There is no winget package for xorriso, minisign or wget.
- macOS has not been run. The Homebrew folders and `mdls` were never compiled by the lane that wrote the lookup. System32's `convert.exe` and `tw --version` were left alone on purpose.
- The registry's tests use stand-in programs. The version commands were run against the real programs in the project's Linux container and on one Windows computer.
- `7za` and `7zr` read no RAR, so the lookup tries `7z` and `7zz` first.
- The helper components were pinned by a job on the project's CI runner, 23 files with the publishers' own digests. No helper component has been installed and run on a computer yet.
- Some helper files are held: ExifTool for Windows, 7-Zip for Windows and FFmpeg for Linux, each for a reason [optional components](components.md) gives. ImageMagick on Linux and macOS, and FFmpeg on macOS, are package commands.
- After a helper is installed, the library runs its version command, and the program does not say the line yet: "pandoc: 3.13, from components" shows in `diro doctor`, and the end of an install says "Ready".
- The first-run list on Windows offers 7-Zip, which has no file for Windows yet, and gives its size as 0 bytes.

## File types and groups

[File types and groups](file-types.md) describes the model. These are the limits.

- 178 types come built in, not the hundred or so first planned. `tar` is typed by its name only, because its marker is past the first 64 bytes. `.ts` is TypeScript, and transport streams are `.mts` and `.m2ts`.
- Only a file's first 64 bytes are read. Of the zip-based files, EPUB, JAR and an APK whose manifest comes first are told apart by their head, and so are Office files whose own folder comes first. Office files that start with `[Content_Types].xml` read as "Office Open XML file", OpenDocument is cut short, XPS and CBZ are typed by name only, and an old-style APK reads as a JAR. With its real name, the more exact type wins.
- The contents speak first: a `.py` file that starts with `#!/bin/sh` is a shell script.
- MediaWiki, DokuWiki and Jira files have no entry in the table, so their rows say "file". Org and reStructuredText files have one.
- A type of yours that names a parent with an extension and no signature hides a child known by its own extension, because the tree reaches a child only through its parent. When two types of yours share an id, the first wins.
- The ten numbered user actions have no key until you bind one in `keymap.toml`.
- A theme's group colors show in the terminal only at 256 colors or true color, and not in the High Contrast theme.
- Open with on Windows does not read Explorer's own order, and a Store app, Edge or Chrome starts by its class through the shell without COM set up first. "Choose another program" hands the file to Windows' own dialog, which has not been tried. That dialog read well with NVDA when it was looked at, and JAWS has not been tried.
- Open with on Linux leaves out programs that run in a terminal, and gives a program's `%u` a path. On macOS a binary `Info.plist` is skipped and counted, the default program is not read, and the code was compiled only in the project's CI.
- The encoding chooser has Cyrillic IBM866 as its only DOS code page. The others need an encoding type of their own.
- Not tried: Linux natively, macOS at all, a network share, and starting any program from the Open with list.

## Previews

[Previews and the quick view's formats](previews.md) says what each preview reads. These are the limits.

- A TIFF's page count and a JPEG XL's size are not read. For raw and HEIF pictures the Exif data is looked for in the first 1 MiB and 16 MiB.
- RTF: bold and italic are not said, and deleted revisions are left out.
- A spreadsheet's formulas are not worked out again. A part nested deeper than 256 levels is read as flattened text, with a note.
- Preview and auto-preview gate the whole PDF preview in the quick view, its properties line included. A PDF off this computer gives its properties and no text, because Poppler's standard-input mode has not been tried.
- A picture that is not on a local disk goes to Tesseract on standard input, up to 64 MiB.
- The window's picture view is the Show picture command, which works only in the window. Stepping to the next picture in the viewer does not say the Enter hint again. The picture view has not been tried on a picture inside an archive or on an SFTP site, with a decode past its ten-second deadline, on a real Wayland compositor, or on NVDA or JAWS.
- F3 opens a picture, a PDF, a document, a sound or a video on its words, and a file no previewer reads in hex. A plain binary file, and a picture that no previewer takes, still open in hex: "big.bin looks binary; hex mode". The details list's Privacy and Checksum rows are not among the viewer's words.
- A PDF's pages as pictures, a HEIC, AVIF or Photoshop file through ImageMagick, and the viewer's PDF text through pdftotext have been run only through stand-ins in the tests, never with the real programs. pdftoppm is called with `-f N -l N -png -singlefile -scale-to 2048` and no output name, which relies on Poppler's build writing the PNG to standard output. Xpdf's build does not, and Git for Windows' pdftotext is Xpdf's, so pages need Poppler. The Poppler package that [Tools and diro doctor](tools.md) names for pdftotext holds pdftoppm too (on Windows winget `oschwartz10612.Poppler`, a community build); that page has no pdftoppm line of its own yet. Office documents are read by diroxide itself inside the viewer, and pandoc has not been run for it.
- From a panel, Show picture on a PDF says its page without the count: "Page 1". With the PDF's words open in the viewer, the count is said: "Page 1 of 3".
- Not tried: macOS, Linux natively, a real XLS, camera raw, HEIC or AVIF file, and a real Tesseract, ExifTool or ffprobe run inside a test. Nothing in the previews has been heard on a screen reader.

## The details view and access facts

[The details view](details.md) describes the list. These are the limits.

- Closing the list stops the read of a file's facts: between readers, at the next read, and an ffprobe run that the read started. Show all's ExifTool run is not stopped by closing the list. It finishes within its own time limit.
- Show all needs a file on a local disk. Folder sizes are not computed, and a folder has no Privacy row and no Checksum row.
- The Checksum row shows only the values the Checksum command computed in this session, while the file is as it was. Otherwise it says "Checksum: not computed".
- `diro info` knows the built-in file types and the built-in tools list, and does not read your `filetypes.toml` or `tools.toml`.
- Nothing about the order of rows, "Read 9 more details" or how Show all pans on a Braille display has been heard.
- Who can do what is read for the focused file or folder only, never for every row of a panel. "You can" is advice from the system's own check, not a promise. A macOS access list reads "Access list: not read on this system". The list's help says what "You can" and an access list's mask mean, and each row has no help of its own. Not tried: macOS and the BSDs, a network share, a system with SELinux on, a non-owner account on Windows, Explorer's Effective Access tab, a real FAT disk for time rounding, and the immutable flag set.

## Columns, notes and fields

[Columns, notes and fields](columns.md) describes them. These are the limits.

- A background column reads only the rows around the focus. A row elsewhere says "reading" until the focus comes near or Fill columns reads it.
- Sorting is by a cell's words, except where a field gives a sort value: the duration and dimensions columns sort as numbers in alpha 5.
- A file says "reading" for a column until the readers have been through it, even for a kind of file that no reader takes for that key, such as `audio.artist` on a zip. Its cell is then left out. The program does not yet know in advance which readers take a file.
- The window shows a field column in each row's words and has no header or grid for it.
- A saved view keeps the columns in use when it was saved.
- The reading of cells was measured with 100,000 entries and three columns, warm, on one machine. A cold disk and a network share were not tried.
- Not tried: the real git plugin's program started from the app, a real ExifTool, ffprobe or Tesseract run for a column, Linux natively and macOS.

## Checksums and verifying

[Checksums and verifying](checksums.md) describes them. These are the limits.

- A list's OpenPGP signature is checked with gpg, and its minisign signature is not checked by Verify checksum, which has no public key for it: it says "Cannot check: no public key given" and then "List not verified: 12 of 12 match". Verify signature checks a minisign signature with the key you give.
- With no gpg, a list's OpenPGP signature says "gpg not found. Install GnuPG first." and the list is "not verified". On Windows the check can be slow when gpg's agent has to start again.
- SHA-1 lines in an old list are read and said as not checked, because diroxide computes no SHA-1.
- A CRC-32 or MD5 match carries the cautions "CRC-32 is for damage, not tampering." and "MD5 is old, not safe against tampering.".
- The Checksum row of the details view holds this session's values only.

## Encryption and signatures

[Encryption and signatures](encryption.md) describes them. These are the limits.

- The three age commands need the Encryption plugin, an optional component. The plugin writes its files itself, and the program records them for Undo from the plugin's answer. That record lives for the session only, and does not survive a restart.
- ASCII armor is offered for age, and for gpg as the two commands "Encrypt with gpg, as text" and "Sign with gpg, as text", whose results are `.asc` text. The plain gpg commands still write `.gpg` or `.sig`.
- age encrypts with a passphrase or an age public key, and decrypts with a passphrase or an identity file. SSH keys, plugin keys and hardware keys are refused, and the age program is not used as a road around the plugin.
- On Windows, gpg's agent is stopped with each run, so a passphrase is not remembered from one command to the next, and a command that needs the agent waited about six seconds on the one computer it was measured on.
- A minisign public key is asked every time and kept nowhere. Verify checksum does not check a list's minisign signature.
- The commands are in the program. No dedicated `diro` command encrypts, decrypts, signs or checks a signature from a shell yet, and `diro run` has nobody to type a passphrase.
- Not tried: a real gpg program started by the program's own commands, a hardware token, a real signed download, Gpg4win, macOS, Linux natively and a screen reader. From the program, the age commands were tried only against a test plugin standing in for the real one, and the gpg commands only against a fake gpg. The plugin's own tests run the real age library, and gpg was run for the spikes in throwaway homes, through the tools registry.
- Nothing about how the dialogs and verdicts read on a Braille display has been heard.

## Privacy before sharing

[Privacy before sharing](before-sharing.md) describes them. These are the limits.

- Check before sharing is not a job you can cancel. The readers' own time limits bound it.
- Strip takes JPEG, PNG and WebP pictures only, up to 256 MiB. It does not crop or rename.
- A removed internet mark comes back with Ctrl+Z only in the same session, and only while no job or label change has come after it.
- The test for Remove the internet mark skips itself on a drive that keeps no extended attributes, so nothing is known there.

## Conversions

[Conversions](conversions.md) describes them. These are the limits.

- The app's jobs line uses the job engine's general progress words. "Converting 5 of 13 images" is said only by `diro convert`, on standard error.
- The Convert list types files by their names, and the plan types them by their contents.
- The program has no Save report command. `diro convert --report FILE` writes one, with names only.
- A plugin's converters are not in the Convert list yet. `diro plugin convert` runs one on a single file.
- A report keeps no hash of the files, and a plan names no tool version.
- carta reads UTF-8 text only, and a deeply nested Org file can take minutes. Nothing is made as PDF. Opus keeps no cover art, and percent progress for sound and video needs ffprobe.
- Not tried: macOS, Linux natively, HEIC, ImageMagick natively and a real network place for the spool.

## Watched folders

[Watched folders](watched-folders.md) describes them. These are the limits.

- A task's four folders, `in`, `out`, `processed` and `failed`, are made by you. The engine can make them, and nothing in the program or in `diro watch --task` asks it to yet. A missing folder pauses the task.
- A script task and a plugin task are built, and have not been run through the real script host or a real plugin. A plugin's command pauses the task.
- A task's filter is read by the program, and the count in the catch-up question ignores it.
- On Linux and macOS a writer that does not lock its file is seen only by its quiet period. On Windows diroxide looks again through a share-mode open and never locks a file in `in`.
- `online-only = "fetch"` downloads a cloud placeholder.
- `diro watch --task` answers the first start's catch-up question No when nobody is at the terminal. `--once` gives up on a file still changing after 2 minutes.
- The Watched folders list's "done today" counts come from each task's log, while Say Status counts this session's notes.
- A task cannot watch an existing folder such as Downloads in place. It watches the `in` folder of its home, and there is no key yet for a queue folder outside `home/in`.
- The form for making a task without writing the file comes in a later alpha.
- A hot folder was run for seconds in tests, never for hours. APFS, a network share, macOS, Linux natively and Windows long paths were not tried.

## Disk images

[Disk images](disk-images.md) describes the plugin. These are the limits.

- Images are read only. Writing to one, making one and formatting are a later alpha. Open as raw disk image, which would show a hybrid ISO's partitions, and a Disk image details command are not built, and a hybrid ISO opens as the ISO.
- A BitLocker or LUKS partition is shown and never opened. An encrypted image through 7-Zip says "it is encrypted, and this plugin asks no password".
- An NTFS file that NTFS compressed or encrypted is listed, and reading a compressed one says that only 7-Zip reads it. The plugin does not hand such a file to 7-Zip yet.
- A member of an image that 7-Zip must unpack, over about 6 MB, needs 7-Zip 25.01 or later, and a listing over about 7 MB from 7-Zip is refused. An older 7-Zip says "7-Zip 24.08 is too old: needs 25.01".
- The plugin keeps at most sixteen images open, because the program does not tell it when an image's last use ends.
- A plugin's own next step is replaced by "Try again; if it keeps failing, check the disk." for every refusal except unsupported and too large.
- Not tried: macOS, p7zip, UDF, HFS+, APFS and exFAT through 7-Zip (no image could be made), an ext4 image made by `mke2fs` (small ext2 and ext4 volumes written by the tests stand in), an NTFS-compressed file, and a password prompt. The tools image has no `e2fsprogs` or `exfatprogs` yet.

## Git

[Git](git.md) describes the plugin. These are the limits.

- Blame asks for the line number, because a plugin command is not given the viewer's cursor line.
- The title line can stay stale until the folder is listed again. A commit made in a terminal shows after the next reload, because nothing tells the plugin that a folder changed.
- Add to .gitignore gets the program's own question, "Replace .gitignore? It cannot be undone.", because the program has no way to append to a file for a plugin.
- Names that are not UTF-8 cannot be staged or compared, and non-UTF-8 text shows stand-ins in diffs.
- An untrusted repository's question appears when the panel enters it.
- Not built: a diff between any two revisions, a list of linked worktrees, and counts by kind in the status list's title.
- Git 2.30, the oldest it accepts, has not been tried. It was tried on 2.54.0 natively and 2.47.3 in the project's Linux container.
- Not tried: macOS, a real repository git refuses for dubious ownership (a refusal was faked), cancelling a running clone (a partial clone was faked), and any screen reader.

## Photos

[Photos](photos.md) describes them. These are the limits.

- A burst needs a camera and a date taken in the picture's own details. The Taken column says the date to the minute, and bursts read it to the second from the Exif block.
- Review's Label keeps the same picture, so one more key goes on.
- Move into month folders is planned by the program and run by the job engine, so a plugin's before-move hook does not run for it.
- The helpers read the pictures of one folder, never subfolders, and never draw a picture. A grid of thumbnails comes in a later alpha. They are in the program, with no dedicated `diro` command for a shell.
- Real camera files, a screen reader and a Braille display were not tried. The tests use small pictures made at test time, with dates and cameras written for the test.

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
- Access lists and flags such as immutable and locked are shown in the details view and are not edited. Extended attributes and a file's internet mark are read for the details view and said in words, and are not edited.
- Change owner on Windows hands the focused item to the system's Security dialog and does not wait for it. The call is one that Microsoft says may change. If Windows does not open the dialog you hear "Could not open the Security dialog: Windows did not open it. In File Explorer, open Properties, then Security." and do those steps by hand. The tests check the call's shape and open no window, so the dialog has not been seen or heard opening from diroxide, with NVDA or JAWS or without them.
- On macOS and the BSDs diroxide does not read your own groups. Change owner's Group row then says "Group: staff" with no word about whether it is one of yours, its group field completes every group, and the system decides when the change runs.
- Change owner never starts as an administrator, and it refuses the system's own folders. Putting an owner back can need an administrator.
- Change time cannot set the change time, which the system keeps, and sets a created time on Windows and macOS only. A typed date is read as your local time, in the form `2026-10-10 22:09`. A disk that rounds a time, such as a FAT disk, is read back after the job and said, and a real FAT disk was not tried.
- Change time and Change owner were not tried on a real network share, on macOS, with SELinux turned on, or with an account that is not the owner on Windows.
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

## The GUI

[The GUI](gui.md) describes the window. These are its limits in this alpha. None is dated: each is to come in a later alpha.

Opening it:

- `diro` by itself opens the window on a desktop, which is a change from the earlier alphas, where it opened the terminal. Open the terminal with `diro --tui`, or set `frontend` to `terminal`. Over SSH, with no display, with a pipe or a file, and in a build made without the window, a bare `diro` still opens the terminal.
- The window needs a display. With a graphics card it draws with it, and without one it draws on the processor. When every way of drawing fails, one sentence says why and the terminal opens. Without a display, a bare `diro` opens the terminal with no sentence, and `diro --gui` says why before the terminal opens.
- Drawing on the processor has been tried on Windows, and on Linux in a virtual X11 display. It has not been tried on Wayland or macOS. It is slower than a graphics card on a big window, and the accessibility tree is the same either way.
- A console that Windows opened for a shortcut can show for a moment before `diro` lets it go.
- On Linux without Fontconfig only the two bundled Atkinson families draw. A character they lack, such as a Chinese or Japanese name or an emoji, is drawn as a missing glyph. The names a screen reader reads are the full names.

What is said:

- Nothing in the window has been heard on NVDA, JAWS, Orca or VoiceOver, or read on a Braille display. Tests and reports read its accessibility tree through UI Automation on Windows and AT-SPI on Linux, with no screen reader. macOS has not been tried.
- The dialogs have been read only in the tests' harness. The UI Automation report cannot open a dialog without taking the foreground, so a real window's dialogs have not been read through UI Automation.
- Low messages are shown in the status line and not announced: "Top of the list", "Bottom of the list", "At the top already" and a running job's progress. Pressing Up on the first row is silent. Ctrl+D says where you are and how a job is going. Whether this is too quiet has not been decided by listening.
- A running job's progress is not spoken by the program. The "Job progress" indicator changes at the program's pace, and whether your screen reader speaks a progress bar's changes is its own setting. Nobody has listened to it.
- A dialog's name is said once, by the dialog, and the program leaves its own line for the same words out of the live region. Tests check that. It has not been heard.

Dialogs, the viewer and the clipboard:

- Every field except a password field opens an input method session. It has not been tried with a real input method. On Linux an input method might deliver typed text twice, once as keys and once as a commit.
- On Wayland, text the window copies reaches the clipboard through the window's own connection to the compositor, which has not been tried against a real compositor. Pasting into a field under Wayland with no XWayland still reads nothing, because the window toolkit's paste goes through the X11 clipboard. Copying works.
- A field that keeps a history changes between a text field and a combo box as its candidates open, so a screen reader may announce the change.
- F4 starts the editor directly, with no terminal. A console editor such as vi has no terminal to run in on Linux. Use an editor that has a window of its own.
- Starting a shell, a command or an editor on macOS has not been tried by a person.
- The viewer takes no mouse. A token class that a theme draws reversed is drawn underlined.

Colors and bars:

- The window draws the 24 built-in themes and your own theme files, which the program finds by name in the `themes` folder at its start. For a name it does not know it draws Galaxy and says "Unknown theme NAME. Galaxy is in use." once. A theme file's `[group-colors]` are drawn on the rows of plain files and archives in the window, and in the terminal at 256 colors or true color.
- Ten themes are below AA. The window draws the parts that fall short in the text color, and says how many. [Themes and colors](themes.md) names the ten. No italic face is built in, so a part that a theme draws in italics, such as the archive and hidden rows, may draw upright. Their words say what they are.
- The system's light or dark setting is read once, when the window starts. Windows' High Contrast is read at every step.
- Full screen and focus mode are commands, in the View menu and the palette, and F11 is Full screen's key. The panels' sort buttons and the native menu bar are not under focus mode's veil. A window opened with `--background` never goes full screen. In a terminal, Full screen changes nothing and says so, because full screen there is the terminal program's.
- There is no setting for the text size or the font. macOS shows no icon, and the mark's smallest sizes, 16 to 24 pixels, have not been looked at by a sighted person. Thumbnails are a setting that the reference marks as coming later.

Menus and the mouse:

- On macOS the menus are the list, as with `menus` set to `list`. The system's own menu bar there needs a binding that is not built.
- The native menu bar on Windows was read through UI Automation while it was shown. Entering it by key, choosing an item and its hiding again were not tried on a real desktop. The bar's Search menu shows its access key as "Alt+r", in lower case, where the other menus show upper case. The bar's Plugins menu is built from the same list as the list menus, and only its Manage plugins item is checked by a test.
- The mouse has few jobs: a click focuses a row, a double click opens it, the wheel scrolls, and the buttons press. There is no right-click menu (Shift+F10 opens the actions list) and no drag and drop.
- Keys the system keeps never arrive: Alt+F4 closes the window, and Alt+Space, Alt+Tab, Ctrl+Escape and Ctrl+Alt+Delete are the system's. Insert is the screen reader's, and chords of the Windows key are dropped. No default key uses one.

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
- The lines of the Volumes list, the trash browser, free space and the first-run list were written to carry their meaning in the first 40 cells and are checked in tests, not heard. That includes "Mounted: Backup, removable, 412 GB free", "Restore 3 items? No is the default. y or n", "Not enough room on build:" and "Optional components to get now". A volume's name and a plugin's lines may run past 40 cells.

- Nothing new in alpha 4 has been heard on NVDA, JAWS or Orca, or read on a Braille display: not the tools, the details view, the previews, columns, checksums, conversions, watched folders, disk images, git, encryption, access rows, Change time and owner, or the photo helpers. The lines were written to carry their meaning in the first 40 cells and are checked in tests, and the window's scripted walk checks the first words of the new lists the same way. Lines known to pass 40 cells with the meaning first include the Install lines of `diro doctor`, its Built in line, and "RAR is read only in diroxide".

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
- Listing a folder and reading each file's details cost more than they did in the build of October 16, 2026. On the bench machine, quiet, 100,000 entries list in 134 ms and are read for their details in 6.66 seconds, where that October code, run on the same machine, took 97 ms and 4.4 seconds. The cause is known: the details read opens each file a second time, and a listing builds every entry's full path, checks every name for an unfinished copy and reads the platform's attributes. Alpha 5 has removed both causes: a stat opens each entry once, and a listing builds an entry's full path only for a link. It was measured back to back with the old code on a busy machine only, so whether the quiet numbers are back to 97 ms and 4.4 seconds waits for the next saved baseline. [The bench](dev/bench.md) has the other numbers.
- How long before a copy is called stalled (20 seconds), and whether it keeps dates or checks a move, are built into the job engine. They are not settings. The number of tries on a lost network and the first wait are settings, in the Jobs and transfers group.

## See also

- [The accessibility statement](accessibility.md)
- [The GUI](gui.md)
- [Plugins](plugins.md)
- [Hooks](hooks.md)
- [Remote places over SFTP](remote-places.md)
- [The transfer queue](transfer-queue.md)
- [Volumes and the trash](volumes-trash.md)
- [Writing a plugin](writing-a-plugin.md)
- [Script commands](script-commands.md)
- [Optional components](components.md)
- [Security](security.md)
- [Archives](archives.md)
- [The viewer](viewer.md)
- [The quick view](quick-view.md)
- [Previews and the quick view's formats](previews.md)
- [The details view](details.md)
- [File types and groups](file-types.md)
- [Tools and diro doctor](tools.md)
- [Disk images](disk-images.md)
- [Git](git.md)
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
