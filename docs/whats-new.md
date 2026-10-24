# What is new

This page says in plain words what each alpha added. [The changelog](../CHANGELOG.md) has the full list, by area, and [known limits](known-limits.md) says what is still missing.

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
- [Quick start](quickstart.md)
- [Plugins](plugins.md)
- [Remote places over SFTP](remote-places.md)
- [The transfer queue](transfer-queue.md)
- [Volumes and the trash](volumes-trash.md)
- [Hooks](hooks.md)
- [Optional components](components.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
