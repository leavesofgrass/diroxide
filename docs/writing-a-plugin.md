# Writing a plugin

This guide is for making your own plugin. It covers which kind to write, how `diro plugin new` starts one, the manifest, the protocol in a page, the services diroxide offers a plugin, lists of rows that are not files, places, previewers and converters, the conformance checks, and the nine examples that come with diroxide, one at a time.

If you only want to run plugins, read [plugins](plugins.md). If you want the smallest kind, a single script, read [script commands](script-commands.md) first.

Every command in this guide was run. Every example plugin in it passes `diro plugin check`, and the three that have code pass the conformance checks too. The sentences quoted are the ones the program says.

## Which kind to write

- Write a script command when one file in bash, Python or PowerShell can do it, the work is "act on the selected files", and you want it in five minutes. A script has no protocol to learn.
- Write a plugin with no code when the work is "run this program on the files". You give the program and its arguments in `plugin.toml`, and diroxide fills in the file names itself. A diff tool, pandoc and pdftotext are all this kind.
- Write a process plugin when you need more than a command: a list of rows that are not files, a place that serves a file system, a dialog that asks questions, settings that change while it runs, or progress and cancel. It is a program in any language that talks JSON lines with diroxide.

You can start with a script and move to a plugin later.

## Start from a skeleton

```bash
diro plugin new line-counter --language python
```

This writes a working plugin, copied from the shipped example, into your plugins folder, and says:

```text
Wrote line-counter with 4 files: plugin.toml, main.py, test_main.py, README.txt.
Check it: diro plugin check line-counter
Try it once: diro plugin run line-counter -- FILE
```

The options:

- `--language bash` writes one script, `NAME.sh`, in the `scripts` folder. `--language python` writes a folder.
- `--kind command` is the default for Python. `--kind lister` adds a list of rows that are not files. `--kind provider` writes a toy file system. All three are cut from the shipped examples, so they cannot drift from them.
- `--dir FOLDER` writes into a folder you choose instead of your plugins folder. The last two lines then say "Check it:" and "Install it:" with the full path.
- `--portable` uses the folders beside the program.

The name must be plain: lowercase letters, digits, `-` and `_`, from 2 to 40 characters, starting and ending with a letter or a digit. A name with anything else is refused with the first bad character spelled out, for example "Could not make Bad Name: U+0042 is an uppercase letter; use lowercase. Use lowercase letters, digits, - and _." A name that is taken is refused too: "Could not make my-changes: it exists. Choose another name." Nothing is overwritten. Files are written as UTF-8 with `\n` line ends on every system.

A skeleton comes with a test of its own that uses the standard library only:

```bash
py -3 -B -I -m unittest
```

On Linux and macOS the same command is `python3 -B -I -m unittest`. Run it in the plugin's folder. The `-I` keeps your own modules and settings out of the run, and `-B` keeps Python from leaving cache folders behind.

## Check it, try it, read its log

```bash
diro plugin check line-counter
```

Reads the manifest and says every problem, the field first and then the fix. A good plugin says "Checked 1 plugin: no problems." Here is what a manifest with mistakes says:

```text
Error, plugin.id: "Broken One" is not a plain id. Use lowercase letters, digits, - and _, 2 to 40 characters.
Error, plugin.name: is 48 characters, over the 40 of one Braille line. Use plain words, 40 characters at most, meaning first.
Error, permissions.fs: "everything" is not a known value. Use one of: none, read, write.
Error, permissions.tools[0]: bash is an interpreter or a shell, which can run anything. Remove it, or set permissions.declared.programs = "any program".
Warning, plugin.description: starts with filler ("this plugin"). Say what it does first, within 40 characters.
Checked 1 plugin: 4 errors, 1 warning.
```

An error means diroxide will not use the manifest, and the exit code is 1. A warning is used and reported. An unknown key is a warning too, so a plugin written for a newer diroxide still loads in an older one.

```bash
diro plugin run line-counter -- notes.txt
```

Runs it once on the files after the dashes, after one question, without turning it on. You hear "Lines: 4, files: 1. By Line counter."

```bash
diro plugin restart line-counter
```

Starts the program, checks that it answers, and stops it. Use it after you edit.

```bash
diro plugin log line-counter
```

Shows the end of its log: its standard error text and what diroxide wrote about it. When you change the program or the manifest, diroxide asks again before it runs, as [plugins](plugins.md) says. That is the consent working, not a bug.

## The manifest, table by table

`plugin.toml` is TOML. This is the whole manifest of the line counter, which passes `diro plugin check`:

```toml
[plugin]
id = "line-counter"
name = "Line counter"
version = "0.1.0"
description = "Counts the lines of the selected files and says the total."
license = "Apache-2.0 OR MIT"
protocol = 1
min_host = "0.1.0-alpha.1"

[run]
command = ["python3", "main.py"]
start = "on_demand"
concurrency = 1
idle_timeout = "5m"

[run.windows]
command = ["py", "-3", "main.py"]

[permissions]
fs = "read"
panel = "none"

[permissions.declared]
network = "no network"
files = "only the files it is given"
programs = "no other programs"

[[command]]
id = "count_lines"
name = "Count lines"
description = "Says how many lines the selected files hold."
selection = "any"
places = "local"

[[setting]]
key = "skip_blank"
type = "bool"
default = false
description = "Leave blank lines out of the count."
```

- `[plugin]` says who it is. `id` is lowercase letters, digits, `-` and `_`. `name` is at most 40 characters, which is one Braille line. `description` says what it does first, within 40 characters, and does not begin with "This plugin" or "Lets you". `protocol` is 1. `min_host` is the oldest diroxide it works with.
- `[run]` says how to start it: an argument list, never a shell line. `[run.windows]`, `[run.linux]` and `[run.macos]` replace any key for one system. On Windows use `py -3` and not `python3`, because `python3` there is often the Store's stub. `start` is `on_demand` (the first time you use it, the default) or `startup`. `concurrency` is how many requests may be in flight, 1 to 16. `idle_timeout` is how long it may rest before diroxide stops it politely, five minutes by default, or `"never"` when its data lives in the process. `startup_timeout` (10 seconds) and `shutdown_grace` (3 seconds) are the other limits.
- `[permissions]` is what diroxide checks. `fs` and `panel` are `none`, `read` or `write`. `tools` names programs the plugin may run through diroxide. `launch` names programs it may start detached. `commands` lists command ids it may run, with `*` for a word. A call outside these gets "Line counter may not do that." and "Its manifest does not allow it."
- `tools` and `launch` refuse interpreters and shells: `sh`, `bash`, `python`, `pwsh`, `cmd`, `env`, `find` and the like. Naming `bash` would make the list meaningless. If a plugin truly needs any program, it says `programs = "any program"` under `[permissions.declared]`, and the consent text then says "This plugin can run any program."
- `[permissions.declared]` is what the plugin says about itself and diroxide cannot check: `network`, `files` and `programs`, each at most 80 characters. These words show in the consent question under "The plugin states, unchecked:". Write them as plain claims you mean to keep.
- `[[command]]` is a command. `id` is what the plugin receives. `name` is what you see in the menu and the palette; keep it within 40 characters and put the meaning first. `selection` is `none`, `one`, `two`, `any` or `many`. `places` is `any` or `local`. `needs` names tools that must be found, `os` limits it to some systems, and `key` proposes a key that binds only when it is free. A command can also carry the run keys of a script command, such as `mode`, `prompt`, `confirm` and `argv`; [script commands](script-commands.md) lists them.
- `[[setting]]` is a setting the person who uses the plugin can change. `type` is `bool`, `int`, `string`, `choice`, `path` or `argv`. The plugin gets the whole settings object in `initialize` and again in `settings/changed` after each change.

The other tables are `[[lister]]` with its `[[lister.columns]]` and `[[lister.action]]`, `[[provider]]`, `[[previewer]]`, `[[converter]]` and `[[hook]]`. Their sections are below.

## The protocol in a page

diroxide starts the program from `[run]` and talks to it over standard input and standard output. The full reference is generated from the code: [the plugin protocol reference](dev/plugin-protocol.md). This is the part you need to start.

- One JSON object a line, UTF-8 with no byte order mark, in both directions. Flush after every message. The longest line is 8 MiB by default.
- Standard error is yours: free text for the log, never for the screen.
- A request has `id`, `method` and `params`. A notification has no `id` and gets no answer. An answer has the request's `id` and either `result` or `error`.
- A line that does not start with `{` is stray output. diroxide logs it and never answers it. A line that starts with `{` and does not parse is answered with -32700. A batch, a line that starts with `[`, is refused with -32600.
- Unknown fields are ignored. An unknown request is answered with -32601 and an unknown notification is ignored. This is what lets a plugin and a newer diroxide still work together.

### A whole session

This is a real run of the line counter, with the lines diroxide sends marked `>` and the lines the plugin sends marked `<`. The plugin gets `initialize`, says what it can do, runs a command, announces a result, and shuts down:

```text
> {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":1,"host":{"name":"diroxide","version":"0.1.0-alpha.2"},"plugin_id":"line-counter","locale":"en-US","settings":{}}}
< {"jsonrpc":"2.0","id":1,"result":{"protocol":1,"plugin":{"name":"line-counter","version":"0.1.0"},"plugin_capabilities":{"commands":["count_lines"]}}}
> {"jsonrpc":"2.0","method":"initialized"}
> {"jsonrpc":"2.0","id":2,"method":"command/run","params":{"command":"line-counter.count_lines","context":{"protocol":1,"selection":[{"name":"lines.txt","kind":"file","place":"D:/notes/lines.txt"}]}}}
< {"jsonrpc":"2.0","method":"ui/announce","params":{"text":"Lines: 4, files: 1","importance":"info"}}
< {"jsonrpc":"2.0","id":2,"result":{"done":1,"skipped":0,"failed":[]}}
> {"jsonrpc":"2.0","id":3,"method":"shutdown"}
< {"jsonrpc":"2.0","id":3,"result":null}
> {"jsonrpc":"2.0","method":"exit"}
```

The steps:

1. `initialize` carries the protocol major, diroxide's name and version, the plugin's id, your locale and the plugin's settings. The line above is shortened. A real one also carries diroxide's process id, the services diroxide offers and the plugin's own folders, and a plugin ignores what it does not know. The answer names the protocol the plugin speaks and the version of the plugin. A different protocol major is refused in words: "Line counter needs a newer diroxide: it speaks protocol 2."
2. `initialized` says diroxide accepted the answer. Until `initialize` is answered, a request gets -32002.
3. `command/run` carries the command's id and a context: the selection (each entry with `name`, `kind` and `place`), the cursor and both panels' places. The plugin returns `done`, `skipped` and `failed` lists. Send `failed` entries as `{"place", "reason"}` with the reason meaning first.
4. `ui/announce` is a notification. diroxide says the text, cleaned and with the plugin's name added last, so the person using it hears "Lines: 4, files: 1. By Line counter." Do not put your own name in the text.
5. `shutdown` is answered with null, and then only `exit` follows. The program exits with code 0. A program also exits when its input ends.

### Cancel, progress and errors

- `$/cancel_request` arrives as a notification with the id of the request to stop. The request is still answered, either with its result or with an error of code -32800. A command that works for a while should look for a cancel between steps. The line counter reads what is waiting between files.
- `$/progress` is a notification you send with a token. It begins with kind `begin`, goes on with `report` and ends with `end`, each with words and counts. diroxide turns it into the jobs line, and in screen-reader mode speaks it at most every ten seconds.
- An error is `{"code", "message", "data": {"kind"}}`. The message is one short sentence naming the cause only, meaning first. diroxide builds the sentence you hear from it: "Could not run Count lines: cannot read a.txt." The codes you will use most are -32602 invalid parameters, -32800 cancelled, -32010 not found, -32011 already exists, -32012 permission denied, -32016 invalid name, -32017 unsupported and -32099 failed. [The reference](dev/plugin-protocol.md) lists them all.
- `log` is a notification that writes a line to the plugin's log. Use it for what you would otherwise print.

### Two things to get right

- Handle a name that is not Unicode. An entry may carry `name_raw` instead of a usable `name`. The line counter reports it as failed with the reason "name is not Unicode", which is honest for a plain-text example.
- Never read a file name from a line-based source if the name could hold a line break. Use the entries the context gives you.

## What you can ask diroxide for

A plugin asks diroxide with requests of its own. Each is allowed by something in the manifest. A call that is not allowed gets -32030, and the person using the plugin hears one sentence.

- Say, ask and show: `ui/announce`, `ui/ask` and `ui/show_text` are always allowed, because they only show things. `ui/ask` has five kinds: `confirm`, `choose`, `text`, `path` and `secret`. A confirm of a destructive thing starts on No. Escape is an answer, `{"cancelled": true}`, not an error. A secret is never echoed, never logged, never kept in history and never completed. The dialog is diroxide's own, its title ends with your plugin's name, and the person's thinking time does not count against your timeout.
- Panels: `panel/get` reads a panel's place, cursor and selection (needs `panel = "read"`). `panel/set` changes a place, the cursor or the marks (needs `panel = "write"`). `command/execute` runs a diroxide command by id (needs a matching `commands` pattern).
- Files: `fs/list`, `fs/stat` and `fs/read` need `fs = "read"`. `fs/write`, `fs/mkdir`, `fs/rename`, `fs/copy`, `fs/move` and `fs/delete` need `fs = "write"`, and they run as jobs. A copy of yours shows the same plan, the same question and the same Undo as a copy started from the panels. Delete goes to the trash by default, after a listing. You cannot turn any of this off. Names you create follow the portable-name rules, so a plugin cannot make a name with a control character, a colon or a trailing dot through diroxide.
- Programs: `tools/run` runs a program named in `permissions.tools`, with an argument list and never a shell line. `tools/find` says whether a tool is found. `tools/launch` starts a program from `permissions.launch` detached.
- Large data goes through the spool: a folder diroxide makes for your plugin, with owner-only rights. A `spool_path` must be a regular file directly inside it. Anything else is refused.

Say in the manifest only what you need. The shorter the list in the consent question, the easier it is to say yes with open eyes.

## Lists of rows that are not files

A lister shows rows that are not files in a panel. The changed-files example shows git's changed files, the state first. This is the lister table from its manifest:

```toml
[[lister]]
id = "changes"
name = "Changed files"
activate = "diff"

[[lister.columns]]
id = "state"
header = "State"
kind = "text"
sorts = true

[[lister.columns]]
id = "path"
header = "Path"
kind = "text"
sorts = true

[[lister.action]]
key = "alt+s"
command = "stage"
selection = "many"
```

The command `show` asks the active panel where it is with `panel/get`, runs git through `tools/run`, and opens the lister with `lister/open`. Each row's name puts the state first, "Modified, src/lib.rs", so the meaning is in the first cells of the Braille line. Enter runs the `activate` command on the row. The rows are sortable, filterable and markable like any rows, and a built-in file command on one answers "This row is not a file." because the row is not one.

Build it with `diro plugin new my-changes --language python --kind lister`.

## Places

A provider serves a file system at a name of its own, such as `memfs:/`. The memory-files example is the contract in one small file, with the data in a dictionary, so quitting diroxide loses it. The name says so, "Memory files (toy)". Its manifest declares what the place can do:

```toml
[[provider]]
scheme = "memfs"
name = "Memory files (toy)"
separator = "/"
case_sensitive = true
max_name_length = 255
reserved_names = []
writable = true
trash = false
links = false
attributes = false
watch = false
server_side_copy = false
atomic_rename = true
methods = [
  "connect",
  "list",
  "stat",
  "read",
  "write_begin",
  "write_chunk",
  "write_end",
  "create_dir",
  "rename",
  "remove",
]
lease = "listers"
```

- `methods` lists what the plugin implements. diroxide never calls one that is not listed.
- A write is a session. `write_begin` starts it, `write_chunk` sends pieces, and `write_end` with commit true replaces the file whole or with commit false leaves nothing. A crash in the middle leaves nothing either.
- `provider/list` answers in one go, or in batches as `$/progress` partial values for a big folder, so the first rows show while the rest arrive.
- `lease` says what keeps the plugin alive while a panel shows it.
- When the place is lost, say so with `provider/lost` and the reason in words.

Build one with `diro plugin new my-files --language python --kind provider`.

## Previewers and converters

These need no code. They declare a program and its arguments, and the host runs them without a shell.

The previewer example runs `pdftotext` on the first page of a PDF:

```toml
[plugin]
id = "pdf-first-page"
name = "PDF first page, as text"
version = "0.1.0"
description = "Shows the first page of a PDF as text, through pdftotext."
license = "Apache-2.0 OR MIT"
protocol = 1
min_host = "0.1.0-alpha.1"

[permissions.declared]
network = "no network"
files = "only the file it previews"
programs = "pdftotext only"

[[previewer]]
id = "first_page"
name = "PDF first page, as text"
priority = "normal"
needs = ["pdftotext"]
argv = ["pdftotext", "-l", "1", "-layout", "{input}", "-"]

[previewer.match]
mime = ["application/pdf"]
extensions = ["pdf"]
```

```bash
diro plugin preview pdf-first-page first_page notes.pdf
```

This runs the recipe the way the preview region will. It runs the program on the file, removes escape sequences, cuts the output at 64 KB, and prints the first line with words, which is the summary, and then the body. A failure prints its reason in words, for example "Could not preview fake.pdf: pdftotext ended with code 1." and then "Last error:" with the program's last line.

The converter example runs `pandoc` and gives it a staging path as `{output}`:

```toml
[plugin]
id = "markdown-to-word"
name = "Markdown to Word"
version = "0.1.0"
description = "Converts Markdown files to Word documents, through pandoc."
license = "Apache-2.0 OR MIT"
protocol = 1
min_host = "0.1.0-alpha.1"

[permissions.declared]
network = "no network"
files = "the file it converts, and the document it writes"
programs = "pandoc only"

[[converter]]
id = "md_to_docx"
name = "Markdown to Word"
needs = ["pandoc"]
argv = ["pandoc", "{input}", "-o", "{output}"]

[converter.from]
extensions = ["md", "markdown"]

[[converter.to]]
id = "docx"
name = "Word document"
extension = "docx"
```

```bash
diro plugin convert markdown-to-word md_to_docx notes.md
```

The host renames the staged output into place only when the program succeeds, so a failed or refused conversion leaves nothing. You hear "Converted: notes.md to notes.docx." An existing output is not overwritten without a question: "Not converted: notes.docx exists." when it cannot ask. `--to FOLDER` puts the result in another folder.

The preview region and the conversions dialog come with a later alpha. Until then these two commands are how you try a recipe.

## The conformance checks

```bash
diro plugin conformance line-counter
```

This plays diroxide's side against your plugin, starting it the way diroxide does, and prints a line for each group:

```text
Pass: Manifest
Pass: Accessibility
Pass: Lifecycle
Pass: Framing
Pass: Methods
Pass: Commands
Pass: Cancel
Pass: Settings
Skipped: Providers: none declared
Skipped: Hooks: none declared
Pass: Services
  Note: it called no services
Pass: Output hygiene
Conformance: 10 passed, 0 failed, 2 skipped (Line counter)
```

The exit code is 0 when every group that ran passed. A failure names the check, and the sentence says what to fix.

- Manifest: the same rules as `diro plugin check`.
- Accessibility: names, summaries and labels fit one 40-cell line, with the meaning first.
- Lifecycle: `initialize` is answered with the protocol and the capabilities, a request before it gets -32002, `shutdown` is answered with null, and the program exits with code 0, and also when its input ends.
- Framing: stray lines are tolerated, a line that starts with `{` and is not JSON gets -32700, a batch gets -32600, and the reply is flushed after every message.
- Methods: an unknown request gets -32601, an unknown notification is ignored, and unknown fields are ignored.
- Commands: each declared command runs with no selection, one file, many files, a name that is not Unicode, a name with a line break and a name that starts with a dash.
- Cancel: a cancel is answered, and progress goes `begin`, `report`, `end` in order.
- Settings: `settings/changed` is applied.
- Providers and hooks, when declared.
- Services: only the services the manifest allows are called.
- Output hygiene: stray lines on standard output are counted and reported, but never fatal.

A plugin with no code of its own runs the manifest and accessibility groups and skips the rest, which need a process.

## The nine examples

They are in the `examples/plugins` folder of the source, and in the optional component "Example plugins". The skeletons are copied from them. Run `diro plugin check` on any of them to see it pass, and `diro plugin conformance` on any of the folders.

1. Total size of selection (`scripts/total-size.sh`). A short bash script that reads the selection list and says how many items there are and how big they are. It shows the header, the NUL list, the directives file and the exit codes. [Script commands](script-commands.md) takes it apart.
2. Select empty folders (`scripts/select-empty-folders.sh`). Marks the empty folders of the active panel with `select` directives, which wait until the script ends so the panel does not move while you listen.
3. Line counter (`line-counter`). The protocol example above: settings, cancel, progress, error kinds and a per-file result list, standard library only.
4. Search the whole computer (`system-index`). Three commands and no code. Everything's `es` on Windows, `plocate` on Linux and `mdfind` on macOS take a typed query, and the output lines become a list of real files in a panel. It shows `prompt`, `mode = "lister"`, `list`, `max_rows`, `os`, `needs` and `exit_words`. You hear "214 results for report, from Everything", or "No results for report.", or "First 5,000 shown, narrow the search.".
5. Compare in a diff tool (`compare-files`). Two commands and no code, with a setting that holds your diff program and its options. It shows `mode = "launch"` and the placeholders `{file}`, `{other_file}`, `{files}` and `{setting.tool}`. A terminal diff such as `vimdiff` needs `mode = "terminal-wait"`.
6. Changed files (`changed-files`). The lister example above: rows that are not files, Enter bound to a command, a context action, raw tool output through `tools/run`, and a text document with `ui/show_text`.
7. Memory files, toy (`memfs`). The provider example above.
8. PDF first page, as text (`pdf-first-page`). A previewer with no code.
9. Markdown to Word (`markdown-to-word`). A converter with no code.

To try one, install it from a copy of the folder and run it once. For the script that marks empty folders, from the folder that holds it:

```bash
diro plugin install select-empty-folders.sh
```

Then `diro plugin run select-empty-folders` asks "Run it once? y or n" and runs it. Nothing is turned on.

## Write plugins that read well by ear

These are the rules every plugin shipped with diroxide keeps. The conformance checks look at the first one.

- Put the meaning in the first 40 characters of every name, description and message. A Braille line is 40 cells, and a screen reader says the start first.
- Use words before symbols, and never let a color or a symbol carry the meaning alone. Write "Modified, src/lib.rs", not an icon. Emoji and box drawing come out as noise on a Braille display.
- Never put your plugin's name in the text. diroxide adds it last, so a plugin never sounds like diroxide.
- Say the cause in `error.message`, meaning first, and never an exit code alone.
- Keep short lines. Keep a status for a long job as `$/progress`, not as a flood of announcements. More than a few announcements a second are folded into "12 more messages from Line counter".

And for safety and privacy:

- Run programs with an argument list through `tools/run`. There is no shell, so a name with spaces, quotes or `$(...)` is inert. Put `--` before names, or use `./` in front of a name that starts with a dash.
- Take file names from the entries diroxide gives you, as bytes if you must. Do not parse them out of text.
- Do not remove anything with your own code if `fs/delete` will do it. It lists first, asks, uses the trash and can be undone.
- Do not log secrets, tokens or file contents. Do not collect names, user names or computer names, and do not send anything off the computer without saying so in `permissions.declared`.
- Say what you really do under `permissions.declared`. People decide on those words.

## See also

- [Plugins](plugins.md)
- [Script commands](script-commands.md)
- [Optional components](components.md)
- [Security](security.md)
- [The plugin protocol reference](dev/plugin-protocol.md)
- [The plugin protocol decision](adr/0006-the-plugin-protocol.md)
- [Settings and where the files live](settings.md)
- [The documentation index](README.md)
