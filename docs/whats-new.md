# What is new

This page says in plain words what each alpha added. [The changelog](../CHANGELOG.md) has the full list, by area, and [known limits](known-limits.md) says what is still missing.

## Unreleased

The changes since alpha 2 that the next alpha, alpha 3, will carry. None of this is released yet, and more is coming before it is.

The GUI, a window version of diroxide.

- The foundation of the window is built. `diro --gui` opens a window with the two panels, the status bar and the key bar. The terminal is still what `diro` opens when you type it alone. [The GUI](gui.md) is the new guide: how the window opens, its parts in reading order, its keys and menus, what it says and when, and what it does not do yet.
- Each panel is one list, and each row is an option that a screen reader reads name first, with "selected" as a state and the position as a property. The focused row is what a screen reader follows. The title line, the two panels' sort buttons, the status bar with its two lines and the key bar are named parts of the window.
- Messages go to a live region by importance. Normal is said politely, high is said at once, and low is only shown. The program's start-up lines are held until a screen reader asks for the window, then said once.
- On Windows the menus are the system's menu bar. It stays hidden until Alt pressed alone, F9, F10 or Alt with a menu's letter shows it. On Linux and macOS the menus are lists in the window.
- New settings: Opens in (`frontend`), GUI graphics (`graphics`) and GUI menus (`menus`). Hide the menu bar (`auto_hide_menu`), which did nothing before, now works on Windows. New options: `diro --graphics NAME`, `diro --background` and `diro --log FILE`. With no display, or when every way of drawing fails, the window says one sentence and the terminal opens.
- The key bar and each panel's sort buttons are buttons with names and keyboard shortcuts. A click focuses a row, and a double click opens it.
- The fonts are inside the program: Atkinson Hyperlegible Next and Atkinson Hyperlegible Mono. [Installing diroxide](install.md#what-the-gui-needs) says what the window needs and what it costs to build.
- Reports read the window's accessibility tree through UI Automation on Windows and AT-SPI on Linux, with no screen reader. Nobody has listened to the window yet.
- Not in the window yet: icons, your own theme files, and a native menu bar on macOS.

The window's dialogs, viewer and quick view.

- A confirmation is two real buttons, No first, with the focus on No, and a list to read of what it touches: "Listing, 3 lines". A question is named by its first line, and a long message is a list read line by line: "Message, 11 lines". A password has a check box, "Show the password" (Alt+S). A dialog's name is said once. Every dialog the program can open, 77 of them, is drawn and tested. [The GUI](gui.md#the-dialogs) has the shapes.
- Fields with completion are combo boxes, a paste types its first line, and every field except a password field takes an input method's text. Copied text, such as a file's path, goes to the system clipboard.
- The viewer (F3) takes the panels' place as one list of the rows on screen, in text or hex, and a file of 100,000 lines is a screen of nodes. The quick view (Shift+F3) is the other panel's list, "Right panel, Preview, notes.md".
- A running job shows a progress indicator, "Job progress", that moves with the jobs line and is not spoken by the program. Escape stops the job in front. The Jobs list works in the window, with Pause, Held rows, and Retry and Forget on the Interrupted group.
- Change key in the Keys list takes a real key press, such as Ctrl+Shift+K, and saves it to `keymap.toml`.
- Shell here, Run a command and F4 start the system's terminal or the editor in a window of their own, and the window goes on: "Started a shell in work", "Started the editor on notes.txt". There is no "exit the shell" line in the window.
- The toolkit's helper window on Windows is named "diroxide background helper".

Themes, drawing without a graphics card, and focus mode.

- The window draws all 24 themes, your `[colors]`, and Windows' High Contrast, and it can follow the system's light or dark setting. A theme chosen is said once with its contrast verdict: "Theme Galaxy Light, meets AA." Ten themes fall short of AA. The window draws the parts that fall short in the text color and says how many. `diro --theme NAME` works in the window. [Themes and colors](themes.md) is the new guide.
- A graphics card is no longer needed. With none, or only a software adapter, the window draws on the processor itself, and `--graphics software` asks for that. The window says once, "Drawing on the processor: no graphics adapter was found." `diro --log FILE` records how the window drew. [The GUI](gui.md#how-the-window-draws) has the order.
- Focus mode (a setting) and full screen (F11) fade the title line, the status bar and the key bar after three seconds of rest. Any key brings them back. Nothing is said.

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
- [Quick start](quickstart.md)
- [Plugins](plugins.md)
- [Remote places over SFTP](remote-places.md)
- [The transfer queue](transfer-queue.md)
- [Volumes and the trash](volumes-trash.md)
- [Hooks](hooks.md)
- [Optional components](components.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
