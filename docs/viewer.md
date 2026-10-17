# The viewer

This guide covers the file viewer: opening a file with F3, reading it a line at a time, wrapping and line numbers, jumping and searching, hex mode, stepping through the files of a folder, following a growing log, how a large file is handled, and how code is read by ear. The viewer comes with alpha.2.

What this guide says about the spoken lines comes from the design and from the tests that check each sentence. Nothing in alpha.2 had been heard on a screen reader when this guide was written. Where a key is not fixed yet, the guide says the command is in the palette and marks the key to be confirmed.

<!-- to confirm with the wiring (W2f): every key in this guide that is not F3, Escape, q, w, l, slash, n, p, Ctrl+R, Ctrl+D or R; the layer's name; and which commands are in the menus. -->

## Open and close

F3 opens the focused file in the viewer, on top of the panels. It works on a file in a folder and on a member of an open archive. Escape or q closes it, and the cursor is back on the file's row.

The viewer is for reading. Editing is F4, which hands the file to your editor ([operations and undo](operations.md) describes it).

The title puts the file's name first. Inside an archive it names both: "notes.txt, in course.zip", and for a member in a folder of the archive, "a.txt, in course.zip!week1". [Archives](archives.md) explains the member form.

<!-- to confirm with the wiring: the title of a member in a folder of an archive (the plan's two lines disagree: "notes.txt, in course.zip" and "a.txt, in course.zip!week1"). -->
## A file is a list of lines

The viewer shows a file as a list of rows, one for each line, with the hardware cursor on the first cell of the row you are on. A screen reader and a Braille display follow the cursor as they do in a panel, so nothing about reading changes.

- Up and Down move one line. Page Up and Page Down move a page. Home goes to the top. End goes to the last line and says "Line 14 of 14, the last".
- Left and Right move along the line by token (a word, a number, a symbol), saying each one. In hex mode they move by byte.
- Ctrl+D says where you are: "Line 120 of 4,000, 3 percent".
- A tab shows as spaces. Control, escape, direction and other invisible characters are shown as a question mark, so a file cannot move your cursor, color the screen or hide text from you.
- A line longer than 64 KiB is cut and its row ends with "(line cut)".

The first screen of a file appears at once, however large it is. The viewer reads only what it needs, a little at a time, and builds the index of line starts as far as you go. A command that has to look far, such as a jump to line 5,000,000, says "Finding line..." once while it works and then where it landed.

## Wrap and line numbers

- w turns wrap on or off: "Wrap on", "Wrap off". Wrap is on by default. A long line is continued on the next rows, and every row after the first starts with "continued:" so a Braille line never begins in the middle of a word without a cue.
- l turns line numbers on or off: "Line numbers on". The number of the line you are on is always said on request with Ctrl+D.

The `viewer.wrap_width` setting chooses where lines wrap: 0 is the width of the screen, and a number such as 40 wraps at that many characters, which is one line of a 40-cell Braille display. `viewer.wrap`, `viewer.line_numbers` and `viewer.follow` set the starting state.

## Jump

Jump goes to a place and says where it landed, from the palette (the key is to be confirmed). It reads what you type:

- A line: `120` or `line 120`.
- A percent: `50%` or `50 percent`.
- An offset in bytes: `0x1f40`, `offset 8000` or `@8000`.
- The ends: `top`, `start`, `end`, `bottom`.

The answer is "At line 7 of 14, 50 percent". Something it cannot read is explained: "Not a line, offset or percent: abc".

## Search

The slash key opens the search prompt, and a question mark searches backward. The palette has Search too, which matters when single-letter keys are turned off.

What you type is read this way:

- Plain text is a regular expression with smart case: capitals matter only if you type one. `^` and `$` are the start and end of a line.
- Text in double quotes is matched exactly as typed, with smart case.
- `hex:` followed by byte pairs matches bytes: `hex: 48 65 6c`. In hex mode bare byte pairs do, and quoted text adds its bytes: `48 65 "llo"`.

diroxide scans the whole file once so the count is real, then moves between matches:

- "Match 3 of 12, line 120" says the match, how many there are and the line.
- n goes to the next match and p to the previous. At the end the search wraps and says so: "Search wrapped to the top, match 1 of 5".
- "No match for zebra." when there is nothing.
- A pattern that cannot be read is explained in one line: "Pattern needs care: bad expression".
- Past 100,000 matches the count says "more than".

A long scan is done in slices, so the screen never freezes, and a slow one says "Searching for..." once.

## Hex mode

Hex mode shows the file as bytes. Switch with Hex mode in the palette (the key is to be confirmed): "Hex mode, 8 bytes a row", and again for "Text mode". The cursor keeps its place in the file.

A row is an offset, the bytes and their letters. In screen-reader mode a row is eight bytes and fits 40 characters: `000010: 48 65 6c 6c 6f 20 77 6f Hello wo`. Otherwise a row is sixteen bytes. In a file larger than 16 MiB the row has six bytes in screen-reader mode, so the longer offset still fits. The `viewer.hex_row` setting chooses auto, narrow (eight) or wide (sixteen).

A screen reader reads a row of hex slowly. Say byte, in the palette (key to be confirmed), speaks the byte under the cursor instead: "Byte 16 is 48 hex, the letter H". Left and Right move a byte at a time and say each one.

A file that looks binary opens in hex mode and says why: "report.bin looks binary; hex mode".

## Which characters: encodings

The viewer decides how to read a file from its first bytes. UTF-8 is tried first; a byte-order mark means UTF-16, and the viewer says so in words: "Encoding: UTF-16 (byte-order mark)". Older Windows text in Windows-1252 is recognized too. A file that fits none of these opens in hex mode. A chooser for other encodings comes with alpha.7.

## The next file, and back to the folder

Next file and Previous file step through the files of the folder you came from, in the panel's order, and say where you are: "report-2.txt, 2 of 4". They are in the palette; the plan's keys for them are Alt+N and Alt+P (to be confirmed). A file that looks binary opens in hex mode and says so.

Show in folder closes the viewer and puts the panel's cursor on the file: "folder, report-3.txt". Copy path and Copy name put the file's path or name on the clipboard and say it: "Path copied: D:\docs\report-3.txt". In a terminal the clipboard is the terminal's, where it allows that; otherwise the text appears on the status line.

<!-- to confirm with the wiring: the names of these commands in the palette, the clipboard route in the terminal and what is said when the terminal does not allow it. -->

## Reload and follow

- Ctrl+R reads the file again: "Reloaded notes.txt, 14 lines".
- Follow reads more of a file as it grows, as a log does, and stays at the end. The palette has Follow, and the plan's key is f (to be confirmed); "Follow on". It does not read each new line aloud. At the progress interval it says "12 new lines". The `viewer.follow` setting starts every file in follow mode.

## Large files

A file on a drive is read in place, so a 1 GiB log is as quick to open as a short note. A compressed member of an archive is different. It can only be read forward, so the viewer keeps what it has decoded, up to a cap, and says so when it stops short.

- "Showing the first 64 MB of 800 MB." The cap is the `viewer.member_cache_mib` setting, 64 by default.
- R reads more, and says how much it now shows: "Showing the first 128 MB of 800 MB."

An archive that must be read from the start before a member can be shown asks first: "Read 3.2 GB first to open note.txt? y or n". [Archives](archives.md) describes that question.

## Code, read by ear

Code in the viewer is colored for sighted users and silent for screen readers. In screen-reader mode nothing about colors is said unless you ask. Color is never the only way to learn anything. The commands below say what the colors would show, in words, from the same reading of the file.

The colors cover nine languages: Markdown, JSON, TOML, YAML, shell (sh, bash and zsh), Rust, Python, HTML and CSS. The language comes from the file's name and its first line. The `highlighting` setting is `auto` (colors, except in screen-reader mode), `on` or `off`. The colors come from the theme, with seventeen token roles you can override in the `[colors]` table.

The commands are in the palette. The plan's keys are in parentheses and are to be confirmed:

- Token kind (k): the kind of the word under the cursor. "Function name: parse_args, line 21".
- Line by kind (Shift+K): the kinds in a line, in order, repeats collapsed, at most four. "Line 21: keyword, function name, variable, operator and 1 more".
- Outline (o): a list of the file's symbols, kind first, and Enter goes to one. "Outline: 7 symbols in main.rs", then rows such as "function main, line 40".
- Next symbol and Previous symbol (s and Shift+S): "module greeting, line 37".
- Bracket jump (m): goes to the matching bracket. "Matches } on line 35".
- Skip comment (c): goes to the next line of code. "Next code, line 47".
- Indent (i): "Indent level 2". The palette also moves to the next line at the same level, or to the parent or child level.
- Context (x): where you are. "In function parse_args, in impl Config".

JSON and TOML are checked before they are read, and the result is said once: "JSON, 3.1 KB, valid" or "JSON invalid at line 12, column 5". YAML has no checker here and says nothing.

The caps keep a big file from stalling the viewer. Past a cap, colors stay off and the viewer says why, once:

- "Highlighting off: over 512 KiB" (`viewer.highlight_max_kib`).
- "Highlighting off: over 10,000 lines" (`viewer.highlight_max_lines`).
- "Highlighting off: a line is too long" (`viewer.highlight_max_line_chars`, 2,000).
- "Highlighting stopped: took too long" (`viewer.highlight_deadline_ms`, 2,000). After that, colors pause for the rest of the session.
- "Highlighting off: not in this build" in a slim `diro` built without the syntax set ([installing diroxide](install.md)).

The outline, the bracket jump, skip comment and context need the file read whole, within the caps. Over them they say "Needs a file under 512 KiB". Indentation and brackets work on any text file within the caps, with or without a language.

## When single-letter keys are off

The viewer's w, l, q, n, p and slash are single-letter keys. If you turn single-letter keys off for dictation software, every viewer command is still in the palette, and Escape still closes the viewer. [Settings](settings.md) describes the `single_letters` setting.

## Settings

The Viewer and quick view group of [the settings reference](settings-reference.md) lists the `viewer.*` settings with their defaults and ranges: `viewer.wrap`, `viewer.wrap_width`, `viewer.line_numbers`, `viewer.follow`, `viewer.hex_row`, `viewer.member_cache_mib` and the four highlight caps. `highlighting` is in the Look group.

## See also

- [Archives](archives.md)
- [The quick view](quick-view.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
