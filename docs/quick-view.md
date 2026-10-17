# The quick view

This guide covers the quick view: a live preview of the focused file in the other panel. It tells what Shift+F3 does, what diroxide says when the preview changes, how to move through a preview by heading, paragraph, table and link, how to search it, which formats it reads, and how to open the file in textweaver. The quick view comes with alpha.2.

What this guide says about the quick view comes from its design. Nothing in alpha.2 had been heard on a screen reader when this guide was written, and the quick view's own notes were not final when this first draft was written, so the lines below are what the quick view is built to say.

<!-- to confirm with W2e's notes and the wiring: every sentence in this guide that names a key inside the preview, the exact announcement lines, the extensions carta's formats are recognized by, what the preview does in the single layout, what a link to another file does, and the Open as command for DokuWiki and Jira. -->

## What it is

The quick view is the orthodox quick view of Norton Commander, Total Commander and Midnight Commander. It turns the panel you are not using into a preview of the file under your cursor in the panel you are using. Move to another file and the preview follows. F3 stays the full viewer. The quick view is for glancing at a document while you choose among files.

Shift+F3 turns it on and off: "Quick view on" and "Quick view off". If your terminal does not send Shift+F3, the View menu and the palette have Quick view.

## It follows the cursor, and stays quiet

- When you move to a file, the preview waits about half a second after the cursor stops, then shows the file. Moving quickly through a folder does not read each file on the way. The wait is the `quick_view.wait_ms` setting, 500 by default.
- A newer request replaces an older one, so ten fast arrow presses cost one preview.
- A change is announced in one line and never read in full. In screen-reader mode the other panel is normally redrawn only when you ask, and the quick view's one line is the exception that still arrives.
- The preview is shown again when the file changes on disk, such as a note being written in another program: "Preview updated: report.md changed". This follows the `quick_view.follow_changes` setting, on by default. A file inside an archive is not watched.

What the announcement says, one line, with the file first:

- A Markdown file: "Preview: report.md, 12 headings".
- Code: "Preview: main.rs, Rust, 412 lines".
- Plain text: "Preview: notes.txt, 1,204 lines".
- One of carta's formats: "Preview: notes.org, Org, 8 headings".
- A file larger than the preview reads: "Preview: big.log, first 1 MiB of 2 GB". The limit is `quick_view.max_kib`, 1,024 by default.
- A binary file: "Preview: a.bin, binary, hex".
- A kind with no previewer yet: "No preview: photo.png, PNG image, 2 MB".

## Going into the preview

Tab cycles the active panel, the preview and the other way back to the panel. On entering the preview, diroxide says its title and the line the cursor is on: "Preview, report.md" and then the line, with its position last. The hardware cursor sits on the first cell of the line you are on, as everywhere in diroxide, so a screen reader or a Braille display follows it.

Inside the preview:

- Up and Down move by line. Home and End go to the first and last line.
- Heading, paragraph, table and link moves go to the next of each kind. Each move says what it found with its position last: "Heading 2: Installation, 3 of 12". The palette has every one of these commands; the keys are to be confirmed.
- Previous file and Next file load the previous or next file of the folder into the preview without leaving it.
- Escape returns to the panel, with your cursor on the row you left.

### Tables

A table is read by row and cell, with the header said.

- Entering a table says its size: "Table 1 of 2, 5 rows, 3 columns".
- A cell says its column's header first, then its value, then where it is: "Age: 36, row 2 of 5, column 2".

### Links

The preview lists the links of the document. "Link 2 of 5: docs, to docs/index.md" says the text, then the target.

diroxide opens a link only through the safe opener, and always asks first. Only web addresses and mail addresses open. A link that points at a file on your computer (`file:`) is never followed. The question puts the address first: "Open https://example.org in the browser? y or n", and starts on No.

### Find

Find searches the preview and says the count: "Match 2 of 7 in the preview".

## What it reads

In alpha.2 the quick view reads Markdown, plain text, code and the formats of carta.

- Markdown: front matter, lists, tables, code blocks, links, footnotes and callouts are read. Math is kept as written. Embeds, the Obsidian way of pulling in another note, are never followed.
- Plain text: lines, with an optional line-number column (`quick_view.line_numbers`).
- Code: the same nine languages the viewer colors, with the same caps. The preview uses a code block's language for fenced code in a Markdown file.
- carta's formats: Org, reStructuredText and MediaWiki, recognized by their extensions, and DokuWiki and Jira, which have no extension of their own and open through Open as. carta is a converter written in Rust. diroxide pins it to one exact version and never requires it. It never follows an include directive, because that would read a named file from anywhere on your computer, and it does not use carta's Typst reader for the same reason.
- If carta cannot read a file the preview says so and shows the text instead: "Could not read notes.org: not valid Org." and then "Showing the text."

What comes with alpha.7: DOCX, PDF, HTML, EPUB, spreadsheets, and images described in words. Until then the quick view says "No preview" for them and names the kind and size.

A preview never blocks. A file that takes too long to read stops at the `quick_view.deadline_ms` limit, 2,000 milliseconds by default, and the preview shows the file's text instead.

## Open in textweaver

Open in textweaver hands the file to `tw`, textweaver's command, for full reading with speech. It works when textweaver is installed. When it is not, diroxide says so and how to install it. Settings shows `tw` in the list of tools as found with its version, or not found.

## Settings

The Viewer and quick view group of [the settings reference](settings-reference.md) has five quick view settings: `quick_view.wait_ms`, `quick_view.max_kib`, `quick_view.deadline_ms`, `quick_view.follow_changes` and `quick_view.line_numbers`. `highlighting` in the Look group also decides whether code in the preview is colored.

## See also

- [The viewer](viewer.md)
- [Archives](archives.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
