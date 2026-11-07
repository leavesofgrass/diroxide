# Conversions

This guide covers turning files into other formats: the Convert command, the recipes it offers, the plan you answer before anything is written, the report afterward and Undo, a file type that names its own recipe, your own recipes in `conversions.toml`, and `diro convert` for the shell.

The short version: Convert lists the recipes that take the selected files, plans the conversion into the other panel's folder, and asks first, starting on No. It never writes over a file. If the name is taken, the new file gets a number. Ctrl+Z puts what it made in the trash.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## The Convert command

Convert is in the File menu and the palette, and has no key until you give it one in `keymap.toml` ([Customizing keys](customizing-keys.md)).

Mark the files you want to convert, or leave the focus on one, and put the folder the new files should go to in the other panel. Run Convert.

1. A list opens, titled "Convert notes.md" for one file or "Convert 13 items" for several. Each row is a recipe: its name first, then its sentence. A recipe that cannot run here, because its program is missing, is listed last and unavailable, with what it needs in place of the sentence: "Markdown to Word, needs pandoc". If some of the files have no recipe, the first row says "Some files have no recipe", and the list shows the recipes that take the most of them.
2. Enter on a recipe makes the plan: "Planning the conversion".
3. The plan's question starts on No: "Convert 13 files to JPEG? No is the default. y or n". Under it, the plan's rows say what will happen, below.
4. Yes starts the job: "Convert started: 13 files to JPEG". It is a job in the jobs line and Cancel stops it. No says "Nothing changed."
5. When it ends, the report is said: "Report: 12 converted" and, if one failed, "Report: 1 failed, 12 converted" with the first failure after it.

With nothing that any recipe takes, Convert says "No recipe takes these files". A folder in the selection is converted one level: the files directly inside it, not its subfolders. `diro convert --recursive` does a whole tree.

### The plan

The plan is made before anything is touched. Its rows come in this order:

1. What is refused: "Refused: a.webp, a.png makes the same a.jpg". The reasons are an output that would take its source's own name, two sources that would make one output, a file larger than the recipe's limit, a file that is only online, or a format the tool cannot read.
2. What is skipped, with the reason: "Skipped b.png: output exists" and "Skipped a.png: output is up to date".
3. What is left out: "Left out: 1 file it does not take".
4. The work, one row a file: "notes.md to notes.txt".
5. The tool: "Tool: ImageMagick", or "Tool: carta, built in" for a recipe that needs no program.
6. The input, "Input: 13 files, 4.2 MB", and a warning when the target is short of room: "Low space: 1.2 GB free, input 3.0 GB".

If nothing can be converted, there is no question. Convert says the count and why, such as "Convert plan: 0 files, 1 skipped. Skipped b.png: output exists." or "Convert plan: 0 files. Nothing to convert."

### Names are never replaced

A conversion never writes over a file, whether you are there or not. If the output's name is taken, the new file is numbered the way a copy is: `report (2).pdf`. The other policies, used by `diro convert --conflicts`, are skip, which leaves the taken name alone, and newer, which skips an output that is already up to date.

Each output is written under a temporary name in the target folder, `.diro-part-` followed by eight characters and the extension, and renamed to its real name only when the program has succeeded. A cancel or a failure leaves no half-written file under the real name.

### The report and Undo

The report lists what happened, failures first. diroxide keeps the last 20 reports.

- Conversion reports (File menu and palette) lists the last 20, newest first, each row its summary with when it ended. The title says how many: "Conversion reports: 1 kept". Enter reads one: the summary, then the failures, the refusals, the skips and any notes, then what was made, "Made: notes.org from notes.md", then "Recipe:" and "Tool:", and last "Finished:" with the date and time. With none: "No conversion reports yet".
- Ctrl+Z undoes the whole conversion: "Undid conversion: 1 item is in the trash". The files it made go to the trash, and any folder it made that is now empty is removed. Undo of a conversion works after a restart.
- The report holds names, never full paths. There is no Save report command in the program; `diro convert --report FILE` writes one.

## The recipes

A recipe says what it takes, what it makes and which program runs it. The 31 built-in recipes are in five groups. A recipe is offered when its program is found. [Tools and diro doctor](tools.md) says how each is found and installed.

Pictures, through ImageMagick:

- Image to JPEG, Image to PNG and Image to WebP.
- Shrink to 1600 pixels, which makes a JPEG.
- HEIC to JPEG, which depends on the ImageMagick installed: some builds cannot read HEIC, and the plan refuses it then.

Sound and video, through FFmpeg:

- Audio to MP3, Audio to FLAC and Audio to Opus.
- Video to WebM, Video to MP4 and Sound of a video to Opus.
- Each needs an encoder in the FFmpeg installed: MP3 needs libmp3lame, FLAC the flac encoder, Opus libopus, WebM libvpx-vp9 and MP4 libx264. A build without it says so in the list: "Video to MP4, needs libx264", and in the plan, "this FFmpeg has no libx264 encoder". Tags and cover art are kept for FLAC and MP3. Opus keeps no cover art, since FFmpeg cannot put one in an Opus file.

Documents, through pandoc:

- Markdown to Word, Markdown to a web page and Markdown to EPUB.
- Word to Markdown, EPUB to Markdown, OpenDocument to Markdown and RTF to Markdown.
- pandoc runs with its sandbox on, which leaves pictures out. The pandoc 3.1 that some Linux packages carry cannot write Word or EPUB under the sandbox; version 3.9 can.

PDF, through pdftotext:

- PDF to text and PDF to text with layout.

Built in with carta, which needs nothing installed:

- Markdown to LaTeX, AsciiDoc, Typst, MediaWiki, Org and plain text.
- Markdown to Word, built in, and Markdown to EPUB, built in.
- Org to Markdown, reStructuredText to Markdown and MediaWiki to Markdown.

carta is a converter written in Rust. It reads Markdown, Org, reStructuredText, MediaWiki, DokuWiki, Jira, HTML, DOCX, EPUB, ODT, CSV, TSV and LaTeX, and writes AsciiDoc, Typst, LaTeX, MediaWiki, Org, plain text, DOCX and EPUB. It reads text as UTF-8 only. Its ODT and RTF writers are off for now, and HTML is built for the quick view and is not offered here. A carta recipe runs in a separate `diro` process that Cancel can stop. A recipe has 10 minutes by default, and a longer one says "carta took longer than 600 seconds and was stopped".

What the engine adds to every run, so a recipe does not need to:

- FFmpeg runs quietly, with no questions, and reads files only: `-protocol_whitelist file` goes before each input.
- ImageMagick runs with resource limits, so a picture with a huge claimed size is stopped, and reads each file as the format diroxide found in it, not the format its name says.
- A file's name never reaches a tool as an option: a name that starts with a dash gets `./` in front. A file that is not on this computer, or whose name ImageMagick would misread (one that ends in `]`), is copied through diroxide's spool first.

## A file type that names a recipe

A file type in `filetypes.toml` can give its `convert` action a `recipe` key instead of a `tool`. Then Convert on a file of that type skips the list and plans with that recipe at once:

```toml
[[type]]
id = "mynotes"
name = "Notes file"
extensions = ["notes"]

[[type.actions]]
name = "To Org"
slot = "convert"
recipe = "markdown-to-org"
```

An action that names a recipe is also listed by Actions for this file, and runs the same way. A file whose type has no convert action gets a Convert row in that list, which opens the recipes list. [File types and groups](file-types.md) has the rest of the action format.

## Your own recipes

`conversions.toml` in the config folder holds your own recipes, in the same shape as the built-in ones. It is read every time you run Convert, so a change applies at once. A recipe with the id of a built-in recipe replaces it, in its place in the list.

A recipe is a `[[recipe]]` table with these keys:

- `id`: letters, digits, `-` and `_`.
- `name`: the name said in lists.
- `about`: one sentence, meaning first.
- `from`: the extensions it takes, without the dot. `from-group` takes a file group, such as `images`, and `from-type` takes type ids.
- `to`: the extension it makes.
- `tool`: the id of a tool in [the tools list](tools.md). `carta` is the built-in converter.
- `args`: the arguments after the program, each one whole. No shell ever reads them. A plugin's converter calls the same list `argv`.
- `timeout`: how long one file may take, such as `90s`, `10m` or `2h`. The default is 10 minutes, and the most is a day.
- `max-input`: the largest file it takes, such as `500 MB` or `2GiB`.
- `needs-encoder`: an FFmpeg encoder it needs, such as `libmp3lame`. The recipe is offered only when the FFmpeg installed has it.

The placeholders an argument may hold:

- `{input}` and `{file}`: the file being converted.
- `{typed_input}`: the file as `FORMAT:PATH`, by diroxide's own detection, for ImageMagick.
- `{output}`: the staging name in the target folder, renamed when the tool succeeds.
- `{stem}` and `{name}`: the file's name without and with its extension. A name that starts with a dash gets `./` in front.
- `{dir}`: the file's folder. `{dest}` and `{other}`: the target folder.
- `{{` and `}}` write a brace.

What goes wrong is said at start, meaning first, and the recipe is left out while the others load:

- A placeholder that is not one: "Recipe png-to-jpeg: {size} is not a placeholder."
- An id missing: "Recipe 4: it has no id." The number is the recipe's place in the file.
- An id used twice: "Recipe png-to-jpeg: id used twice, the second left out."
- A key that is not one of those above is a problem too, so a misspelled key never silently does nothing.

## From a shell

`diro convert` does the same work without the panels. It prints one line for each result, the name first, then a summary line.

```bash
diro convert notes.md more.md --to org --out out
```

prints "Made: notes.org", "Made: more.org" and "Report: 2 converted". `--to` takes an extension or a format's words, `docx` or `Word`, and picks for each file the first recipe that takes it, carta's first. `--recipe ID` uses one recipe instead.

The options:

- `--out FOLDER` puts the outputs there. Without it, each goes beside its file.
- `--conflicts WHICH` says what a taken name gets: `keep-both` (the default), `skip` or `newer`. Nothing is ever replaced.
- `--recursive` converts a folder with everything below it, mirroring the tree into the target. Without it, a folder is converted one level.
- `--dry-run` says each plan, "notes.md to notes.org", and writes nothing. It ends with "Dry run: 1 would convert".
- `--report FILE` writes the report as Markdown, with names only and no full path. The program keeps the report as well, so Conversion reports lists it.
- `--json` writes one JSON object a line, with `result` and `name`, and a last one with `summary`.
- `--print0` writes only the made files' paths, each ended by a NUL character, for `xargs -0`.
- `--list` lists the recipes, and `--list FILE` the recipes that take that file. Each line starts with the id: "markdown-to-org: Markdown to Org, carta, built in, ready", then what it needs when it cannot run: "markdown-to-word: Markdown to Word, pandoc, needs pandoc".

The result lines are "Made: notes.org", "Skipped: notes.md, output exists", "Refused: NAME, REASON" and "Failed: blob.qqq, no recipe makes org from it". The summary says what went wrong first: "Report: 1 failed, 2 converted, 1 skipped". Progress words go to standard error at most every ten seconds, and only when it is a terminal: "Converting 12 of 40 images". There is no spinner.

A recipe that cannot run says why before any file is touched: "Could not convert: pandoc is not installed." An unknown id: "Could not convert: no recipe is named no-such."

The exit codes are the family's: 0 when everything was done, 1 when nothing could be done (a missing tool, no recipe), 2 for wrong use, and 3 when some files were done and some failed, with the lines saying which. `diro convert --help` shows the usage.

## Known limits

- The app's jobs line uses the job engine's general progress words. "Converting 5 of 13 images" is said only by `diro convert`, on standard error.
- The Convert list types files by their names. The plan types them by their contents, so a file named wrongly can be refused in the plan after the list offered a recipe.
- There is no Save report command in the program. `diro convert --report FILE` writes one.
- A plugin's converters are not in the Convert list yet. `diro plugin convert` runs one on a single file.
- A report keeps no hash of the files. It holds names, the outcomes and the finish time.
- carta reads UTF-8 text only, and a very deeply nested Org file can take minutes. There is no PDF output.
- Percent progress for sound and video needs ffprobe.
- The built-in recipes were run with the real programs in the project's Linux container, all but Markdown to Word and to EPUB through pandoc 3.1 and HEIC, and natively on Windows, all but ImageMagick's, MP4 and HEIC. macOS and Linux natively have not been tried.
- Nothing about how the Convert list or the report list read on a Braille display has been heard yet.

## See also

- [File types and groups](file-types.md)
- [Tools and diro doctor](tools.md)
- [Watched folders](watched-folders.md), which run a recipe on each file that arrives
- [Operations and undo](operations.md)
- [Optional components](components.md)
- [Writing a plugin](writing-a-plugin.md)
- [Customizing keys](customizing-keys.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
