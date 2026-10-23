# Privacy

This page says what diroxide keeps on your computer, when it uses the network, what plugins can see, and what the log file holds. The short answer: everything stays on your computer. The one thing in diroxide that uses the network is the download of an optional component, and it asks first.

## What stays on your computer

diroxide keeps its files in two folders. [The settings guide](settings.md) says where they are on each system.

- The config folder holds the files you can edit: `config.toml`, `keymap.toml`, `menus.toml`, the banks of command buttons and your own themes, and two read-only copies of the built-in keys and menus.
- The state folder holds what diroxide remembers for you: where each panel was and which row was focused when you quit (`session.toml`), the history of the prompts you used (the `history` folder), the folders you visited so path jump can rank them, the undo journal of your last jobs, the labels, bookmarks, saved views and saved searches you made, and `terminal.log`.
- With `--portable`, both folders sit beside the program instead, so you can carry diroxide on a drive.

All of these are plain files. You can read them, copy them and delete them. Deleting the state folder only makes diroxide forget.

diroxide never sends any of this anywhere. It has no account, no sign-in, no usage statistics, no crash reports and no update check.

## When the network is used

The network is used only by what you ask for.

- Downloading an optional component is the one place diroxide opens a connection itself. It names the component, its size and its license, and waits for your answer. The default is No. Nothing is requested before a yes, and the tests check that. See "Optional components" below.
- A plugin you turn on may reach the network. A plugin says whether it does before you turn it on, in the rows under "The plugin states, unchecked:", and diroxide cannot check those words. Nothing is turned on by default.
- A tool you have installed, such as git or ffmpeg, may use the network on its own. diroxide starts it only when you run a command that uses it.

Later alphas add a connection to another machine over SSH. It will happen when you connect, and host keys will be shown to you before they are trusted.

When a request does leave your computer, it carries only what that feature needs. It never carries your name, your user name, your computer's name or your paths.

## Optional components

When you say yes to a download, diroxide sends an ordinary web request for the file. What the request carries:

- The address of the file, which is the project's public release address or the address in the component list, or the place you named in `components_source`.
- One identifying header, the User-Agent `diroxide-research (+https://diroxide.app)`. It names the project and nobody else. No name, no user name, no computer name, no version of your system.
- When it resumes a stopped download, a range and a validator, which say where to go on from.
- No password, token or sign-in. A private source that needs one is not built yet. When it is, the token will go only to that source, never on a command line and never to any other address.

The server sees your computer's network address, as any web server does. diroxide sends no cookies and keeps no account, and the list of components is part of your diroxide, so no list is fetched from the network.

The file is checked against its pinned size and SHA-256 before it is used. A file that does not match is refused and removed. The download never asks your name, never reports that you did it, and never checks for updates by itself.

`diro components download --source FOLDER` makes no request at all: it reads a folder you name.

## The clipboard and other programs

- Cut, copy and paste of files use diroxide's own clipboard. They do not touch the system clipboard.
- When you press Enter on a file, diroxide hands it to your system's own program for that kind of file. What that program does with the file is up to it.
- F4 hands the file to the editor you have chosen.
- In the quick view, Enter on a web or mail link asks first, and the question starts on No. If you say yes, diroxide hands the address to your system's browser or mail program. The connection, if there is one, is that program's.

## The log file

diroxide writes one log file, `terminal.log`, in the state folder. You can open it, move it or delete it at any time. Each start replaces it.

- It holds what the terminal reported about itself: whether diroxide drew on the alternate screen or inline, that mouse capture is off, what the terminal answered about its enhanced keyboard protocol, and how many colors it supports.
- It holds no file names, no paths and no messages. Errors and failed operations are told to you on the screen and in the job history, and they are not written to the log.
- It never holds a password or other secret. A test checks that nothing secret reaches a log, a debug output or the undo journal.

## The undo journal, the history and what Run a command writes

The undo journal lists the paths of the last jobs so that Undo can reverse them, even after a restart. The prompt histories and the path jump list hold paths you typed or visited. When you use Run a command or Shell here, diroxide writes the paths you had selected to a list file in the `run` folder of the state folder, so your command can read them. All of this stays in the state folder. It is the reason to treat the state folder as private when you share your screen or your files.

## Plugins and script commands

A plugin runs with your own rights and can read what you can read, so turn on only plugins you trust. [Security](security.md) says what that means.

What a plugin or a script sees:

- What you hand it: the files and folders of a command it runs, and your locale. The plugin's own folders in the state folder are named to it.
- The answers you type into a question it asks. That is the point of asking.
- If a plugin supplies the choices for a prompt, it sees the text you have typed so far in that prompt, so that it can suggest the rest. This is for non-secret prompts only. Text typed into a secret prompt is never completed, never kept in a history, and does not reach the plugin until you answer.
- Whatever is in the environment diroxide runs it in. diroxide removes its own `DIRO_` variables from the environment it inherited and sets the ones the plugin needs, and passes the rest. A plugin can read your user name and your home folder from there.

What diroxide keeps about plugins, all in the state folder and all plain files:

- `consent.toml` records what you turned on, with the fingerprints of the manifest and of the program. It holds no file names.
- Each plugin has a log. It holds the plugin's standard error text and what diroxide noted about it. A plugin's error text often has a file name in it, so read a log before you share it.
- A running script gets a short-lived folder in `runs` with the list of the files you selected, and diroxide removes it when the run ends. A script's own `data` folder stays.

diroxide never sends a plugin's output, log or your selection anywhere. A plugin that does is the plugin's doing, and it should have said so in the rows you read when you turned it on.

## Reporting a problem

When you report a problem, say what you did, what you heard or saw, the output of `diro --version`, and the name and version of your terminal and screen reader. Leave out file names you do not want to share. The log file holds no file names, so it is safe to share as it is.

## See also

- [Settings and where the files live](settings.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
