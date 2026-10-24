# Remote places over SFTP

This guide covers folders on other computers: what you need, how to open one, what the questions say, saved sites and passwords, how to work in a remote folder, and what is not built yet. The feature is the SFTP places plugin, and it uses the OpenSSH program that is already on your computer.

What this guide quotes is what the program says. Each sentence is checked in tests against the 40 cells of a Braille line, and each is written so its meaning comes first. None of it has been heard yet on NVDA, JAWS or a Braille display, and none of it has been tried on a real server by hand: the tests run against a fake server and against OpenSSH's own server in a container. If your experience differs, that is a finding.

[The transfer queue](transfer-queue.md) says what happens to copies and moves between your panels and a server. [Security](security.md) and [privacy](privacy.md) say what the plugin trusts, what it sends and where passwords live.

## What it is

SFTP is file transfer over SSH. diroxide does not carry an SSH program of its own. It starts the `ssh` that is on your computer and speaks SFTP through it, so your keys, your agent, your known hosts file and your ssh configuration work as they are.

- A remote folder is a place like any other. It has an address, `sftp://alice@build.example/srv`, and it opens in a panel.
- Copy, move, rename, make folder, delete and the permissions dialog work there. Copies and moves are jobs with a plan, a question that starts on No and a result.
- The plugin is called "SFTP places". It is a plugin like any other, so nothing runs until you turn it on. [Plugins](plugins.md) says how.
- The plugin makes no network call of its own. The only traffic is ssh's.

## What you need

### The OpenSSH program

On Windows, OpenSSH is an optional feature: Settings, then Optional features. Git for Windows brings its own `ssh`. On Linux and macOS it is the system's `ssh`.

diroxide looks for `ssh` in a fixed list of full paths and never runs a bare name or a program from the current folder:

- the program named by the `ssh_program` setting, if you set one;
- on Windows, the `System32\OpenSSH` folder and the `Program Files\OpenSSH` folder;
- every full path on your PATH.

It asks each one for its version and uses the highest. On a tie the Windows build wins. If Git's `ssh` is newer than the one in Windows, Git's is used. To choose one yourself, set `ssh_program` to its full path.

Which one was chosen is the first line of the doctor report, below.

### Passwords need OpenSSH 8.4

ssh asks for a password through a helper program, and that way exists from OpenSSH 8.4. With an older ssh the plugin works with keys and the agent only, and says so:

```text
This ssh is too old for passwords.
Use a key, or update OpenSSH to 8.4.
winget install Microsoft.OpenSSH.Preview
```

The last line appears on Windows only. The command is the one to run in a terminal.

If ssh cannot be found at all, you hear "OpenSSH was not found." and then the next step: "Add it in Settings, Optional features." on Windows, or "Install the OpenSSH client package." elsewhere.

### The agent

If you use keys with a passphrase, the ssh agent can hold them unlocked so that you type the passphrase once. On Windows the ssh-agent service is off until you turn it on. Doctor says "Agent: not running." when no agent answers, and a key with a passphrase then asks every time.

### The plugin

The plugin comes as an optional component for Windows x86_64, Linux x86_64 and macOS (Manage optional components in the Tools menu, or `diro components download diro-sftp`); on any other computer diroxide says "No build for this computer." and names the `cargo install` line that builds it from the source tree. See [Optional components](components.md).

```bash
cargo build --release --manifest-path plugins/diro-sftp/Cargo.toml
```

Make a folder named `sftp`, and put two files in it: `plugins/diro-sftp/plugin.toml` and the program the build made, `diro-sftp` (`diro-sftp.exe` on Windows), which is in `plugins/diro-sftp/target/release` unless you set another target folder. The folder must be named `sftp`. `diro plugin check` gives a warning if it is not.

```bash
diro plugin check ./sftp
```

```bash
diro plugin install ./sftp
```

```text
Installed: SFTP places. It is not enabled.
Next: diro plugin enable sftp
```

```bash
diro plugin enable sftp
```

The question lists what the plugin does:

```text
Enable SFTP places, version 0.1.0-alpha.2?
It runs: ./diro-sftp.exe
diroxide checks: it may change the panels.
The plugin states, unchecked:
connects to the servers you open, through ssh.
its own state folder; ssh reads your config and keys.
ssh, found by the rules in its settings.
While it runs it can do what you can.
Enable it? y or n
```

On Linux and macOS the second line reads "It runs: ./diro-sftp". The plugin may change the panels because Connect opens a site's start folders. [Plugins](plugins.md) explains every row.

## Check it with doctor

Doctor is one plain report of what the plugin stands on. It makes no network connection. It asks ssh for its version, asks the agent how many keys it holds (it counts them and never reads one out), reads your ssh configuration for host names, reads the sites file, names the key store without opening it, and runs the password helper once against a test question.

Run it in a terminal, from the `sftp` folder or by the program's full path:

```bash
diro-sftp doctor
```

This run used a test configuration:

```text
ssh: OpenSSH 10.3 from PATH
Passwords: asked through the helper.
Agent: not running.
Config: 3 hosts.
Config: 2 names left out, unsafe.
Sites: none saved.
Key store: Windows Credential Manager.
Endpoint: owner-only pipe and proof.
Helper: works.
Plugin: SFTP places 0.1.0-alpha.2.
```

In the program, the Plugins menu has SFTP doctor, and it shows the same lines as text. The fact comes first in every line:

- "ssh:" names the program in use and where it was found. Windows says "ssh: OpenSSH for Windows 9.5 in System32". Another build says "from PATH" or "in Program Files".
- "Passwords:" says whether ssh can ask them. An old ssh says "Passwords: no, ssh below 8.4; keys work."
- "Agent:" says "running, 2 keys", "running, no keys", "not running" or "not checked, no ssh-add".
- "Config:" counts the hosts in your ssh configuration. A host name that is not safe to use is left out and counted.
- "Sites:" counts your saved sites. A sites file that holds a password is refused, and doctor says so.
- "Key store:" names the system's store for saved passwords.
- "Endpoint:" says how the password helper reaches the plugin: "owner-only pipe and proof" on Windows, "owner-only socket and proof" on Linux and macOS. "loopback, proof only" means no private folder was available, and the proof alone guards the road. If you ever see it, report it.
- "Helper:" is the self-test. "Helper: works." is the line you want. Anything else says why, for example "Helper failed: its proof was refused."

`diro-sftp doctor` takes two options, `--ssh-program PATH` and `--ssh-config PATH`, to try a different ssh or configuration.

## Opening a place

There are three ways in.

- Go to a path (Ctrl+L): type an address and press Enter.
- Go to a place (Ctrl+G): your saved sites are in the list.
- The Sites list (Plugins menu, SFTP sites): pick a site and press Enter.

### Typing an address

```text
sftp://alice@build.example/srv
sftp://alice@build.example:2222/srv
sftp://build.example/
sftp://nas/
```

The user and the port are optional. Left out, ssh decides, from your configuration. A name from your ssh configuration, such as `nas`, works as the host. The path is a full path on the server.

A host that starts with a dash, or holds a space or an odd character, is refused before ssh is started. The same goes for a user name.

### Go to a place

A saved site is a row in the list. The row says its kind first, then the site's name, and the rest follows: "Site, build, saved, alice@build.example, by SFTP places". The state is a word, "saved" or "connected" or "lost", and the plugin's name is last. Enter on a site connects through the plugin and opens the site's start folder in the active panel. A site you are connected to also has a row in the Volumes list, "Connected: build, sftp, 1.2 TB free"; [volumes and the trash](volumes-trash.md) describes it.

### Connecting

You hear "Connecting to build.example." and then "Connected to build.example." In between, ssh may ask questions. Each is a dialog of diroxide's own, with the plugin's name last, and ssh does not go on until you answer.

Escape in a question is a no. A question that waits five minutes for an answer is given up.

If the connection fails, the first line says what failed and the next line says what to try, and ssh's own last words come last: "ssh said: ..."

## The questions ssh asks

### A new server key

The first time you reach a server, ssh does not know its key and asks whether to trust it. The dialog is titled "New server key". It starts on No. Its rows, in order:

```text
New server key for build.example.
Check it before you accept it.
Key type: ED25519.
Fingerprint, in groups of four:
Part 1 of 4: 2134 141b 8436 8a94
Part 2 of 4: a6d8 d5c1 2155 8759
Part 3 of 4: 8d34 a500 27c6 fb7d
Part 4 of 4: c4f9 8430 2e65 fb1d
As ssh printed it:
SHA256:ITQUG4Q2ipSm2NXBIVWHWY00pQAnxvt9xPmEMC5l+x0
Trust this key and connect?
```

The fingerprint is in hexadecimal, in groups of four, four groups a row, because hexadecimal has no O, l or I to mistake for a digit. The last fingerprint row is exactly what ssh printed, so you can compare it with what the server's administrator gave you, by ear or by pasting. Your choices are "Trust and connect" and "Do not connect".

Check the fingerprint against a source you trust before you say yes. A yes goes to ssh, and ssh writes the key to your known hosts file itself. The plugin never writes that file.

### A password

ssh's own password question opens a secret prompt, titled with the account and the host:

```text
Password for alice at build.example
```

The prompt is masked, and what you type is never read back. If the server refuses the password and ssh asks again, the prompt says which try it is: "Try 2 of 3."

### A key's passphrase

```text
Key passphrase for id_ed25519
```

A passphrase is never saved. If you want to type it once, let the agent hold the key: set `add_key_to_agent` for the site, below.

### A question from the server

Some servers ask for a one-time code or a PIN. The prompt is titled "Question from build.example:", and the server's own words come after it, cleaned and cut to 300 characters. The title is always the plugin's own, so a server cannot pass for diroxide.

### A changed server key

If the key a server shows is not the one your known hosts file holds, ssh refuses, and so does the plugin. There is no question and no way to accept it from diroxide. You hear:

```text
Host key changed. Not connected.
The server may be fake or rebuilt.
Ask the server's owner first.
diroxide never edits known_hosts.
To forget the old key, run:
ssh-keygen -R build.example
```

The last line is a command for you to run yourself, in a terminal, after you have asked the server's owner whether the key really changed. For a port other than 22 it reads `ssh-keygen -R "[build.example]:2222"`. diroxide shows the line and never runs it.

## Saved sites

A site is a name for a server, with the user, the port, a start folder and how its password is kept. The Sites list is the place to manage them.

Open it from the Plugins menu (Alt+P), SFTP sites, or from the palette. It opens over the panels, with a title that gives the counts first:

```text
Sites, 3 saved, 5 from ssh config
```

Each row puts the name first and the state right after it, as a word:

```text
build, saved, alice@build.example
build, connected, alice@build.example
build, lost, alice@build.example
nas, from ssh config
```

Saved sites come first, by group and name. The hosts from your ssh configuration come after them, with nothing saved. The Sites list reads the `Host` names of your ssh configuration and its included files, and leaves out patterns. It asks ssh for the details of a host only when you open it.

The keys in the Sites list:

- Enter: connect.
- Alt+N: new site. On a row from your ssh configuration it saves that host as a site, under its own name.
- Alt+E: edit the site.
- Alt+D: delete the site. The question starts on No: "Delete the site build?" with "Its saved password goes too."
- Alt+X: disconnect, quietly: "Disconnected from build."
- Alt+C: cancel the connection. It ends ssh at once, even in the middle of a transfer.
- Alt+F: forget the saved password.

Every one of these is also a command in the Plugins menu and the palette. Run from a panel that shows a remote place, a command acts on that place's connection. You can give any of them a key of your own in `keymap.toml`; [plugins](plugins.md) says how.

Connect opens the site's start folder in the active panel. If the site names a local folder, the other panel opens that.

New site asks for the address, then a name, then how to keep the password. Edit site asks for the name, the address, the group, the start folder on the server and the password mode, each with its current value. An address that is not valid is refused: "Not saved: the address was not valid."

### The sites file

The sites live in `sites.toml` in the plugin's own folder of the state folder. You can edit it. A site looks like this:

```toml
[[site]]
id = "k3f9"
name = "build"
group = "Work/Servers"
host = "build.example"
port = 22
user = "alice"
key_file = ""
start_remote = "/srv/projects"
start_local = ""
secret = "never"
add_key_to_agent = true
note = ""
```

- `id` is plain and random and never spoken.
- A site names `host`, or `alias` for a name from your ssh configuration, and never both. An alias site lets ssh supply the rest.
- `key_file` becomes ssh's `-i`. `add_key_to_agent` hands an unlocked key to the agent.
- `sftp_server_program` is for a server with no SFTP subsystem: a full path that ssh runs on the server instead.
- `start_remote` must be a full path on the server.
- `secret` is `never`, `session` or `keyring`, below.

Comments, blank lines and the order of your lines stay when diroxide saves. A site that fails the checks is left out and counted: "2 sites skipped: not valid." A file that is not TOML, or is larger than a megabyte, is not read, and it says so.

A site's `bookmarks` list is read and kept. Nothing offers those folders yet.

## Passwords: three modes

Each site has a mode for its password. The default keeps nothing.

- Never remember (`never`, the default). ssh asks every time, or the agent answers.
- Remember for this session (`session`). The plugin keeps the password you typed in its memory, and forgets it when diroxide quits, when the server refuses it, on Forget password, and when your own Disconnect closes the last connection.
- Remember in the system keyring (`keyring`). After a successful login with a password you typed, a question asks "Remember this password? y or n". It starts on No. Say yes and you hear "Saved in the system keyring, not a file."

The system keyring is the operating system's own store: Windows Credential Manager, the macOS keychain, or the Secret Service on Linux. diroxide hands the password to the store and takes it back. It does no encryption of its own for this. A machine with no store says "No key store here; nothing saved." and "Kept for this session only." Doctor names the store on this machine.

The rules that keep a saved password from going to the wrong place:

- No password is ever in `sites.toml`, in a log or in a message. A sites file in which a table holds a password, a token or a secret that is not one of the three mode words is refused whole: "Sites file refused: it holds a password." The next line names the key and the site, "Remove password from build." and "diroxide keeps no password in a file." Nothing is read from such a file and nothing is written over it.
- A saved password is given to ssh at most once for each connection, and only for ssh's own password question for the account and host of that connection.
- If ssh asks again, the password was refused. diroxide forgets it, says "Saved password refused; asking you." and asks you.
- Through a jump host or a proxy command, nothing is filled in and nothing is remembered. Another server on the road could word its question like ssh's own, and it must not learn the password of the server behind it. The question "Remember this password?" is not asked there either. The password is asked every time.
- Key passphrases are never saved.
- A connection that is not interactive, such as an automatic reconnect or a background job, never uses the keyring.

Windows Credential Manager entries can be read by other programs that run as you, and the store limits how long a saved secret can be. The keyring is a convenience that fits a computer you trust. If other programs of yours should not be able to read a password, keep the mode on never.

### Forget password

Alt+F in the Sites list, or the command Forget the site's password. It asks first and starts on No: "Forget the saved password for build?" Then "Password forgotten for build." With nothing saved, "No password saved for build." It removes the password from this session and from the keyring. Deleting a site removes it too.

## Working in a remote folder

### Listings

A big folder arrives in batches of a thousand rows. The panel opens at once with the first batch. In screen-reader mode a line says how far it has come, at most every ten seconds: "Listing reports: 2,000 so far", where "reports" is the folder's name. Escape stops the listing and keeps what came: "Stopping the listing." and then "Listing stopped: 1,000 items shown". Leaving the folder stops it too. Your focus and your marks stay on the same rows as more arrive.

A name the server sends that cannot be shown safely, such as a name with a slash in it, is left out, and the count is said: "2 names skipped: unsafe." A name that is not valid UTF-8 is kept as the server sent it.

### Copy and move

F5 and F6 work between a remote folder and anything else, in either direction. A copy or a move that touches a server is a remote job. It waits its turn in the queue, two at a time for one server. [The transfer queue](transfer-queue.md) explains the Jobs list, retries, resume and the speed limit.

- A file is written to a temporary name that starts with `.diro-part-` and renamed when it is complete. A cancelled copy removes its part file.
- A move to or from a server reads the copy back and compares it with the original before the original is removed. That doubles the traffic and is on purpose: nothing is destroyed until the replacement is verified.
- Two ssh sessions serve one server by default, so a listing does not wait behind a long copy. The second session asks you nothing. If you typed a password, it is reused from memory for the same account only.
- A copy between two folders of the same account on the same server is made by the server itself when the server supports it (OpenSSH 9.0 and newer do). Only files are copied that way. Between two servers or two accounts, and on a server that cannot copy on its own, the copy goes through your computer.
- A copy that will not fit is caught before it starts when the server reports its free space, with a question that says "Not enough room." and asks "Go on anyway?", starting on No. A server that does not report free space cannot be checked. If the disk fills during the write, you hear "The server's disk or quota is full."

### Delete is permanent

SFTP has no trash. F8 on a remote item does not move it anywhere. diroxide says the place has no trash and offers the listed delete instead, as it does for a drive with no trash. The list comes first, deepest paths first, and the question starts on No. [Operations and undo](operations.md) has its words. A delete over SFTP cannot be undone.

### Rename, make folder, links and permissions

- Rename and make folder work as everywhere. A name that is taken says "The name is taken." A folder that is not empty says "The folder is not empty."
- A server that cannot rename over a file says "This server cannot rename over a file."
- A server that cannot make links says "This server cannot make links."
- Change permissions works where the server allows it. A server may keep other bits than you asked for, and the plugin reads back what it kept.
- Dates: SFTP keeps dates from 1970 to 2106. A date outside that range is not kept, and you are told: "SFTP keeps dates from 1970 to 2106."

### Programs

F4, Shell here and Run a command start a program in a folder on this computer. In a remote folder they say that programs start only in folders on this computer. There is no editing of a remote file in place. Copy it down, edit it, and copy it back.

## When something goes wrong

The first line says what failed, with the cause first. The next line says what to try. ssh's own last words come last, cleaned, after "ssh said:".

- "Connection refused by build.example." then "Is its SSH server running on that port?"
- "Unknown host: build.example." then "Check the name, or your network."
- "No route to build.example." then "Check the address and the network."
- "No answer from build.example." then "Check the address and the network." ssh waited 20 seconds for the server, or the time your ssh configuration sets.
- "Cannot connect to build.example." then "Check the address and the network." ssh's own words did not tell the cause, so they come last.
- "Sign-in refused by build.example." then "Check the user name, key or password."
- "No SFTP service on build.example." then "Ask for the sftp subsystem to be on."
- "The server's shell printed text first." then "Make its startup file quiet for sftp." A startup file such as `.bashrc` printed text before the protocol began.
- "The server did not speak SFTP."
- "Your ssh config has an error." then "Fix the line ssh names, then try again."
- "Connection closed by build.example." then "Open the place again to reconnect."
- "Host key not trusted. Not connected." then "Accept the new key only if you know it." You said no to a new key.
- "Host key changed. Not connected." The changed key, above.
- "It does not exist on the server."
- "Permission denied by the server."
- "The server cannot do this."
- "The file is too large for the server."
- "The server's disk or quota is full."

A server that is hard to read gets "Could not list the folder." and the like, with ssh's words after it.

### A lost connection

ssh sends keepalives every 15 seconds when your configuration sets none, and gives up on a link that misses three of them. When the link ends you hear "Connection lost: build.example." and then "Open the place again to reconnect."

diroxide tries one automatic reconnect, quietly, and only when you typed no secret for this connection. When it works you hear "Reconnected to build.example." A connection that needed a typed password waits for you.

A copy or a move on a lost server does not fail at once. The job waits, and a gate asks once for all the jobs that wait. [The transfer queue](transfer-queue.md) says how.

Cancel connection kills ssh at once, whatever it is doing, and says "Connection cancelled: build.example." The jobs that were using it wait or stop as for any lost connection.

## What diroxide asks of ssh

For anyone who wants to know what the plugin runs. It starts `ssh` with an argument list, never a shell line, and the host comes after `--`, so a host cannot be taken for an option. It asks ssh for the `sftp` subsystem, and for no terminal, no remote command, no port forwarding, no agent forwarding, no X11 forwarding and no local command. It sets the log level to errors only.

- On Windows, ssh gets `ControlMaster=no` and `ControlPath=none`, because the Windows build cannot share connections and fails on a configuration that asks for it. On Linux and macOS your own multiplexing lines are left alone.
- If your configuration turns host key checking off, the plugin asks ssh to ask instead, because a changed key must be refused.
- ssh is told not to prompt when diroxide cannot ask you, which is for a background reconnect and for ssh below 8.4.
- ssh's own debug log is off. The `ssh_debug_log` setting turns it on, into `ssh-debug.log` in the plugin's folder of the state folder. That log can hold host names, user names and key file names, so read it before you share it.

## Settings

The plugin's settings are in the Settings list, on the Plugin settings row, under SFTP places. A change reaches the plugin at once. [Plugins](plugins.md) shows the way in.

- `ssh_program`: the full path of the ssh to use. Empty finds the newest on this computer.
- `ssh_config_file`: a full path to an ssh configuration file to use instead of your own. Empty uses yours.
- `connect_timeout` (20): seconds to wait for a server, when your ssh configuration sets none.
- `keepalive_seconds` (15): seconds between keepalives, when your configuration sets none.
- `requests` (64): SFTP requests in flight for one transfer.
- `chunk_kib` (32): kilobytes in one request, at most 128.
- `sessions` (2): sessions one server may get. Two let a listing run beside a copy.
- `ssh_debug_log` (off): write ssh's debug log.

## What is not built

[Known limits](known-limits.md) has the full list. The main points:

- No editing of a remote file in place, and no synchronized browsing.
- The Sites list and other lists the plugin opens come over the panels, not inside one.
- No importing of sites from PuTTY, WinSCP or FileZilla.
- Only SFTP. There is no FTP.
- The plugin is built from the source tree. A download comes with a later build.

## See also

- [The transfer queue](transfer-queue.md)
- [Plugins](plugins.md)
- [Volumes and the trash](volumes-trash.md)
- [Security](security.md)
- [Privacy](privacy.md)
- [Operations and undo](operations.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [The documentation index](README.md)
