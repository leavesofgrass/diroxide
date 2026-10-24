# Optional components

This guide covers optional components: the plugins, helper programs and data packs that diroxide can use but does not ship inside itself. It covers what a component is, the SFTP places plugin and the Volume actions plugin, the question diroxide asks before it fetches anything, how every file is checked, `diro components` and the Manage optional components screen, the list that opens once at the first start, installing from a file with no network, removing, and where the files come from.

The short version: diroxide never fetches a component without your yes, checks every file against a size and a SHA-256 hash before it uses it, and takes away only the files it put there.

The sentences quoted here are the ones the program says, checked in tests against 40 cells. They have not yet been heard on a screen reader or a Braille display. If your experience differs, that is a finding.

## What a component is

- A helper is a program such as 7-Zip or pandoc, from its publisher, for the features that use it.
- A plugin is a program that adds a feature, such as the SFTP places plugin, that not everyone wants or that needs a license or a language the main program does not carry.
- A data pack is a set of files, such as the example plugins.

Each has an id, a title that says what it is for, a version, a size, a license, a credit and its files. Each file is pinned by name, size and SHA-256.

diroxide stays small and pure Rust because these are optional. A component comes only when you ask for it, and only after the question below.

The list has two components:

- Example plugins (id `diro-examples`): a data pack of the eleven example plugins and scripts, about 124 KB, license Apache-2.0 OR MIT. It unpacks into a folder of its own, with one folder or script for each example. Volume actions, which unmounts, ejects and mounts for the Volumes list, is one of them. Installing the pack turns nothing on. The pack is not published with a release yet. Until it is, get it from a folder with `--source`, or install it from a file, as below.
- SFTP places plugin (id `diro-sftp`, version 0.1.0-alpha.1): the plugin that opens folders on other computers over SSH, license Apache-2.0 OR MIT. It needs the OpenSSH program on your computer; [remote places over SFTP](remote-places.md) says more. It is in the list of the release files, because the release builds the program for each computer below and pins it. A build you made from source does not have it in its list, because there is no program to pin. Build the plugin yourself, as that guide says.

`diro components list` always shows what this build knows. The list is part of each build, so a component newer than your diroxide arrives with your next update.

### Which computers have a build

The SFTP places plugin has a build for three kinds of computer:

- Windows on x86-64.
- Linux on x86-64 (glibc).
- macOS, Apple silicon and Intel, as one universal file.

Windows on ARM and Linux on ARM have none yet. The Example plugins pack is the same everywhere, but its Volume actions plugin needs Python 3 and the system's own tools; [volumes and the trash](volumes-trash.md) lists them.

## Nothing is fetched without your yes

When a feature needs a component you do not have, diroxide asks at that moment. The question comes in rows, one to a line, with the size and the license before the question, so the question is the last thing you hear. The default is No.

For a helper:

```text
Reading a RAR file needs 7-Zip 25.01.
Size 3.1 MB, license LGPL-2.1-or-later.
Download it now? y or n
```

For a plugin:

```text
Opening SFTP places needs a plugin.
License Apache-2.0 OR MIT. From the public manifest.
Download diro-sftp 0.1.0-alpha.1, 4.2 MB? y or n
```

The sizes in these examples are only examples. The question gives the real size of the file for your computer.

If the component comes from your own source, the license row ends "From your own components source." and the address is never spoken.

- Say no, and you hear "Not downloaded." The feature then says in words what it cannot do. The question comes again the next time you ask, because diroxide does not remember a no between questions yet.
- Say yes, and the download starts as a job. You hear "Downloading diro-sftp, 4.2 MB." and then a progress sentence, "Downloading diro-sftp: 40 percent." A step is spoken only when the percentage crosses a tenth and at least the progress interval has passed since the last one, ten seconds by default, so a short download says little more than its start and "Ready". The start, a stop, a failure and the end are always spoken.
- Escape stops the download: "Download stopped; it resumes later." The next try goes on from the bytes already there.
- When every file has arrived: "Checking the files." Then, for an archive, "Unpacking the files." Then "Ready: diro-sftp 0.1.0." The feature starts without a restart.

When there is no build for your computer, you hear "No build for this computer." and then what to do. What it says depends on the kind of component.

For a helper:

```text
No build for this computer.
Install 7-Zip from your system's packages.
```

For a plugin of this project, such as the SFTP places plugin, it names the command that builds it on your computer from the source. You need a Rust toolchain. [Remote places over SFTP](remote-places.md) says where the program goes:

```text
No build for this computer.
Build it from source with cargo:
cargo install --locked --git https://github.com/leavesofgrass/diroxide diro-sftp
```

For anything else:

```text
No build for this computer.
Install it from a file, if you have one.
```

In the Manage optional components list the row says "no build for this computer" in place of "not installed", and Download is not offered for it. Asking for a download of such a component, with a yes in the first-run list or with `diro components download diro-sftp`, gives these lines and downloads nothing. From a shell they go to standard error, and the exit code is 1.

Before it starts, diroxide checks that there is room for the files and what they unpack to. If there is not: "Not downloaded: needs 300 MB, 120 free." and then "Make room, then try again."

Nothing is requested before your yes. The tests check this: they record every request and fail if one comes first.

## How every file is checked

- A file is pinned by its name, its size and its SHA-256. The pins are the contract, not the address. A file from the public source, a mirror, your own folder or a memory stick must match the same pin.
- A file that does not match is refused and removed: "Not installed: a file did not match." and then "Get it again, or check the source."
- A file is fetched into a staging folder and checked there. Nothing moves into the component's folder until every file has matched, so the folder never holds a file that did not.
- The list of components is part of your diroxide. It is made at build time from the real files, with their sizes and hashes computed and never typed. diroxide fetches no list from the network.
- A list from your own source may add components. It can never change the pins of a component diroxide already knows: "already known; a list cannot change its pins".
- A downloaded program gets the right to run only if the list names it as the program. Nothing it contains is run by the download.

The builds of diroxide are not code-signed, so Windows' SmartScreen or another scanner may react to a helper program the first time. That reaction is about the publisher, not about the check above.

## From a shell

`diro components` does the same jobs with plain lines. Progress goes to standard error, as sentences, never as a spinner. The exit codes are the family's: 0 done, 1 failed with nothing changed, 2 wrong use, 3 partly done, with the reasons printed. `--portable` uses the settings and the components beside the program.

```bash
diro components list
```

One line a component: the title, the state, the size, the license, what it is, and the id:

```text
Example plugins: not installed, 124 KB, license Apache-2.0 OR MIT, a data pack, id diro-examples
```

The state is "installed", "not installed", "partly installed", "damaged" or "no build for this computer". `--json` writes one JSON object a line, with the same facts and the origin, for a script.

```bash
diro components download diro-examples
```

Says what it is, the size and the license, and asks. It starts on No:

```text
Example plugins 0.1.0, diro-examples.
Size 124 KB, license Apache-2.0 OR MIT.
Download it now? y or n
```

Answer n and you hear "Not downloaded." and the exit code is 1. Answer y, or add `--yes` to answer ahead (the question is still printed, and it says "yes, from --yes"), and you hear:

```text
Downloading diro-examples, 124 KB.
Checking the files.
Unpacking the files.
Ready: diro-examples 0.1.0.
```

If it is already there: "Already installed: diro-examples 0.1.0."

`--source FOLDER` uses a folder as the components source for this one run, instead of the setting. This is how you download with no network and no public address:

```bash
diro components download diro-examples --source D:\components --yes
```

```bash
diro components verify
```

Hashes every installed file. With nothing wrong: "Files check out: Example plugins." With a problem, the count, then a line for each file with the state first, and the exit code 1:

```text
1 file does not check out: Example plugins.
Wrong size: plugins/memfs/plugin.toml
```

The state of the component is then "damaged". Remove it and download it again. Name one component to check just that one: `diro components verify diro-examples`.

```bash
diro components remove diro-examples
```

Lists every path it will remove, one a line, then "Removes 28 files." and the question "Remove Example plugins? y or n". It starts on No. A yes removes those files and nothing else: "Removed: Example plugins." A file you put in the component's folder stays, and the count line says "Yours stay." The list is printed first and the removal works from that list.

```bash
diro components install diro-examples D:\components\diro-examples-0.1.0
```

Installs from a folder or a zip file you already have, with no network. See below.

An unknown id says "Could not find the component nothing. Run diro components list for the ids." and exits with 1.

## Manage optional components

In the program, Manage optional components is in the Tools menu (Alt+T, then O) and in the command palette. It opens a list named "Optional components", one row for each component, the title first and the state after it:

```text
Example plugins: installed, 124 KB, license Apache-2.0 OR MIT
SFTP places plugin: not installed, 4.2 MB, license Apache-2.0 OR MIT
```

The size of the second row is only an example. The row is in the list of the release files; in a build made from source the list holds only the first. On a computer with no build, the second row says "no build for this computer" where it says "not installed" here.

Enter on a row opens a list named for the component, with what can be done now:

- "Download" when it is not installed, or "Download again" when it is partly installed or damaged. It asks the question above.
- "Verify the files" and "Remove", when something is installed. Verify says "Checking the files." and then "Files check out: Example plugins." or, with a problem, a list that names each file. Remove lists every path it will take away, then "Removes 28 files. Yours stay.", and asks "Remove Example plugins? No is the default. y or n". A no says "Nothing was removed."
- "Install from a zip file" and "Install from a folder", always. Each opens a field, "Install Example plugins from a folder", where you type a place on this computer. The install runs as a job, "Installing Example plugins".

In the program, the question puts the first line and the question together, and lists the size and the license under them. The Example plugins question reads "Getting example plugins needs Example plugins 0.1.0. Download it now? No is the default. y or n", and under it "Size 124 KB, license Apache-2.0 OR MIT."

The Plugins menu has Get example plugins (Alt+P, then G), which asks for the pack with that question. If the pack is already installed, it says "Already installed: Example plugins. Manage plugins lists them." After a yes, you hear "Ready: diro-examples 0.1.0.", and Manage plugins lists the examples, each "not enabled".

A download runs as a job. The jobs line shows its progress in words, a tenth at a time and never more often than the progress interval. Escape stops it: "Download: cancelling", and the next try goes on from the bytes already there.

The Tools menu also has Ask first-run choices again (Alt+T, then F). The next section is about it.

## The first-run list

At the very first start, when diroxide has no session of yours yet, a list named "Optional components to get now" opens once. Nothing is chosen in it. It holds every component except the example plugins pack, so in the release files that is the SFTP places plugin. A build whose list has nothing else to offer does not open it, and the first start goes on without it.

```text
SFTP places plugin: not installed, 4.2 MB, license Apache-2.0 OR MIT
```

- Space marks the row you are on, or unmarks it, and moves down. You hear "Marked" or "Unmarked".
- Enter with nothing marked says "Nothing marked. Space marks one; Escape skips." and the list stays.
- Enter with some marked asks once for all of them. The question starts on No, and the marked rows are listed under it: "Download 1 component? No is the default. y or n".
- n says "Not downloaded." and goes back to the list with your marks kept. y starts each download as a job in the jobs line, and the list closes. Each download then goes through the steps above, and a computer with no build hears the "No build for this computer." lines instead.
- Escape skips the list: "Nothing downloaded. Tools, Manage optional components has them."

Nothing is fetched before a yes. The list is shown once: opening it turns on the setting First-run list shown (`components_first_run_shown`), and it does not come back at the next start.

To see it again:

- The setting Ask about components again (`components_ask_again`, in the Plugins group, off by default). Turn it on, and the list opens once at the next start with nothing chosen. The setting turns itself off when the list opens.
- Ask first-run choices again, in the Tools menu (Alt+T, then F), opens the list now. When there is nothing to choose from, it says "No optional components to choose from yet", and Escape says "Nothing downloaded. Tools, Manage optional components has them."

The list opens only over an empty screen, so it never covers a dialog that is open at that moment. If that happens, the next start is no longer a first start, and Ask first-run choices again opens the list later. [Settings and where the files live](settings.md) lists both settings.

## Install from a file

A computer with no network, or a download that keeps failing, can still get a component. Use Install from a zip file or Install from a folder in the Manage list, or `diro components install ID PATH`.

diroxide looks in the folder or the zip for each file the component is pinned to, by its name, or by its size and hash. It checks each against the same pins. A file that is not one of the component's files is left out with the reason: "Left out a.txt: not one of the component's files." Success is "Ready: diro-examples 0.1.0." A folder or a zip that holds none of its files says "Not downloaded: it was not found in" and the name, and installs nothing.

When a component is an archive, diroxide unpacks it itself, with strict rules:

- Every member must have a plain name. A member that would leave the folder, with `..`, an absolute path, a backslash, a Windows device name, a colon (which could name a hidden stream on NTFS) or a name that differs from another only by case, is left out with a line: "Left out ../x: name leaves the archive."
- A link inside the archive is never made.
- The archive may not expand past the size the list states, or have too many members. An archive that tries is stopped and what it wrote is removed.
- A receipt, a small file named after the archive, takes the archive's place and lists every file that was written. Remove takes away exactly those files.

## Where components live

- Under the `components` folder of the state folder, one folder for each component: `components/diro-examples`. With `--portable`, beside the program.
- During a download there is a staging folder, `diro-examples.partial`, beside the component's folder, with a lock file inside it. A second program that tries to fetch the same component hears "Already downloading: diro-examples." and "Wait for it to finish."
- A plugin component such as the SFTP places plugin unpacks to its own folder, `components/diro-sftp`, with its `plugin.toml` and its program. The program looks there, and Manage plugins lists the plugin, not enabled. Turning it on is the usual question; downloading it never turns it on.
- The example pack unpacks to `components/diro-examples/plugins`, with one folder for each plugin and a `scripts` folder for the scripts. In the program, Manage plugins lists them there, each with "example" beside it, and a plugin of your own with the same id wins. From a shell, copy one into your plugins folder to use it:

```bash
diro plugin install changed-files
```

Run it from that `plugins` folder, or give the full path to the one you want. It says "Installed: Changed files. It is not enabled." and nothing runs until you turn it on.

## Where files come from

diroxide tries these addresses for each file, in order:

1. Your own source, if you set one (below).
2. The file's address in the list, or, when it has none, the project's public releases. Each component is published under its own tag there, its id and its version, such as `diro-examples-0.1.0` and `diro-sftp-0.1.0-alpha.1`.

It fetches with a small client built into diroxide. If that client cannot reach the server at all, because there is no answer, no connection or a TLS problem, diroxide tries `curl` and then `wget` from your PATH. A server's answer, such as "not found", is final and is not retried with another program. A build of diroxide made without the built-in client uses `curl` and then `wget` alone. When none of them is there you hear "Not downloaded: curl was not found." and "Install curl, or install from a file."

What a request carries:

- `https` only. A redirect to anything else is refused. At most five redirects.
- One identifying header, the neutral User-Agent `diroxide-research (+https://diroxide.app)`. It names the project and nobody else.
- To resume a stopped download, a range and a validator that make the server answer from the byte asked for. A server that ignores the range, or answers from the wrong byte, is detected and the file starts again from the beginning. A file that fails its check after a resume is fetched once more from zero before diroxide gives up on that source.
- The size is pinned, so more bytes than the pin are refused as they arrive. A transfer that stalls is stopped.

A request does reveal your computer's network address to the server, as any web request does. [Privacy](privacy.md) says what else leaves your computer, which is nothing.

### Your own source

The setting `components_source` names where you keep your own copy of the components, tried before the public address. It is yours alone: put it in your own `config.toml`, and nothing else names it.

```toml
components_source = "D:/components"
```

- A folder: files are read from `FOLDER/ID-VERSION/FILENAME`, for example `D:/components/diro-examples-0.1.0/diro-examples-0.1.0.zip`. A `components.toml` at the top of the folder may add components.
- A repository written `owner/name`: its release downloads are used the same way.
- The question says "From your own components source." and never says the address.
- No token is sent today. A private source that needs a sign-in, such as a private repository reached through the GitHub CLI, is not built yet. When it is, the token will go only to that source, never as a command-line argument.

Whatever the source, a file is accepted only when it matches the pin. A source is trusted by the pins, not by its address.

## See also

- [Plugins](plugins.md)
- [Remote places over SFTP](remote-places.md)
- [Volumes and the trash](volumes-trash.md)
- [Writing a plugin](writing-a-plugin.md)
- [Script commands](script-commands.md)
- [Security](security.md)
- [Privacy](privacy.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
