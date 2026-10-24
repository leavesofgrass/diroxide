# Changelog

Notable changes to diroxide, newest first. Agents write under a scratch heading with their id; the orchestrator regroups the entries by area before each alpha. Versions follow the alpha series in the plan; nothing is released until the owner says so.

## Unreleased

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
