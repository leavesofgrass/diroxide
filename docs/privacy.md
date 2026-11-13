# Privacy

This page says what diroxide keeps on your computer, when it uses the network, what plugins can see, where the secrets of remote places live, and what the log file holds. The short answer: everything stays on your computer. The network is used by the download of an optional component, which asks first, and by the places you open on other computers, which you name yourself.

## What stays on your computer

diroxide keeps its files in two folders. [The settings guide](settings.md) says where they are on each system.

- The config folder holds the files you can edit: `config.toml`, `keymap.toml`, `menus.toml`, the banks of command buttons and your own themes, and two read-only copies of the built-in keys and menus.
- The state folder holds what diroxide remembers for you: where each panel was and which row was focused when you quit, with every tab of each panel (`session.toml`), the tab sets you saved by name (`tab-sets.toml`), the files you opened or edited from diroxide (`recent.toml`), the history of the prompts you used (the `history` folder), the folders you visited so path jump can rank them, the undo journal of your last jobs, a ledger of partial files that a lost network left (`partials.json`), the jobs that were waiting or running when you quit or crashed (`queue.json`, with a lock file beside it, `queue.lock`), the labels (with your notes and fields), bookmarks, saved views, saved searches and collections you made, the last rules of a batch rename (`rename-last.toml`) and the names file of each batch rename you edited in a text editor (`rename-NUMBER.txt`), the last 20 conversion reports (the `conversions` folder), the state, ledger and log of each watched folder (the `hot` folder), and `terminal.log`.
- With `--portable`, both folders sit beside the program instead, so you can carry diroxide on a drive.

All of these are plain files. You can read them, copy them and delete them. Deleting the state folder only makes diroxide forget.

Several of them hold paths, so they say where you have been working: `session.toml` holds each tab's folder and its history of folders, `tab-sets.toml` the folders of each saved tab set, the `saved` folder the folders a saved search looks in and the full path of every item of every collection, each `rename-NUMBER.txt` the old and new names of a batch rename, and `recent.toml` the full path and the time of each file you opened or edited, at most 50, newest first. Keep that in mind before you share a state folder, or a copy of one, in a bug report. diroxide never sends them anywhere. The setting `recent_files` (Safety group of Settings, on by default) turns the recent files off: nothing is kept, and the list is emptied. Deleting `tab-sets.toml` or `recent.toml` is safe, and only makes diroxide forget. Removing a collection removes only its list: no item's file is touched. The rename presets you save are in `rename-presets.toml` in the config folder, and hold rules, which can name text from your files' names. A list made by `diro -` from standard input is not kept anywhere unless you save it as a collection.

diroxide never sends any of this anywhere. It has no account, no sign-in, no usage statistics, no crash reports and no update check.

## When the network is used

The network is used only by what you ask for.

- Downloading an optional component is the one place diroxide opens a connection itself. It names the component, its size and its license, and waits for your answer. The default is No. Nothing is requested before a yes, and the tests check that. See "Optional components" below.
- Opening a remote place over SFTP connects to the server you named. The connection is made by the ssh program on your computer, not by diroxide. The SFTP places plugin sends nothing of its own. "Remote places over SSH" below says what ssh sends.
- A plugin you turn on may reach the network. A plugin says whether it does before you turn it on, in the rows under "The plugin states, unchecked:", and diroxide cannot check those words. Nothing is turned on by default.
- A tool you have installed, such as git or ffmpeg, may use the network on its own. diroxide starts it only when you run a command that uses it.

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

## Remote places over SSH

The SFTP places plugin talks to a server only through `ssh`, the OpenSSH program on your computer. [Remote places over SFTP](remote-places.md) is the guide.

What it sends:

- Nothing of its own. The plugin makes no network call. It starts `ssh` with an argument list and speaks to it through pipes. Everything that crosses the network is ssh's own traffic, to the server you named and, if you set a jump host in your ssh configuration, to that host first.
- The server sees what any SSH server sees: your network address, the user name you sign in with, ssh's version banner, the key or password you offer, and the names and contents of the files you list, copy, move or delete there. diroxide adds no identifier of its own. The user name is the one in the address you typed, in the saved site, or in your ssh configuration.
- A jump host sees that a connection passes through it, and not what is inside. diroxide also never fills a saved password in through a jump host or a proxy command, so a question from the host in the middle cannot collect the password of the server behind it.
- ssh writes a new server's key to your known hosts file, after you said yes, unless your own ssh configuration says to accept new keys. The plugin never writes that file.
- ssh's debug log is off. The `ssh_debug_log` setting turns it on, into `ssh-debug.log` in the plugin's folder of the state folder. That log can hold host names, user names and key file names. Read it before you share it.

What stays on your computer, and where:

- `sites.toml`, in the plugin's folder of the state folder. It holds the name, host, user, port, start folders, key file path and note of each site, and the word that says how its password is kept. These are identifiers, so treat the file as private when you share your files or screen. It holds no password, no passphrase and no token. A file in which a table holds one is refused whole and nothing is read from it.
- A password you typed, kept by the site's mode:
  - never (the default): not kept;
  - this session: in the plugin's memory only, wiped when diroxide quits, when the server refuses it, on Forget password, and when you close the last connection yourself;
  - the system keyring: in Windows Credential Manager, the macOS keychain or the Secret Service on Linux, under the service name `diroxide-sftp`, after you said yes to "Remember this password? y or n". The question starts on No.
- Windows Credential Manager entries can be read by other programs that run as you, and the store limits how long a saved secret can be. The system keyring is a convenience for a computer you trust. Keep the mode on never if other programs of yours should not be able to read the password.
- A key's passphrase is never kept. The agent can hold an unlocked key for you, with the site's `add_key_to_agent` setting.
- `partials.json` in the state folder lists the partial files a lost connection left, with the paths of their targets and sources, so a retry can go on from them. [The transfer queue](transfer-queue.md) says more.
- The plugin's log, like any plugin's, can hold host names and user names. It never holds a password: tests scan what the plugin writes for the secrets they use.

diroxide never sends the contents of `sites.toml`, a log or a ledger anywhere.

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

The undo journal lists the paths of the last jobs so that Undo can reverse them, even after a restart. The saved queue, `queue.json`, holds paths to your files too: for each job that was waiting, held or running when you quit or crashed, it keeps the kind, the state, a short description such as "12 items to build", and the request the job was planned from, which names the full paths or addresses of the files and of the folder they were going to. That is what lets the next start list the job under "Interrupted" and plan it again. diroxide removes the file when no job is left, and keeps a copy it cannot read beside it as `queue.json.bad`, never deleting it, so that copy holds the same paths. `queue.lock` marks which window keeps the queue; diroxide writes nothing into it. [The transfer queue](transfer-queue.md#the-queue-at-the-next-start) says how the queue is kept. The prompt histories and the path jump list hold paths you typed or visited. When you use Run a command or Shell here, diroxide writes the paths you had selected to a list file in the `run` folder of the state folder, so your command can read them. All of this stays in the state folder. It is the reason to treat the state folder as private when you share your screen or your files.

## Facts read from your files, conversions and watched folders

- Columns, the details view, Check before sharing and `diro info` read what is inside your files, on this computer. What they read is shown to you and never sent. A private detail, such as the place a photo was taken, is shown in the details and left out of Copy details unless you ask for all. [The details view](details.md) and [privacy before sharing](before-sharing.md) say more.
- Helper programs that read a file for you, such as ffprobe, ExifTool, Tesseract and pdftotext, run on your computer when a command asks for them, started with a list of arguments and never through a shell.
- A note and a field you set are kept in `labels.toml` in the state folder, never in the file.
- A checksum is not stored anywhere. The details view keeps the value for the session, and a `SHA256SUMS` file is written only when you choose Save as.
- Strip metadata writes a copy and leaves the original as it was. Check before sharing writes nothing. Remove the internet mark changes one file, after asking, and Undo puts the mark back.
- A watched folder's ledger and log list the names of the files it handled, what happened to each and when, in the state folder. Nothing in them leaves your computer. A conversion's report stays there too, the last 20 of them. [Watched folders](watched-folders.md) and [conversions](conversions.md) say what each holds.

## Encryption, signatures and keys

- Encrypt, Decrypt, Sign with gpg and Verify signature read the files you choose, on this computer, and send nothing anywhere. [Encryption and signatures](encryption.md) is the guide.
- A passphrase, an age identity file and a minisign public key are asked each time and stored nowhere: not in a setting, the state folder, the log or the system's keyring.
- gpg is started so that it never looks a key up on a server, because a lookup would send an email address away. A key you need has to be in your own keyring already. When it is not, you hear "Cannot check: public key is missing" and "diroxide does not fetch keys."
- A signature check says the key's last four digits first. The name of the key's owner is said only after them, because it is text the key's maker chose.
- Encryption makes a new file beside the original and leaves the original alone. The trash is not a secure delete, and diroxide says so when it offers it.

## Plugins and script commands

A plugin runs with your own rights and can read what you can read, so turn on only plugins you trust. [Security](security.md) says what that means.

What a plugin or a script sees:

- What you hand it: the files and folders of a command it runs, and your locale. The plugin's own folders in the state folder are named to it.
- The answers you type into a question it asks. That is the point of asking.
- If a plugin supplies the choices for a prompt, it sees the text you have typed so far in that prompt, so that it can suggest the rest. This is for non-secret prompts only. Text typed into a secret prompt is never completed, never kept in a history, and does not reach the plugin until you answer.
- If the plugin has a hook, it is told about the jobs it hooks: the paths of what you copy, move, delete, rename or create, and where they go. A navigation hook is told the folder you enter, how many rows are marked, and the name of the focused row. [Hooks](hooks.md) has the events. A plugin you did not turn on is never told.
- Whatever is in the environment diroxide runs it in. diroxide removes its own `DIRO_` variables from the environment it inherited and sets the ones the plugin needs, and passes the rest. A plugin can read your user name and your home folder from there.

What diroxide keeps about plugins, all in the state folder and all plain files:

- `consent.toml` records what you turned on, with the fingerprints of the manifest and of the program. It holds no file names.
- Each plugin has a log. It holds the plugin's standard error text and what diroxide noted about it. A plugin's error text often has a file name in it, so read a log before you share it.
- A running script gets a short-lived folder in `runs` with the list of the files you selected, and diroxide removes it when the run ends. A script's own `data` folder stays.

diroxide never sends a plugin's output, log or your selection anywhere. A plugin that does is the plugin's doing, and it should have said so in the rows you read when you turned it on.

## Reporting a problem

When you report a problem, say what you did, what you heard or saw, the output of `diro --version`, and the name and version of your terminal and screen reader. Leave out file names you do not want to share. The log file holds no file names, so it is safe to share as it is.

## See also

- [Remote places over SFTP](remote-places.md)
- [Privacy before sharing](before-sharing.md)
- [Encryption and signatures](encryption.md)
- [Watched folders](watched-folders.md)
- [Conversions](conversions.md)
- [Hooks](hooks.md)
- [Settings and where the files live](settings.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
