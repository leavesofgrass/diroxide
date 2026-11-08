# The viewer

This guide covers the file viewer: opening a file with F3, reading it a line at a time, wrapping and line numbers, jumping and searching, pictures and documents shown on their own words, hex mode, stepping through the files of a folder, following a growing log, how a large file is handled, and how code is read by ear.

Two keys first. L shows or hides line numbers: "Line numbers on". H switches between text and hex. Both are in the palette and in the View menu's Viewer submenu (Alt+V, then V), under the names "Line numbers" and "Hex mode". The setting `viewer.line_numbers` starts every file with the numbers on, and Ctrl+D says the number of the line you are on whether they are shown or not. [Wrap and line numbers](#wrap-and-line-numbers) has the rest.

What this guide says about the spoken lines comes from the design and from the tests that check each sentence. Nothing in alpha 1 had been heard on a screen reader when this guide was written, because the listening tests come after the build.

## Open and close

F3 opens the focused file in the viewer, on top of the panels. It works on a file in a folder and on a member of an open archive. F3 on a folder says "Not a file: week1, folder" and opens nothing. Escape or Q closes the viewer: "Viewer closed: notes.txt". The cursor is back on the file's row.

The viewer is for reading. Editing is F4, which hands the file to your editor ([operations and undo](operations.md) describes it).

What F3 shows depends on the file. A text file opens as text. A picture, a PDF, a Word, EPUB or other document, a sound or a video opens on the words the quick view would say for it, and a file that no previewer reads opens in hex. [Pictures and documents](#pictures-and-documents) below has each case. In the window F3 on a picture opens the window's picture view at once; [the GUI](gui.md#the-picture-view) has it.

The title puts the file's name first, then where it is: "notes.txt, in D:\demo". Inside an archive it names both: "notes.txt, in course.zip", and for a member in a folder of the archive, "a.txt, in course.zip!week1". [Archives](archives.md) explains the member form.

## Pictures and documents

F3 on a file that a previewer reads opens the viewer on the preview's words, not on the file's bytes. That covers pictures, PDF, Word, Excel, OpenDocument, EPUB and RTF documents, sound and video. Archives keep their own road ([archives](archives.md)). A file that no previewer reads, such as a plain binary file, still opens in hex: "blob.bin looks binary; hex mode". [Previews](previews.md) says what each kind of file says.

The words come in this order, each line starting with its meaning:

- The title, with the word preview: "report.pdf, preview".
- The summary sentence: "PDF, 3 pages, Example Report". A Word file says "DOCX, 1 heading". A picture says "PNG, 4 by 3" and then what the quick view adds.
- The rows behind it, such as "Pages: 3", "Title: Example Report" or, for a picture, "Dimensions:" and its size.
- For a document, its text. A PDF gives a heading for each page, "Page 1", and then the page's lines. The text of a PDF comes from pdftotext; the other documents are read by diroxide itself.
- The last line says how to reach the bytes: "H shows the bytes in hex".

Hex is one command away and back. H, or Hex mode in the palette, says "Hex mode: the bytes of report.pdf" and shows the file's bytes. H again says "report.pdf, preview" and returns to the words. Ctrl+R reads the preview again: "Reloaded report.pdf, preview". Escape closes the viewer and the cursor is back on the file's row. Hex mode from the palette with no viewer open shows the bytes of any file.

A helper that is missing is said in words, and the viewer never falls into hex without telling you. A PDF with no pdftotext gives "No preview: pdftotext not found", then an "Install:" line, and the properties are still shown ("Pages: 3"). [Tools and diro doctor](tools.md) lists the helpers.

The details list's Privacy and Checksum rows are not among these words. Ask for them with the details command ([details](details.md)).

### In the window

- F3 on a picture opens the picture view at once, with no stop in the viewer. It says the picture's sentence first, "PNG, 40 by 30" and the rest, and then the keys: "Plus, Minus zoom; 0 fits; Escape closes". Escape closes the picture and the cursor is back on the file's row. Enter in the picture never reaches the row behind it. [The GUI](gui.md#the-picture-view) has the zoom keys.
- A picture the built-in decoder cannot read, such as HEIC, AVIF or Photoshop, is drawn through ImageMagick when it is installed: "HEIC, 4032 by 3024, through ImageMagick". Without ImageMagick the window says "Picture: not shown, needs ImageMagick" and the install line.
- F3 on a PDF opens its words, and says "Enter shows the pages as pictures." Enter shows the page the cursor is on, as a picture, through pdftoppm. [Previews](previews.md#the-picture-view-in-the-window) has the pages.
- Hex mode from the palette, with no viewer open, on a picture shows its bytes: "photo.png looks binary; hex mode". The window then says "Enter shows the picture." once, so the picture is one key away.

### In a terminal

A terminal draws no picture. F3 on a picture opens the viewer on its words: the title, "photo.png, preview", then the sentence, then one line that says where pictures show: "Pictures show in the window: diro --gui" (39 cells). It never opens in hex and never says no. A PDF and the other documents open on their words as in the window, without the picture hint.

## A file is a list of lines

The viewer shows a file as a list of rows, one for each line, with the hardware cursor on the first cell of the row you are on. A screen reader and a Braille display follow the cursor as they do in a panel, so nothing about reading changes.

- Up and Down move one line. Page Up and Page Down move a page. Home goes to the top. End goes to the last line and says "Line 14 of 14, the last".
- Left and Right move along the line by token (a word, a number, a symbol), saying each one: "main". In hex mode they move by byte.
- Ctrl+D says where you are: "Line 120 of 4,000, 3 percent".
- A tab shows as spaces. Control, escape, direction and other invisible characters are shown as a question mark, so a file cannot move your cursor, color the screen or hide text from you.
- A line longer than 64 KiB is cut and its row ends with "(line cut)".

The first screen of a file appears at once, however large it is. The viewer reads only what it needs, a little at a time, and builds the index of line starts as far as you go. A command that has to look far, such as a jump to line 5,000,000, says "Finding line 5,000,000" once while it works and then where it landed.

## Wrap and line numbers

- W turns wrap on or off: "Wrap on", "Wrap off". Wrap is on by default. A long line is continued on the next rows, and every row after the first starts with "continued:" so a Braille line never begins in the middle of a word without a cue.
- L turns line numbers on or off: "Line numbers on", "Line numbers off". The same command is Line numbers in the palette and in the View menu's Viewer submenu. The number of the line you are on is always said on request with Ctrl+D, with the numbers shown or not.

The `viewer.wrap_width` setting chooses where lines wrap: 0 is the width of the screen, and a number such as 40 wraps at that many characters, which is one line of a 40-cell Braille display. `viewer.wrap`, `viewer.line_numbers` and `viewer.follow` set the starting state.

## Jump

G or Ctrl+G opens the jump prompt, "Go to line, offset or percent", and says where it landed. It reads what you type:

- A line: `120` or `line 120`.
- A percent: `50%` or `50 percent`.
- An offset in bytes: `0x1f40`, `offset 8000` or `@8000`.
- The ends: `top`, `start`, `end`, `bottom`.

The answer is "At line 7 of 14, 50 percent". Something it cannot read is explained: "Not a line, offset or percent: abc".

## Search

The slash key or F7 opens the search prompt, "Search for", and a question mark opens "Search backward for". Search in file is in the palette too, which matters when single-letter keys are turned off.

What you type is read this way:

- Plain text is a regular expression with smart case: capitals matter only if you type one. `^` and `$` are the start and end of a line.
- Text in double quotes is matched exactly as typed, with smart case.
- `hex:` followed by byte pairs matches bytes: `hex: 48 65 6c`. In hex mode bare byte pairs do, and quoted text adds its bytes: `48 65 "llo"`.

diroxide scans the whole file once so the count is real, then moves between matches:

- "Match 3 of 12, line 120" says the match, how many there are and the line.
- N goes to the next match and P to the previous. At the end the search wraps and says so: "Search wrapped to the top, match 1 of 5".
- "No match for zebra." when there is nothing.
- A pattern that cannot be read is explained in one line: "Pattern needs care: bad expression".
- Past 100,000 matches the count says "more than".

A long scan is done in slices, so the screen never freezes, and a slow one says "Searching for" and what you typed, once.

## Hex mode

H switches between text and hex: "Hex mode, 8 bytes a row", and again for "Text mode". The cursor keeps its place in the file.

A row is an offset, the bytes and their letters. In screen-reader mode a row is eight bytes and fits 40 characters: `000010: 48 65 6c 6c 6f 20 77 6f Hello wo`. Otherwise a row is sixteen bytes. In a file larger than 16 MiB the row has six bytes in screen-reader mode, so the longer offset still fits. The `viewer.hex_row` setting chooses auto, narrow (eight) or wide (sixteen).

A screen reader reads a row of hex slowly. B (Say byte) speaks the byte under the cursor instead: "Byte 16 is 48 hex, the letter H". Left and Right move a byte at a time and say each one.

A file that looks binary and that no previewer reads opens in hex mode and says why: "big.bin looks binary; hex mode". A picture, a PDF or a document opens on its words instead, with hex one key away ([Pictures and documents](#pictures-and-documents)).

## Which characters: encodings

The viewer decides how to read a file from its first bytes. UTF-8 is tried first; a byte-order mark means UTF-16, and the viewer says so in words: "Encoding: UTF-16 (byte-order mark)". Older Windows text in Windows-1252 is recognized too. A file that fits none of these opens in hex mode.

When the automatic choice is wrong, Encoding in the View menu (its Viewer submenu) and in the palette lets you choose. It opens a list titled with the file, "Encoding of legacy.txt", whose first row is "Automatic" with what was found, and then the encodings grouped by language. Enter reads the file again in that encoding and says "Encoding Windows-1251, chosen by you". The choice lasts for that file in that session. [File types and groups](file-types.md#the-encoding-chooser) has the whole list.

## The next file, and back to the folder

Alt+N (Next file) and Alt+P (Previous file) step through the files of the folder you came from, in the panel's order, and say where you are: "report-2.txt, 2 of 4". With no other file they say "No other files to step to". A file that looks binary opens in hex mode and says so.

Three commands are in the palette only. Show in folder closes the viewer and puts the panel's cursor on the file. Copy viewed file's path and Copy viewed file's name put the full path or the name on the system clipboard and say it: "Path copied: D:\docs\report-3.txt", "Name copied: report-3.txt". In a terminal diroxide asks the terminal to set its clipboard with the standard clipboard sequence (OSC 52), so a terminal that does not allow that will not receive the text.

## Reload and follow

- Ctrl+R reads the file again: "Reloaded notes.txt, 14 lines".
- F (Follow file) reads more of a file as it grows, as a log does, and stays at the end: "Follow on". It does not read each new line aloud. At the progress interval it says "12 new lines". The `viewer.follow` setting starts every file in follow mode.
- Text read from a picture by Read text in picture opens in the viewer, titled as a machine's guess: "Text in picture (machine read), sign.png, 9 words". Reload does not read the picture's bytes as text. It says "This text was read from the picture. Read text in picture reads it again." [Previews](previews.md#read-text-in-picture) has the rest.

## Large files

A file on a drive is read in place, so a 1 GiB log is as quick to open as a short note. A compressed member of an archive is different. It can only be read forward, so the viewer keeps what it has decoded, up to a cap, and says so when it stops short.

- "Showing the first 64 MB of 800 MB." The cap is the `viewer.member_cache_mib` setting, 64 by default.
- Shift+R (Read more) reads more, and says how much it now shows: "Showing the first 128 MB of 800 MB."

An archive that must be read from the start before a member can be shown asks first: "Read 3.2 GB first to open note.txt? No is the default. y or n". [Archives](archives.md) describes that question.

## Code, read by ear

Code in the viewer is colored for sighted users and silent for screen readers. In screen-reader mode nothing about colors is said unless you ask. Color is never the only way to learn anything. The commands below say what the colors would show, in words, from the same reading of the file.

The colors cover nine languages: Markdown, JSON, TOML, YAML, shell (sh, bash and zsh), Rust, Python, HTML and CSS. The language comes from the file's name and its first line. The `highlighting` setting is `auto` (colors, except in screen-reader mode), `on` or `off`. The colors come from the theme, with seventeen token roles you can override in the `[colors]` table.

The commands, with their keys:

- K (Say token kind): the kind of the word under the cursor. "Function name: main, line 4".
- Shift+K (Say line by kind): the kinds in a line, in order, repeats collapsed, at most four. "Line 4: keyword, function name".
- O (Outline): a list of the file's symbols, kind first, and Enter goes to one. "Outline: 2 symbols in main.rs", then rows such as "function main, line 4, 1 of 2".
- S and Shift+S (Next symbol, Previous symbol): "module greeting, line 37".
- M (Matching bracket): goes to the matching bracket. "Matches } on line 35".
- C (Skip comment): goes to the next line of code. "Next code, line 47".
- I (Say indent): "Indent level 2". The palette also has Next at this indent, Previous at this indent, Up an indent level and Into the next indent.
- X (Say context): where you are. "In function parse_args, in impl Config".

JSON and TOML are checked before they are read, and the result is said once: "JSON, 3.1 KB, valid" or "JSON invalid at line 12, column 5". YAML has no checker here and says nothing.

The caps keep a big file from stalling the viewer. Past a cap, colors stay off and the viewer says why, once:

- "Highlighting off: over 512 KiB" (`viewer.highlight_max_kib`).
- "Highlighting off: over 10,000 lines" (`viewer.highlight_max_lines`).
- "Highlighting off: a line is too long" (`viewer.highlight_max_line_chars`, 2,000).
- "Highlighting stopped: took too long" (`viewer.highlight_deadline_ms`, 2,000). After that, colors pause for the rest of the session.
- "Highlighting off: not in this build" in a slim `diro` built without the syntax set ([installing diroxide](install.md)).

The outline, the bracket jump, skip comment and context need the file read whole, within the caps. Over them they say "Needs a file under 512 KiB". Indentation and brackets work on any text file within the caps, with or without a language.

## When single-letter keys are off

The viewer's W, L, Q, N, P, H and most of its other keys are single letters. If you turn single-letter keys off for dictation software, every viewer command is still in the palette and in the View menu's Viewer submenu (Alt+V, then V), and Escape still closes the viewer. [Settings](settings.md) describes the `single_letters` setting.

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
