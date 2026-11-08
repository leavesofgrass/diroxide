# What is new

This page says in plain words what each alpha added. [The changelog](../CHANGELOG.md) has the full list, by area, and [known limits](known-limits.md) says what is still missing.

## Alpha 4

What alpha 4 added, released on Sunday, November 8, 2026. The sections follow the order you meet things in: the helper programs, what a file is, what is in it, what you can do with it, and then the window, the plugins and the command line.

### Tools and optional components

- diroxide knows 22 helper programs, 23 on macOS: ImageMagick, FFmpeg, pandoc, 7-Zip, Tesseract, git and the rest. It looks for each in `tools.toml`, in the installed components, on PATH and in the folders the program's own installer uses, and says which step found it: "pandoc: 3.9.0.2, on PATH". A feature that needs a program says so, names it and says how to get it. Nothing runs an install command. [Tools and diro doctor](tools.md) is the new guide.
- The Tools list, in Settings, the Tools menu and the palette, shows each program and where it was found, or what it would download. `tools.toml` gives a program a path, turns it off, or marks it needed.
- Helper programs are optional components now. pandoc, 7-Zip, FFmpeg, ImageMagick and GitHub CLI come from their publishers, each pinned by size and checksum, where a file exists for your computer. Where none does, you hear the install command for your system. A few files are held for now, ExifTool for Windows among them. [Optional components](components.md) lists them.
- Four plugins are components too, in the release files: the SFTP places, disk image, git and Encryption plugins.

### File types and groups

- Every row names its file's type. The type comes from the name at once and from the file's first 64 bytes in the background, so a row can say "type pending" for a moment and then change quietly. When the two disagree, the details say so: "Name says PNG; contents say JPEG".
- 178 types come built in, in 8 groups: images, music, video, documents, archives, code, text and screenshots. A screenshot is in two groups. A file you add in `filetypes.toml` can be a type or a group of your own. The Groups list in the Select menu selects a group's files, Select by kind lists your groups, and `group:` in a panel's filter takes them. A search still goes by extension, with seven groups.
- A retro pack of 40 old formats is built in and off (`filetypes.retro`). `check.skip` names slow folders whose files are typed by their names alone.
- A type can name what Enter, View, Preview and Convert do, and ten numbered actions. Actions for this file, in the File menu, lists them. Open with lists the programs each system has for a file, and on Windows hands "Choose another program" to the system's own dialog.
- The viewer and the quick view have an encoding chooser: "Encoding of legacy.txt", the automatic choice first, then 29 encodings grouped by language.
- A theme file can give a group a color with `[group-colors]`, drawn in the window and in the terminal. [File types and groups](file-types.md) is the new guide.

### Previews and the details view

- The quick view reads more: pictures said in words from their headers, SVG drawings read as a screen reader reads them, sound and video through ffprobe, archive listings, PDF, Word, EPUB, OpenDocument, RTF, spreadsheets and delimited text, each with caps that keep a hostile file from stalling it. Nothing is played.
- Preview, in the View menu, runs the helper programs on the focused file at once. The setting `quick_view.auto_preview` decides whether they also run by themselves, and it is off in screen reader mode. Read text in picture has Tesseract read the words in a picture, as a job Cancel stops.
- Alt+Enter lists every fact about the focused entry in ten groups, the most important first. A file always has a Privacy row ("Privacy: nothing private found") and a Checksum row ("Checksum: not computed"). The facts read from inside the file join the list a moment later: "Read 9 more details". An archive says "Entries: 8 files, 2 folders" before its size. Closing the list stops the read, ffprobe included.
- Show all asks ExifTool for every tag. Say details, Copy details and Copy all details take a file's details without opening the list, and Copy details leaves the private rows out.
- F3 reads what the quick view reads. A picture opens the picture view in the window and its words in a terminal, and a PDF, a Word, EPUB or other document, a sound or a video opens the viewer on its words, titled "report.pdf, preview". Hex is one key away, H, and back. A PDF's pages show as pictures through pdftoppm, with Page Down and Page Up, and HEIC, AVIF and Photoshop files through ImageMagick. A missing helper is said in words, with its install line. The viewer's line numbers are L, the Line numbers item and `viewer.line_numbers`. [The viewer](viewer.md#pictures-and-documents) has the words.
- [Previews and the quick view's formats](previews.md) and [the details view](details.md) are the new guides.

### Columns, notes and fields

- A key such as `audio.artist`, `doc.pages` or `image.dimensions` in the `columns` setting adds a column read from inside the files. Each row says its cell after its label, "artist Ada Example", and a cell not read yet says so, "artist reading". Sound tags are read for the whole folder, other facts for the rows around the focus, and a few only when you ask with Fill columns. Nothing is read on a remote place, inside an archive or in a folder named in `check.skip`.
- Sorting by a column reads the files first, and above 200 files it asks, starting on No, with an estimate: "Read 12,000 files to sort by Artist? About 40 seconds."
- `[regex_columns]` makes a column from a pattern over the name. A plugin can fill a column too: the git plugin's Git column.
- Set note and Set field, in the Labels menu, keep your own words about a file in `labels.toml`, never in the file. A label, a note and a field now follow a file that diroxide moves or renames, and an undo of that.
- [Columns, notes and fields](columns.md) is the new guide.

### Checksums and privacy

- Checksum computes a file's SHA-256, with BLAKE3 in the same read, and lists the value to read, copy, compare with a pasted one, or save as `SHA256SUMS`, never over a file. A pasted value of another length computes that algorithm.
- Verify checksum finds the list beside a download, or checks every file a list names, the verdict first. A match tells you the file is not damaged, never that it is safe. When a signature sits beside the list it is checked too, and the verdict says whether the list is signed.
- Check before sharing says what a file carries that could say who made it, where or with what, and what a strip cannot reach. Strip metadata into a copy makes a `NAME-clean` copy of a JPEG, PNG or WebP picture without them, and Undo puts the copy in the trash. Remove the internet mark asks first, and Undo writes the mark back.
- [Checksums and verifying](checksums.md) and [Privacy before sharing](before-sharing.md) are the new guides.

### Conversions

- Convert lists the recipes that take the selected files, plans the conversion into the other panel's folder, and asks first, starting on No. 31 recipes come built in, through ImageMagick, FFmpeg, pandoc and pdftotext, and 11 need nothing installed because carta, a converter written in Rust, runs them. A conversion never replaces a file, its report is kept with the last 20, and Undo puts what it made in the trash.
- A file type can name a recipe for its Convert action, and your own recipes go in `conversions.toml`. [Conversions](conversions.md) is the new guide.

### Watched folders

- A folder can convert, move, copy or note each file that arrives, from a task file in the `tasks` folder. You hear what the task will do in words and say yes first, and again after any change. It never replaces or removes a file, and Ctrl+Z never undoes it.
- The Watched folders list, in the Tools menu, shows each task with what it did today, and turns it on or off, pauses it, previews it, shows its log, puts failed files back, says its rule and opens its file. A task says one polite line at a time, pauses with its reason, retries a file after 10 seconds, then 1 minute, then 10 minutes, and pauses after ten failures in a row for one cause.
- [Watched folders](watched-folders.md) is the new guide.

### Archives and disk images

- RAR, CAB, ARJ, LZH, RPM, cpio, ar, spanned zip sets and disk images no plugin takes are read through 7-Zip, read only. Unpacking to disk needs 7-Zip 25.01 or later, and an older one says so. diroxide never writes RAR. A member with accented or Japanese letters in its name is found under a C language setting too.
- Delete, Rename and Make folder work inside a zip. Each rewrites the zip beside the old one, tests it, and only then sends the old copy to the trash, so Undo restores it. The question says the cost and what a rewrite loses, starting on No. The File menu lists Make folder, Rename, Move to trash and Delete for good inside a zip. [Archives](archives.md) has the words.
- A disk image opens like a folder, read only: ISO, raw disks, VHD, VHDX, VDI, VMDK, QCOW2 and squashfs, with their partitions as folders, through the disk image plugin. A partition's row says its words after its name, "Partition 2, home, ext4". [Disk images](disk-images.md) is the guide.

### Encryption and signatures

- The File menu has an Encryption submenu (Alt+F, then Y). Encrypt with a passphrase, Encrypt for a recipient and Decrypt use age, through the Encryption plugin. Encrypt with gpg, Decrypt with gpg and Sign with gpg use your own GnuPG. A passphrase is typed in a hidden field, kept nowhere and wiped from memory when the job ends. Encryption never writes over a file and never removes an original: it asks about moving the original to the trash, starting on No, and says the trash is not secure. Undo says what it did: "Undid encrypt: 1 item is in the trash".
- Verify signature checks a signature beside a file, with gpg or minisign, the verdict first: "Signature good: key ending 8469", then "Good means the key fits, not who holds it." diroxide never fetches a key.
- Verify checksum checks the signature beside a list and says "List is signed: 12 of 12 files match", "List not signed: 12 of 12 match", or the signature's verdict and "List not verified: 12 of 12 match".
- [Encryption and signatures](encryption.md) is the new guide.

### Access and ownership

- The details view says who can do what with the focused file: "You can: read and write", then the owner, the group and the others, the mode, Linux access lists and flags, and on Windows the entries of the access list with where each comes from. "You can" is advice, and the list's help says so.
- Change time sets the modified, accessed or created time to now, to a date you type, or to the other panel's file's, said with its weekday, after asking, and Undo puts the old times back. Change owner changes the owner and group on Linux and macOS, never as an administrator. On Windows it opens the system's Security dialog for the focused item and does not wait for it.
- [Links, permissions and times](links-permissions.md) has both.

### Photos

- Four photo columns, `photo.taken`, `photo.camera`, `photo.pixels` and `photo.kind`, and Sort by date taken, with the modified time standing in where a picture has no date taken, and saying so.
- Next day and Previous day move through the dates. Select the screenshots, Select the bursts and Select pictures with no date mark them. Review pictures goes through them one key an answer, and Move into month folders files them under `2026-10` style folders, as one job with one Undo.
- [Photos](photos.md) is the new guide.

### The window

- The window has a picture view. Show picture, in the View menu and the palette, opens it, and Enter does the same on a picture in the viewer or the preview. Plus and Minus zoom, 0 fits and 1 shows the actual size. A terminal says "Pictures show in the window: diro --gui".
- Copied text now reaches the clipboard on Wayland. A paste under Wayland with no XWayland still reads nothing.
- The window reads your theme files, and says "Unknown theme NAME. Galaxy is in use." once for a name it does not know. The terminal draws a theme's group colors too, at 256 colors or true color.
- Every list and question of alpha 4 has a name of its own in the window, 112 dialog purposes in all, and the window's scripted walk checks the first words of the new ones within 40 cells. [Using diroxide with a screen reader](screen-readers.md) has the lines.

### The plugins

- A plugin can ask a form of several fields, show two texts in the compare screens, describe a folder for the title line, claim the disk images found inside other places, fill a column, and write an output through the host so Undo can put it in the trash. [Writing a plugin](writing-a-plugin.md) says how.
- The git plugin puts the branch and the number of changes in the title line and a Git column, lists the changed files, stages, commits, stashes, fetches, pulls and pushes, and shows the log, blame and diffs, and its questions start on No. A repository you did not make is asked about before git runs anything from its own configuration. [Git](git.md) is the guide.

### The command line

- `diro doctor` says what this `diro` has and finds, one fact a line, name first. Its exit codes are 0, 1 when a tool you need is missing, and 2 for wrong use.
- `diro sum` computes and checks checksums, with `--algorithm`, `--list` and `--check`, and `diro info` prints a file's details as plain lines.
- `diro convert` converts from a shell, with `--to`, `--recipe`, `--out`, `--conflicts`, `--recursive`, `--dry-run`, `--report`, `--json`, `--print0` and `--list`, and the family's exit codes, 0 to 3.
- `diro watch --task NAME` runs a watched folder's task from a shell, with `--once` for a catch-up scan that a scheduler can run, `--dry-run` and `--list`.

### For people who build diroxide

- `cargo xtask demo-tools PATH` writes the demo set of the alpha 4 check, the same bytes every run, with a steps file. [Building](dev/building.md) says what it makes.
- The nightly fuzz run covers all five fuzz folders, 39 targets, and the bench has a baseline of its own. Six tests that failed now and then under load were made steadier. [Fuzzing](dev/fuzzing.md) and [the bench](dev/bench.md) say more.

## Alpha 3

What alpha 3 added, released on Tuesday, November 3, 2026.

The GUI, a window version of diroxide.

- The window is built, and on a desktop it is what `diro` opens. Typing `diro` opens a window with the two panels, the status bar and the key bar, where the earlier alphas opened the terminal. `diro --tui` opens the terminal, `diro --gui` asks for the window outright, and the setting Opens in (`frontend`) set to `terminal` makes a bare `diro` open the terminal. Over SSH, with no display, with a pipe or a file, and in a build made without the window, `diro` still opens the terminal. [The GUI](gui.md) is the new guide: how the window opens, its parts in reading order, its keys and menus, what it says and when, and what it does not do yet.
- Each panel is one list, and each row is an option that a screen reader reads name first, with "selected" as a state and the position as a property. The focused row is what a screen reader follows. The title line, the two panels' sort buttons, the status bar with its two lines and the key bar are named parts of the window.
- Messages go to a live region by importance. Normal is said politely, high is said at once, and low is only shown. The program's start-up lines are held until a screen reader asks for the window, then said once.
- On Windows the menus are the system's menu bar. It stays hidden until Alt pressed alone, F9, F10 or Alt with a menu's letter shows it. On Linux and macOS the menus are lists in the window.
- New settings: Opens in (`frontend`), GUI graphics (`graphics`) and GUI menus (`menus`). Hide the menu bar (`auto_hide_menu`), which did nothing before, now works on Windows. New options: `diro --graphics NAME`, `diro --background` and `diro --log FILE`. When you ask for the window and there is no display, or when every way of drawing fails, one sentence is said and the terminal opens.
- The key bar and each panel's sort buttons are buttons with names and keyboard shortcuts. A click focuses a row, and a double click opens it.
- The fonts are inside the program: Atkinson Hyperlegible Next and Atkinson Hyperlegible Mono. [Installing diroxide](install.md#what-the-gui-needs) says what the window needs and what it costs to build.
- Reports read the window's accessibility tree through UI Automation on Windows and AT-SPI on Linux, with no screen reader. Nobody has listened to the window yet.
- Not in the window yet: an icon on macOS, your own theme files, and a native menu bar on macOS.

The window's dialogs, viewer and quick view.

- A confirmation is two real buttons, No first, with the focus on No, and a list to read of what it touches: "Listing, 3 lines". A question is named by its first line, and a long message is a list read line by line: "Message, 11 lines". A password has a check box, "Show the password" (Alt+S). A dialog's name is said once. Every dialog the program can open, 77 of them, is drawn and tested. [The GUI](gui.md#the-dialogs) has the shapes.
- Fields with completion are combo boxes, a paste types its first line, and every field except a password field takes an input method's text. Copied text, such as a file's path, goes to the system clipboard.
- The viewer (F3) takes the panels' place as one list of the rows on screen, in text or hex, and a file of 100,000 lines is a screen of nodes. The quick view (Shift+F3) is the other panel's list, "Right panel, Preview, notes.md".
- A running job shows a progress indicator, "Job progress", that moves with the jobs line and is not spoken by the program. Escape stops the job in front. The Jobs list works in the window, with Pause, Held rows, and Retry and Forget on the Interrupted group.
- Change key in the Keys list takes a real key press, such as Ctrl+Shift+K, and saves it to `keymap.toml`.
- Shell here, Run a command and F4 start the system's terminal or the editor in a window of their own, and the window goes on: "Started a shell in work", "Started the editor on notes.txt". There is no "exit the shell" line in the window.
- The toolkit's helper window on Windows is named "diroxide background helper".

Themes, drawing without a graphics card, and focus mode.

- The window draws all 24 themes, your `[colors]`, and Windows' High Contrast, and it can follow the system's light or dark setting. A theme chosen, in Settings or with Next theme, is said once with its contrast verdict: "Theme Galaxy Light, meets AA" or "Theme Nord, below AA: 1 check falls short". Ten themes fall short of AA. The window draws the parts that fall short in the text color and says how many. `diro --theme NAME` works in the window. [Themes and colors](themes.md) is the new guide.
- Your own theme files are found by name. The program reads the theme files in your `themes` folder when it starts: the Settings theme chooser lists them after the built-in themes, Next theme steps through them, and the verdict is said. The terminal draws them, as before. The window does not read them yet, and draws Galaxy for a name it does not know.
- A graphics card is no longer needed. With none, or only a software adapter, the window draws on the processor itself, and `--graphics software` asks for that. The window says once, "Drawing on the processor: no graphics adapter was found." `diro --log FILE` records how the window drew. [The GUI](gui.md#how-the-window-draws) has the order.
- Focus mode and full screen fade the title line, the status bar and the key bar after three seconds of rest. Any key brings them back. The fade says nothing. Both are commands now, in the View menu (Full screen, letter U, on F11; Focus mode, letter M) and the palette, and each says what it did: "Full screen on.", "Full screen off.", "Focus mode on: the bars fade after three seconds." and "Focus mode off." The Settings switch says the same words. In a terminal, Full screen says "Full screen is the window's; the terminal program may have its own." and changes nothing, and Focus mode says "Focus mode on, for the window; the terminal keeps its bars."

The icon.

- diroxide has an icon, its mark. On Windows, Explorer shows it for `diro.exe`, and the taskbar and the title bar show it on the window. On Linux, the AppImage carries it in every size a desktop's icon theme asks for, and in two menu entries: "diroxide", which opens the window, and "diroxide in a terminal", which opens the terminal. macOS shows no icon yet. [The GUI](gui.md#the-look) says more.

A quit keeps a copy's part file.

- A quit now stops a running copy the way a lost connection does. A copy of 4 MiB or more on this computer's disks keeps its part file, and Retry on its Interrupted row at the next start goes on from it: "Resuming big.bin at 2.1 MB of 5.2 MB". Cancel still removes the part file. [The transfer queue](transfer-queue.md#the-queue-at-the-next-start) has the details.
- The program now has a state word for a held job, so a lone held job says what it holds: "Moving to the trash is held in line". The first start in the window says "The key bar names the keys."

Plugin lists inside a panel.

- A list of rows that are not files, such as the changed files of a git folder or the Sites list, opens inside the panel the plugin names, over the panel's folder, instead of over both panels. The focus moves to that panel. You hear the list's name, its count and the panel first, with the plugin's name last: "Changed files, 2 rows, left panel (Changed files)".
- The panel's keys work on the list's rows. A file key such as F5 says "This row is not a file." Left, Backspace and Alt+Up leave the list and say "Plugin list closed (Changed files)", and then the folder again. [The panels](panels.md#plugin-lists) has the rest.

Pause for waiting jobs, and the queue across restarts.

- Pause on a job that waits its turn holds it in its place: "Held: copy 12 items to build". The jobs behind it can start. Resume lets it go: "Back in line: copy 12 items to build". Pause all jobs holds every waiting job, and Resume all jobs lets them go.
- The queue is kept across a restart, in `queue.json` in the state folder. The next start says once, "2 jobs were interrupted by the last quit. The Jobs list has them.", and the Jobs list shows them first, under "Interrupted, 2 jobs". None of them starts on its own. Enter on one offers Retry and Forget. [The transfer queue](transfer-queue.md#the-queue-at-the-next-start) has the details.

The nightly fuzz run.

- Every night the project feeds random input to the code that reads what other people make, for ten minutes on each of 17 targets: names, filters, archives, the highlighter, the components list, plugin messages, script headers and the SFTP plugin's packets. A crash fails the run and saves the input that caused it. This changes nothing in the program you run. It looks for bugs before you meet them. [Fuzzing](dev/fuzzing.md) is in the contributor pages.

## Alpha 2

Plugins, script commands and optional components.

- Script commands. A one-file script in bash, Python, PowerShell or a batch file becomes a command by adding a few comment lines at the top. It gets the selection in a list that holds any name, tells diroxide what to say and what to do next, and ends with an exit code that diroxide says in words. [Script commands](script-commands.md) has the details.
- Plugins. A plugin is a folder with a `plugin.toml` and, if it needs code, a program in any language that talks to diroxide in lines of JSON. It can add commands, lists of rows that are not files, and places. A plugin with no code of its own can run a program such as a diff tool or pandoc. [Plugins](plugins.md) and [writing a plugin](writing-a-plugin.md) explain both sides.
- Nothing runs until you say so. Turning a plugin on shows what it runs, what diroxide checks, and what the plugin only promises, and the answer starts on No. If the plugin's manifest or its program changes, it asks again. `diro --no-plugins` starts with every plugin off.
- A plugin cannot pass for diroxide. Everything it says ends with its name, so the meaning stays in the first 40 characters of a Braille line. [Security](security.md) says plainly what a plugin could still do.
- Optional components. Helper programs, plugins and data packs that diroxide does not ship are fetched only after you say yes, checked by size and SHA-256 before they are used, and removed by a list that shows every file first. [Optional components](components.md) describes it.
- In the program. The Plugins menu (Alt+P) lists Manage plugins and Get example plugins, then the commands of each plugin you turned on. A command runs as a job that Escape cancels. A plugin's questions and lists open as dialogs of diroxide's own, with its name last. The Tools menu has Manage optional components. [Plugins](plugins.md) walks through each screen.
- Ten example plugins, small on purpose: two bash scripts, a Python line counter, a search through the system's own index, a diff-tool opener, a changed-files list, a toy file system in memory, a PDF previewer, a Markdown to Word converter and a folder guard. `diro plugin new` starts your own from the same files.
- The command line. `diro plugin` lists, creates, checks, turns on and off, runs, restarts and tests plugins and scripts. `diro components` lists, downloads, verifies, removes and installs components. `diro run` runs one of diroxide's own commands, or an enabled plugin's, without the panels.
- Privacy. [The privacy page](privacy.md) now says what a plugin can see, and that the one request diroxide makes by itself, the download of a component, asks first and sends only the project's name.

Remote places over SFTP, the transfer queue and hooks.

- Remote places. A folder on another computer opens in a panel like any folder: type `sftp://alice@build.example/srv` in Go to a path, pick a saved site in Go to a place, or open the Sites list. diroxide uses the OpenSSH program on your computer, so your keys, your agent and your known hosts work as they are. [Remote places over SFTP](remote-places.md) is the guide.
- A new server key asks first, with the fingerprint in groups of four digits that a Braille display reads one row at a time. A changed key is refused with no way to accept it, and the command that forgets the old key is shown and never run.
- Saved sites keep a name, a user, a start folder and how the password is kept: never (the default), for this session, or in the system keyring after a question that starts on No. No password is ever in a file. A saved password is never given through a jump host.
- `diro-sftp doctor` is a plain report of which ssh, whether the agent runs and whether the password helper works, with no network.
- The transfer queue. Jobs on a server wait their turn, two at a time for one server. The Jobs list (Alt+T, then J) shows running, waiting and finished jobs, each row with its state first, and lets you move a job, run it now, pause it, cancel it or set a speed limit for it. [The transfer queue](transfer-queue.md) describes it.
- A server that stops answering is one question for all the jobs on it, with Keep waiting first. A copy cut by a lost network goes on from where it stopped when the part file checks out.
- Retries are settings now: three tries, waiting 5, 15 and 45 seconds, and a job can continue and list its errors instead of asking. A Jobs and transfers group in Settings holds seven settings. Quitting asks while a job waits.
- Hooks. A plugin can be asked before a copy, a move, a delete, a rename or a new folder, and stop it with a reason: "Delete stopped: folder is protected. By Folder guard." [Hooks](hooks.md) has the events and the example.
- A plugin's commands take keys from `keymap.toml`, and the Keys list shows them.
- Streamed listings. A big remote folder arrives in batches, says how far it has come, and Escape stops it and keeps what came.

Volumes, the trash and free space.

- The Volumes list (Alt+G, then V). Every drive, mount, connected storage and device is a row that puts its state first: "Mounted: Backup, removable, 412 GB free", "Not mounted: Backup, removable, 931 GB", "Connected: build, sftp, 1.2 TB free". Free space fills in as each volume answers, and a volume that does not answer in 2 seconds says "free space not known", never zero. [Volumes and the trash](volumes-trash.md) is the guide.
- Open, Mount, Unmount, Eject and Hide from a row. The Volume actions plugin, an eleventh example plugin, does the mounting and ejecting with the system's own tool, never with a force option. A busy volume stays where it is: "Could not eject Backup: files are open." Windows has Eject only. Without the plugin the actions are listed as unavailable, with the reason, and the example plugins are offered.
- Hide keeps a volume out of the list, and the Hidden volumes setting says which.
- The trash browser (Alt+G, then C). What is in the trash, newest first, each row its name first: "report.pdf, deleted Tuesday, D:\work". Space marks. Restore never writes over a name that is taken and says so. Delete for good and Empty the trash list every item first, start on No, and ask you to type the count over 1,000. macOS cannot list its trash and says so.
- Free space in the plan. A copy to a server with too little room says "Not enough room on build:" and starts on No. A server that cannot say, or is slow, says "Free space on build is not known." and the copy goes on. Say Status (Ctrl+D) ends with the free space of the place you are in.
- Go to a place has Site, Volume and Trash rows, each with its kind first. Enter on a site connects through the plugin.
- The SFTP places plugin is a component for Windows on x86-64, Linux on x86-64 and macOS, in the release files. A computer with no build hears "No build for this computer." and the `cargo install` line that builds it. [Optional components](components.md) has the details.
- The first-run list. At the first start, a list named "Optional components to get now" opens once with nothing chosen, when there is something besides the example plugins to choose. The setting Ask about components again, and Ask first-run choices again in the Tools menu, bring it back.

## Alpha 1

The first alpha is a terminal file manager for local folders and archives, built to be heard.

- Two panels, in three layouts, with every row putting its meaning in its first 40 characters.
- Copy, move, rename, the trash, Undo and jobs that list a plan before they write anything.
- Archives that open like folders, the viewer, the quick view, split and join, links and permissions.
- One filter language for the panel, select, search and saved searches, labels and bookmarks, and Compare files with the verdict first.
- A Keys list, six key presets, 24 themes, settings with help on every row, and a screen reader guide.
- A command line that prints plain lines.

## See also

- [Known limits](known-limits.md)
- [The GUI](gui.md)
- [Themes and colors](themes.md)
- [Tools and diro doctor](tools.md)
- [File types and groups](file-types.md)
- [The details view](details.md)
- [Previews and the quick view's formats](previews.md)
- [Columns, notes and fields](columns.md)
- [Checksums and verifying](checksums.md)
- [Privacy before sharing](before-sharing.md)
- [Conversions](conversions.md)
- [Watched folders](watched-folders.md)
- [Archives](archives.md)
- [Disk images](disk-images.md)
- [Git](git.md)
- [Quick start](quickstart.md)
- [Plugins](plugins.md)
- [Remote places over SFTP](remote-places.md)
- [The transfer queue](transfer-queue.md)
- [Volumes and the trash](volumes-trash.md)
- [Hooks](hooks.md)
- [Optional components](components.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
