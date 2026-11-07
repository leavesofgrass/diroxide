# File types and groups

This guide covers how diroxide knows what a file is: the type each row says, how a type is found, the groups that gather types, the file `filetypes.toml` where you add your own, the retro pack of old formats, the setting that keeps slow folders from being read, what Enter, View and the numbered user actions do for each type, Open with and the programs each system lists, and the encoding chooser.

The short version: a row says its type at once from its name, and a background check reads the first 64 bytes of files whose names say nothing. The type decides which group a file is in, and which actions Enter and View run.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## What a row says

The Type column and the details say a type in words: "PNG", "shell", "PDF". There are four other things a row can say in its place.

- "folder" for a folder. A folder is never tested.
- "type pending" where the name says nothing, such as a file with no extension, until the check has read the file's first bytes: "n00, type pending, 20 bytes". The check runs in the background, starts with the rows you are looking at, and starts again from the new place when you scroll a page away.
- "unknown" when no type takes the file.
- A type's own short name once it is known: "photo.png, PNG, 20 bytes".

When the check settles a row, the row changes quietly. Nothing is announced and nothing is read aloud, so a folder of files without extensions does not talk while it works.

When a file's name and its contents disagree, diroxide says so only when you ask:

- In the details, a row says "Name says PNG; contents say JPEG", and another says "File group: Images".
- When you press Enter on that file, the opening line carries it, with the meaning first: "Opening photo.png. Name says PNG; contents say JPEG."

The contents speak first. A `.py` file that starts with `#!/bin/sh` is a shell script. When the contents only refine the name, as a zip that is a Word document does, nothing is said.

## How a type is found

File types form a tree. A file is tested in three stages, always in this order:

1. A signature: bytes at fixed places in the file's first 64 bytes. `??` stands for any byte.
2. A condition: one test in [the filter language](search.md) over the entry and its first 64 bytes, such as `content:re:"^#!"`.
3. The name: an extension, or a name pattern.

For each type, the first stage that passes is its hit. The walk starts at the types with no parent and goes into a type's children only when that type was hit, or has no tests of its own (a heading over a family), or could not be decided because the contents were not read yet. That is how a child refines its parent: DOCX under zip, or a camera's raw file under TIFF. A file named `report.docx` reads as a Word document at once, before its first bytes are read.

Among the types hit:

- A child always beats its parent, whatever the numbers. DOCX is a child of zip by its name, so it beats zip's signature.
- Otherwise the higher `priority` wins. Priorities are never added together.
- Still tied, the higher stage wins (signature, then condition, then name), and then the earlier type.

Some details:

- A signature of fewer than three fixed bytes counts only when the type's condition or name passes too, because two bytes such as gzip's start plain text files by chance.
- `content` in a condition is the first bytes read as text, with bytes that are not text replaced. It can look for text markers such as `#!`, `<?php` or `<!doctype html`, and for words inside a header. Bytes that are not text belong in a signature. When the first bytes have not been read yet, a content test is unknown and the child types are still tried.
- Name patterns ignore case. A pattern is a glob over the whole name (`*.tar.gz`), several globs separated by commas (`*.tar.gz, *.tgz`), or `re:` and a regular expression over the name.
- The content stages are skipped for a link, a special file, an empty file, an online-only file, and a file in a folder you listed in `check.skip`. The name still runs, and so does a condition that does not read contents.

## Folders that are not read

Reading a file's first bytes on a slow share is slow. List such folders in the `check.skip` setting, in the Settings list or in `config.toml`, and files there are typed by their names alone.

```toml
[check]
skip = ["\\\\nas\\share", "/mnt/slow"]
```

- A folder in the list covers everything below it. A sibling that only shares a prefix is not below it.
- Backslash and slash are the same, and case is ignored.
- Files inside archives, in plugin places and in remote places are typed by name alone, with or without this setting.
- In a place typed by name alone, a file whose name says nothing reads "file", as in "data, file, 5 bytes", and never "type pending", because nothing will be read.

## The shipped types and groups

diroxide ships a table of 178 types, with their media types, and 8 groups. Each type has one group. The groups:

- Images: pictures and photos, such as PNG, JPEG, GIF, WebP and camera raw files.
- Music: sound and music, such as MP3, FLAC, Ogg and WAV.
- Video: MP4, Matroska, WebM, QuickTime and the like.
- Documents: documents, sheets, slides and e-books, such as PDF, Word, OpenDocument and EPUB.
- Archives: archives and disk images, such as zip, 7z, tar and its compressed forms, and ISO.
- Code: source code, scripts and settings files, such as Rust, C, Python, HTML, JSON and TOML.
- Text: plain text, notes and logs, such as TXT, Markdown, CSV and INI.
- Screenshots: screen pictures, found by their names alone.

Every shipped type name fits 40 cells. The longest is "PowerPoint 97 presentation", which the details say as "Type: PowerPoint 97 presentation".

### The Screenshots group

A screenshot is in two groups, Images and Screenshots, so `group:images` finds it and so does `group:screenshots`. The group knows each system's names:

- Windows, Windows key with Print Screen: `Screenshot (3).png`, the word Screenshot, a number in parentheses.
- macOS, older and newer: `Screen Shot` or `Screenshot`, the date, "at", the time with dots, and an optional AM or PM, as `.png`. On macOS 14 and later the space before AM or PM is a narrow no-break space, and the pattern accepts it.
- iOS: a PNG named like a camera file, `IMG_` and four digits, with a capital `.PNG`. This is a weak sign.
- Android, stock and Samsung: `Screenshot_`, eight digits, a dash or underscore, six digits, as `.png`, `.jpg` or `.jpeg`.
- GNOME: `Screenshot from`, the date and the time with dashes, as `.png`.
- KDE's Spectacle: `Screenshot_`, eight digits, an underscore, six digits, as `.png`.

## The retro pack

The retro pack knows the old formats: Amiga and tracker music, PowerPacker, the IFF family, old office files and the like, 40 types. It is built into the program and is off. Off, those files read as unknown. The setting is `filetypes.retro`, and the last row of the Groups list turns it on and off: "Retro types: off", then "Retro types: on".

## Your own types and groups: filetypes.toml

`filetypes.toml` is in the config folder ([settings](settings.md) says where). It has the same shape as the table diroxide ships, and it is read after it. A type or group of yours with the id of a shipped one replaces it. When two of yours have the same id, the first wins.

A type is a `[[type]]` table. Every key is optional except `id`:

- `id`: a stable id, such as `pdf`.
- `name`: the name said in the details, "PDF document".
- `short`: the short name for rows, "PDF".
- `mime`: the media type, `application/pdf`.
- `group`: the group's id, one group a type.
- `parent`: the type this one refines.
- `priority`: a number. Higher wins among types that are not parent and child.
- `extensions`: a list, without the dot.
- `patterns`: name patterns, as above.
- `magic`: signatures, each `{ at = 0, hex = "25 50 44 46" }`, with `at` from 0 to 63 and `??` allowed in the hex.
- `when`: a condition, one filter expression over the entry and its first bytes.
- `retro`: `true` to put the type in the retro pack, read only while the retro setting is on.
- `[[type.actions]]`: the actions, below.

A group is a `[[group]]` table with `id`, `name`, `about` (one sentence) and `patterns`. The patterns put a file in the group by its name whatever its type, as the Screenshots group does.

```toml
[[group]]
id = "receipts"
name = "Receipts"
about = "Receipts, by their names."
patterns = ["receipt-*"]

[[type]]
id = "pdfa"
name = "PDF/A document"
short = "PDF/A"
parent = "pdf"
extensions = ["pdfa"]
```

What goes wrong is said at start, meaning first, with the file's name in the config problems list: "filetypes.toml: 1 problem. Config problems in the Help menu lists them." Each problem is one line that starts "File type NAME:", and the rest of the type is used:

- A signature that does not read is left out: its line says why, such as the text not being whole pairs of hexadecimal digits, or ending past the first 64 bytes.
- A condition or a pattern that does not read is left out the same way.
- An action with no command, tool or plugin: "File type half: action Nothing left out. It names no command, tool or plugin."
- A parent that no type has leaves the type out: "File type orphan left out." followed by the reason, "Type orphan names the parent nothing, which no type has."
- A slot that is not one of the slots: "File type zip: slot user12 unknown. Action Count is listed by name."

## Groups in the program

Three things use groups: the Groups list, Select by kind, and the `group:` test.

The Groups list is in the Select menu, and in the palette. It lists the groups with how many files of each are here, then a last row for the retro types. Its title says how many groups there are: "Groups: 8 built in, 1 yours".

- "Images: 2 files here", "Music: no files here", "Text: 1 file here".
- "Receipts: 2 files here", for a group of yours.
- "Retro types: off".

Enter on a group selects its files and says so: "Selected 2 receipts. 2 selected." Enter on the last row turns the retro types on or off and says "Retro types: on".

Select by kind lists the kinds, then each group, yours included: "Receipts, group", "Screenshots, group".

`group:` in a filter takes any group by its id, yours too: `group:receipts`. A file in two groups matches either. Folders and plugin items are in no group. Before the check has settled a file whose name says nothing, its group is not known, and a filter that depends on it is undecided for that row.

A theme file can map a group to one of its existing color roles with a `[group-colors]` table. [Themes and colors](themes.md#group-colors) has the words. The window draws those colors on the rows of plain files and archives, the terminal does not yet, and the row's words never change.

## What Enter and View do for a type

Each type has actions. An action is one of three things: a command of diroxide's, a tool from [the tools list](tools.md) run with arguments, or a command of a plugin. They are never a shell string. A type gives an action a slot, or no slot, and the slot says when it runs.

- `enter`: what Enter runs on a file of this type. With none, Enter opens the file in the system's default program, as before.
- `view`: what View (F3) runs. With none, the built-in viewer.
- `preview` and `convert`: reserved for the Preview and Convert commands. For now they run from the Actions list only.
- `user0` to `user9`: the ten numbered user actions. The commands User action 0 to User action 9 run them. They have no key until you bind one in `keymap.toml`. A file whose type has none says "notes.txt has no user action 3."
- No slot: a named action, listed by Actions for this file.

A type with no actions of its own takes its parent's.

```toml
[[type]]
id = "pdf"
name = "PDF document"
short = "PDF"
group = "documents"
extensions = ["pdf"]

[[type.actions]]
name = "To text"
slot = "convert"
tool = "pandoc"
args = ["{file}", "-o", "{name}.txt"]

[[type.actions]]
name = "Make thumbnail"
slot = "user3"
tool = "magick"
args = ["{file}", "thumb.png"]
```

An action names one of `command` (a command id), `tool` (a tool's id, with `args`) or `plugin` (a plugin's name, with its command as the first of `args`). A tool's arguments are filled one at a time, so a name with spaces or quotes arrives whole:

- `{file}`: the focused file's full path.
- `{name}`: the focused file's bare name. A name that starts with a dash gets `./` in front, so a tool never reads it as an option.
- `{dir}`: the active panel's folder.
- `{other}` and `{dest}`: the other panel's folder.
- `{files}`: every selected file, each its own argument. It must stand alone as an argument.

### Actions for this file

Actions for this file is in the File menu and the palette. It lists the focused file's actions, the one Enter runs first, each with its slot in words:

```text
Actions for report.pdf
Open in reader, Enter
View, the built-in viewer
To text, Convert
Make thumbnail, unavailable, user action 3, Needs ImageMagick, not found
Say hello
```

A tool action whose tool is missing is shown unavailable, with why: "Needs ImageMagick, not found." or "Needs ImageMagick, off in tools.toml." Running one gives the same words. Running one that is here says "Running To text on report.pdf".

The program matches an action to its slot by the action's name, so give each action of a type a name of its own.

## Open with

Open with is in the File menu and the palette. It lists the programs your system has for the focused file, and Enter starts the one you choose. The system is asked in the background, so you hear "Finding programs for report.pdf" first. Then the list opens with its title and its first row: "Open report.pdf with: Default program, Reader, 1 of 4".

- "Default program" is the first row. It opens the file as Enter does, in the system's own choice.
- Then each program by name. The system's default says "Reader, the default".
- On Windows, "Choose another program" is the last row.
- Enter on a program says "Opening report.pdf with Writer".

No program is started through a shell, so a name with a space or a quote arrives whole. Each program is started and not waited for.

How each system builds its list:

- Windows reads the programs registered for the file's extension in the registry, your own settings before the machine's, with each program's own open command. A handler with no command line, such as a Store app like Notepad, is started through the system by its class, the way Explorer does. "Choose another program" hands the file to Windows' own Open with dialog. That dialog read well with NVDA when it was tried; JAWS has not been tried.
- Linux and the BSDs read `mimeapps.list`, `mimeinfo.cache` and each program's desktop file, by the file's media type. Programs that run in a terminal are left out.
- macOS reads the application folders, and each application's `Info.plist` for the kinds of document it declares, and starts the program with `open -a`. An application whose `Info.plist` is binary is not read, and the list says how many: "3 programs not listed: binary Info.plist". macOS has no "Choose another program" row.

On a system with no dialog of its own, there is nothing to choose in: "No Open with dialog on this system."

## The encoding chooser

The viewer and the quick view decide how to read a text file from its first bytes: a byte-order mark first, then a declaration in the file such as an HTML `charset` or an XML declaration, then UTF-8 when it is valid, and Windows-1252 otherwise. UTF-16 with no byte-order mark is recognized when nearly every other byte is zero. ISO-8859-1 is read as Windows-1252, as browsers read it.

When that is wrong, Encoding lets you choose. It is in the Viewer and Preview submenus of the View menu and in the palette. It opens a list that is titled with the file: "Encoding of legacy.txt".

- The first row is "Automatic", with what it found: "Automatic: Windows-1252 (not UTF-8)" or "Automatic: UTF-8 (byte-order mark)".
- Then 29 encodings grouped by language, the best known of each first: Unicode, Western European, Central European, Cyrillic, Greek, Turkish, Hebrew, Arabic, Baltic, Vietnamese, Thai, Japanese, Chinese and Korean. Each row is the language and then the encoding: "Western European (Windows-1252)", "Cyrillic (Windows-1251)".
- Enter reads the file again in that encoding and says "Encoding Windows-1251, chosen by you".
- The list opens on your last choice. Choose Automatic to go back.

The choice lasts for that file in that session. Outside the viewer and the quick view, Encoding says "Encoding needs the viewer or quick view." Of the old DOS code pages only Cyrillic IBM866 is there.

## Known limits

- The shipped table has 178 types, not the hundred or so first planned, and `tar` is typed by its name only, because its marker sits past the first 64 bytes. `.ts` is TypeScript; transport streams are `.mts` and `.m2ts`.
- A zip-based file is told apart from the 64-byte head alone only for EPUB, JAR and an APK whose manifest comes first, and for Office files whose own folder comes first. Office files that start with `[Content_Types].xml` read as "Office Open XML file", and OpenDocument is cut short. XPS and CBZ are typed by name only. An old-style APK reads as a JAR by its head. With its real name the more exact type wins every time.
- A type's child is tried only through its parent. A type of yours with an extension and no signature, used as a parent, can hide a child that is known by its own extension.
- An action's slot is matched by the action's name, so each action of a type needs a name of its own.
- The Preview and Convert slots run from the Actions list only, for now.
- Group colors in a theme file are drawn by the window and not yet by the terminal.
- Windows: Explorer's own order of programs is not read, and Store apps, Edge and Chrome start by class through the shell. Explorer also lists shell verbs that other programs add, which the list does not read.
- Linux: programs that run in a terminal are left out of Open with.
- macOS: a binary `Info.plist` is skipped and counted, the default program is not read, and this code has been compiled in the project's tests on other systems only, never run on a Mac.
- Nothing here has been run on Linux natively; the Linux code was run in the project's development container.

## See also

- [Search and the filter language](search.md), for `group:` and conditions
- [Operations and undo](operations.md)
- [The viewer](viewer.md) and [the quick view](quick-view.md), which take the encoding chooser
- [Tools and diro doctor](tools.md), for the tools an action can run
- [Themes and colors](themes.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
