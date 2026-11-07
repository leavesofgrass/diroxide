# Columns, notes and fields

This guide covers the columns a panel can show beyond its built-in ones: facts read from inside the files (a song's artist, a document's pages, a photo's camera), your own notes and fields, a pattern over the file name, and a plugin's column. It also covers Set note and Set field, which keep your own words about a file, and Fill columns, which reads every column of a folder now.

The short version: add a field's key to the `columns` setting, for example `audio.artist`, and every row says it after its other cells, with its label first: "artist Ada Example". A cell not read yet says so in words ("artist reading"), and the cells arrive quietly while you move, without being announced.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## Choosing columns

The `columns` setting lists the cells a row says after its name, in order. The built-in ones are `type`, `size`, `modified` and the others [the panels guide](panels.md) names. A key with a dot in it adds a column read from the files.

- In the program: Settings, then the Columns row. Enter opens a field with the list as text, and Enter again keeps it. diroxide says "Columns: type, image.dimensions".
- In `config.toml`: `columns = ["type", "audio.artist"]`.
- Both panels take the change at once, and their cells are read.

A key that nothing gives, such as one with a typing mistake, adds a column that stays empty and says nothing. Check the spelling against the lists below.

## What a row says

A row says each cell after its label, the way the built-in cells say "modified Tuesday", so the words carry their meaning wherever they are read: "photo.png, PNG, dimensions 4 by 3". The label is the last part of the key with small letters: `audio.artist` says "artist", `doc.last_saved_by` says "last saved by".

- A file the column does not apply to says nothing for it. A text file has no "dimensions", so the cell is left out and the row stays short.
- A cell not read yet says why: "artist reading" while it waits, "lines not computed" for a column that is read only on request, and "git pending" for a plugin's column that has not answered.
- A folder is never read for a field. Its row has no cell for a column that reads files.
- The name still comes first and your columns come after the built-in ones, in the order of the `columns` setting.

## The keys

Every key says which loading mode its reader uses. The modes are in the next section.

Fast, shown for the whole folder, the rows near the focus first:

- Sound tags: `audio.title`, `audio.artist`, `audio.album_artist`, `audio.album`, `audio.track`, `audio.disc`, `audio.date`, `audio.genre`, `audio.composer`, `audio.comment` and `audio.tags`.

Background, filled for the rows on screen and two pages either side:

- Sound properties: `audio.length`, `audio.bitrate`, `audio.sample_rate`, `audio.channels`, `audio.bit_depth`, `audio.cover` and `audio.format`.
- Documents, from PDF, Word, OpenDocument and EPUB files: `doc.title`, `doc.author`, `doc.subject`, `doc.keywords`, `doc.comments`, `doc.category`, `doc.last_saved_by`, `doc.revision`, `doc.created`, `doc.modified`, `doc.application`, `doc.producer`, `doc.pages`, `doc.words`, `doc.slides`, `doc.company`, `doc.manager`, `doc.template`, `doc.language`, `doc.publisher`, `doc.published`, `doc.identifier`, `doc.rights`, `doc.pdf_version`, `doc.encrypted`, `doc.custom` and `doc.review_comments`. Which of them a file has depends on its kind.
- Photos, from the file's own tags: `image.taken`, `image.camera`, `image.lens`, `image.software`, `image.gps`, `image.serial`, `image.lens_serial`, `image.owner`, `image.artist`, `image.copyright`, `image.comment`, `image.thumbnail`, `image.xmp`, `image.iptc` and `image.more_pictures`. Several of these are private, and [privacy before sharing](before-sharing.md) says how to find and remove them.
- Pictures, from the header alone: `image.description`, `image.dimensions`, `image.shape`, `image.color`, `image.animation`, `image.sizes`, `image.stored_size`, `image.orientation`, `image.profile` and `image.shown`.
- Zip archives: `archive.entries`, `archive.unpacked`, `archive.encrypted` and `archive.comment`.
- What the system keeps about the file: `file.origin`, `file.tags`, `file.comment` and `file.extended`.

On demand, read only when you ask:

- Text files: `text.encoding`, `text.lines` and `text.line_endings`.
- Sound and video through ffprobe, which is a program: `media.title`, `media.artist`, `media.album`, `media.duration`, `media.dimensions` and `media.bit_rate`. ffprobe has to be installed; [tools and diro doctor](tools.md) says how to find it.

Three more kinds of key are not read from the file's insides.

- Your own: `custom.note` is the note you keep with Set note, and `custom.NAME` is a field you keep with Set field, such as `custom.project`. They come from your labels file at once.
- A pattern over the name: `regex.NAME`, below.
- A plugin's column: the plugin's id, a dot, and the column's id. The git plugin's Git column is `git.git`.

## The three loading modes

A column's reader decides when its cells arrive. You do not choose the mode.

- Fast: the whole folder is read, the rows nearest the focus first. A folder of songs has its artists in a moment.
- Background: the rows on screen and two pages either side are read, and the reading follows the focus as you move. A row far from the focus says "reading" until you come near, or until Fill columns reads it.
- On demand: nothing is read until you ask with Fill columns or by sorting on the column. Until then the cell says "not computed".

What the readers do, whichever mode:

- Four files are read at once on a disk of this computer, and one at a time on a network share.
- Nothing is read on a remote place such as SFTP, inside an archive, in a plugin's place, or in a folder you named in `check.skip`. [File types and groups](file-types.md) explains `check.skip`.
- The cells reach the rows quietly. A batch that lands near the focus is drawn at once and is never announced. In screen reader mode the other panel is not redrawn behind your back.
- Moving away stops the old reading and starts a new one from where you are.

Reading 100,000 entries with three columns did not delay the first drawing in the project's measurements: the rows say "reading" in their own words, so the list appears at its usual speed.

## Sort by a column

The Sort list (View menu, then Sort, then Sort list) offers each column you added after the built-in keys: "Artist", "Lines". Choosing one sorts by it, and diroxide says "Sorted by artist, A to Z". Choosing it again reverses.

If the column's cells are not all read yet, diroxide reads the files first and sorts when the reading ends. The order does not change until then.

- With 200 files or fewer waiting, it reads at once: "Reading 12 files to sort by Artist".
- With more, it asks first, and the question starts on No: "Read 12,000 files to sort by Artist? About 40 seconds. No is the default. y or n". The estimate counts 300 files a second on a local disk and 30 on a share. No says "Nothing read; the order stays."
- A plugin's column reads nothing itself, so it asks the plugin for the cells instead: "Asking Git for 1 entry to sort by it". The plugin answers a page of rows at a time, and the sort happens when every cell is in.

A column sorts by its words, so a duration or a long count may not sort the way its number would.

## Fill columns

Fill columns is in the View menu under Sort. It reads every column of this folder from its files now, the on-demand ones too, so nothing says "not computed" afterward. It asks first the same way when more than 200 files wait: "Read 12,000 files for the columns?" with the estimate, starting on No.

- "Reading 12 files for the columns" starts it, and "Columns read: 12 files" says it is done.
- "Every column here is read." means nothing was waiting.
- "No column here is read from the files. The columns setting adds one." means the panel has no field column.
- "Columns are not read here: only on a disk of this computer." means the place is remote, an archive or a plugin's.

## Columns from a name

A regex column makes a cell from the file's name, with no reading at all. Put the pattern in the `[regex_columns]` table of `config.toml`, in single quotes so backslashes stay as typed, and add `regex.NAME` to the columns:

```toml
columns = ["type", "size", "regex.episode"]

[regex_columns]
episode = 'S(\d+)E\d+'
```

The cell is the pattern's first group in parentheses, or the whole match when it has none. For `show.S02E07.txt` the row ends with "episode 07". A name the pattern does not match has no cell.

The column's name uses small letters, digits, `-` and `_`. Another name is refused: "regex_columns cannot name a column "Episode": use small letters, digits, - and _." A pattern that cannot be read is said at start in the startup lines: "Regex column episode: its pattern cannot be read".

## Notes and fields

A note is one line of your own words about a file or folder. A field is a named value, such as `project: Apollo`. Both live in diroxide's labels file, `labels.toml` in the state folder, and never in the file itself. They are in the Labels submenu of the Edit menu (Alt+E, then B), next to the labels of [labels and bookmarks](labels-bookmarks.md), and in the palette.

Set note (Alt+E, B, N) acts on the marked rows, or the focused row when nothing is marked.

- A field opens, titled "Note on report.pdf". With one item it holds that item's note, so you can change it. Enter keeps what you typed.
- diroxide says "Note set on report.pdf". An empty note removes it: "Note removed from report.pdf".
- A note is one line of at most 300 characters. A longer one is refused: "A note is at most 300 characters."

Set field (Alt+E, B, F) keeps a named value the same way.

- A field opens, titled "Field on report.pdf, as name: value", with the names already in use offered for completion. Type `project: Apollo`, with a colon or an equals sign.
- diroxide says "Field project set on report.pdf", or "Field project set on 2 items". An empty value removes the field.
- A name uses small letters, digits, `-` and `_`, at most 24 characters, and is never `note`. Another name is refused: "Field names use small letters, digits, - and _ only." Without a colon: "Type the field as name: value."

Ctrl+Z undoes the last one: "Undid field project on 2 items", "Undid note on report.pdf". Like a label's undo, it lasts for the session.

A note and a field show where you look for them:

- In the details view, in the Yours group: "Note: call about the invoice", "Project: Apollo".
- As columns: `custom.note` says "note call about the invoice" and `custom.project` says "project Apollo", at once, with no reading.

They follow the file. When diroxide moves or renames a file, or undoes that, its labels, note and fields move with it, and so do those of everything inside a moved folder. A copy does not take them. A file moved by another program loses them until you set them again, and the Labels list counts such items as missing.

The file is `labels.toml`, format 2. A file from before notes and fields, format 1, loads as it is.

## Columns from a plugin

A plugin that declares a column in its manifest (`[[column]]`) can fill it. The git plugin's Git column says each file's state, "git Staged", "git Clean", and a folder's count, "git 3 changed inside". The plugin answers a page of rows at a time, the rows on screen first, so a cell says "git pending" until its answer comes. [Git](git.md) describes the column, and [writing a plugin](writing-a-plugin.md) says how a plugin declares one.

## Known limits

- A background column reads only the rows around the focus. A row elsewhere says "reading" until the focus comes near or Fill columns reads it.
- Sorting is by the cell's words, so a duration sorts as text.
- A saved view for a folder keeps the columns in use when it was saved. The Columns setting changes both panels, and a folder with a saved view brings its own.
- The window shows the cells in each row's words, after the name. It has no column headers or grid for them yet.
- Nothing about "artist reading" changing quietly to a name, or about how the rows pan on a Braille display, has been heard yet.

## See also

- [The panels](panels.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [The details view](details.md)
- [Privacy before sharing](before-sharing.md)
- [File types and groups](file-types.md)
- [Tools and diro doctor](tools.md)
- [Git](git.md)
- [Settings reference](settings-reference.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
