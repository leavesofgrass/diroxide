# Privacy

This page says what diroxide keeps on your computer, when it uses the network, and what the log file holds. The short answer: everything stays on your computer, and alpha.1 does not use the network at all.

## What stays on your computer

diroxide keeps its files in two folders. [The settings guide](settings.md) says where they are on each system.

- The config folder holds the files you can edit: `config.toml`, `keymap.toml`, `menus.toml` and the banks of command buttons.
- The state folder holds what diroxide remembers for you: where each panel was and which row was focused when you quit, the history of the prompts you used, the folders you visited (so path jump can rank them), the undo journal of your last jobs, and the log file.
- With `--portable`, both folders sit beside the program instead, so you can carry diroxide on a drive.

All of these are plain files. You can read them, copy them and delete them. Deleting the state folder only makes diroxide forget. The Forget history command clears the history of your prompts without touching anything else.

diroxide never sends any of this anywhere. It has no account, no sign-in, no usage statistics, no crash reports and no update check.

## When the network is used

The network is used only by what you ask for. alpha.1 does not use it for anything: no feature in it opens a connection.

Later alphas add features that do, and each one asks first:

- A plugin you enable may reach the network. A plugin shows what it is allowed to do before you enable it, and nothing is enabled by default.
- A download of an optional component names the file, its size and its license and waits for your answer. Every downloaded file is checked against a published checksum.
- A connection to another machine over SSH happens when you connect, and host keys are shown to you before they are trusted.
- A tool you have installed, such as git or ffmpeg, may use the network on its own; diroxide only starts it when you run a command that uses it.

When a request does leave your computer, it carries only what that feature needs. It never carries your name, your user name, your computer's name or your paths, and this page will list the exact request when the feature ships.

## The clipboard and other programs

- Cut, copy and paste of files use diroxide's own clipboard. They do not touch the system clipboard.
- When you press Enter on a file, diroxide hands it to your system's own program for that kind of file. What that program does with the file is up to it.
- F4 hands the file to the editor you have chosen.

## The log file

diroxide writes a log file in the state folder. You can open it, move it or delete it at any time.

- It records how diroxide started: the version, the platform, which settings files it read, any problems it found in them, and what the terminal said about its keyboard.
- It records errors and failed operations with the reason in words. For an error that involves a file, it may include that file's path and name. Read the log before you share it, and take out anything private.
- It never holds a password or other secret. A test checks that nothing secret reaches a log, a debug output or the undo journal.

## The undo journal and the history

The undo journal lists the paths of the last jobs so that Undo can reverse them, even after a restart. The prompt history and the path jump list hold paths you typed or visited. They stay in the state folder. They are the reason to treat the state folder as private when you share your screen or your files.

## Plugins

Plugins arrive with alpha.4. A plugin runs with your own rights and can read what you can read, so enable only plugins you trust. Each plugin has its own log in the state folder.

## Reporting a problem

When you report a problem, say what you did, what you heard or saw, the output of `diro --version`, and the name and version of your terminal and screen reader. Leave out file names you do not want to share, and trim the log to the lines that matter.

## See also

- [Settings and where the files live](settings.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
