# The quick view

This guide covers the quick view: a live preview of the focused file in the other panel. It tells what Shift+F3 does, what diroxide says when the preview changes, how to move through a preview by heading, paragraph, table and link, how to search it, which formats it reads, and how to open the file in textweaver.

What this guide says about the quick view comes from its design and from the tests that check each sentence. Nothing in alpha 1 had been heard on a screen reader when this guide was written, because the listening tests come after the build, so the lines below are what the quick view is built to say.

## What it is

The quick view is the orthodox quick view of Norton Commander, Total Commander and Midnight Commander. It turns the panel you are not using into a preview of the file under your cursor in the panel you are using. Move to another file and the preview follows. F3 stays the full viewer. The quick view is for glancing at a document while you choose among files.

Shift+F3 turns it on and off, everywhere: "Quick view on" and "Quick view off". The View menu has Quick view too (Alt+V, then Q), and so does the palette. Some terminals send Shift with a function key as another key; Shift+F3 may arrive as F15. If Shift+F3 does nothing in your terminal, the View menu and the palette are the way. The key test (`diro keys test`) says what your terminal sent.

## It follows the cursor, and stays quiet

- When you move to a file, the preview waits about half a second after the cursor stops, then shows the file. Moving quickly through a folder does not read each file on the way. The wait is the `quick_view.wait_ms` setting, 500 by default. Turning the quick view on shows the file at once, without the wait.
- A newer request replaces an older one, so ten fast arrow presses cost one preview.
- A change is announced in one line and never read in full. You hear each row as you move, then one line when the cursor stops.
- The preview is shown again when the file changes on disk, such as a note being written in another program: "Preview updated: report.md changed", said once. This follows the `quick_view.follow_changes` setting, on by default. If the file is removed: "Preview: report.md was removed".

What the announcement says, one line, with the file first:

- A Markdown file: "Preview: report.md, 12 headings".
- Code: "Preview: main.rs, Rust, 412 lines".
- Plain text: "Preview: notes.txt, 1,204 lines".
- One of carta's formats: "Preview: notes.org, Org, 2 headings".
- A file larger than the preview reads: "Preview: big.log, first 1 MiB of 2.0 GB". The limit is `quick_view.max_kib`, 1,024 by default.
- A binary file: "Preview: a.bin, binary, hex".
- A kind with no previewer yet: "No preview: locked.zip, archive, 197 bytes".
- A file that cannot be read as its format: "Could not read notes.org: not valid Org.", then "Showing the text."

Alt+O (Say other panel) names the preview: "Other panel: Preview, report.md".

### In screen-reader mode

While the focus is in the panel, the other panel shows only "Preview" and "Tab goes into the preview", never the document. A terminal screen reader speaks text as it appears, so drawing every new preview would read it all aloud. The one line above still says what is shown. Tab draws the real content. In sighted mode the preview is drawn live beside the panel.

### In the single layout

The single layout shows only the active panel, so the preview is drawn only while you are in it. The one-line announcement still arrives as you move.

## Going into the preview

Tab goes into the preview, and Tab or Escape comes back. On entering, diroxide says the title and the line you are on as one line, with the position last: "Preview, report.md, Report, heading 1, 1 of 25". The hardware cursor sits on the first cell of the line, as everywhere in diroxide, so a screen reader or a Braille display follows it. Escape returns to the panel with your cursor on the row you left, and nothing is read twice. You hear the panel's own line on the way out, such as "Left panel: demo, 13 items".

Inside the preview:

- Up and Down move by line. Home, End, Page Up and Page Down work as in a list. At the edges: "Top of the preview", "Bottom of the preview".
- H and Shift+H go to the next and previous heading, with its level said: "Heading 2: Part 1, 2 of 12". At the ends: "No heading below", "No heading above".
- Ctrl+Down and Ctrl+Up go to the next and previous paragraph. A table or a code block counts as one. On macOS the system keeps Ctrl with the arrows; the View menu's Preview submenu (Alt+V, then P) has Next paragraph and Previous paragraph.
- T and Shift+T go to the next and previous table. Ctrl+D (Say status) says where you are: "Preview, report.md, 3 of 25".
- Alt+N and Alt+P show the next and the previous file of the panel without leaving the preview, and the panel's cursor follows. At the ends: "No next file", "No previous file".
- Slash finds text in the preview. Escape leaves the region.

While you are in the preview, the key bar names its keys.

### Tables

A table is read by row and cell, with the header said.

- T says its size: "Table 1 of 1, 2 rows, 2 columns", and puts you on its first cell.
- Left and Right move to the cell on the left and right. Shift+Up and Shift+Down move to the cell above and below.
- A header cell says "Age, header, row 1 of 2, column 2". A cell below it says its column's header first, then its value, then where it is: "Age: 36, row 2 of 2, column 2".
- At the edges: "Start of the row", "End of the row", "First row of the table", "Last row of the table". Outside a table: "Not in a table".

### Links

L lists the document's links, with the text first and where each goes after: the list is titled "Links: 5 in report.md", and Enter goes to the link. Shift+L goes to the next link and Ctrl+L to the previous one, each said with its place: "Link 2 of 5: docs, to docs/index.md". At the ends: "No link below", "No link above", and "No links" when there are none.

Enter on a line with a link opens it, through the safe opener, after a question that starts on No. Only web addresses and mail addresses open:

- "Open https://example.org in the browser? No is the default. y or n"
- "Open mail to ada@example.org? No is the default. y or n"

A link to a file, a relative path or anything else is refused, and diroxide never follows a `file:` link: "Could not open docs/index.md: only web and mail links open from the preview." Answer n and it says "Not opened". With no link on the line: "No link on this line".

### Find

Slash opens the prompt "Find in the preview" and searches the preview, ignoring case, and says the count: "Match 1 of 22 in the preview". N goes to the next match and P to the previous: "Match 2 of 22 in the preview". When nothing matches: "No match for zebra in the preview". N before any find says "Nothing found yet".

## What it reads

The quick view reads Markdown, plain text, code and the formats of carta.

- Markdown, by the extensions `.md`, `.markdown`, `.mdown`, `.mkd`, `.mkdn`, `.mdwn`, `.mdtxt` and `.rmd`: front matter, lists, tables, code blocks, links, footnotes and callouts are read, and math is kept as written. A title in the front matter becomes the document's title, not a heading row. Embeds, the Obsidian way of pulling in another note, are never followed. Raw HTML keeps its text and drops its tags.
- Plain text and code: lines, with an optional line-number column (`quick_view.line_numbers`). The preview shows code as plain lines. The viewer (F3) colors it; the preview does not draw code colors.
- carta's formats, by extension: Org (`.org`), reStructuredText (`.rst`, `.rest`), MediaWiki (`.mediawiki`, `.wiki`), DokuWiki (`.dokuwiki`) and Jira (`.jira`). carta is a converter written in Rust. diroxide pins it to one exact version and never requires it. An Org `#+TITLE` becomes the document's title, not a heading row. A document that carta cannot read shows its text instead, as above.
- Anything else: text shows as text, and bytes that are not text show as hex.

In the panel an Org file's row reads "file", not "Org", because the file type table has no entry for it yet. The preview line says "Org".

The preview reads only the file in front of it. It never follows an include or an embed to another file, because that would read a named file from anywhere on your computer. A reStructuredText include is left out and the preview says so once: "An include directive was left out, so the file it names is not read." carta's Typst reader is not in the build, for the same reason.

DOCX, PDF, HTML, EPUB, spreadsheets, and images described in words come in a later alpha. Until then the quick view says "No preview" and names the kind and size.

A preview never blocks. A reader that takes too long stops at the `quick_view.deadline_ms` limit, 2,000 milliseconds by default: "Could not read big.org: took too long.", then "Showing the text." A reader that ran past its deadline cannot be stopped, so it is left to finish. At most three readers run at once, and when they are all busy the next file says "Could not read notes.org: an earlier document is still being read."

## Open in textweaver

Open in textweaver hands the file in the preview to `tw`, textweaver's command, for full reading with speech. It is in the palette and in the View menu (Alt+V, then O); it has no key of its own. It works when textweaver is installed: "Opening report.md in textweaver". When it is not: "Could not open report.md in textweaver: textweaver is not installed. Install textweaver from its releases page or with cargo." If the tool is turned off: "Could not open report.md in textweaver: textweaver is turned off in tools.toml. Turn it on in tools.toml." A file inside an archive is not handed over: "Could not open notes.md in textweaver: textweaver opens files on this computer only. Copy it out first." With the quick view off: "Open in textweaver opens the quick view's file. Turn the quick view on first." Settings shows `tw` in the list of tools as found with its version, or not found.

## Settings

The Viewer and quick view group of [the settings reference](settings-reference.md) has five quick view settings: `quick_view.wait_ms`, `quick_view.max_kib`, `quick_view.deadline_ms`, `quick_view.follow_changes` and `quick_view.line_numbers`.

## See also

- [The viewer](viewer.md)
- [Archives](archives.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [Every key](keyboard.md)
- [The documentation index](README.md)
