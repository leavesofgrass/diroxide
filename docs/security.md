# Security

This page says plainly what a plugin, a script command and an optional component can and cannot do, what diroxide checks, what it protects even from a plugin, and what no limit in diroxide can stop. It is for people who turn plugins on and for people who write them.

The short version: a plugin is a program you chose to run. While it runs, it can do anything you can do. diroxide limits what a plugin may ask of diroxide, and checks those limits. It cannot limit what a program does on its own.

## What diroxide checks, and what it only shows

A plugin's manifest says what it wants. At the first turn-on, the question lists it in words, in two groups.

Checked by diroxide ("diroxide checks:"). The limits are enforced:

- Reading or changing files through diroxide: `fs` is none, read or write.
- Reading or changing the panels: `panel`.
- Running named programs through diroxide: `tools`, by name.
- Running diroxide commands: `commands`, by pattern.
- Starting detached programs: `launch`, by name.

A call outside these gets "Line counter may not do that." and "Its manifest does not allow it."

Always allowed, because they only show things: announcing, asking, showing text, opening a list and logging. Each is cleaned, cut to a length, limited in rate and given the plugin's name, as below.

Stated by the plugin and not checkable ("The plugin states, unchecked:"): whether it uses the network, which other files it touches, and which other programs it starts. These are the plugin's promise. diroxide shows the words and cannot check them.

## Nothing is on until you say so

- The first turn-on shows the question and starts on No. A plugin or script is turned on by name.
- diroxide records a fingerprint of what you agreed to: the SHA-256 of the manifest, and of the program file the plugin runs. For a script, of the whole file. If either changes, the plugin asks again: "Line counter changed since you enabled it." and "Enable it again? y or n". Code cannot change under a yes.
- Installing a plugin, downloading a component or unpacking a pack never turns anything on.
- `diro plugin run NAME` runs a plugin once without turning it on, after one question.
- `diro --no-plugins` starts nothing for that session.

## What diroxide protects, even from a plugin

These are checks diroxide makes on what a plugin sends and asks. They keep a plugin from fooling you or from slipping past the file safety. They are not a sandbox.

- Your ears and eyes. Every string a plugin sends is cleaned of control, formatting and direction characters and terminal escape sequences, cut to a length, and limited to a few announcements a second. The rest are folded into "12 more messages from Line counter". A plugin's dialogs are diroxide's own, named for the plugin, closed by Escape, with focus returned.
- Attribution, last. Every announcement or dialog a plugin causes ends with its name, "Lines: 42, files: 1. By Line counter." and "Replace a file? (Line counter)". The name goes last so the meaning stays in the first 40 characters, and so text that imitates diroxide, such as "Moved to the trash", still ends by saying who said it. The built-in confirmations for permanent delete and overwrite are never made from a plugin's words.
- File operations get the engine's protections. A plugin's copy, move, rename and delete run as jobs with the plan, a confirmation that starts on No, the trash by default, the refusals near the root of a drive, the boundary rules and the undo log. A plugin cannot switch these off.
- Dangerous names. A plugin that asks diroxide to make or rename an entry cannot create a name that needs care: a control, private-use or direction character, a reserved character such as a colon, a trailing dot or space, or a device name. Such names break other programs and other systems, and a name with a hidden character can be taken for another.
- Places are parsed, not trusted. A place from a plugin goes through the same parser as one you type. A bare drive letter such as `C:` is refused as ambiguous, `..` is resolved and checked, device paths are refused, and the result is shown in full before anything is acted on.
- The spool is confined. When a plugin hands diroxide a file path for a transfer, it must be a regular file directly inside the folder diroxide made for that plugin, with owner-only rights. A link, a `..`, a device name or a colon is refused and logged.
- Programs by name. A plugin runs programs through diroxide only by name, found by the one PATH rule, with an argument list and never a shell line, with its environment limited, its time limited and its output capped. Interpreters and shells, such as `sh`, `bash`, `python`, `pwsh`, `cmd`, `env`, `xargs` and `find`, are refused as tool names unless the plugin declares "any program" and the consent question then says "This plugin can run any program." Even then, naming a tool is naming what it can do. `git` can run hooks, `pandoc` can run filters and `ffmpeg` can read other files, so a plugin that names one can be made to do those things.
- No identity travels with a message. The `initialize` message carries your locale, diroxide's name and version, and the plugin's own folders. It does not carry your name. A folder's path can contain your user name, as any path in your profile does.
- A plugin that crashes, hangs or floods is announced and contained. Three crashes in a minute turn it off.
- Scripts run only from the scripts folder, not from the current folder or an archive. On Linux and macOS a script whose file or folder can be changed by others is refused. The scratch folder a script gets is removed by listing it, checking that its full path is inside the state folder, and removing what the list names.

## What a plugin could still do

Said plainly. A process plugin or a script is a program that runs with your rights. It can:

- Read any file you can read, such as your SSH keys, browser profiles, documents, notes and diroxide's own settings, and send what it reads anywhere over the network.
- Write, change, encrypt or delete any file you can, with its own code, going around the job engine, the trash and Undo.
- Start any program, install itself to run at login (startup entries, scheduled tasks, shell startup files), and keep running after diroxide quits. On Linux and macOS a process can leave its group and run on its own. On Windows the job object that diroxide puts it in ends it when diroxide ends, unless it was allowed to break away, which diroxide does not allow.
- Read all the environment it inherited: tokens, `AWS_*` and `GITHUB_TOKEN` variables, your user name, and your folders in `HOME` and `USERPROFILE`. Some plugins need these on purpose, so diroxide passes them. The rule that diroxide never sends your identifiers anywhere protects what diroxide sends, not what a plugin can read.
- Read the memory of other processes of yours on systems that allow it, including a password you just typed into a secret prompt, and read the clipboard and the screen.
- Edit diroxide's configuration and its own consent record, turning itself or other plugins on. Edit other plugins' files. Replace the `diro` program if you can write to its folder.
- Create files whose names break the naming rules (trailing dots, `CON`, a colon that makes a hidden stream on NTFS, private-use characters) with its own file calls. diroxide flags them in the panels as "name needs care" but cannot stop their creation.
- Plant links in folders you later process. The boundary rules stop diroxide's own recursion from following them.
- Ask a misleading question. The dialog says who asks, but not whether the question is honest, and a plugin could ask for a "secret" under a misleading title.
- Flood its log, its spool or the disk up to the volume's limits, use all the processor and memory it is given, start processes without bound, or hold a lease forever. diroxide sets no process limits yet.
- Rewrite its own code after you said yes. The program-file fingerprint catches that at the next start, not while it runs.

A script command is the same. A header with a `needs` or `places` line says what it expects, and the script can still do anything its interpreter can.

## What stops all of this

Only your choice to turn it on, the pins on downloaded plugins, and, later, a sandboxed way to run a plugin. A WebAssembly transport is the fallback recorded in the plugin decision: the manifest's list becomes the set of calls diroxide links in, and the sandbox enforces it. Until then this page says the words the question says:

While it runs, this plugin can do anything you can do.

## Optional components

- Integrity is the pins: size and SHA-256 from the list that is part of each build. The transport is for privacy and does not vouch for a file. A file that does not match is refused and removed, whoever served it.
- A file is placed only after its check. An archive's members have plain names, no links, and stay within the size the list states. A receipt lists what was written, and Remove takes away exactly that.
- A source of your own can offer a component with its own correct pins. That is its job. The question names it as "your own components source", and a plugin from it still meets the turn-on question before it can run.
- A downloaded program gets the right to run only if the list names it as the program.
- The builds of diroxide are not code-signed. Windows' SmartScreen and a virus scanner may react to a downloaded program.
- Nothing is requested before your yes, and the only header that identifies anything is the neutral project User-Agent. [Optional components](components.md) and [privacy](privacy.md) say more.

## How to stay safe

- Turn on only plugins you wrote, read or trust. The nine examples are small on purpose, so you can read them.
- Read the question. The last two groups of rows tell you what the plugin asks for and what it promises. A plugin that asks to read and change files and says "no network" is a different bargain from one that says nothing.
- Say no when a plugin asks again after you did not change it. "Changed since you enabled it" means its manifest or its program is different from what you read.
- Run an unfamiliar plugin once with `diro plugin run` before you turn it on, and read its log.
- Keep scripts in the scripts folder, writable by you alone.
- Write your plugin to ask for as little as it needs, and to say what it does.

If you find a way for a plugin to get past a limit on this page, such as an announcement that passes as diroxide's own or a path that reaches outside the spool, that is a bug in diroxide. [Troubleshooting](troubleshooting.md) says how to report it.

## See also

- [Plugins](plugins.md)
- [Writing a plugin](writing-a-plugin.md)
- [Script commands](script-commands.md)
- [Optional components](components.md)
- [Privacy](privacy.md)
- [Known limits](known-limits.md)
- [The plugin protocol decision](adr/0006-the-plugin-protocol.md)
- [Safety rules decision](adr/0008-safety-rules.md)
- [Troubleshooting](troubleshooting.md)
- [The documentation index](README.md)
