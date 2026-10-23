# Script commands

This guide covers script commands: one file in bash, Python, PowerShell or a batch file that becomes a command in diroxide. It covers how to make one, the header, what the script is given, how it talks back, the output modes, exit codes, which program runs it on each system, what PowerShell's policy override means, and commands with no script at all.

A script command is the smallest plugin. It has no protocol to learn. If you need dialogs, settings that change while it runs, lists of rows that are not files or file systems, read [writing a plugin](writing-a-plugin.md) instead. If you want to run or turn on plugins, read [plugins](plugins.md).

Every command line and every example script in this guide was run, and every script passes `diro plugin check`. The sentences quoted are the ones the program says.

## What a script command is

- A script is one file in the `plugins/scripts` folder of your config folder.
- Its extension picks the program that runs it: `.sh` and `.bash` with bash, `.py` with Python 3, `.ps1` with PowerShell, and `.cmd` and `.bat` on Windows as themselves. On Linux and macOS any other file runs directly when it is executable, so the system reads its first line.
- Its first lines are a header of comments that names the command and says how it runs.
- The file's name without its extension is the script's id: lowercase letters, digits, `-` and `_`, from 2 to 40 characters, starting and ending with a letter or a digit. A name with anything else is refused with the first bad character spelled out, for example "U+0020 is not allowed in a name", because a name that needs quoting is how files go wrong.
- Nothing runs until you turn it on. Scripts run only from `plugins/scripts`, never from the current folder and never from an archive.
- On Linux and macOS, diroxide refuses to start a script when its file or its folder can be changed by anyone but you. It says "others can change its file" or "others can change its folder".

## Make one

```bash
diro plugin new total-size --language bash
```

```text
Wrote total-size.sh in the scripts folder.
Check it: diro plugin check total-size
Try it once: diro plugin run total-size -- FILE
```

(The first line names the full folder.) It writes a working script, copied from the shipped example, with the header explained in comments. The command is named "Total size" until you change the `name` line.

```bash
diro plugin check total-size
```

Says "Checked 1 plugin: no problems." or every problem, the line first and then the fix.

```bash
diro plugin run total-size -- a.txt b.txt
```

Runs it once on the two files, after one question, "Run it once? y or n". The files after the two dashes are the selection. You hear:

```text
Selected: 2, size: 2.0 KB. By Total size.
```

```bash
diro plugin enable total-size
```

Shows what it runs and asks, as [plugins](plugins.md) describes, starting on No. After a yes, the command is in the Plugins menu, the command palette and the banks, and it can have a key.

## The first example, line by line

This is `total-size.sh`, one of the nine examples, without its explaining comments. It says how many items are selected and how big they are:

```bash
#!/usr/bin/env bash
# diro: name = "Total size of selection"
# diro: description = "Says how many items are selected and how big they are."
# diro: selection = "any"
# diro: selection_file = "nul"
# diro: places = "local"
# diro: timeout = "5m"

directives="${DIRO_DIRECTIVES:?}"
all="${DIRO_SELECTION_COUNT:-0}"
count=0
skipped=0
total_kb=0

while IFS= read -r -d '' name; do
  count=$((count + 1))
  if size=$(du -sk -- "$name" 2>/dev/null); then
    total_kb=$((total_kb + ${size%%[!0-9]*}))
  else
    skipped=$((skipped + 1))
  fi
  if [ "$all" -gt 50 ] && [ $((count % 50)) -eq 0 ]; then
    printf 'progress %d of %d\n' "$count" "$all" >> "$directives"
  fi
done < "${DIRO_SELECTION_FILE0:?}"

# The size in words, with whole-number arithmetic only: du counts KiB.
bytes=$((total_kb * 1024))
if [ "$bytes" -lt 1000 ]; then
  size="$bytes bytes"
elif [ "$bytes" -lt 1000000 ]; then
  size="$((bytes / 1000)).$((bytes % 1000 / 100)) KB"
elif [ "$bytes" -lt 1000000000 ]; then
  size="$((bytes / 1000000)).$((bytes % 1000000 / 100000)) MB"
else
  size="$((bytes / 1000000000)).$((bytes % 1000000000 / 100000000)) GB"
fi

if [ "$skipped" -gt 0 ]; then
  printf 'announce Selected: %d, size: %s\nwarn Skipped %d items it could not read\n' \
    "$count" "$size" "$skipped" >> "$directives"
  exit 3
fi
printf 'announce Selected: %d, size: %s\n' "$count" "$size" >> "$directives"
exit 0
```

What to notice:

- The header says the script needs any number of items, reads the NUL list, works on local places only, and may run for five minutes.
- It reads the list with a loop that reads up to each NUL byte, from a redirected file and never from a pipe. A pipe would run the loop in a subshell and lose the counts. The loop works for any name, a name with a line break included, and runs on the bash 3.2 that macOS ships.
- It writes the size with whole-number arithmetic only, because `du` counts in KiB and macOS has no `numfmt`.
- It writes both directives of the partly-done case with one `printf`, so lines never interleave, and ends with exit code 3, partly done, when `du` could not read a name.
- It puts `--` before the name, so a name that starts with a dash is not read as an option.
- It writes every variable in a path as `${VAR:?}`, so an empty one stops the script instead of naming the wrong folder.
- It does not use `set -e`, because a file it cannot read is counted and said, not fatal.
- It removes nothing.

## The header

The header is the `diro:` lines in the comments at the top of the file.

- It is read from the first 40 lines. A first line that starts with `#!` is skipped, and so is `@echo off` in a batch file. Blank lines and ordinary comments are passed over. Reading stops at the first line that is neither.
- A header line is a comment mark, then `diro:`, then `key = value`. The comment marks are `#`, `//`, `--`, `::` and `REM` in any case, so the same header works in a shell script, a PowerShell script, a Python script and a batch file.
- The value is a TOML value: text in quotes, a whole number, `true` or `false`, a list, or a small table. Text without quotes is an error that says what to do: "Put the value in quotes".
- An unknown key is a warning and is ignored, so a script written for a newer diroxide still runs in an older one.

Here is what `diro plugin check` says about a header with mistakes:

```text
Error, name (line 2): is not in quotes. Put the value in quotes: "Bad header". (bad-header)
Error, selection (line 3): "several" is not a known value. Use one of: none, one, two, any, many. (bad-header)
Error, prompt (line 6): asks a question, and the command is unattended. Remove prompt, or remove unattended. (bad-header)
Error, timeout: is not set, and an unattended command must end. Add a timeout such as timeout = "5m". (bad-header)
Warning, colour (line 4): is not a header key, so it is ignored. Check its spelling, or remove the line. (bad-header)
Checked 1 plugin: 4 errors, 1 warning.
```

### The keys

About the command:

- `name`: what the menu and the palette show. Put the meaning first, within 40 characters. Without it the file's name is used, with underscores read as spaces.
- `description`: one sentence for the help key. Say what it does first, and do not start with "This script".
- `key`: a key you propose, such as `"ctrl+alt+t"`. It binds only when it is free, and it shows as proposed. You can always change it in the Keys list.
- `menu`: where it shows. The Plugins menu when absent.

What it needs:

- `selection`: how many items it takes. `none`, `one`, `two`, `any` (the default) or `many`, which is two or more. `any` takes the marked items, or the cursor's item when nothing is marked.
- `cursor_fallback`: `true` by default. When it is `false` and nothing is marked, diroxide says "Mark something first: this command will not use the cursor row" and does not run it. Set it to `false` for anything that changes or removes.
- `places`: `any` or `local`. A `local` command is shown disabled in a place that is not on this computer.
- `needs`: a list of tools it needs, such as `["pandoc"]`. A command whose tool is missing is shown disabled with its reason, "Needs pandoc, not found." Each tool found is also given to the script as `DIRO_TOOL_PANDOC`.
- `os`: a list of `windows`, `linux` and `macos`. The command exists only there.
- `protocol`: the protocol major, 1.

How it runs:

- `mode`: where the output goes. See below. `output-pane` is the default.
- `per`: `selection` (the default) runs the script once for all items, and `entry` runs it once for each item, with `DIRO_CURSOR` set to that item.
- `stdin`: `json` (the default) gives the script the context as JSON on standard input, then the end of input. `none` gives it nothing.
- `cwd`: the folder it starts in. `active` (the default), `other` or `script`.
- `timeout`: the longest it may run, such as `"30s"` or `"5m"`. None by default. The run is cancelled when it passes.
- `interpreter`: a list that replaces the choice by extension, such as `["python3", "-I"]`. The program comes first and then its arguments.

Questions:

- `confirm`: a question diroxide asks first. It starts on No.
- `prompt`: one line diroxide asks first, with the history and completion of other text entries. The answer arrives as `DIRO_INPUT` and as `{input}`. An empty answer cancels the run.

Promises:

- `dry_run`: `"supported"` promises that when `DIRO_DRY_RUN=1` is set, the script prints what it would do, changes nothing and exits 0. A command that makes this promise also gets a "(preview)" entry in the palette. A script without the key gets none, because diroxide cannot look inside a script and never claims a preview it cannot guarantee.
- `unattended`: `"ok"` promises that the script never asks (no `prompt`, no `confirm`, no terminal mode and no reading of standard input), that it always ends within its `timeout`, that it honors `DIRO_UNATTENDED=1`, and that under `DIRO_DRY_RUN=1` it changes nothing. `diro plugin check` fails a script that says `unattended` and also sets a prompt, a question, a terminal mode or no timeout. Only such a script may be run by a saved task with nobody there, and the consent question then adds "It can run without asking."

Reading and talking back:

- `selection_file`: `lines` or `nul`. Which selection file the script reads. See below.
- `stdout_directives`: `true` lets lines on standard output that start with exactly `diro:` act as directives. Off by default, so a script that lists a folder can print a file called `diro:cd /elsewhere` without steering the panel.
- `exit_words`: sentences for exit codes that mean something, such as `{ 8 = "Everything is not running. Start it, then try again." }`.
- `list` and `max_rows`: for the `lister` mode. `list` is `lines` or `nul`, and `max_rows` is the most rows kept, 5,000 by default and up to 100,000.

## What the script is given

The script starts in the folder `cwd` names, with no shell, no console window and a private run folder that diroxide removes when the run ends. Variables that start with `DIRO_` and were already set are removed first, and these are set:

- `DIRO_ACTIVE` and `DIRO_OTHER`: the places of the active panel and the other panel. The other panel is the destination, as in a copy.
- `DIRO_CURSOR` and `DIRO_CURSOR_NAME`: the cursor's entry, and its bare name. `DIRO_OTHER_CURSOR` is the other panel's.
- `DIRO_SELECTION_COUNT`: how many items are selected. `DIRO_SELECTION_FROM_CURSOR` is `1` when the cursor's item stood in because nothing was marked.
- `DIRO_SELECTION_FILE0` and `DIRO_SELECTION_FILE`: the selection files, below.
- `DIRO_CONTEXT_FILE`: the context as JSON, the same text that arrives on standard input.
- `DIRO_DIRECTIVES`: the file the script writes directives to.
- `DIRO_CANCEL_FILE`: a file that appears when you cancel.
- `DIRO_TEMP`: a private folder for scratch files, removed afterward. Use it and nothing outside it.
- `DIRO_DATA_DIR`: a folder that stays between runs, for the script's own memory.
- `DIRO_INPUT`: the answer to `prompt`.
- `DIRO_DRY_RUN` and `DIRO_UNATTENDED`: set to `1` for a preview run and an unattended run.
- `DIRO_COMMAND`, `DIRO_SCRIPT`, `DIRO_VERSION`, `DIRO_PROTOCOL`, `DIRO_LOCALE`, `DIRO_ACTIVE_PROVIDER` and `DIRO_OTHER_PROVIDER`: facts about the run.
- `DIRO_LEVEL`: 1 in a program that diroxide started, and one more when it is already set, so a script that starts diroxide can tell how deep it is.
- `DIRO_TOOL_NAME`: the full path of each tool in `needs`.

On Windows, a script run by Git's bash gets every place with forward slashes, such as `D:/photos/a.jpg`, which both Windows and bash read.

### The selection files

There are two, and they hold the same places.

- `DIRO_SELECTION_FILE0` has each place ended by a NUL byte. This form holds any name, because a NUL is the one byte a name cannot hold. Read it with a loop that reads up to each NUL, as the example does.
- `DIRO_SELECTION_FILE` has one place a line. A name with a line break cannot be written there. When a selection holds one, a script that reads this file is refused with a sentence, and a script that says `selection_file = "nul"` runs, with this variable left empty so it is not read by mistake.

Say `selection_file = "nul"` in the header whenever the script reads the NUL file. Never loop over `ls`, never use `xargs` without its NUL option, and never build a path from a name without `--`.

### The context

The same JSON arrives on standard input and in `DIRO_CONTEXT_FILE`. It has the protocol and the command's id, the locale, `active` and `other` with their place and provider, the `cursor`, the `selection` (each entry with `name`, `place` and `kind`), `selection_count`, `selection_from_cursor` and `selection_truncated`. The JSON carries at most 10,000 entries. The selection files are never cut. A Python script can read the JSON, and a bash script can ignore it and use the variables.

## Directives

A directive is a request to diroxide from a running script. The script appends lines to the file named by `DIRO_DIRECTIVES`, one a line, in UTF-8, as `VERB` or `VERB text`. A file cannot collide with output: a script that lists a folder prints names, and a file called `diro:cd /elsewhere` cannot steer the panel.

Write each directive with one `printf` or one write, so lines never interleave.

These act at once:

- `announce TEXT`, `warn TEXT` and `error TEXT`: say the text at that importance. The text is cleaned, and the script's name is added last: "Selected: 2, size: 2.0 KB. By Total size." `error` says an error but does not change the exit code.
- `progress N of M` and `progress N of M TEXT`: set the job's progress sentence, "12 of 300, copying (Total size)".
- `refresh`: list both panels again.

These wait until the script ends, because a panel that moves while you listen is hard to follow by ear:

- `cd PLACE` and `cd_other PLACE`: open a place in the active or the other panel.
- `select PLACE` and `unselect PLACE`: mark or unmark an entry.
- `cursor PLACE`: move the cursor to an entry.

An unknown verb is logged and ignored. A line longer than 8 KiB is ignored. A directive line is one line, so a name with a line break cannot be passed to `select`; the second example leaves such names out and says so.

### The second example: select empty folders

This script marks the empty folders of the active panel, and shows a directive that waits:

```bash
#!/usr/bin/env bash
# diro: name = "Select empty folders"
# diro: description = "Marks the empty folders of the active panel."
# diro: selection = "none"
# diro: places = "local"
# diro: timeout = "1m"

directives="${DIRO_DIRECTIVES:?}"
found="${DIRO_TEMP:?}/empty-folders"
newline='
'

if ! find "${DIRO_ACTIVE:?}" -mindepth 1 -maxdepth 1 -type d -empty -print0 > "$found"; then
  printf 'error Could not look through this folder\n' >> "$directives"
  exit 1
fi

count=0
skipped=0
while IFS= read -r -d '' folder; do
  case "$folder" in
    *"$newline"*)
      skipped=$((skipped + 1))
      continue
      ;;
  esac
  printf 'select %s\n' "$folder" >> "$directives"
  count=$((count + 1))
done < "$found"

if [ "$skipped" -gt 0 ]; then
  printf 'warn Left out %d folders whose names have a line break\n' "$skipped" >> "$directives"
fi
case "$count" in
  0) printf 'announce No empty folders here\n' >> "$directives" ;;
  1) printf 'announce 1 empty folder selected\n' >> "$directives" ;;
  *) printf 'announce %d empty folders selected\n' "$count" >> "$directives" ;;
esac
exit 0
```

It needs no selection, so `selection` is `none`. It changes only the marks, which you can clear with the usual key, so it declares no preview. It writes `find`'s output to a file in `DIRO_TEMP` and reads that file through a redirect, never through a pipe.

## Output modes

The `mode` key says where a script's output lines go.

- `output-pane` (the default): each line shows as it comes.
- `run`: the output is kept in the command's log and shown only if the script fails.
- `output-viewer`: the output is shown in the viewer when the script ends.
- `output-column`: the script runs once for each item, and its first output line becomes that item's custom column.
- `terminal`: the screen is handed over to the script, then given back. For a program that needs the terminal, such as an editor.
- `terminal-wait`: as `terminal`, then diroxide says "Press Enter to go back." before it returns.
- `lister`: each output line is a place, and the places open as a list of real files. View, copy and the filter work on them as on any panel. `list` says whether the places end in a line break or a NUL byte, and `max_rows` caps the list. A list cut at its cap says "First 5,000 shown, narrow the search." The title puts the count first: "214 results for report, from Everything". A place that no longer exists stays in the list as "Missing, report-old.pdf".
- `launch`: diroxide starts the program on its own, so it stays open after diroxide returns, and answers at once with "Started meld." Use it for a diff tool or an editor.

Output is cleaned of escape sequences and control characters, a line is cut at 2,000 characters, and the whole output at 1 MB. "Output cut at 1 MB." says it happened.

From a shell, `diro plugin run` prints the lines of any mode. A script in `lister` mode prints its places and then a title line:

```text
D:/notes/a.txt
D:/notes/b.txt
2 results, from Find text files
```

## Exit codes, cancel and time

The exit code is the status. diroxide says it in words:

- 0: "Total size done." or whatever the script announced.
- 1: "Could not run Total size: code 1, nothing changed." and then "Last error:" with the last line the script wrote to standard error.
- 2: "Could not run Total size: code 2, wrong use."
- 3: "Total size partly done: code 3." The script's own output says which.
- 64 to 113 are free for a script's own meanings. Give them sentences with `exit_words`, and the sentence ends with the script's name: "The two files are the same size. By Compare sizes."
- 126: "Could not run Total size: it is not executable."
- 127: "Could not run Total size: a program was not found."
- A script stopped by a signal on Linux or macOS: "Could not run Total size: it was stopped by a signal."

`diro plugin run` passes 0, 1, 2 and 3 on as its own exit code and says 1 for every other.

A cancel makes the file named by `DIRO_CANCEL_FILE` appear. A long script can look for it now and then and stop cleanly. After three seconds diroxide ends the script and every program it started, on every system. The jobs line says "Total size: cancelled."

A script that passes its `timeout` is ended the same way, and you hear "Total size timed out after 5 minutes."

## Which program runs a script

diroxide picks the program for the extension, and `--explain` says which. `interpreter` in the header replaces the choice.

- Bash: `.sh` and `.bash`, with `--noprofile --norc`. On Windows it is Git for Windows' bash, found beside `git` or under Program Files. The `bash.exe` in the system folder is WSL's launcher, which runs Linux with `/mnt/c` paths and breaks every path diroxide passes, so it is refused with words: "Only WSL's bash was found; it breaks paths. Install Git for Windows." With no bash: "bash was not found. Install Git for Windows, which brings it."
- Python: `.py`, with `-X utf8 -u` and, from Python 3.11, `-P`, so a script named `json.py` in the scripts folder cannot shadow the standard library for its neighbors. On Windows the `py -3` launcher comes first. Anything in a `WindowsApps` folder is refused without being run, because it is often the Store's stub that opens the Store. Then you hear "Python is not installed; the Store stub was found. Install Python, or use a bash script command." With no Python at all: "Python 3 was not found. Install Python 3, or name one in the interpreter key."
- PowerShell: `.ps1`, with `pwsh` (PowerShell 7) first and Windows PowerShell 5.1 after it, run with `-NoProfile -NonInteractive`. With none: "PowerShell was not found. Install PowerShell 7."
- Batch: `.cmd` and `.bat`, on Windows only. Elsewhere: "A .cmd or .bat script runs on Windows only."
- Another extension: on Linux and macOS it runs directly if it is executable. Otherwise: "No program runs this file. Name it .sh, .py or .ps1, or add an interpreter key."
- A program named in `interpreter` that is missing: "NAME was not found. Install it, or fix the interpreter key."

### PowerShell's policy

Windows keeps a policy that can refuse to run a script file. For a script you turned on, diroxide starts PowerShell with `-ExecutionPolicy Bypass` for that one process only. It changes nothing about your system's policy, and the consent question says so: "It overrides PowerShell's script policy." You turn the script on knowing that.

Three things to know when you write for PowerShell:

- Write the directives file without a byte order mark. Windows PowerShell 5.1 adds a mark with `Out-File -Encoding utf8`, and a mark at the start of the file spoils the first directive: you hear the later lines only, and the log says "unknown directive announce, ignored", with the mark hidden in front of the word. The example below writes the file with .NET and no mark.
- Use `-LiteralPath` for paths, so brackets and wildcards in a name are plain characters.
- Say `selection_file = "nul"` and read the NUL file as bytes if a name could hold a line break.

This script passes `diro plugin check` and was run:

```powershell
# diro: name = "Count items"
# diro: description = "Says how many items are selected."
# diro: selection = "any"
$text = "announce Items selected: $($env:DIRO_SELECTION_COUNT)`n"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::AppendAllText($env:DIRO_DIRECTIVES, $text, $utf8)
```

Its run says "Items selected: 2. By Count items." The consent question for it has the extra row about the policy.

## More examples

A Python script has the same header with the `#` mark. It reads the NUL list as bytes, so a name that is not Unicode survives, and it writes the directive with an explicit UTF-8 encoding and a `\n` line end:

```python
# diro: name = "Biggest file"
# diro: description = "Says which selected file is the biggest."
# diro: selection = "any"
# diro: selection_file = "nul"
# diro: places = "local"
import os

with open(os.environ["DIRO_SELECTION_FILE0"], "rb") as handle:
    names = [n for n in handle.read().split(b"\0") if n]

sizes = [(os.path.getsize(n), os.fsdecode(n)) for n in names if os.path.isfile(n)]
if not sizes:
    raise SystemExit(1)
size, name = max(sizes)
with open(os.environ["DIRO_DIRECTIVES"], "a", encoding="utf-8", newline="\n") as out:
    out.write("announce Biggest: %s, %d bytes\n" % (os.path.basename(name), size))
```

A script that needs exactly two files, with the cursor not standing in, and a sentence for an exit code of its own:

```bash
#!/usr/bin/env bash
# diro: name = "Compare sizes"
# diro: description = "Says which of two marked files is bigger."
# diro: selection = "two"
# diro: cursor_fallback = false
# diro: places = "local"
# diro: selection_file = "nul"
# diro: timeout = "30s"
# diro: exit_words = { 70 = "The two files are the same size." }
a=''; b=''
{ IFS= read -r -d '' a; IFS= read -r -d '' b; } < "${DIRO_SELECTION_FILE0:?}"
size_a=$(wc -c < "$a"); size_b=$(wc -c < "$b")
if [ "$size_a" -eq "$size_b" ]; then exit 70; fi
if [ "$size_a" -gt "$size_b" ]; then bigger="${a##*/}"; else bigger="${b##*/}"; fi
printf 'announce %s is bigger\n' "$bigger" >> "${DIRO_DIRECTIVES:?}"
```

With one file, it does not run: "Compare sizes needs two items; 1 is selected." With two of different sizes: "a.txt is bigger. By Compare sizes." With two of one size: "The two files are the same size. By Compare sizes."

A script that asks first. `confirm` is the question, `prompt` the typed line. Both are diroxide's own dialogs:

```bash
#!/usr/bin/env bash
# diro: name = "Ask first"
# diro: description = "Greets the name you type, after a question."
# diro: confirm = "Say hello to the name you type?"
# diro: prompt = "Name to greet:"
# diro: selection = "none"
printf 'announce Hello, %s\n' "${DIRO_INPUT:-nobody}" >> "${DIRO_DIRECTIVES:?}"
```

It asks "Say hello to the name you type? y or n", then "Name to greet:". An answer of "Ada Example" gives "Hello, Ada Example. By Ask first." An empty answer gives "Ask first: cancelled." From a shell, `diro plugin run` reads the typed line from standard input.

A script that lists files as a panel, in `lister` mode:

```bash
#!/usr/bin/env bash
# diro: name = "Find text files"
# diro: description = "Lists the text files in the active folder as a list of real files."
# diro: selection = "none"
# diro: places = "local"
# diro: mode = "lister"
# diro: list = "nul"
# diro: max_rows = 100
find "${DIRO_ACTIVE:?}" -maxdepth 1 -type f -name '*.txt' -print0
```

## Commands with no script

A command that only runs a program needs no script. It is a `[[command]]` table in a `plugin.toml` with an `argv`, a list of arguments, and a plugin of that kind has no code of its own. The compare-in-a-diff-tool example has two of them:

```toml
[[command]]
id = "compare_marked"
name = "Compare the two marked files"
description = "Opens the two marked files in your diff program."
selection = "two"
places = "local"
mode = "launch"
argv = ["{setting.tool}", "{files}"]
```

The command takes every key of a header, with the same meaning. The program is `argv[0]`, and diroxide finds it on the PATH or in your tool settings. There is no shell, so a name with spaces, quotes or `$(...)` is inert.

diroxide fills placeholders one argument at a time, after the list is split:

- `{file}`: the cursor's entry. `{files}`: every selected entry, each as its own argument, and it must stand alone as an argument. `{name}`: the cursor's bare name, with `./` in front when it starts with a dash.
- `{dir}`: the active panel's place. `{other}` and `{dest}`: the other panel's. `{other_file}`: the other panel's cursor entry.
- `{input}`: the answer to `prompt`, or a previewer's or converter's input file. `{output}`: a converter's staging path. `{max_rows}`: a lister's cap.
- `{selection_file}` and `{selection_file0}`: the two list files.
- `{setting.KEY}`: a plugin setting. One of type `argv` is a list and, like `{files}`, must stand alone.

A `{files}` list too long for one command line is split into batches, like `xargs`, and diroxide says "Running in 3 batches." Every other argument repeats in each batch. The line holds at most 32,000 characters on Windows and 128 KiB elsewhere.

`diro plugin check` lists any placeholder a command does not have: "{nope} is not a placeholder. Use one of: file, files, name, dir, other, dest, other_file, input, output, max_rows, selection_file, selection_file0, or setting.KEY." A placeholder with nothing to fill it is said when the command runs, for example "Choose tool in the plugin's settings first."

## Say what would run

```bash
diro plugin run total-size --explain -- a.txt b.txt
```

`--explain` says which program would run and with which arguments, and runs nothing. It works for scripts and for commands with no script. The paths are your own:

```text
Would run C:\Program Files\Git\bin\bash.exe with 3 arguments.
Argument 1: --noprofile
Argument 2: --norc
Argument 3: D:/tools/diroxide/config/plugins/scripts/total-size.sh
Nothing was run.
```

## Rules for scripts that behave

- Read lists from the NUL file. Never loop over `ls`, and never parse names out of text.
- Put `--` before names. Write every variable in a path as `${VAR:?}`.
- Remove nothing with the shell. A script that must remove files should set `cursor_fallback = false`, support `dry_run`, and use `diro trash` or a protocol plugin's `fs/delete`, which list first, ask, use the trash and can be undone.
- Keep scratch files in `DIRO_TEMP`, and nothing in the system's temporary folder or on any other drive.
- Write each directive with one write.
- Put the meaning first in everything you say. A Braille line is 40 cells. Do not put the script's name in your text, because diroxide adds it last.
- Words before symbols. No emoji and no color codes.
- Do not read secrets, tokens or personal files that the task does not need, and send nothing off the computer without saying so in the description.

## See also

- [Plugins](plugins.md)
- [Writing a plugin](writing-a-plugin.md)
- [Optional components](components.md)
- [Security](security.md)
- [Settings and where the files live](settings.md)
- [Customizing keys](customizing-keys.md)
- [The documentation index](README.md)
