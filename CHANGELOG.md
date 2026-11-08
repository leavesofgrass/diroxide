# Changelog

Notable changes to diroxide, newest first. Agents write under a scratch heading with their id; the orchestrator regroups the entries by area before each alpha. Versions follow the alpha series in the plan; nothing is released until the owner says so.

## Unreleased, to be 0.1.0-alpha.4

Alpha 4 teaches diroxide what a file is, what is in it and what can be done with it: a tools registry with `diro doctor` and helper programs as optional components, a file type model with groups, a details view with who can do what, richer previews, custom columns, notes and fields, checksums with signature checks, privacy checks, conversions, watched folders, archive formats read through 7-Zip and edited inside a zip, disk images and git as plugins, encryption and signatures, Change time and Change owner, the photo helpers, the window's picture view, and new commands in `diro`. It is the planned alpha.7. The heading, the date and the version are set when it is released.

### Tools and optional components

- 22 helper programs are known, 23 on macOS, found through `tools.toml`, the installed components, PATH and each program's installer folders, and said with the step that found them: "pandoc: 3.9.0.2, on PATH". A feature that needs one names it and says how to get it. Nothing runs an install command.
- The Tools list, in Settings, the Tools menu and the palette, shows each program, where it was found or what it would download. `tools.toml` sets a path, turns a program off or marks it needed.
- pandoc, 7-Zip, FFmpeg, ImageMagick and GitHub CLI are optional components, pinned by size and checksum where a publisher makes a file for the computer. ExifTool for Windows, 7-Zip for Windows and FFmpeg for Linux are held for now. Where no file exists the install command for the system is said. The pins come from a CI job that runs by hand and monthly, `cargo xtask components --pin`.
- The release files carry four plugin components: SFTP places, disk images, git and Encryption.
- `diro doctor` lists what this program has and finds, one fact a line, name first, and says what a missing tool would download. Exit codes 0, 1 for a needed tool missing, 2 for wrong use.

### File types and groups

- Every row names its file's type, from the name at once and from the first 64 bytes in the background ("type pending"), and says when they disagree: "Name says PNG; contents say JPEG". 178 types come built in, in 8 groups, plus a retro pack of 40 off by default (`filetypes.retro`) and `check.skip` for slow folders.
- `filetypes.toml` adds types and groups. A type can name what Enter, View, Preview and Convert do and ten numbered actions, through a command, a tool, a plugin's command or a conversion recipe. Actions for this file lists them.
- The Groups list, Select by kind and `group:` in the filter take the groups, yours included.
- Open with lists the programs each system has for a file, and on Windows hands "Choose another program" to the system's own dialog.
- The viewer and the quick view have an encoding chooser with 29 encodings.
- A theme file's `[group-colors]` color a group's rows, in the window and in the terminal at 256 colors or true color, never in High Contrast.

### Previews and the details view

- The quick view reads pictures in words from their headers behind a pixel guard, SVG as a screen reader exposes it, sound and video through ffprobe, archive listings, PDF, Word, EPUB, OpenDocument, RTF, spreadsheets and delimited text, each with caps against hostile files. Nothing is played.
- Preview runs the helper programs at once, `quick_view.auto_preview` decides whether they run by themselves (off in screen reader mode), and Read text in picture runs Tesseract as a job Cancel stops.
- Alt+Enter lists every fact in ten groups, most important first, with Privacy and Checksum rows for a file and the facts read from inside joining a moment later. Show all asks ExifTool. Say details, Copy details and Copy all details work without the list. Closing the list now stops the read, ffprobe included.
- The details say who can do what: "You can: read and write", the owner, group and others, the mode, Linux access lists, flags and SELinux labels, and on Windows the access list entries in the order Windows checks them. "You can" is advice.

### Columns, notes and fields

- Keys such as `audio.artist`, `doc.pages` and `image.dimensions` in `columns` add columns read from inside files, each cell said after its label and "reading" until read, in three loading modes, with Fill columns. `[regex_columns]` makes a column from a pattern over the name, and a plugin can fill one.
- Sorting by a column reads first, and above 200 files asks, starting on No, with an estimate.
- Set note and Set field keep your words in `labels.toml`. Labels, notes and fields follow a file diroxide moves or renames, and an undo of that.

### Checksums and privacy

- Checksum computes SHA-256 with BLAKE3 in the same read, to read, copy, compare with a pasted value or save as `SHA256SUMS`, never over a file.
- Verify checksum checks the list beside a download or a list itself, the verdict first, and never says "safe". A signature beside the list is checked, with the verdict "List is signed", "List not signed" or "List not verified".
- Check before sharing reports what a file carries about who, where and with what. Strip metadata makes a `NAME-clean` copy of a JPEG, PNG or WebP. Remove the internet mark asks first. Undo works for the last two.

### Conversions

- Convert lists recipes, plans into the other panel's folder, asks first starting on No, never replaces a file, keeps the last 20 reports and Undo trashes what it made. 31 recipes are built in, 11 of them through carta, a converter in Rust that needs nothing installed.
- A file type can name a recipe, and `conversions.toml` holds yours.

### Watched folders

- A task file in the `tasks` folder converts, moves, copies or notes each file that arrives in a folder. The task is read to you in words and approved first, and again after any change. It never replaces or removes a file and Ctrl+Z never undoes it.
- The Watched folders list turns a task on or off, pauses it, previews it, shows its log, puts failed files back and says its rule. A task retries after 10 seconds, 1 minute and 10 minutes, and pauses after ten failures in a row for one cause.

### Archives and disk images

- RAR, CAB, ARJ, LZH, RPM, cpio, ar, spanned zip sets and unclaimed disk images are read through 7-Zip, read only, needing 7-Zip 25.01 to unpack. diroxide never writes RAR. 7-Zip runs set `LANG=C.UTF-8` so member names with accents or Japanese letters are found.
- Delete, Rename and Make folder work inside a zip by rewriting it beside the old one, testing it, and sending the old copy to the trash. The question starts on No and says the cost. The File menu shows the four commands inside a zip through the new menu conditions `edits` and `removes`.
- The disk image plugin opens ISO, raw disks, VHD, VHDX, VDI, VMDK, QCOW2 and squashfs read only, with partitions as folders. A plugin's words about an entry now reach its row, Say Details and the details list ("About: Partition 2, home, ext4").

### Encryption and signatures

- The File menu's Encryption submenu (Alt+F, then Y) holds Encrypt with a passphrase, Encrypt for a recipient and Decrypt through the Encryption plugin (age), and Encrypt, Decrypt and Sign with gpg through your own GnuPG. Secrets are typed in hidden fields, kept nowhere and wiped when the job ends, and `ToolInput::Secret` carries them to 7-Zip and gpg. Nothing is written over a file or removes an original. Undo says "Undid encrypt: 1 item is in the trash".
- Verify signature checks a gpg or minisign signature, verdict first. diroxide never fetches a key.

### Access and ownership

- Change time sets the modified, accessed or created time to now, a typed date or the other panel's file's, with the weekday said, and Undo puts the old times back.
- Change owner changes owner and group on Linux and macOS, never as an administrator, refusing the system's folders. On Windows it opens the system's Security dialog for the focused item and does not wait for it.

### Photos

- Columns `photo.taken`, `photo.camera`, `photo.pixels` and `photo.kind`, Sort by date taken, Next day and Previous day, selecting screenshots, bursts and pictures with no date (`photos.burst_gap_seconds`), Review pictures one key an answer, and Move into month folders as one job with one Undo.

### The window

- A picture view: Show picture, Enter in the viewer or the preview, Plus and Minus, 0 and 1. Copied text reaches the clipboard on Wayland.
- The window reads your theme files and says "Unknown theme NAME. Galaxy is in use." once. Every new list and question has a name of its own, 112 dialog purposes in all, and the scripted walk checks the first 40 cells of the new ones.

### The plugins

- Protocol: `ui/form`, `ui/show_diff`, `place/describe`, `column/format`, `fs/stage` and `fs/commit` are served, and a `[[provider]]` table takes `extensions` and `wants`. A plugin's output committed through the host joins Undo.
- The git plugin shows the branch and changes in the title line and a Git column, and stages, commits, stashes, fetches, pulls, pushes and shows the log, blame and diffs, with questions that start on No and a trust question before git runs anything from a repository's own configuration.

### The command line

- `diro doctor`, `diro info`, `diro sum` (`--algorithm`, `--list`, `--check`), `diro convert` (`--to`, `--recipe`, `--out`, `--conflicts`, `--recursive`, `--dry-run`, `--report`, `--json`, `--print0`, `--list`) and `diro watch --task NAME` (`--once`, `--dry-run`, `--list`), with the family's exit codes.

### For contributors

- `cargo xtask demo-tools PATH` writes the alpha 4 demo set and its steps file. The nightly fuzz run covers 39 targets in five folders. The bench has a new baseline and found that listing and details reads cost more than in the build of October 16, 2026. Six load flakes were made steadier.

## 0.1.0-alpha.3

Alpha 3 adds the GUI: the same `diro` opens a window on a desktop, with each panel one list a screen reader reads name first, the dialogs as real controls, the twenty-four themes with their contrast verdicts said, native menus on Windows, focus mode and full screen, drawing on the processor where there is no graphics driver, and the icons from the mark; beside it, plugin lists inside a panel, Pause for waiting jobs and the queue kept across restarts, the nightly fuzz workflow and the two archive bugs it found, the Markdown reader's link loop fixed, and the bench baseline. It is the planned alpha.6. This entry was written on Tuesday, November 3, 2026.

The example plugins pack is published as `diro-examples-0.1.1`: its plugins ask for host 0.1.0-alpha.3, so the pack's bytes changed, and a file published under a component's tag is never replaced. The release workflow's components and publish jobs now take the components' definitions from the commit the run is on, so a dispatched run can republish a tag's program with a component's new version.

### The orchestrator: the Markdown reader's link loop

Written on Tuesday, November 3, 2026.

- A Markdown document with a link right after a heading inside a list item (`- #` on one line, `[x](y)` indented on the next) no longer ends the program. The document builder looped without end when a link began while no row was open, so the quick view's read of such a file ran until its thread's stack overflowed, which aborts the whole program. CI met it three times as a random text in the Markdown property test since October 23, 2026, and memory pressure was blamed; a probe over 200,000 generated texts found one in about 42,000, and 11 bytes of it were enough. The link now waits for its row, and a test reads five such texts.

### Lane A6: the app's third set of contract items

Written on Tuesday, November 3, 2026.

- Full screen is a command now, `view.full-screen`, in the View menu and the palette, with F11 in the GUI's keys. With the GUI's keys it says "Full screen on." and "Full screen off."; in a terminal it changes nothing and says "Full screen is the window's; the terminal program may have its own." F11 has left the keyboard page's list of keys kept for later alphas. Until the GUI's next pass wires the command, the window still turns full screen on and off with F11 itself, saying nothing.
- Focus mode is a command now, `view.focus-mode`, in the View menu and the palette. It turns the `focus_mode` setting on and off and writes it, and says "Focus mode on: the bars fade after three seconds." or "Focus mode off."; from a terminal, "Focus mode on, for the window; the terminal keeps its bars." The Settings list's switch says the same words.
- A theme set through the Settings list is said with its contrast verdict, the same line Next theme says, in place of "Theme: nord": "Theme Nord, below AA: 1 check falls short". Back to default says the default theme's line.
- The owner's theme files in the config folder's `themes` folder are found by name beside the built-in themes: the theme chooser lists them after the built-in ones, Next theme goes through them, and the verdict, in the Settings list's row and when one is set, is the file's own: "Theme Harbor, meets AA". The terminal already drew them. For the GUI, the app now hands over the themes it read and their folder, for its next pass to draw from.

### Lane A5: the app's second set of contract items

Written on Tuesday, November 3, 2026.

- A program that runs in a window of its own, as every program the GUI starts does, is now said in the app's own words once it has started: "Started a shell in work", "Started the command in work", "Started the editor on notes.txt", "Started textweaver". One that could not start says why and what to do: "Could not start a shell in work: no terminal program was found. Set the TERMINAL variable to a terminal program, such as xterm." With the GUI's keys, Shell here no longer says "exit the shell to come back" first, since nothing is handed over. The terminal's handover is unchanged: "Shell here: exit the shell to come back.", then "Back in diroxide, work".
- Job history still opens the Jobs list at its Finished group, and Show what failed on a finished job lists its items. The separate Job history list from alpha 1, which nothing could open since the Jobs list took its place, is gone.
- For the GUI, the app now gives what its next lane will wire: a dialog's row focused by its place and a preview's row by its id, for a click; the job in front's last progress report, for the progress indicator; a launcher that says it does not wait, for the handover above; and the open dialog's purpose by name, for the tests. The dependency check lets the GUI's tests, and only its tests, use the file system crate for a test trash and a provider double.

### Lane A4: the app's contract items from the merged lanes

Written on Tuesday, November 3, 2026.

- A copy cut by a quit now goes on where it stopped. Quit stops a running job the way a lost connection does, so a copy of 4 MiB or more on a place that keeps part files leaves its part file and its line in `partials.json`. At the next start, Retry on its Interrupted row resumes it when the source has not changed and the part file's first 64 KiB and last 1 MiB still match: "Resuming report.iso at 2.1 MB of 5.2 MB". Cancel still removes the part file. The quit's question keeps its words.
- A held job's state is the engine's own, "held in line"; the words are unchanged: "Held 2 of 5: copy 3 items to build", "Held: copy 3 items to build", "Back in line: copy 3 items to build", and a lone held job's "Copying is held in line".
- In the GUI, the first start says "The key bar names the keys.", since its key bar is a toolbar; the terminal still says "The key bar on the last line names the keys."
- For the GUI, the app now gives what its dialogs lane will wire: a way to move a plugin list's focus to the row a click or a screen reader's Focus names, while a plugin's or the server's `panel/set` still moves the folder's cursor beneath the list; a menu's rows with the enabled plugins' commands, as the list menus show them; the bank the key bar shows, chosen in one place; and the frontend, graphics and menus settings.

### Lane X: the archive readers' fuzz findings

Written on Tuesday, November 3, 2026.

- A 7z whose header gives its coders stream counts that add up past 64 bits is refused as damaged: "it is damaged: its header's stream counts are too large". A debug build stopped there with a panic, and a release build counted wrong. The 7z reader's other sums from the file stop at the largest number instead of wrapping.
- Every xz decoder, for a tar.xz or a plain .xz, first checks the list of blocks at the end of each stream, each time a read starts: a count of blocks the list has no bytes for is "it is damaged: its index is damaged", and one whose memory passes the decoder's limit is refused with that limit. Before, 25 bytes could make the decoder ask for 506 GB. An xz of more than 100,000 streams is refused: "it uses more than 100,000 xz streams, which is not supported".
- The check at open counts an xz's blocks against `archives.max_entries` before it keeps them, and keeps one number for a 7z header's sizes instead of a list.
- The two inputs the fuzz run found are tests, and the xz one is also a seed. The `archive_xz` and `archive_stream` targets decode an xz only after the checks the program makes, as the program does.


### Lane F: the nightly fuzz workflow

Written on Tuesday, November 3, 2026.

- Every fuzz target builds again, and all seventeen run ten minutes each every night with AddressSanitizer (`.github/workflows/fuzz.yml`); on Windows the targets in `fuzz/` build with `--no-include-main-msvc`. `docs/dev/fuzzing.md` says how to run one and where a crash input goes.

### Lane Q2: Pause for waiting jobs, and the queue across restarts

Written on Monday, November 2, 2026.

- Pause on a job that waits its turn, from its row in the Jobs list, holds it in its place in line: "Held: copy 12 items to build". The queue passes over it, so the jobs behind it can start, and it holds back none of them. Its row says "Held 3 of 5: copy 12 items to build", it is counted among the waiting, and a lone held job's line is "Copying is held in line". Resume, first in a held job's actions, lets it go in the place it kept: "Back in line: copy 12 items to build". It can still be moved, run now, cancelled, or given another error policy or speed limit.
- Pause all jobs holds every waiting job as well as pausing the queue and the running jobs; Resume all jobs lets every one of them go.
- The queue is kept across a restart. After every change the waiting, held and running jobs are written to `queue.json` in the state folder, beside `partials.json`, by a helper in the background, whole or not at all, and the file is removed when no job is left. A quit writes the queue as it stood before it stops the jobs; the quit's question keeps its words.
- The next start lists those jobs first in the Jobs list, under "Interrupted, 2 jobs", each row "Interrupted: copy 12 items to build", and starts none of them. It says once, at normal importance, "2 jobs were interrupted by the last quit. The Jobs list has them.", or "... when the last session ended." after a crash.
- An interrupted job offers Retry and Forget. Retry plans its request again as a new job, with the usual question; one that had started skips what is already there. The row leaves when the new job is confirmed. Forget, or Delete on the row, says "Forgotten: copy 12 items to build". Interrupted jobs stay listed until one of those, across restarts.
- A `queue.json` that does not read is said in words and renamed aside as `queue.json.bad` (then `.bad2` and on), never deleted. A write that fails is said once. A second window on the same state folder leaves the first one's queue alone.

### Lane L: plugin lists inside a panel

Written on Monday, November 2, 2026.

- A list of rows that are not files, which a plugin opens with `lister/open`, now opens inside the panel the plugin names, over the panel's folder, instead of over both panels. The focus moves to that panel, and the list is said meaning first with the plugin's name last: "Changed files, 2 rows, left panel (Changed files)". The folder beneath keeps its rows, marks, history and watch, its own listings stay quiet while the list is up, and the list is never saved.
- In a plugin list the panel's keys act on its rows: moving (more rows are asked for at the end), Space ("Marked", "Unmarked"), select all, none and invert, a letter to jump, Enter for the plugin's own command as a job, the plugin's action keys, the Sort commands over the list's columns, the filter field over every column, Ctrl+R, Ctrl+D, Say Other Panel and F1. The title line and the window title name the list.
- Every file command refuses with "This row is not a file." before a file is looked for, and so does a command that works on the folder beneath, such as Make folder, a label or Select by kind.
- Left, Backspace and Alt+Up leave the list; Escape leaves it when no job or list filter is there to cancel; Back and Forward only leave it, and act on the history at the next press. Leaving says "Plugin list closed (Changed files)", then the folder's own landing line, and tells the plugin with `lister/closed`.
- A place opened in the panel closes the list: a plugin's `panel/set` quietly, so the Sites list's Connect still lands in the start folder; Go to a path, Home or a place list with the closing line.
- `panel/get` and a command's context keep the folder beneath as the place, cursor and selection, and gain `list`: null, or the plugin list's plugin, id, title and count of rows (`active.list` and `other.list` in the context). `diro serve`'s `panel/changed` names a list that opens or closes. No method was added; the protocol reference is regenerated.

## 0.1.0-alpha.2

Alpha 2 adds plugins in any language, script commands, optional components, SFTP places over the machine's OpenSSH client, a transfer queue with retries, resume and a speed limit, hooks, the Volumes list and the trash browser. It is the planned alpha.4 and alpha.5 as one. This entry was written on Friday, October 23, 2026.

- The planned graphical interface is the GUI in every guide, message and keymap table, no longer the window. The keymap file's `[window.*]` tables are `[gui.*]`; the Keys list says "in the GUI".

### Plugins, script commands and optional components

- Script commands: a one-file script in bash, Python, PowerShell or a batch file becomes a command through a header of `diro:` comment lines. It gets the selection in a NUL-separated list that holds any name, the context as JSON and as `DIRO_` variables, a cancel file, a private scratch folder, and a directives file to tell diroxide what to say and do. Eight output modes, exit codes said in words, `exit_words`, placeholders for commands with no script, `--explain`, and the choice of program per system: Git's bash on Windows (WSL's launcher refused), `py -3` for Python (the Store stub refused), PowerShell 7 or Windows PowerShell 5.1 with a per-process policy override that the consent text says.
- Plugins: a folder with a `plugin.toml` and a program in any language that talks JSON lines on standard input and output. Commands, lists of rows that are not files, places served by a plugin, previewers and converters. The host has timeouts, an idle stop, a crash policy that turns a plugin off after three crashes in a minute, and a flood policy.
- Consent: nothing is on by default. The turn-on question lists what the plugin runs, what diroxide checks and what the plugin only promises, and starts on No. The record hashes the manifest and the program file, so a change asks again.
- Every line a plugin causes carries the plugin's name last, so the meaning stays in the first 40 cells and a plugin cannot pass for diroxide. A plugin's text is cleaned and rate-limited.
- File operations from a plugin run as jobs with the plan, the confirmation, the trash and Undo. The names a plugin can create follow the portable-name rules. Spool paths are confined to a folder diroxide makes. Interpreters and shells cannot be named as tools unless the plugin declares "any program".
- Ten example plugins under `examples/plugins`, all passing `diro plugin check` and the conformance checks: two bash scripts, a Python line counter, a search through the system's own index, a diff-tool opener, a changed-files list, a toy file system in memory, a PDF previewer, a Markdown to Word converter and a folder guard that stops deletes in folders you protect.
- `diro plugin`: list, new, enable, disable, run, restart, log, check, install, preview, convert and conformance. `diro plugin new` writes a skeleton copied from the examples, so they cannot drift. `diro run` runs one of diroxide's own commands, or an enabled plugin's, without the panels.
- Optional components: helper programs, plugins and data packs fetched only after a yes that starts on No, checked by size and SHA-256 before use, with the list built into each build. A built-in client, then `curl`, then `wget`, with one neutral User-Agent. Resume after Escape. Install from a zip file or a folder with no network. Unpacking with plain names, no links and a size ceiling, and a receipt so Remove takes away exactly what was written. `diro components` (list, download, verify, remove and install) and the `components_source` setting.
- The first component is the Example plugins pack, `diro-examples`.
- In the program: the Plugins menu (Alt+P) with Manage plugins and Get example plugins, then each enabled plugin's commands; Manage plugins with each plugin's question, commands and keys, settings and log (the last error first); a plugin's questions, text and lists of rows that are not files as dialogs with the plugin's name last, waiting behind a confirmation; a plugin's command as a job that Escape cancels; the Settings list's Plugin settings row; Manage optional components and Ask first-run choices again in the Tools menu. `diro serve` follows the plugins' session and sends `panel/changed` events.
- `diro --no-plugins` starts with every plugin off.

### Remote places over SFTP

- The SFTP places plugin (`plugins/diro-sftp`): folders on other computers open in a panel through the OpenSSH program on the computer. `sftp://user@host:port/path` in Go to a path, a Sites group in Go to a place, and a Sites list in the Plugins menu. The plugin makes no network call of its own; the only traffic is ssh's.
- ssh is found by full path only, never a bare name or the current folder, and the highest version wins, the Windows build on a tie. Below OpenSSH 8.4 the plugin works with keys and the agent only and says so. The `ssh_program` setting chooses another.
- Passwords, passphrases and the server's questions come through a helper that is the plugin's own program in a second mode. It proves itself to the plugin with a keyed hash over a per-connection token; the endpoint is a socket only the account can reach, or on Windows a named pipe only the account can open.
- A new server key asks first, starting on No, with the fingerprint in hexadecimal groups of four, four to a row, and the text as ssh printed it. A changed key is refused, with the `ssh-keygen -R` line shown and never run. The plugin never writes the known hosts file.
- Sites in `sites.toml` with a name, group, host or ssh alias, user, port, key file, start folders and a password mode: never (the default), this session, or the system keyring after a question that starts on No. A file that holds a password is refused whole. A saved password goes to ssh once per connection and only for ssh's own password question for that account; through a jump host or a proxy command nothing is filled in or remembered. Forget password asks first.
- Our own SFTP codec and request multiplexer, with names as bytes, the OpenSSH extensions where the server offers them (atomic rename, free space, server-side copy), up to 64 requests of 32 KiB in flight, and a fuzz target. Up to two ssh sessions for one server, so a listing never waits behind a long copy.
- Streamed listings: a big remote folder arrives in batches, the count is said at the progress interval, and Escape stops it and keeps what came.
- Delete over SFTP is permanent, with the full listing and a question that starts on No. A move to or from a server reads the copy back before the original goes. A server that cannot rename over a file, make links or report free space says so in words, with the cause first.
- `diro-sftp doctor` and the SFTP doctor command: one plain report, each fact first, with no network.
- Tested against a fake server and against OpenSSH's own server in a container.

### The transfer queue

- The Jobs list (Alt+T, then J): running, waiting and finished jobs under headings that give their counts, each row with its state first. Move to the front, Move up, Move down, Move to the end, Run now, Pause, Resume, Set a speed limit, an error policy for a waiting job, Cancel, Show what failed, and Retry the failed items. The queue as a whole can be paused.
- Remote jobs wait their turn: two at once for one server and four in all, by setting. Local jobs start at once. The jobs line puts the count first for several jobs: "3 jobs, 1 waiting: Copying 40 percent".
- One gate for each server. A server that stops answering is one fact: every job on it says once that it waits, the gate tries after 5, 15 and 45 seconds, and when the waits run out one question opens with Keep waiting first. The question waits behind any open dialog.
- Retries are settings: the number of tries (3) and the first wait (5 seconds, then three times longer each time, at most ten minutes), counted and reported at the end. An error policy per job: ask (the default), continue and list errors, or stop at the first.
- Resume. A copy of 4 MiB or more cut by a lost connection keeps its part file, recorded in `partials.json` in the state folder, and goes on from where it stopped when the source is unchanged and the first 64 KiB and last 1 MiB of the part file match. Otherwise it says why and starts again. Cancel removes the part file. A move re-reads the resumed prefix before anything is removed.
- A speed limit for one job or for every remote job, in plain units such as `500 KB`, honored between chunks so Escape is never held up.
- Quitting with unfinished jobs asks first: "2 jobs are not finished." and "Quit and stop them? No is the default. y or n".
- A Jobs and transfers group in Settings with seven settings: `jobs.per_host`, `jobs.remote`, `jobs.speed_limit_kb`, `jobs.speed_limit_scope`, `jobs.network_retries`, `jobs.network_retry_seconds` and `jobs.on_errors`. The retry count and the first wait were built-in values before.

### Hooks

- A plugin can be asked before a copy, a move, a delete, a rename or a new folder (`before_copy`, `before_move`, `before_delete`, `before_rename`, `before_mkdir`) and stop it with a reason, after the plan and before the question. It is told afterward (`after_*`) with the result of each item, and told when you enter a folder (`enter_folder`) or change the selection (`select`), debounced.
- A stop reads the reason first and the plugin's name last: "Delete stopped: folder is protected. By Folder guard." A hook that does not answer is never a quiet veto: the manifest's `on_timeout` decides, and diroxide says which, "Delete goes on: no answer. By Folder guard." or "Delete stopped: no answer. By Folder guard."
- A before hook runs on a thread of its own, with a time limit of 2 seconds unless the plugin sets another. A queued job runs its before hooks when it is made.
- The `folder-guard` example plugin, in Python with the standard library only, shows both kinds of hook.
- A plugin's commands take their keys from `keymap.toml` by the command's id, and the Keys list has a row for each.

### Volumes, the trash and free space

- The Volumes list (Go menu, then V; the palette; no default key; command `go.volumes`): one list over the local drives and mounts, each plugin's storages (`provider/volumes`) and the devices the Volume actions plugin reports. Every row puts its state first ("Mounted: Backup, removable, 412 GB free", "Not mounted: Backup, removable, 931 GB", "Connected: build, sftp, 1.2 TB free"). Free space is asked on worker threads with a limit of 2 seconds, and a volume that does not answer says "free space not known", never zero. Only the label, the size and the free space are asked, so a drive's media is never read. Ctrl+R reads the volumes again.
- Enter on a row offers Open, Mount, Unmount, Eject and Hide. Windows has Eject only. An action that needs the Volume actions plugin, when it is not turned on, is listed as unavailable with its reason ("Eject needs the Volume actions plugin."), and Enter on it offers the example plugins, asked first. Hide keeps the volume's device or place in the new setting Hidden volumes (`hidden_volumes`).
- The `volume-actions` example plugin (`examples/plugins/volume-actions`, the eleventh): Python with the standard library only. It runs `udisksctl` and `lsblk` on Linux, `diskutil` on macOS and the shell's Eject verb through PowerShell, with no console window, on Windows. Every tool runs by an argument list, never a shell and never with a force option, and a busy volume says "Could not eject Backup: files are open." and stays mounted. Its commands declare `dry_run`.
- The trash browser (Go menu, then C; the palette; the Trash row of Go to a place; commands `go.trash` and `trash.empty`): what the system trash holds, newest first, each row its name first ("report.pdf, deleted Tuesday, D:\work"). Space marks; Enter offers Restore, Delete for good and Empty the trash, each question starting on No. The local provider lists, restores and deletes from the trash on Windows and the freedesktop systems, on worker threads with a time limit; macOS says "The trash cannot be listed on this system." Two new job kinds, Restore and Purge. A restore never writes over a taken name: the plan leaves it out and says so, and the result counts it ("Restored 2 items. 1 name was taken."). Delete for good and Empty the trash list every item first and ask for the typed count over 1,000.
- Free space. A copy or a move to a server with too little room says "Not enough room on build:" and "Needs 4.0 GB, 1.2 GB free." and starts on No. A server that cannot say, or is slower than 2 seconds, gets "Free space on build is not known." and the copy goes on. Say Status ends with the free space of the place you are in ("412 GB free of 931 GB"), once the place has answered.
- Go to a place gains Site, Volume and Trash rows, each saying its kind first. Enter on a site connects through the plugin's own Connect and lands in the folder. Alt+N works in an empty Sites list, and the list refreshes after its commands and after a lost connection.
- Optional components: the release workflow builds `diro-sftp` for each target, and `cargo xtask components --assets FOLDER` adds the programs with their sizes and SHA-256 hashes to the list that is built into diro. Components are published under their own tags (`diro-examples-0.1.0`, `diro-sftp-0.1.0-alpha.1`). A computer with no build hears "No build for this computer." and the `cargo install` line that builds the plugin there. The program finds plugin components in their own folder under `components`.
- The first-run list of optional components is shown: at the first start, once, with nothing chosen, when the list holds something besides the example plugins. The new setting Ask about components again (`components_ask_again`) opens it at the next start; Ask first-run choices again in the Tools menu opens it now.

### The documentation

- New guide: [volumes and the trash](docs/volumes-trash.md). The [optional components](docs/components.md), [panels](docs/panels.md), [remote places](docs/remote-places.md), [settings](docs/settings.md), [known limits](docs/known-limits.md), [index](docs/README.md), [what is new](docs/whats-new.md) and [quick start](docs/quickstart.md) pages say what the Volumes list, the trash browser and the first-run list change. The generated [keyboard](docs/keyboard.md) and [settings reference](docs/settings-reference.md) pages were regenerated.
- New guides: [plugins](docs/plugins.md), [writing a plugin](docs/writing-a-plugin.md), [script commands](docs/script-commands.md) and [optional components](docs/components.md). New pages: [security](docs/security.md) and [what is new](docs/whats-new.md). The [privacy](docs/privacy.md), [known limits](docs/known-limits.md), [settings](docs/settings.md) and [index](docs/README.md) pages say what plugins and components change.
- New guides: [remote places over SFTP](docs/remote-places.md), [the transfer queue](docs/transfer-queue.md) and [hooks](docs/hooks.md). The [plugins](docs/plugins.md), [operations](docs/operations.md), [customizing keys](docs/customizing-keys.md), [security](docs/security.md), [privacy](docs/privacy.md), [known limits](docs/known-limits.md) and [settings](docs/settings.md) pages say what they change. Retries are settings now, and `operations.md` no longer calls them built in.

## 0.1.0-alpha.1

Alpha 1 is a terminal file manager for local folders and archives. This entry was written on Saturday, October 17, 2026. [The accessibility statement](docs/accessibility.md) says what has been checked by ear, and [known limits](docs/known-limits.md) lists what alpha 1 does not do.

### Two panels and the job engine

- Two panels in the terminal, in three layouts: side by side, stacked and single. Every row puts its meaning in its first 40 characters. The hardware cursor stays on the focused row's first cell in every state, so a screen reader and a Braille display follow it.
- Moving and finding your way: the arrow keys, type-ahead, path jump, go to a path or a place, folder history, and sorting with the Norton keys and a Sort list. Marking by space, by pattern, by range and by inverting, with the count said in words.
- Copy, move, rename, make folder, duplicate, the trash, a separate permanent delete, a file clipboard and Undo. Every delete lists what it will remove and starts on No.
- Jobs with a plan before anything is written, progress in words that changes slowly in screen-reader mode, and a conflict list for each name that is taken, with "apply to the rest".
- A job engine that creates, verifies and only then destroys. A replaced file goes to the trash first. The undo journal is written to disk before the first step, so Undo works after a restart. A lost network is retried, and a stalled copy is named as stalled.
- The trash is used only where it can be trusted. Where it cannot, the delete is listed and asks, starting on No. Long paths work at any depth.
- Settings, themes and menus: a config folder and a state folder (or `--portable`), a Settings list with help on every row, 24 themes that each report their contrast in words, menus and banks you can change, a command palette that finds any command by name, and a key bar on the last line. A settings reference is generated from the program's own list.
- A keyboard sheet for every key, generated from the keymap, with the keys terminals keep and the keys screen readers keep marked.

### Archives, the viewer, the quick view, split and join, links and permissions

- Archives open like folders: zip, 7z, tar, tar.gz, tar.xz and xz, and a split archive as one. Copy out, Unpack, Pack and Test archive are jobs with a plan, a question and Undo. Passwords go through a masked prompt that says what it is for. Limits protect you from an archive made to fill a drive, and a name that would leave the folder is skipped and said.
- The viewer on F3: lines, wrap, line numbers, jump, search, hex mode, following a growing log, large files, and code colored for nine languages, with commands that say each word's kind by ear.
- The quick view on Shift+F3: a live preview of the focused file in the other panel. Tab goes into it to move by line, heading, paragraph, table cell and link, and to search it. It reads Markdown, plain text, code and the formats of carta (Org, reStructuredText, MediaWiki, DokuWiki and Jira). Open in textweaver hands the file on.
- Split and join: seven built-in sizes (email, GitHub, CD, FAT32, DVD, Blu-ray and floppy), equal parts, custom sizes and your own presets; a checksum file written last; a join that names a gap and checks the result.
- Links, permissions and times: symbolic links, junctions and hard links; change a link's target in one step; permissions said in words before the octal number, with a recursive option and Windows attributes; copies, moves and unpacks keep modified times; a details list for any entry.

### Finding, remembering, comparing and keys

- One filter language for the panel filter, select by pattern, Search and saved searches: names, kinds, sizes, dates, labels and file contents. A filter that cannot be read is explained in two spoken parts, and Say filter reads one back in a sentence.
- Search: Find files with a results list of real files, content search, skips said as sentences, saved searches, the Space list (by kind, by age, largest items), folder sizes and the newest files.
- Labels in six colors that never use red or green, with favorite-label keys; bookmarks with alias letters; a History list of places; and a view saved for one folder.
- Compare files: the verdict first in 40 cells, then the counts, the direction and one row for each change. Nine cases that sound the same are named; binary and large files are handled; compare properties and select the differences.
- A Keys list that shows every command's key, Change key that listens for a key and checks it, a key test and a key probe, and six key presets: diroxide defaults, Total Commander, Midnight Commander, Explorer and Finder, Worker and vim-style, each with a generated sheet. Your changes go to `keymap.toml` without losing your comments.
- Watch this folder: one polite "Arrived" line when a new file has finished arriving.

### The command line

- `diro`, `diro PATH` and `diro LEFT RIGHT` open the terminal screen; `--inline` draws below the prompt, `--theme` picks a theme for a session, `--portable` keeps settings beside the program, and `--cwd-file` with `diro init` leaves your shell in the folder you quit in.
- Subcommands that print plain lines, name first: `diro ls`, `diro search`, `diro diff`, `diro watch`, `diro archive` (list, pack, unpack and test), `diro split`, `diro join`, `diro keys` (check, print, preset, test and probe), `diro completions` and `diro serve --stdio`.
- Exit codes are the same everywhere: 0 done, 1 failed or nothing found, 2 wrong use and 3 partly done.

### The documentation

- A guide for every area above, a quick start, an installing guide, terminals and troubleshooting, the known limits, privacy, the accessibility statement and the screen reader guide with a check you can run. A documentation check keeps every link alive and every guide ending with a See also list.
