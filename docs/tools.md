# Tools and diro doctor

This guide covers the helper programs diroxide can use, such as ImageMagick, FFmpeg and pandoc. It says how diroxide finds each one, how to tell it where one lives or to leave one alone, what the Tools list in the program says, what `diro doctor` reports and what its exit codes mean, and the install command for every helper on each system.

The short version: diroxide works without any of these programs. A feature that needs one says so, names it, and says how to get it. diroxide never runs an install command, and it never downloads a program without asking first.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## What a tool is

A tool is a program on your computer that diroxide starts for you, always with a list of arguments and never through a shell. Most are optional helpers. Two are special: the system opener, which starts a file in its default program, and carta, the document converter that is built into `diro` itself and so is always found.

The registry knows 22 tools on Windows and Linux, and 23 on macOS, where `mdls` is added. diroxide lists them in this order, and the Tools list and `diro doctor` use the same order.

- ImageMagick (`magick`): image details and conversions.
- FFmpeg (`ffmpeg`): audio and video conversions.
- FFprobe (`ffprobe`): audio and video details. It comes with FFmpeg.
- pandoc (`pandoc`): document previews and conversions.
- 7-Zip (`7z`): reading RAR, CAB and other archives, and cross-checking.
- pdftotext (`pdftotext`): PDF text previews.
- pdfinfo (`pdfinfo`): PDF details. It comes with the same package as pdftotext.
- ExifTool (`exiftool`): photo, audio and document details.
- Tesseract (`tesseract`): reading text in pictures.
- qemu-img (`qemu-img`): converting disk images.
- xorriso (`xorriso`): making ISO images.
- GnuPG (`gpg`): OpenPGP encryption and signatures.
- gpgv (`gpgv`): checking OpenPGP signatures. It comes with GnuPG.
- age (`age`): age encryption.
- minisign (`minisign`): checking minisign signatures.
- git (`git`): git status and commands.
- GitHub CLI (`gh`): GitHub from git places, and private component sources.
- The system opener (`open`): opening files in their default programs. It is Explorer on Windows, `open` on macOS and `xdg-open` on Linux.
- textweaver (`tw`): reading a document aloud, from the quick view.
- curl (`curl`): downloading components when the built-in client is off.
- wget (`wget`): downloading components when curl is missing.
- mdls (`mdls`), on macOS only: Spotlight's details of a file. It comes with macOS.
- carta (`carta`): document conversions in `diro` itself. It is built in, version 0.0.13, so it is always found.

The id in code type is what you write in `tools.toml` and after `diro doctor --check`.

## Where diroxide looks

For each program, diroxide looks in this order and stops at the first answer:

1. The `path` you gave in `tools.toml`.
2. An installed helper component, from [Optional components](components.md).
3. PATH, folder by folder. A relative folder on PATH is skipped, so the current folder is never searched.
4. The folders the program's own installer uses.

The report says which step found it, after the version:

- "from tools.toml".
- "from components".
- "on PATH".
- "in Program Files", "in Git for Windows", "in Gpg4win" and "in your programs folder" on Windows.
- "in Homebrew" on macOS, where a window does not get the shell's PATH, so diroxide looks in Homebrew's two folders as well.
- "built in", for carta.

Some details of the lookup:

- On Windows a name without an extension tries the extensions in PATHEXT first, and the bare name last, because a bare script beside `whisper.exe` is often something Windows cannot start. Only `.com`, `.exe`, `.bat` and `.cmd` count.
- Only `magick` counts as ImageMagick on Windows, because `convert.exe` in the system folder is a disk tool. On Linux and macOS ImageMagick 6's `convert` counts when its version line says ImageMagick.
- 7-Zip is looked for as `7z` and `7zz` first, then `7za` and `7zr`. The last two read fewer formats and no RAR, so they come last. On Windows the folders are Program Files and Program Files (x86).
- Pandoc, Tesseract, GitHub CLI, git and GnuPG have install folders on Windows. Git for Windows keeps its `gpg` off PATH, so diroxide looks in Git for Windows' own folder for it.

To learn a program's version, diroxide runs its version command and nothing else: `ffmpeg -version`, `pandoc --version`, `7z i`, `pdftotext -v`. Each run is given two seconds at most, and the runs go side by side. Some programs, such as pdftotext and pdfinfo, write their version on standard error; diroxide reads that too.

## tools.toml

`tools.toml` is in the config folder. [Settings and where the files live](settings.md) says where that is on each system. It holds one table for each tool, named by the tool's id, and three keys:

- `path`: the program's full path, used instead of looking anywhere else.
- `disabled`: set to `true` to never use the tool.
- `needed`: set to `true` to say you need the tool. `diro doctor --check` with no tool named checks every tool marked needed.

```toml
[pandoc]
path = "D:\\tools\\pandoc.exe"

[ffmpeg]
disabled = true

[magick]
needed = true
```

The words each choice produces:

- A path with nothing at it stops the lookup there, because you chose that path: "pandoc: not at its tools.toml path", then "Check its path in tools.toml."
- A tool you turned off: "FFmpeg: turned off in tools.toml". A feature that needs it says so: "Needs ImageMagick, off in tools.toml."

## What a line says

Every report line puts the tool's name first, then one fact. The same lines serve the Tools list and `diro doctor`.

- A tool found, with its version and where: "ImageMagick: 7.1.2-35, on PATH".
- A program with no version command, such as the system opener on Windows: "the system opener: found, on PATH".
- A program whose version command said nothing readable, or did not finish in time: "Tesseract: version not read, in Program Files".
- A tool not found: "pandoc: not found". When a helper component would bring it, the line says what that would download for your computer: "pandoc: not found, 43.0 MB to download". The next line is the next step.
- carta: "carta: 0.0.13, built in".
- The count, which comes first in a list: "Tools: 13 of 22 found".

The next step for a tool not found is one line of its own. It is the install command for your system when there is one, and a sentence otherwise: "Install: winget install JohnMacFarlane.Pandoc", or "Install xorriso from your package manager."

diroxide reads your system to choose the command. Windows names winget, macOS names Homebrew, and Linux reads `/etc/os-release` and names apt for Debian and Ubuntu and their relatives, dnf for Fedora, Red Hat and CentOS and their relatives, and pacman for Arch and its relatives. Any other system gets the sentence instead. diroxide never runs the command. It is there to read or to copy.

## The Tools list

The Tools list shows each helper program and where it was found. Open it from Settings (the row "Tools list, each helper program and where it was found"), from the Tools menu (Alt+T, then T), or from the palette. While the programs are looked for, you hear "Looking for the helper programs".

The list's title is the count, "Tools: 13 of 22 found". Each row is a tool, its name first: "ImageMagick: 7.1.2-35, on PATH". A tool not found has its next step after its line, so the row reads "Tesseract: not found, Install: winget install UB-Mannheim.TesseractOCR".

When a helper component would bring a missing tool, the row says what it would download: "FFmpeg: not found, 138 MB to download". The size is an example. [Optional components](components.md) says what a helper component is. A build lists a helper component only once its files are pinned by size and SHA-256, so until then a missing tool's row shows the install command alone.

Enter on a row does the one useful thing:

- With a helper component for the tool, it asks first, as every download does, and the question starts on No.
- With an install command, it copies the command to the clipboard and says "Copied: winget install UB-Mannheim.TesseractOCR". You run it yourself.
- With only a sentence, it says the sentence. With a path that has nothing at it, it says "Check its path in tools.toml."
- On a tool that is found, built in or turned off, it says the line again.

Escape closes the list. Nothing is downloaded or installed without a yes.

## diro doctor

```bash
diro doctor
```

`diro doctor` says what this `diro` has and finds, one fact a line, the name first. Pipe it to a file and send it with a bug report; a screen reader and a Braille display can take it a line at a time, and nobody has to read a screenshot. It has these sections, in this order:

1. The program: "diro:" and its version, "System:" and the system, then "Built in:" and what this build includes: the terminal, the GUI, the downloader and the syntax set.
2. The screen reader mode and how it was decided: "Screen reader mode: on, the default", "Screen reader mode: on, from settings" or "Screen reader mode: off, from settings".
3. The terminal: "Terminal: standard output is a terminal" or "Terminal: output goes to a file or pipe", its size when it is one ("Terminal size: 120 by 30"), and what the terminal answered in your last terminal session, read from the log and never asked for now, so no escape sequence is sent. With nothing recorded: "Last terminal session: none recorded".
4. The folders: "Config folder:", "State folder:", "Components folder:" and "Cache folder: none, kept in memory". With `--portable`, "Settings: beside the program, --portable" comes first.
5. The tools: the count, then each tool's line, each missing tool followed by its next step.
6. The components: "Components: 0 of 6 installed", then each component and its state.
7. The plugins: "Plugins: none", or the count and one line for each plugin.
8. "Startup problems: none", or the files with problems and how many each has.

With `--check`, one more line follows, and it decides the exit code.

### Options

- `--check [TOOL...]`: say which needed tools are missing, and exit with 1 when one is. Name tools by name or by id, in any case: `diro doctor --check pandoc ffmpeg`. With none named, the needed tools are those your `conversions.toml` recipes run, and any tool marked `needed` in `tools.toml`.
- `--paths`: say the folders in full. Without it, a folder under your home folder is said from `~`.
- `--portable`: use the settings and components beside the program.
- `--help`: the usage, one line for each form.

The `--check` line says "Needed tools: none named", "Needed tools: all 2 found", or "Needed tools missing: FFmpeg".

### Exit codes

- 0: done, and nothing needed is missing. Without `--check`, a missing tool never changes the exit code.
- 1: with `--check`, a needed tool is missing.
- 2: wrong use, such as an unknown option, or a tool name that is not in the table: "no tool is called nope; diro doctor lists the tools".

### What it never says or does

- It says no secret and no environment. A tool is said by where it was found, never by its path.
- A folder in your home folder is said from `~` unless you ask for `--paths`.
- It runs nothing but the version commands above, two seconds each at most.

A sample, from a computer with ImageMagick missing and FFmpeg turned off:

```text
Tools: 13 of 22 found
ImageMagick: not found, 12.1 MB to download
Install: winget install ImageMagick.ImageMagick
pandoc: 3.9.0.2, on PATH
7-Zip: 24.08, in Program Files
FFmpeg: turned off in tools.toml
carta: 0.0.13, built in
Components: 0 of 6 installed
Example plugins: not installed
```

## The install commands

These are the names in diroxide's own table. The line "Install:" says the one for your system. Where a package manager has no package for a program, the line says so below.

- ImageMagick: winget `ImageMagick.ImageMagick`, Homebrew `imagemagick`, apt `imagemagick`, dnf `ImageMagick`, pacman `imagemagick`.
- FFmpeg and FFprobe: winget `Gyan.FFmpeg`, Homebrew `ffmpeg`, apt `ffmpeg`, dnf `ffmpeg-free`, pacman `ffmpeg`.
- pandoc: winget `JohnMacFarlane.Pandoc`, Homebrew `pandoc`, apt `pandoc`, dnf `pandoc`, pacman `pandoc-cli`.
- 7-Zip: winget `7zip.7zip`, Homebrew `sevenzip`, apt `7zip`, dnf `7zip`, pacman `7zip`.
- pdftotext and pdfinfo: winget `oschwartz10612.Poppler`, Homebrew `poppler`, apt `poppler-utils`, dnf `poppler-utils`, pacman `poppler`.
- ExifTool: winget `OliverBetz.ExifTool`, Homebrew `exiftool`, apt `libimage-exiftool-perl`, dnf `perl-Image-ExifTool`, pacman `perl-image-exiftool`.
- Tesseract: winget `UB-Mannheim.TesseractOCR`, Homebrew `tesseract`, apt `tesseract-ocr`, dnf `tesseract`, pacman `tesseract`.
- qemu-img: winget `SoftwareFreedomConservancy.QEMU`, Homebrew `qemu`, apt `qemu-utils`, dnf `qemu-img`, pacman `qemu-img`.
- xorriso: no winget package, Homebrew `xorriso`, apt `xorriso`, dnf `xorriso`, pacman `libisoburn`.
- GnuPG and gpgv: winget `GnuPG.Gpg4win`, Homebrew `gnupg`, apt `gnupg` (and `gpgv` for gpgv alone), dnf `gnupg2`, pacman `gnupg`.
- age: winget `FiloSottile.age`, Homebrew `age`, apt `age`, dnf `age`, pacman `age`.
- minisign: no winget package, Homebrew `minisign`, apt `minisign`, dnf `minisign`, pacman `minisign`.
- git: winget `Git.Git`, Homebrew `git`, apt `git`, dnf `git`, pacman `git`.
- GitHub CLI: winget `GitHub.cli`, Homebrew `gh`, apt `gh`, dnf `gh`, pacman `github-cli`. Debian's own repositories do not carry it.
- The system opener, on Linux: apt, dnf and pacman `xdg-utils`. Windows and macOS have it already.
- curl: Homebrew, apt, dnf and pacman `curl`. It comes with Windows 10 and later.
- wget: Homebrew, apt, dnf and pacman `wget`. No winget package.
- textweaver: no package. Its line says "Install textweaver from its releases page or with cargo."

These names were written from each package manager's usual names. They have not all been checked against a live package manager. The ones to doubt are pacman's `pandoc-cli`, several of the dnf names, the Gpg4win folders on Windows, and winget's Poppler id, which is a community build. If a command fails, [report it](troubleshooting.md) with the line, and use the program's own web page meanwhile.

## What has not been tried

- macOS: the Homebrew folders and `mdls` were never compiled or run by the lane that wrote the lookup.
- System32's `convert.exe` and `tw --version` were left alone on purpose.
- The registry's tests use stand-in programs. The version commands were run against the real programs in the development container and on one Windows computer, and no real helper component has been installed yet.
- Nothing here has been heard on a screen reader or read on a Braille display. The "Install:" lines and the "Built in:" line are longer than 40 cells, so they pan on a Braille display; the name comes first on each.

## See also

- [Optional components](components.md), for helper components and `diro components`
- [Settings and where the files live](settings.md), for the config folder
- [The quick view](quick-view.md), which calls some of these programs
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
