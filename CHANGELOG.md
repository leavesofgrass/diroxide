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
- Nine example plugins under `examples/plugins`, all passing `diro plugin check` and the conformance checks: two bash scripts, a Python line counter, a search through the system's own index, a diff-tool opener, a changed-files list, a toy file system in memory, a PDF previewer and a Markdown to Word converter.
- `diro plugin`: list, new, enable, disable, run, restart, log, check, install, preview, convert and conformance. `diro plugin new` writes a skeleton copied from the examples, so they cannot drift. `diro run` runs one of diroxide's own commands without the panels.
- Optional components: helper programs, plugins and data packs fetched only after a yes that starts on No, checked by size and SHA-256 before use, with the list built into each build. A built-in client, then `curl`, then `wget`, with one neutral User-Agent. Resume after Escape. Install from a zip file or a folder with no network. Unpacking with plain names, no links and a size ceiling, and a receipt so Remove takes away exactly what was written. `diro components` (list, download, verify, remove and install) and the `components_source` setting.
- The first component is the Example plugins pack, `diro-examples`.
- `diro --no-plugins` starts with every plugin off.

### The documentation

- New guides: [plugins](docs/plugins.md), [writing a plugin](docs/writing-a-plugin.md), [script commands](docs/script-commands.md) and [optional components](docs/components.md). New pages: [security](docs/security.md) and [what is new](docs/whats-new.md). The [privacy](docs/privacy.md), [known limits](docs/known-limits.md), [settings](docs/settings.md) and [index](docs/README.md) pages say what plugins and components change.

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
