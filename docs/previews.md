# Previews and the quick view's formats

This guide covers what the quick view and the details view can say about a file beyond text: pictures and drawings in words, sound and video, archive listings, PDF, Word, EPUB and OpenDocument documents, RTF, spreadsheets and delimited text. It says what each preview reads and how much, which previews run a helper program and when, the Preview command and the `quick_view.auto_preview` setting, Read text in picture, and the limits that keep a hostile or enormous file from stalling the quick view.

The short version: a preview is one line, in words, and then the rows behind it. A picture is never decoded: its header is read and its facts are said. Nothing is played. A helper program runs only when you ask, or when auto-preview is on and you are not using a screen reader.

[The quick view](quick-view.md) says how to move through a preview, by heading, table and link. This guide says what the preview contains.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## What each kind says

The line comes first, with the file's name, then the kind and the facts that matter most. Sometimes it passes 40 cells. The name and the kind are always at its start.

### Pictures

PNG, JPEG, GIF, WebP, BMP, TIFF and ICO are read from their headers alone. No pixel is decoded, so a huge picture costs no more than a small one. The sentence says the format, the size as it is shown, the one fact that matters most, and the file's size:

- "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB".
- "JPEG, 3024 by 4032, portrait, 3.1 MB".
- "JPEG, 4032 by 3024, has location, 3.1 MB". A photo with a place in it says so, and nothing more about where. The details view says the rest.
- "GIF, 3 frames, 48 by 27, " and the size, for an animation. The frames are counted from the file's blocks.
- "ICO, 2 sizes, largest 32 by 32".
- "JPEG, 20000 by 20000, too large to open", for a picture over the pixel guard, below.
- "PNG, unreadable: file is cut short", for a file whose header ends too soon.

HEIC, AVIF, JPEG XL, JPEG 2000, Photoshop and camera raw files are named and sized where that is cheap, and say that they need a component to be shown: "HEIC, 4032 by 3024, needs a component". The quick view and the terminal never draw a picture: the sentence is the preview. The window has a picture view, below.

The details view lists the picture's rows, each in words:

- "Dimensions: 3024 by 4032, as shown". "As shown" means the orientation is applied: a photo stored sideways says its turned size.
- "Shape: landscape, about 16 to 9".
- "Color: full color, 8 bits, transparency".
- "Orientation: turned right". The eight orientations are said as "Upright, as stored", "Rotated a quarter turn clockwise" and the like.
- "Picture: not shown, needs ImageMagick", for the formats above.

Camera, place and serial number are the photo reader's rows, so each is said once, in the details view.

The pixel guard stands in front of anything that would decode pixels: 100 megapixels, 32,768 on a side, 256 MiB for the picture and ten seconds. A picture past it is said, not opened.

### Drawings: SVG

An SVG reads the way a screen reader reads one: its title as a heading, its description, each titled part as a list item in order, then the text it shows. A chart's bar with its own title is a list item. `role="img"` on the drawing makes it one graphic whose parts are not read. A drawing with none of these says "Drawing with no description". The announcement can say the title: "SVG, 640 by 480, titled Sales by quarter".

What is never shown is never read: definitions, styles, scripts, metadata, clip paths, masks, gradients, filters, foreign objects, and anything hidden by an attribute. At most 32 MiB is read, and no outside file is read.

### Sound and video

Sound and video go through ffprobe. diroxide never plays anything.

- "MP3, 3 min 41 sec, 320 kbps, stereo".
- "MP4, 12 min 5 sec, 1920 by 1080".
- The rows: "Video: H.264, 30 frames a second" and "Audio: Opus, mono, 48 kHz, English".
- Without ffprobe: "No preview: ffprobe not found".

ffprobe is given ten seconds and 1 MiB of output. A file on this computer is named by its path. A file anywhere else, such as inside an archive, goes in on standard input, its first 16 MiB. Playlists and MIDI files are never probed. A sound file's tags, such as title, artist and album, come from the tag reader, so the details view says them once.

### Archives

An archive is listed, never unpacked. The members are counted through the same provider that opens the archive, which for a disk image is the disk image plugin.

- The line: "zip, 128 files, 12 folders, 47.0 MB". The size is what the members unpack to.
- The rows: "Entries: 128 files, 12 folders", "Unpacked: 47.0 MB, packed 11.6 MB" and "Locked: 3 entries".
- At most 500 member rows follow, then "and 1,203 more". The walk stops at 50,000 members and says so.

### Documents

- Word (`.docx`), EPUB and OpenDocument text (`.odt`) are read by carta from the file's own parts. Headings, the header row of a table and the nesting of lists are kept.
- HTML and LaTeX are read as text through carta. An HTML page reads as its headings, paragraphs and tables, with no script text.
- RTF is read by diroxide's own reader, and its tables are kept. Bold and italic are not said, and deleted revisions are left out.
- A document that cannot be read says why: "Could not read big.docx: too large."

The quick view's one-line announcement for a document names the kind: "Preview: report.docx, DOCX, 1 heading".

When part of a file is nested too deeply, that part is read as plain text, and the preview says so once: "Part of this file is nested too deeply, so that part is read as plain text." The nesting cut is at 256 levels.

### Spreadsheets and delimited text

Excel (`.xlsx`, `.xlsb`, `.xls`) and OpenDocument spreadsheets (`.ods`) are read by a spreadsheet reader. CSV and TSV go through carta after a pass that cuts them to the same caps. Each sheet is a heading, named as the sheet is. Its used range is a table whose first row is the header. The cells are the values the file last saved: a formula is never worked out again.

- At most 10,000 rows and 200 columns of a sheet are read. The rest is counted: "and 4,200 more rows" and "and 12 more columns".
- At most 256 sheets of a workbook, and 1,000 characters of a cell.
- A spreadsheet or delimited file over 50 MB is refused before it is loaded.
- A cell is said with its column's header first, then its value, then where it is: "Bird: Crow, row 2 of 5, column 1".

### PDF

A PDF's properties, its title, author, page count, version and whether it is encrypted, are read from the file itself. Its text comes from pdftotext. The line is "PDF, 12 pages, Example Report".

- The text is the first 3 pages and at most 64 KiB, a heading for each page with its paragraphs below.
- Local files only. A PDF inside an archive or on a remote place gives its properties and no text.
- Without pdftotext: "No preview: pdftotext not found". The properties are still said.
- Git for Windows ships a build of Xpdf's pdftotext, and it works.
- In the quick view the whole PDF preview waits for the Preview command or for auto-preview, as sound and video do. The details view shows the PDF's properties either way.

## Helper programs, Preview, and auto-preview

Sound, video and a PDF's text need a helper program (ffprobe and pdftotext, from [Tools and diro doctor](tools.md)). Everything else is read by diroxide itself.

By default a preview that needs a helper does not run it by itself when you move through a folder. The quick view says that it waits:

- "Preview on request: song.mp3, MP3 audio, 20 bytes".

Three things run the helper:

- The Preview command. It is in the View menu and the palette and has no key. It turns the quick view on if it is off, and runs the helper at once on the focused file: "Quick view on", then "Preview: song.mp3, MP3, 3 sec, 129 kbps, stereo". Without the helper it says why: "No preview: song.mp3, ffprobe not found".
- The details view, which asks for everything, runs ffprobe for a sound or video file.
- Auto-preview, when it is on.

Auto-preview is the setting `quick_view.auto_preview`, in the Viewer and quick view group of Settings. It has three values:

- `auto`, the default: on, except in screen reader mode. In the Settings list it says "on, except in screen reader mode".
- `on`: the quick view runs the helper after its wait.
- `off`: only the Preview command and the details view run it.

Auto is off in screen reader mode, so nothing starts a program while you move through a folder by ear.

## Read text in picture

Read text in picture has Tesseract read the words in the focused picture, and shows them in the viewer. It is in the File menu and the palette. Tesseract comes from [Tools and diro doctor](tools.md); diroxide never downloads a language.

- You hear "Reading the text in sign.png". It runs as a job in the jobs line, and Cancel stops it: "Read text in picture: cancelling". Tesseract is stopped with everything it started.
- The text opens in the viewer. Its title says the words are a machine's guess: "Text in picture (machine read), sign.png, 9 words". One line of the picture is one line; a blank line separates paragraphs. A picture with no words says "No text was found."
- English is used when Tesseract has it. Otherwise the first language it lists, apart from its orientation data. diroxide asks for one language, because every language added makes the run slower.
- Only a picture goes in. Tesseract reads any other file as a list of picture names and opens each, so diroxide checks the first bytes first: PNG, JPEG, GIF, TIFF, BMP, WebP, JPEG 2000 or PNM. Anything else says "No text: not a picture Tesseract reads".
- At most 120 seconds and 32 MiB of output. A picture on this computer is named by its path. A picture anywhere else, such as inside an archive, goes in on standard input, its first 64 MiB.
- Without Tesseract: "No text: tesseract not found". Nothing runs.
- Reload in the viewer does not read the picture's bytes as text. It says "This text was read from the picture. Read text in picture reads it again."
- The details view then has a row, "Text in picture (machine read): 9 words", for the rest of the session.

## The picture view in the window

The window can show a picture, as one graphic. It is the window's own: the terminal has none.

- In the viewer (F3) or in the quick view's preview, on a picture, the window says once "Enter shows the picture." Enter opens it. While it reads, a picture that takes more than half a second says "Opening picture photo.png".
- A screen reader finds one graphic named by the file, "photo.png", and described by the same sentence the quick view says, "PNG, 1920 by 1080, 8-bit RGBA, 2.3 MB". JAWS may need its describe key to read the description.
- Plus, Equals or the keypad's plus zoom in. Minus zooms out. 0 fits the picture to the window and 1 shows it at actual size. The arrows move a zoomed picture and say nothing. Escape goes back to the viewer or the preview. The window says what each key did: "Fit to the window, 42 percent", "Actual size, 100 percent", "Zoom 125 percent". Two help lines are drawn and said as the picture opens: "Plus, Minus zoom; 0 fits; Escape closes" and "1 actual size; arrows move when zoomed".
- An animation shows its first frame still, and says "Animation: 12 frames, not playing". Nothing plays.
- The pixel guard stands in front of the pixels, with the numbers above. A picture past it is said and never opened: "PNG, 20000 by 20000, too large to open", "PNG, file too large to open" past 256 MiB, or "PNG, unreadable: file is cut short".
- PNG, JPEG, GIF, WebP, BMP, TIFF and ICO are shown. The other formats say that they need a component.

The picture view has no command of its own yet, so it is not on [the keyboard page](keyboard.md), and Enter is the window's own key for it. The window's picture view has not been heard on a screen reader.

## The limits

Every limit protects the quick view. A file that is past one says so in words and shows what it can.

- Documents: 64 MiB for the file (the setting `quick_view.max_document_mb`, 1 to 1,024), 50,000 members, 32 MiB for any one XML part, 64 MiB for any other part, 256 MiB unpacked in all, and a part over 1 MiB that claims to hold more than 1,000 times its packed size is refused. A refused file says "Could not read big.docx: too large."
- Sheets and delimited text: 10,000 rows, 200 columns, 50 MB.
- Pictures: the pixel guard above.
- Archives: 500 rows, 50,000 members.
- ffprobe and pdftotext: ten seconds and 1 MiB of output each.
- A reader that runs past two seconds stops: "Could not read big.org: took too long." At most three readers run at once, and the next file says "Could not read notes.org: an earlier document is still being read."

The previews are kept in memory so that moving back to a file is instant: up to 256 previews or 32 MiB, whichever comes first. Nothing is written to disk, and a quit throws them away. `diro doctor` says "Cache folder: none, kept in memory".

## Known limits

- In the quick view and the terminal a picture is only said, never drawn. The window's picture view draws PNG, JPEG, GIF, WebP, BMP, TIFF and ICO.
- The picture view has not been tried on a picture inside an archive or on an SFTP site.
- A TIFF's page count and a JPEG XL's size are not read. For raw and HEIF pictures, the Exif data is looked for in the first 1 MiB and 16 MiB.
- RTF: bold and italic are not said, and deleted revisions are left out.
- A spreadsheet's formulas are not worked out again. A cell says what the file last saved.
- A part nested deeper than 256 levels is read as flattened text, with a note.
- Preview and auto-preview gate the whole PDF preview in the quick view, its properties line included.
- A picture that is not on a local disk goes to Tesseract on standard input, up to 64 MiB.
- Poppler's standard-input mode has not been tried, so a PDF off this computer gives no text.
- A real XLS, camera raw, HEIC or AVIF file, and macOS, have not been tried. Linux was tried in the project's development container.

## See also

- [The quick view](quick-view.md), for moving through a preview
- [The details view](details.md)
- [The viewer](viewer.md), where Read text in picture shows its text
- [Tools and diro doctor](tools.md), for ffprobe, pdftotext, Tesseract and ExifTool
- [File types and groups](file-types.md)
- [Settings and where the files live](settings.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
