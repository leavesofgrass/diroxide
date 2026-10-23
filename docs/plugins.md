# Plugins

This guide covers plugins: what they are, the three kinds, how you turn one on and what the question says, how to manage them, what you hear when one runs, the nine examples that come with diroxide, and what a plugin can still do after you say yes.

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

- Commands. They show in the Plugins menu, the command palette and the banks. A command that cannot run now is shown disabled with its reason, for example "Search with Everything, disabled" and "Needs es, not found."
- Lists of rows that are not files, such as the changed files of a git folder. A row says its meaning first. Enter runs the plugin's own command for the row. A file command on such a row, for example F5, says "This row is not a file."
- Places. A plugin can serve a file system at a name of its own, such as `memfs:/` in the toy example. Copy, view, rename and delete then work there as in any place, with the same plan, the same confirmation and the same Undo as anywhere else.
- Previewers and converters. Today you run them from a shell with `diro plugin preview` and `diro plugin convert`. The preview region and the conversions dialog come with a later alpha.
- Hooks. A manifest may declare them and `diro plugin check` reads them, but diroxide does not run them yet.

## Where plugins live

- The config folder has a `plugins` folder. A plugin is a folder in it. A script is a file in its `scripts` folder, `plugins/scripts`. Scripts run only from there, never from the current folder and never from an archive.
- The state folder has a `plugins` folder too. It holds `consent.toml`, which records what you turned on, one folder for each plugin with its log and its data, and a `runs` folder for the short-lived files of a running script.
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

On Windows the second row reads "It runs: py -3 main.py". The answer starts on No. After it, you hear "Enabled: Line counter." or "Not enabled."

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

`diro plugin new` writes a working skeleton, and `diro plugin preview` and `diro plugin convert` run a previewer or a converter on one file. Both are in [writing a plugin](writing-a-plugin.md).

## In the program

These screens are the ones to expect in the terminal.

- The Plugins menu (Alt+P, or F9 and then Plugins) lists the commands of the plugins you turned on, under the plugin's name, and ends with Manage plugins and Get example plugins. A plugin that is not turned on adds nothing to the menu. It shows up only in Manage plugins.
- Manage plugins is a list, one row for each plugin and script, the name first and the state after it: "Line counter: not enabled, plugin 0.1.0". These are the words `diro plugin list` prints. Enter on a row offers to turn it on or off, to show its settings and to show its log. Turning on asks the question above.
- A plugin's command runs as a job. The jobs line says "Count lines: started (Line counter)". Escape cancels it. If the plugin takes more than two seconds to answer, the line says "Count lines: waiting for the plugin".
- A question a plugin asks is a dialog of diroxide's own. It has a title with the plugin's name last, for example "Trust this host? (SFTP places)", it closes on Escape, and focus goes back to where it was. It waits behind a confirmation of a delete and never covers one.
- A plugin's settings show in the Settings list under the plugin's name, with the help key on each row. A change reaches the plugin at once.

## What you hear

A plugin cannot pass for diroxide. Every sentence it causes puts its meaning in the first 40 characters and its own name last:

- A plugin's own announcement: "Lines: 42, files: 1. By Line counter."
- The start of a command: "Count lines: started (Line counter)". The end: "Count lines done: 3 files (Line counter)". A cancel: "Count lines: cancelled."
- Progress: "Counting 12 of 300 files (Line counter)". In screen-reader mode it is spoken at most every ten seconds, and Say status (Ctrl+D) gives it when you ask.
- A question's title: "Replace a file? (Line counter)". When a plugin gives no title, it reads "Line counter asks".
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

Nine examples come with diroxide, in the `examples/plugins` folder of the source and in the optional component "Example plugins" (id `diro-examples`). Each is small on purpose, and [writing a plugin](writing-a-plugin.md) takes them one at a time.

- Total size of selection: a bash script. Says how many items are selected and how big they are.
- Select empty folders: a bash script. Marks the empty folders of the active panel.
- Line counter: a Python plugin. Says how many lines the selected files hold.
- Search the whole computer: three commands and no code. Finds files by name with Everything on Windows, plocate on Linux or Spotlight on macOS, and opens the results as a list of real files.
- Compare in a diff tool: two commands and no code. Opens two files in the diff program you choose.
- Changed files: a Python plugin. Lists the changed files of a git folder, the state first, as rows that are not files.
- Memory files (toy): a Python plugin. A file system that lives in memory until diroxide quits.
- PDF first page, as text: a previewer with no code. Needs `pdftotext`.
- Markdown to Word: a converter with no code. Needs `pandoc`.

To get them, ask for the component. It names the size and license and asks first:

```bash
diro components download diro-examples
```

The pack is not published with a release yet, so until it is, add `--source` with a folder that holds it, or install it from a file. [Optional components](components.md) shows how. Then copy the one you want into your plugins folder with `diro plugin install`, and read it before you turn it on.

## Settings

- `plugins.timeout_ms` (10000; 1000 to 600000): how long a plugin may take to answer before it is reported and left.
- `components_source` names your own list of optional components. [Optional components](components.md) describes it.
- A plugin's own settings are listed in its manifest and shown under its name.

[The settings reference](settings-reference.md) lists every setting with its default and range.

## What a plugin can still do

diroxide limits what a plugin may ask of it, and checks those limits. It cannot limit what a program does on its own. A plugin you turn on is a program that runs with your rights. [Security](security.md) says plainly what that means and what protects you. The short version: turn on only what you have read or trust.

## See also

- [Writing a plugin](writing-a-plugin.md)
- [Script commands](script-commands.md)
- [Optional components](components.md)
- [Security](security.md)
- [Privacy](privacy.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [The documentation index](README.md)
