# Hooks

This guide covers hooks: how a plugin can be asked before a copy, a move, a delete, a rename or a new folder, and stop it; how it is told afterward; how it hears about folders you enter; what you hear when a hook stops something or does not answer; and the Folder guard example that you can turn on and try.

What this guide quotes is what the program says. Each sentence is checked in tests against the 40 cells of a Braille line, and each is written so its meaning comes first. None of it has been heard yet on NVDA, JAWS or a Braille display. If your experience differs, that is a finding.

A hook belongs to a plugin, so [plugins](plugins.md) comes first: what a plugin is, and how you turn one on. [Writing a plugin](writing-a-plugin.md) is for the person who makes one.

## What a hook is

A hook is a promise in a plugin's manifest: "tell me when this happens". It comes in two kinds.

- A before hook is asked. diroxide holds the job, sends the plugin the plan, and waits a moment for the answer: go on, or stop with a reason. It can only stop a job. It cannot change the plan.
- An after hook is told. diroxide sends the plugin what happened and does not wait.

Nothing runs until you turn the plugin on, and the turn-on question does not list hooks, because a hook gives a plugin no new power over your files. What a hook does give it is knowledge: it hears the paths of the jobs it hooks. [Privacy](privacy.md) says so.

## The events

A manifest names an event in a `[[hook]]` table:

- `before_copy` and `after_copy`: a copy, or a duplicate.
- `before_move` and `after_move`.
- `before_delete` and `after_delete`: a move to the trash, or a delete for good.
- `before_rename` and `after_rename`.
- `before_mkdir` and `after_mkdir`: a new folder.
- `enter_folder`: the active panel shows a new folder.
- `select`: the marks or the focused row of the active panel changed.

`enter_folder` and `select` are told, never asked. The manifest also accepts `folder_changed`, and diroxide does not send it yet.

### When a before hook runs

It runs after the plan and before the question, so a stop spares you the question. It runs on a thread of its own, never the one that reads your keys, so a slow hook cannot freeze the panels.

A job that waits in the queue runs its before hooks when it is made, not when it gets its turn, and its after hooks when it ends. [The transfer queue](transfer-queue.md) says how jobs wait.

### Which jobs wake a hook

A plugin is woken only for what its `[[hook]]` table asks for. These keys narrow it:

- `schemes`: places by scheme, such as `["sftp"]`. Empty means every place.
- `glob`: a pattern on the place, with `/` between folders, such as `**/*.iso`.
- `kind`: `file` or `folder`.

The filters look at each item at the place it is now: for a copy or a move, where it comes from. A job wakes a hook if any one of its items matches.

## What a before hook is sent

The plugin gets one request, `hook/before`, as a line of JSON:

```text
{"jsonrpc":"2.0","id":7,"method":"hook/before","params":{"event":"before_delete","job":12,"items":[{"from":"D:\\keep\\a.txt","kind":"file"}],"count":1,"summary":"Delete 1 file to the trash"}}
```

- `event` is the event's name, and `job` is the job's number.
- `items` is the first thousand items, each with its `from`, its `to` for a copy, a move or a rename, and its `kind`, `file` or `folder`. `count` says how many there are in all.
- `summary` is the plan in words, such as "Delete 1 file to the trash", and `destination` names the destination folder for a copy or a move.

It answers with a line of JSON, either go on:

```text
{"jsonrpc":"2.0","id":7,"result":{"action":"continue"}}
```

or stop, with the reason in words, meaning first:

```text
{"jsonrpc":"2.0","id":7,"result":{"action":"cancel","reason":"folder is protected"}}
```

## What an after hook is sent

`hook/after` is a notification that carries the job's `event`, its `job` number, its `status` (`done`, `failed` or `cancelled`) and the same list of items, each with a `result`: `done`, `skipped` or `failed`.

For `enter_folder` and `select`, it carries the folder in `place`. For `select` it also carries `selection_count`, how many rows are marked, and `cursor`, the name of the focused row.

Navigation hooks are debounced. diroxide waits until the panels have been still for about a third of a second and sends the last of each kind once, so holding down an arrow key does not send a message for every row.

## What you hear

A stop reads the reason first and the plugin's name last, like everything a plugin causes:

```text
Delete stopped: folder is protected. By Folder guard.
```

The first sentence is 36 cells long and carries the point. The plugin's reason is cleaned, cut to a length and given its full stop by diroxide. A plugin that gives no reason gets "Copy stopped: no reason given. By Folder guard." A veto is not a failure: diroxide does not wrap it in "Could not delete".

A hook that does not answer is never a quiet veto and never a quiet pass. If it takes longer than its time, or fails, or answers in a way diroxide cannot read, the manifest's `on_timeout` decides, and diroxide says which:

```text
Delete goes on: no answer. By Folder guard.
Delete stopped: no answer. By Folder guard.
```

- `on_timeout = "continue"` is the default. The job goes on, and the first line is said as a warning.
- `on_timeout = "cancel"` stops the job, and the second line is said.

`on_timeout` belongs to before hooks only. `diro plugin check` says so if an after hook has it.

### How long a hook has

Two seconds. The plugin can set another for all its hooks in `[timeouts]`, as `hook`, or for one hook with `timeout`. diroxide waits a little longer than that, so the plugin host's own report of a slow plugin arrives first.

A plugin that crashes or floods is handled as any plugin is. [Plugins](plugins.md) lists the lines.

## A manifest with hooks

This one stops a copy of any `.iso` file, and wants to hear about every copy afterward. It passes `diro plugin check`.

```toml
[plugin]
id = "copy-watch"
name = "Copy watch"
version = "0.1.0"
description = "Stops disc images from being copied, and logs every copy."
license = "Apache-2.0 OR MIT"
protocol = 1
min_host = "0.1.0-alpha.2"

[run]
command = ["python3", "main.py"]

[run.windows]
command = ["py", "-3", "main.py"]

[timeouts]
hook = "2s"

[permissions]
fs = "none"
panel = "none"

[permissions.declared]
network = "no network"
files = "reads no files"
programs = "no other programs"

# Asked before a copy, after the plan and before the question.
[[hook]]
event = "before_copy"
glob = "**/*.iso"
kind = "file"
on_timeout = "cancel"
timeout = "5s"

# Told after every copy.
[[hook]]
event = "after_copy"
```

## The Folder guard example

Folder guard is a small Python plugin that comes with diroxide in the example plugins. It stops a delete inside the folders you protect, before the question is asked, and it writes every delete to its log. It uses Python 3 and nothing else, no network, and reads no files.

Get the example plugins as [optional components](components.md) describes, or copy the `folder-guard` folder from the source tree's `examples/plugins`. From the source tree:

```bash
diro plugin check examples/plugins/folder-guard
```

```bash
diro plugin install examples/plugins/folder-guard
```

```bash
diro plugin enable folder-guard
```

```text
Enable Folder guard, version 0.1.0?
It runs: py -3 main.py
The plugin states, unchecked:
no network.
reads no files, writes only its own log.
no other programs.
While it runs it can do what you can.
Enable it? y or n
```

On Linux and macOS the second line reads "It runs: python3 main.py". It asks for no rights over files or panels, so there is no "diroxide checks:" row.

To protect a folder, open the Plugins menu (Alt+P), Manage plugins, Folder guard, Settings, and type the folder's full path in the Protected setting. Several folders are separated by semicolons. The value is kept as you type it, and the guard compares full paths, without case on Windows.

Now press F8 on a file inside that folder:

```text
Delete stopped: folder is protected. By Folder guard.
```

The file is still there, and no question opened. A delete anywhere else goes on as usual. A place on a server is not the guard's to judge, so it lets those go.

The same stop applies to Shift+Delete, because it is a delete too.

To read what the guard saw:

```bash
diro plugin log folder-guard
```

Its own lines read "Stopped a delete in a protected folder: PATH" and, after each delete that ran, "Delete done: PATH, done". A plugin's log can hold paths, so read it before you share it.

Its tests run in its folder, without diroxide:

```bash
py -3 -I -m unittest
```

On Linux and macOS use `python3 -I -m unittest`.

Read `main.py` before you turn it on. It is short, and the lines to change for a guard of your own are marked CHANGE.

## What a hook cannot do

- It cannot change the plan, only stop the job.
- It cannot stop a job that nobody hooks. A plugin you did not turn on is never asked.
- It is not a security boundary. A plugin that is turned on can do anything you can do. A guard is a seat belt for your own mistakes, not a lock against a program that means harm. [Security](security.md) says what a plugin could still do.
- Hooks are asked one at a time, plugins in the order of their ids, and the first stop ends the asking. Any plugin that hooks a job can stop it.

## See also

- [Plugins](plugins.md)
- [Writing a plugin](writing-a-plugin.md)
- [The transfer queue](transfer-queue.md)
- [Operations and undo](operations.md)
- [Security](security.md)
- [Privacy](privacy.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
