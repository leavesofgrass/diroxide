# What is new

This page says in plain words what each alpha added. [The changelog](../CHANGELOG.md) has the full list, by area, and [known limits](known-limits.md) says what is still missing.

## Since alpha 1, not released yet

Plugins, script commands and optional components.

- Script commands. A one-file script in bash, Python, PowerShell or a batch file becomes a command by adding a few comment lines at the top. It gets the selection in a list that holds any name, tells diroxide what to say and what to do next, and ends with an exit code that diroxide says in words. [Script commands](script-commands.md) has the details.
- Plugins. A plugin is a folder with a `plugin.toml` and, if it needs code, a program in any language that talks to diroxide in lines of JSON. It can add commands, lists of rows that are not files, and places. A plugin with no code of its own can run a program such as a diff tool or pandoc. [Plugins](plugins.md) and [writing a plugin](writing-a-plugin.md) explain both sides.
- Nothing runs until you say so. Turning a plugin on shows what it runs, what diroxide checks, and what the plugin only promises, and the answer starts on No. If the plugin's manifest or its program changes, it asks again. `diro --no-plugins` starts with every plugin off.
- A plugin cannot pass for diroxide. Everything it says ends with its name, so the meaning stays in the first 40 characters of a Braille line. [Security](security.md) says plainly what a plugin could still do.
- Optional components. Helper programs, plugins and data packs that diroxide does not ship are fetched only after you say yes, checked by size and SHA-256 before they are used, and removed by a list that shows every file first. [Optional components](components.md) describes it.
- In the program. The Plugins menu (Alt+P) lists Manage plugins and Get example plugins, then the commands of each plugin you turned on. A command runs as a job that Escape cancels. A plugin's questions and lists open as dialogs of diroxide's own, with its name last. The Tools menu has Manage optional components. [Plugins](plugins.md) walks through each screen.
- Nine example plugins, small on purpose: two bash scripts, a Python line counter, a search through the system's own index, a diff-tool opener, a changed-files list, a toy file system in memory, a PDF previewer and a Markdown to Word converter. `diro plugin new` starts your own from the same files.
- The command line. `diro plugin` lists, creates, checks, turns on and off, runs, restarts and tests plugins and scripts. `diro components` lists, downloads, verifies, removes and installs components. `diro run` runs one of diroxide's own commands, or an enabled plugin's, without the panels.
- Privacy. [The privacy page](privacy.md) now says what a plugin can see, and that the one request diroxide makes by itself, the download of a component, asks first and sends only the project's name.

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
- [Optional components](components.md)
- [Privacy](privacy.md)
- [The documentation index](README.md)
