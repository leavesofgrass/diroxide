# Plugins

This guide covers plugins: what they are, the three kinds, how you turn one on and what the question says, how to manage them, what you hear when one runs, the keys a plugin's commands get, the eleven examples that come with diroxide, and what a plugin can still do after you say yes.

If you want to make one, [writing a plugin](writing-a-plugin.md) and [script commands](script-commands.md) are the guides for that. [Optional components](components.md) says how a plugin can come to your computer.

What this guide quotes is what the program says. Each sentence is checked in tests against the 40 cells of a Braille line, and each is written so its meaning comes first. None of it has been heard yet on NVDA, JAWS or a Braille display, because the listening tests come after the build. If your experience differs, that is a finding.

## What a plugin is

A plugin adds something diroxide does not do by itself. It can add a command, a list of rows that are not files, a place such as a remote server, a previewer or a converter.

- A plugin is a separate program. diroxide starts it when you use it and stops it after five minutes of rest.
- A plugin that crashes, hangs or floods the screen with text is stopped and the problem is said in words. diroxide keeps running.
- Nothing is on by default. You turn a plugin on by name, after a question that starts on No.
- `diro --no-plugins` starts diroxide with every plugin off for that session. Opening the Plugins menu then says "Plugins are off for this session."

## The three kinds

- A script command is one file, in bash, Python, PowerShell or a batch file, with a few comment lines at the top that name it. It is the smallest plugin and the quickest to write. [Script commands](script-commands.md) has the details.
- A plugin with no code of its own is a folder with one file, `plugin.toml`. It names a program that is already on your computer and the arguments to give it, such as a diff tool or pandoc. It can add commands, previewers and converters.
- A process plugin is a folder with a `plugin.toml` and a program in any language. diroxide and the program talk in lines of JSON over the program's standard input and output. It can add commands, lists of rows that are not files, and places. [Writing a plugin](writing-a-plugin.md) explains the talk.

## What a plugin can add

- Commands. They show in the Plugins menu and the command palette, and `menus.toml` and `banks.toml` may name them. A command that cannot run now is shown disabled with its reason, for example "Search with Everything, disabled" and "Needs es, not found."
- Lists of rows that are not files, such as the changed files of a git folder. A row says its meaning first. Enter runs the plugin's own command for the row. A file command on such a row, for example F5, says "This row is not a file."
- Places. A plugin can serve a file system at a name of its own, such as `memfs:/` in the toy example, or `sftp://` for the SFTP places plugin. Copy, view, rename and delete then work there as in any place, with the same plan and the same confirmation as anywhere else. [Remote places over SFTP](remote-places.md) is the plugin that reaches other computers.
- Previewers and converters. Today you run them from a shell with `diro plugin preview` and `diro plugin convert`. The preview region and the conversions dialog come with a later alpha.
- Hooks. A plugin can be asked before a copy, a move, a delete, a rename or a new folder, and stop it with a reason. It can be told afterward, and told when you enter a folder. [Hooks](hooks.md) has the events, the lines you hear and the Folder guard example.

## Where plugins live

- The config folder has a `plugins` folder. A plugin is a folder in it. A script is a file in its `scripts` folder, `plugins/scripts`. Scripts run only from there, never from the current folder and never from an archive. The one exception is the example pack, whose plugins and scripts are listed in Manage plugins from the components folder.
- The state folder has a `plugins` folder too. It holds `consent.toml`, which records what you turned on, one folder for each plugin with its log, its data and its `settings.toml`, and a `runs` folder for the short-lived files of a running script.
- A plugin that came as an optional component lives in the `components` folder of the state folder. [Optional components](components.md) says more.
- With `--portable`, all of these sit beside the program. [Settings and where the files live](settings.md) says where the folders are on each system.

## Turning a plugin on

Nothing runs until you say yes. Installing a plugin or downloading one never turns it on.

The question lists what the plugin does, one row a line. For the line counter, from a terminal:

```bash
diro plugin enable line-counter
```

```text
Enable Line counter, version 0.1.0?
It runs: python3 main.py
diroxide checks: it may read files.
The plugin states, unchecked:
no network.
only the files it is given.
no other programs.
While it runs it can do what you can.
Enable it? y or n
```

On Windows the second row reads "It runs: py -3 main.py". The answer starts on No. In the program, the question comes first and the rows are listed under it, as "In the program" says below. After it, you hear "Enabled: Line counter." or "Not enabled."

What the rows mean:

- "It runs:" names the program and its arguments. For a script it names the interpreter and the file, for example "It runs: bash total-size.sh".
- "diroxide checks:" lists what diroxide itself limits. A plugin that asks for more than reading files says so here: it may read and change files, read or change the panels, run named programs, or start named programs. diroxide enforces these.
- "The plugin states, unchecked:" lists what the plugin says about itself: whether it uses the network, which files it touches and which other programs it starts. diroxide cannot check these words. They are the plugin's promise.
- "This plugin can run any program." appears for a plugin that declares it. Treat it as full trust.
- "It can run without asking." appears for a command that declares it never asks and always ends, so that saved tasks may run it later.
- "It overrides PowerShell's script policy." appears for a PowerShell script. diroxide starts PowerShell for that one run with the policy set aside, and changes nothing about your system's policy.
- "While it runs it can do what you can." is always the last row before the question, and it is true. [Security](security.md) says what that means.

A script's question is shorter, because a script has no manifest to read. It names the script, the interpreter and that last row.

### When a plugin changes

diroxide remembers a fingerprint of what you turned on: the manifest, and the program file the plugin runs. A script's fingerprint is the whole file. If either changes, the plugin asks again before it runs:

```text
Line counter changed since you enabled it.
...
Enable it again? y or n
```

This is how an edit to the code, which the manifest would not show, still gets your attention. `diro plugin list` shows such a plugin as "changed, asks again".

### Running once without turning on

`diro plugin run NAME` runs a plugin that is not turned on after one question, "Run it once? y or n". It does not turn it on. This is how you try a plugin you are writing or one you just installed.

### Turning off

```bash
diro plugin disable line-counter
```

You hear "Disabled: Line counter." diroxide forgets the yes. The files stay where they are.

## From a shell

`diro plugin` does everything from a terminal, with plain lines and the usual exit codes: 0 done, 1 failed or declined, 2 wrong use, 3 partly done. `--portable` uses the folders beside the program. `diro plugin --help` lists the whole set.

```bash
diro plugin list
```

One line for each plugin or script, the name first, then the state, the kind and the id:

```text
Line counter: enabled, plugin 0.1.0, id line-counter
Total size: not enabled, script, id total-size
```

The state is "not enabled", "enabled" or "changed, asks again".

```bash
diro plugin check line-counter
```

Reads the manifest or the header and says every problem, the field first and then the fix. A clean one says "Checked 1 plugin: no problems." The exit code is 1 when there is an error. A warning, such as an unknown key, does not fail it.

```bash
diro plugin install compare-files
```

Copies a plugin folder, or a script file, from the path you give into your plugins folder. It says "Installed: Compare in a diff tool. It is not enabled." and then the next command to run. It never turns anything on. If the name is taken, it says "Could not install compare-files: it exists. Remove it first, or choose another name." and changes nothing.

```bash
diro plugin run line-counter -- notes.txt
```

Runs a plugin or script once, with the files after the two dashes as the selection. A plugin with more than one command takes the command's id after its name. It says what the plugin says, with the plugin's name last: "Lines: 4, files: 1. By Line counter."

```bash
diro plugin restart line-counter
```

Starts the program, checks that it answers, and stops it. It says "Started and stopped: Line counter, it answered in under a second."

```bash
diro plugin log line-counter
```

Prints the last lines of the plugin's own log, or "No log yet for Line counter." The log has the plugin's standard error text and the lines diroxide wrote about it. A plugin's error text often holds a file name or a folder path, so read the log before you share it.

```bash
diro plugin conformance line-counter
```

Runs the protocol checks over a plugin folder. [Writing a plugin](writing-a-plugin.md) says what they check.

```bash
diro run line-counter.count_lines
```

Runs an enabled plugin's command without the panels, in the current folder, and says what the app says: "Count lines: started (Line counter)", the plugin's own words, and "Count lines done (Line counter)". A command's id is its plugin's id, a dot, and its own id. A script's is `script.` and its name, for example `script.total-size`. For a plugin that is not turned on, it says "Say hello is not enabled. Enable it in Manage plugins." and runs nothing.

`diro plugin new` writes a working skeleton, and `diro plugin preview` and `diro plugin convert` run a previewer or a converter on one file. Both are in [writing a plugin](writing-a-plugin.md).

## In the program

These are the screens, in the terminal.

### The Plugins menu

Alt+P opens the Plugins menu. It lists Manage plugins (M) and Get example plugins (G) first. After them come the commands of each plugin you turned on, the command's name first and its key after it, for example "Count lines, Ctrl+Alt+L". The same commands are in the command palette. A plugin that is not turned on adds nothing to the menu or the palette. It shows up only in Manage plugins.

A command that cannot run now is read "Search with Everything, disabled", with the reason under it: "Needs es, not found." The reason is also said when you try to run it.

With `diro --no-plugins` the menu holds only Manage plugins and Get example plugins, and it says "Plugins are off for this session."

### Manage plugins

Manage plugins is a list with one row for each plugin and script, the name first, then the state, then the kind: "Line counter: not enabled, plugin 0.1.0" and "Total size: enabled, script". These are the words `diro plugin list` prints. The state is "not enabled", "enabled", "changed, asks again" or "has errors".

Enter on a row opens a list named for the plugin. It offers, in this order:

- Enable, for a plugin that is not enabled. For one that changed: Enable again, and Disable. For an enabled one: Disable.
- "Why it has errors", for a plugin whose manifest or header has errors. It says the first one.
- "Turn it on again", for a plugin that diroxide turned off after repeated crashes.
- "Commands and keys", for a plugin that has commands. Each command is a row with its key beside it: "Ctrl+Alt+L, proposed", or "Ctrl+Alt+L not bound: it already runs Copy", or "no key". A command that cannot run now adds its reason.
- "Settings", for a plugin that has settings.
- "Log, the last error first". The log opens as a list, "Log of Line counter", with "Last error:" at the top and then the lines, newest first.

Enable asks the question. Disable takes effect at once.

### The question

The question is the first line, and the rest is listed under it, one row to a line. For the line counter:

```text
Enable Line counter, version 0.1.0? No is the default. y or n
It runs: python3 main.py
diroxide checks: it may read files.
The plugin states, unchecked:
no network.
only the files it is given.
no other programs.
While it runs it can do what you can.
```

Press n or Escape and you hear "Not enabled." Press y and you hear "Enabled: Line counter." A plugin that changed since you enabled it asks "Line counter changed since you enabled it. Enable it again? No is the default. y or n", with the same rows under it. If you run a command of a changed plugin, this question comes first, and the command runs after your yes.

If a plugin's command is run while it is not enabled, you hear "Line counter is not enabled. Enable it in Manage plugins."

### A command running

A plugin's command runs as a job.

- The jobs line says "Count lines: started (Line counter)". If the plugin takes more than two seconds to answer, it says "Count lines: waiting for the plugin".
- Escape cancels it. You hear "Count lines: cancelling" and then "Count lines: cancelled."
- When it ends, the plugin's own announcement comes first, "Lines: 4, files: 1. By Line counter.", and the job's end is shown: "Count lines done: 1 item (Line counter)". A failure says what failed, then "Last error:" and "Next: diro plugin log line-counter".
- A script that sets a question says it before it runs: "Say hello to the name you type? No is the default. y or n (Ask first)". A script that sets a prompt asks for a line, and the field is named "Name to greet (Ask first)". An empty answer, or Escape, says "Ask first: cancelled."
- When a script in `output-pane` or `output-viewer` mode ends and printed lines, they open as a list named "Output of Print names (Print names)", one line to a row. Escape closes it.
- When a script in `lister` mode ends, its places open as a results list like a search: "214 results for report, from Search the whole computer", with real files that you can view and copy. "No results for report." says there were none.

### A question a plugin asks

A question a plugin asks is a dialog of diroxide's own. It has the plugin's name last: "Trust this host? No is the default. y or n (Line counter)", with the plugin's message lines under it. It closes on Escape, and focus goes back to where it was. There are five kinds: yes or no, a choice from a list (with Space to mark several), a line of text, a place, and a secret that is never shown. A plugin's text prompt can offer choices as you type, and says how many: "3 candidates".

A plugin's question waits behind a confirmation, a password prompt, a job's question, a field you are typing in, and another question, and opens when they close. It never covers one.

### A list of rows that are not files

A plugin can open a list of rows that are not files, such as the changed files of a git folder. It opens over the panels, like search results, and says "Changed files, 2 rows (Line counter)". A row says its name first and its columns after it: "b.txt, state Modified, lines 12, 2 of 2".

- Up and Down move. At the end of a long list, diroxide asks the plugin for more rows by itself.
- Space marks and unmarks a row: you hear "Marked" or "Unmarked".
- The sort keys open "Sort the rows by", a choice of the name and each column. The same choice again reverses it: "Sorted by lines", then "Sorted by lines, reversed".
- Ctrl+S opens "Filter the rows by". Rows whose name or columns hold the text stay. "Filter off" says it is cleared.
- Enter runs the plugin's own command for the marked rows, or for the row you are on. The plugin's other action keys, such as Alt+S for "stage" in the changed-files example, do the same with their commands.
- Ctrl+R asks the plugin for the rows again.
- A key that acts on files, such as F5 or F8, says "This row is not a file."
- Escape closes the list and says "Lister closed (Changed files)".

Today a list always opens over the panels, whichever panel the plugin names.

### The Sites list

The SFTP places plugin opens a list of this kind: your saved sites and the hosts from your ssh configuration. It is in the Plugins menu as SFTP sites once the plugin is turned on. Its title gives the counts first, "Sites, 3 saved, 5 from ssh config", and each row starts with the name and the state: "build, saved, alice@build.example" and "nas, from ssh config". Escape closes it.

Enter connects to the site. The plugin's other action keys act on the row you are on:

- Alt+N: new site. On a host from your ssh configuration it saves that host.
- Alt+E: edit the site.
- Alt+D: delete the site. The question starts on No.
- Alt+X: disconnect.
- Alt+C: cancel the connection, even in the middle of a transfer.
- Alt+F: forget the site's saved password.

These keys work while the list is open. [Remote places over SFTP](remote-places.md) says what each one does.

### Keys for a plugin's commands

A plugin's command has an id: its plugin's id, a dot, and its own id, such as `line-counter.count_lines` or `sftp.new_site`. A manifest may propose a key for a command. The proposal binds only when the key is free and has no concern. Otherwise the command has no key, and Manage plugins, on the plugin's Commands and keys row, says why.

You decide the keys, not the plugin. A line in `keymap.toml` for a plugin's command wins over the proposal. Plugin commands go in the global layer, in the terminal's table and in the GUI's:

```toml
[terminal.global]
"line-counter.count_lines" = "ctrl+alt+k"

[gui.global]
"line-counter.count_lines" = "ctrl+alt+k"
```

- The word `none` unbinds the command.
- A line for a plugin that is not turned on is not an error. It waits in the file and binds when the plugin is found.
- If another command already holds the key, the line takes it, as any line of the file does.
- `diro keys check` does not know plugin commands. It says "Unknown command" for such a line. The program does not.

The Keys list has a row for each command of each plugin that is turned on. The command's name comes first, then its keys, then how it got them, and the plugin's name last:

```text
Count lines, Ctrl+Alt+K, set in keymap.toml, by Line counter
Count lines, Ctrl+Alt+L, proposed, by Line counter
Count lines, no key, Ctrl+Alt+L not bound: it already runs Copy, by Line counter
Count lines, no key, not proposed, by Line counter
```

Enter on such a row says how to change its key:

```text
Count lines: change its key in keymap.toml under "line-counter.count_lines". By Line counter.
```

The Keys list cannot change a plugin's key itself yet. [Customizing keys](customizing-keys.md) describes the list.

### Settings

The Settings list has a row, "Plugin settings", with the words "each plugin's own, under its name". Enter on it lists the plugins that have settings: "Line counter, not enabled, 1 setting". Enter on one opens "Settings of Line counter", one row for each setting: "Skip blank, off". Enter flips a switch, or opens the choices, or opens a field to type in. The help key (F1) says what the setting does, what it holds and its default. A change reaches the plugin at once and is said: "Skip blank: on (Line counter)". The same list opens from Manage plugins, on a plugin's Settings row.

The values are kept in `settings.toml` in the plugin's folder of the state folder.

### Get example plugins

Plugins menu, G. If the example plugins are not here yet, it asks first, as [optional components](components.md) describes. If they are, it says "Already installed: Example plugins. Manage plugins lists them." Nothing in the pack is turned on. After a yes and "Ready: diro-examples 0.1.0.", Manage plugins lists the examples, each "not enabled".

## What you hear

A plugin cannot pass for diroxide. Every sentence it causes puts its meaning in the first 40 characters and its own name last:

- A plugin's own announcement: "Lines: 42, files: 1. By Line counter."
- The start of a command: "Count lines: started (Line counter)". The end: "Count lines done: 3 items (Line counter)". A cancel: "Count lines: cancelling" and then "Count lines: cancelled."
- Progress: "Counting 12 of 300 files (Line counter)". In screen-reader mode it is spoken at most every ten seconds, and Say status (Ctrl+D) gives it when you ask.
- A question: "Replace a file? (Line counter)", or "Trust this host? No is the default. y or n (Line counter)" for a yes or no question. When a plugin gives no title, it reads "Line counter asks".
- A plugin that talks a lot: "12 more messages from Line counter" says that some were folded together.
- A plugin's text is cleaned first. Control and direction characters and terminal escape sequences are removed, and a very long line is cut.

diroxide's own confirmations, such as a permanent delete or an overwrite, are never made from a plugin's words.

### When something goes wrong

The first line says what failed. The next lines give the cause and the next step.

```text
Could not run Count lines: code 2.
Last error: cannot read a.txt.
Next: diro plugin log line-counter
```

Other things you may hear:

- "Count lines timed out after 30 seconds." The plugin was asked to cancel and given two seconds. After that it is reported as not answering. A second time-out within five minutes restarts it.
- "Line counter stopped unexpectedly." and "It starts again when you use it."
- "Line counter is off: it stopped 3 times." Three crashes in a minute turn a plugin off until you turn it on again.
- "Line counter was stopped for flooding." A plugin that sends a flood of lines that are not messages, or hundreds of announcements in a minute, is stopped.
- "Line counter may not do that." and "Its manifest does not allow it." The plugin asked for something its manifest did not declare.
- "Line counter needs a newer diroxide: it speaks protocol 2." or "Line counter is too old for this diroxide: it speaks protocol 0."
- "Line counter wrote stray text; see its log." A line that is not a message was found on its output.
- "Python 3 was not found." and the other words for a missing interpreter name what to install. [Script commands](script-commands.md) lists them.

A missing program, a wrong number of marked items and a name that a script cannot take are each said in words before anything runs.

## The example plugins

Ten examples come with diroxide, in the `examples/plugins` folder of the source and in the optional component "Example plugins" (id `diro-examples`). Each is small on purpose, and [writing a plugin](writing-a-plugin.md) takes them one at a time.

- Total size of selection: a bash script. Says how many items are selected and how big they are.
- Select empty folders: a bash script. Marks the empty folders of the active panel.
- Line counter: a Python plugin. Says how many lines the selected files hold.
- Search the whole computer: three commands and no code. Finds files by name with Everything on Windows, plocate on Linux or Spotlight on macOS, and opens the results as a list of real files.
- Compare in a diff tool: two commands and no code. Opens two files in the diff program you choose.
- Changed files: a Python plugin. Lists the changed files of a git folder, the state first, as rows that are not files.
- Memory files (toy): a Python plugin. A file system that lives in memory until diroxide quits.
- PDF first page, as text: a previewer with no code. Needs `pdftotext`.
- Markdown to Word: a converter with no code. Needs `pandoc`.
- Folder guard: a Python plugin with two hooks. Stops a delete inside the folders you protect, and logs every delete. [Hooks](hooks.md) walks through it.

To get them, ask for the component. It names the size and license and asks first:

```bash
diro components download diro-examples
```

The pack is not published with a release yet, so until it is, add `--source` with a folder that holds it, or install it from a file. [Optional components](components.md) shows how. In the program, Manage plugins then lists the examples where they are, each with "example" beside it, and a plugin of your own with the same id wins. From a shell, copy the one you want into your plugins folder with `diro plugin install`. Read an example before you turn it on.

## Settings

- `plugins.timeout_ms` (10000; 1000 to 600000): how long a plugin may take to answer before it is reported and left.
- `components_source` names your own list of optional components. [Optional components](components.md) describes it.
- A plugin's own settings are listed in its manifest and shown in the Settings list, on the "Plugin settings" row, as the section above says.
- `plugins.enabled` is reserved. You turn a plugin on in Manage plugins or with `diro plugin enable`, and the record is `consent.toml`.

[The settings reference](settings-reference.md) lists every setting with its default and range.

## What a plugin can still do

diroxide limits what a plugin may ask of it, and checks those limits. It cannot limit what a program does on its own. A plugin you turn on is a program that runs with your rights. [Security](security.md) says plainly what that means and what protects you. The short version: turn on only what you have read or trust.

## See also

- [Writing a plugin](writing-a-plugin.md)
- [Script commands](script-commands.md)
- [Hooks](hooks.md)
- [Remote places over SFTP](remote-places.md)
- [Optional components](components.md)
- [Security](security.md)
- [Privacy](privacy.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [The documentation index](README.md)
