# Using diroxide with a screen reader

This guide is for people who use a screen reader such as NVDA, JAWS, Orca or VoiceOver, with or without a Braille display. It says how diroxide works with a screen reader, what you should hear for each key, the settings to check in NVDA and JAWS, how to read it on a 40-cell display, and a short check you can run yourself.

What this guide says about diroxide comes from its design. A setting that comes from a screen reader's own documentation, or that has not been tried here, is marked not yet verified. Nothing in alpha.2 had been heard on a screen reader when this guide was written, so every "you should hear" line below is what diroxide is built to say, not a report of what was heard. If your experience differs, that is a finding. [The accessibility statement](accessibility.md) says what has been checked.

## How diroxide works with your screen reader

diroxide has no voice of its own. Everything it would say is text on the screen, and your screen reader speaks it. It gives your screen reader two signals.

- The hardware cursor. It is always on the first cell of the focused row, so a screen reader or a Braille display that follows the cursor follows the row. In a prompt it sits where the next character will go. In an empty folder it sits on the line that says "Empty folder". Screen readers follow the cursor, not the highlight, which is why diroxide does not rely on a highlight.
- The status line. Messages, moves and questions go to one line that a terminal screen reader speaks when it changes. The same message twice in a row would be silent, so diroxide blanks the line for 150 milliseconds before a repeated message, and you hear it again.

Other things you can rely on:

- Meaning comes first. Every row, label and message puts its meaning in the first 40 characters, which is what a 40-cell Braille display shows.
- Words come before symbols. Status is words such as "selected", "Pass" and "Fail", never a color or a symbol alone. A selected row says "selected" right after its name. When screen-reader mode is off the screen also starts the row with an asterisk; in screen-reader mode it does not, so the row spends no cells on it.
- There are no spinners. Progress is a sentence that changes slowly.
- The default look draws no boxes. Box drawing often comes out as noise on a Braille display.
- diroxide never uses Insert or Caps Lock, which NVDA, JAWS and Orca use as their own key. Its default keys use no Control plus Alt chords, which VoiceOver uses.
- Control and invisible characters in a file name are shown as a question mark, so a hostile name cannot send commands to your terminal or confuse your screen reader.

### Screen-reader mode

The `screen_reader_mode` setting turns on the behavior a screen reader needs. Its values are `auto`, `on` and `off`, and the default is `auto`. diroxide does not detect your screen reader yet, so `auto` behaves as on: you start in screen-reader mode, and `off` is the choice that changes it. Check the setting in Settings (Alt+T, then S). In screen-reader mode:

- the jobs line changes at once when a job changes state, and otherwise at most every ten seconds (`progress_interval_seconds`). Outside screen-reader mode it can change twice a second;
- the other panel is redrawn only when you ask, so a change there does not interrupt you. The quick view's one-line announcement still arrives;
- while you type in a prompt, one line says the count of matches, and the list is drawn only when you move into it;
- highlights carry an attribute and a text marker, never only color;
- code in the viewer is not colored, and a hex row is eight bytes, so it fits 40 cells.

The key bar is the same in every mode. It is the last line of the screen, and it names the function keys in words: "F2 Rename  F3 View  F4 Edit  F5 Copy  F6 Move  F7 Folder  F8 Trash  F9 Menu  Ctrl+Q Quit". It is on by default, in screen-reader mode too, and the `key_bar` setting turns it off. The width of the terminal decides how many pairs fit, with the most common operations first. Say key bar, in the Help menu, reads the line to you whether it is drawn or not.

<!-- to confirm with the wiring (W2f): the key bar is drawn in screen-reader mode, the setting's name and default, and the exact order of the pairs. -->
The `quiet_screen` setting goes further and draws as little as possible, for a screen reader that reads every change.

## What you should hear

These lines show what diroxide says for each key. The status line says the same words that a menu or the palette shows.

<!-- to confirm with the wiring (W2f): the first-run status line now says "Ctrl+Q quits" beside the help key and names the key bar; update the Starting line below, and the first-start lines of the quick start, to match. -->

- Starting: on the very first start the status line says "diroxide alpha 1. Shift+F1 for help." and "Layout: side by side. Ctrl+P, Layout." The title line comes first, "dirocheck, left panel, in D:\", and then the first row. You do not hear the whole screen.
- Down and Up: the cursor moves one row and your screen reader speaks the row: "Photos, folder, modified Tuesday". No message is written for a focus move, and the status line does not change. At the first or last row diroxide says "Top of the list" or "Bottom of the list".
- Down in the side by side layout with the left panel active: your screen reader's current line is the whole screen line, which also holds the right panel's row, so it may read the left row and then the right. With the right panel active, the cursor sits near column 41 and a 40-cell display shows the left row first. This is why stacked is recommended.
- Tab: one message and nothing else, "Right panel: dirocheck, 31 items". Chatter on Tab is the first thing to report.
- Space: "Selected report.pdf. 1 selected." and then the next row is read. The row you moved onto says "selected" after its name if it is marked.
- Ctrl+D (Say Status): "3 selected, 4.2 MB. dirocheck, 31 items, on report.pdf, 3 of 31". With no marks: "dirocheck, 31 items, no selection, on report.pdf, 3 of 31".
- Alt+O (Say Other Panel): "Other panel: Backup, 12 items, on report.pdf, 3 of 12".
- Ctrl+F6 (sort by size): "Sorted by size, largest first". Again: "Sorted by size, smallest first". The title line ends ", sorted by size".
- F5 (copy): "Copy 3 items to Backup, 312 bytes? No is the default. y or n", with the cursor on that line. The question starts on No.
- A name taken: a list opens, "Name taken: report.pdf", "New is newer and larger", the two fact lines ("Existing: ..." and "New: ..."), then "Skip this one, 1 of 9" with the focus on it.
- A copy running: "Copy started: 1 item to Backup", then "Copying 30 percent, 2 minutes left" at most every ten seconds, and once at the end, "Copy finished: 1 done."
- F2 (rename): "Rename report.pdf to: report.pdf", with the cursor before the extension. A name that needs care: "Name needs care: ends in a dot or space. Nothing renamed."
- F8 (trash): "Move 1 item to the trash, 2.1 KB? No is the default. y or n". Then "Moved 1 item to the trash. It can be restored." Ctrl+Z: "Undid trash: 1 item is back".
- Ctrl+Q with a job running: "Quit? 1 job is running. No is the default. y or n".

### Archives

- Enter on an archive: the title puts the kind first, "course.zip, archive, in D:\docs", and the status line says "Opened course.zip: 10 items, 48 KB." A row inside says its name, then its kind, size and position: "notes.txt, text file, 3 KB, 5 of 10". A flag comes right after the name: "secret.docx, locked, Word, 40 KB".
- A big tar.gz or tar.xz: "Reading logs.tar.xz: 3.2 of 14.0 GB" at most every ten seconds.
- A locked archive: the prompt "Password for course.zip (hidden):", with an asterisk for each key you type or nothing, as `secret_echo` says. A wrong password: "Wrong password for course.zip. Try again? y or n".
- A limit: "Expands to 40 GB from 40 KB. Read huge.bin? y or n". The fact is first and the question starts on No.
- Unpack: "Unpack 10 members into course? y or n", then the job lines, then "Unpack finished: 118 done, 2 skipped." and a line for each skip, the name first: "Skipped ../x: name leaves the archive."
- Pack: "Pack 12 items into reports.zip? y or n", the progress, "Pack finished: 300 done, 2 skipped." and "Checked 300 of 300 entries, all good."
- Test archive: "Archive is sound: 10 members."
- A change inside an archive: "Archives are read only here. Use Pack to make a new one."

### The viewer

- F3: the title names the file, "notes.txt, in course.zip", and the cursor is on the first line. Ctrl+D says "Line 120 of 4,000, 3 percent".
- Search: "Match 3 of 12, line 120", and n and p step through them. Nothing found: "No match for zebra."
- Hex mode: eight bytes a row, `000010: 48 65 6c 6c 6f 20 77 6f Hello wo`. That is 40 characters, so a 40-cell display shows a whole row, and a screen reader reads it slowly. Say byte speaks one byte: "Byte 16 is 48 hex, the letter H". Whether eight bytes is the right width is for the listening check to decide, and the `viewer.hex_row` setting changes it.
- Code is silent: no color is spoken. Ask for what the colors would show. Token kind: "Function name: parse_args, line 21". The line by kind: "Line 21: keyword, function name, variable, operator and 1 more". The outline: "Outline: 7 symbols in main.rs", then rows that put the kind first, "function main, line 40". [The viewer](viewer.md) lists the commands.
- A large file: "Showing the first 64 MB of 800 MB." and "report.bin looks binary; hex mode".

### The quick view

- Shift+F3: "Quick view on". When the cursor settles on a file, one line, never the whole document: "Preview: report.md, 12 headings". Move through five files quickly and about one "Preview:" line arrives when you stop.
- Tab into the preview: "Preview, report.md" and the line you are on. Moving by heading: "Heading 2: Installation, 3 of 12". In a table: "Table 1 of 2, 5 rows, 3 columns", and a cell says its header first, "Age: 36, row 2 of 5, column 2". Escape returns to the panel.
- A file changed in another program: "Preview updated: report.md changed", once.

### Split, join, links and permissions

- Split: "12 parts of 20 MB, the last 3.1 MB", "223 MB in all", "Enough space: 12 GB free on D:", then the question. Progress: "Splitting 4 of 12 parts, 1.4 of 4.0 GB".
- Join: "Join 12 parts into report.pdf, 223 MB", a gap named first, "Part 004 is missing. Nothing was joined.", and the verdict, "Joined report.pdf: checksum matches." Then "Move the 12 parts to the trash? y or n", starting on No.
- A link without the right on Windows: "Cannot link: needs developer mode."
- Permissions in words first: "Owner can read and write, group can read, others can read (644)". The octal number is second.

<!-- to confirm with the wiring (W2f) and W2e: every line in this section is the design's text; the final pass checks each against the merged messages and the 40-cell test. -->

## Choosing a layout

Side by side is the default and the first choice of sighted users. For a screen reader, stacked is recommended, because every line is one row at full width and Tab changes only the cursor, a marker and a title. Press Ctrl+P, type "layout" and press Enter to change it; the `layout` setting keeps the choice. The listening check will say which layout to recommend, and this guide will change if it finds that side by side reads well. [The panels guide](panels.md) has the reasons.

## NVDA

These settings are the ones to look at. Names and keys differ between NVDA versions. Every item here is not yet verified by ear.

- Report dynamic content changes (NVDA+5, in Object Presentation settings): on. That is how NVDA speaks the status line as it changes.
- Speak new text in Windows Terminal via (Advanced settings): the default is Diffing. UIA notifications is described in NVDA's guide as incomplete. Try both if new text is missed or read late.
- Diff algorithm (Advanced settings): leave it on Automatic. Diff Match Patch can read choppily, and Difflib can read the text after the cursor when a character is typed in the middle of a line.
- Windows Console support (Advanced settings): Automatic is the default. UIA when available or Legacy applies to the classic console.
- Speak typed characters (NVDA+2) and Speak typed words (NVDA+3): on. diroxide does not echo your typing; NVDA does.
- Read current line: NVDA+Up (laptop layout NVDA+L). It reads the row at the cursor.
- The numeric keypad: NVDA's desktop layout uses the bare keypad for review commands. diroxide's plus, minus and star keys arrive from the keypad as ordinary characters only when Num Lock is on. Keep Num Lock on when you want the keypad to reach diroxide. Not yet verified.

## JAWS

Every item here is not yet verified by ear. JAWS in Windows Terminal is the least known combination. No vendor page describes it, and the first listening check on this project will be the first evidence.

- Screen echo (Insert+S cycles none, highlighted and all): all, so JAWS reads the status line as it changes.
- Typing echo (Insert+2 cycles characters, words, both and none): on.
- Read current line: Insert+Up. It reads the row at the cursor.
- JAWS's PC cursor follows the terminal's cursor, which diroxide keeps on the focused row.
- The numeric keypad: in JAWS's desktop layout the bare keypad is a set of reading commands, and the same Num Lock advice applies.
- If JAWS does not read a full-screen program well in Windows Terminal, try the classic console, or the plain line mode `diro ls`.

## Orca on Linux

Orca reads the text of a terminal through the accessibility bus, so which terminal you use matters.

- Terminals built on VTE work: GNOME Terminal, Console and Ptyxis. Konsole has an accessible text display class that has not been tried. Terminals that draw with the graphics card, such as Alacritty and kitty, expose nothing for Orca to read. Use a VTE terminal.
- Orca says each key you press by default, modifiers included, so every command is announced before diroxide answers. To hear only the answer, turn off key echo in Orca's preferences under Echo. That is Orca's own setting.
- On the text console, BRLTTY, Speakup and Fenrir follow the hardware cursor.
- Orca and diroxide are best effort. Orca's own release notes still list fixes for chattiness in terminals.

## VoiceOver on macOS

- diroxide runs in Terminal.app and iTerm2. VoiceOver reads new text in Terminal as it appears. Basic use has not yet been tried here.
- Alt chords need the Option key to act as Meta. In Terminal, open Settings, Profiles, Keyboard, and turn on "Use Option as Meta key".
- VoiceOver's own keys use Control plus Option, and diroxide's terminal keys use no Control plus Alt chords, so they do not clash.
- The function keys need Fn unless you change the system setting.
- Full-screen programs that grab the mouse have been reported to break VoiceOver's reading in Terminal.app. diroxide never captures the mouse. If reading is poor, try `diro --inline`, or `diro ls`.

## A Braille display

diroxide draws no braille of its own. A Braille display shows what your screen reader shows: the line at the cursor, and new text as it appears. This section is written for a 40-cell display, the HumanWare Mantis Q40, with NVDA or JAWS on Windows, using computer braille, one cell per character. If you read contracted braille, capital signs and number signs take extra cells, so the key fact is kept in the first 30 characters of a line where possible.

What each line shows in its first 40 characters:

- The title line: the folder first, then the panel, then the parent. "Documents, left panel, in D:\work".
- A row: the name, the kind, the size. "report.pdf, PDF, 2.1 MB, modified Tuesday". A selected row: "report.pdf, selected, PDF, 2.1 MB,".
- After a mark or at an edge: "Selected report.pdf. 1 selected." and "Top of the list".
- Say Status: "3 selected, 4.2 MB. Documents, 40 items".
- A sort: "Sorted by size, largest first".
- A question: the action, the count and the target before the answer. "Move 3 items to Backup, 312 bytes? No is the default. y or n".
- Progress: "Copying 30 percent, 2 minutes left". The percent and the estimate sit inside the first 40 cells.
- An archive's title: "course.zip, archive, in D:\docs". A member: "notes.txt, text file, 3 KB, 5 of 10". A locked one: "secret.docx, locked, Word, 40 KB".
- A password prompt: "Password for course.zip (hidden):". A wrong one: "Wrong password for course.zip. Try again? y or n".
- A hex row, eight bytes: `000010: 48 65 6c 6c 6f 20 77 6f Hello wo`, exactly 40 characters.
- The quick view's line: "Preview: report.md, 12 headings".
- A split plan: "12 parts of 20 MB, the last 3.1 MB". A gap: "Part 004 is missing. Nothing was joined."
- The key bar: "F2 Rename  F3 View  F4 Edit  F5 Copy" is the first 36 characters, and the width of the display decides how many pairs show after it.

A line that runs past 40 cells still reads correctly when you pan. Only the start is guaranteed. A test walks a session and checks that each of these lines carries its key fact inside 40 cells.

NVDA braille settings to try, in NVDA's Settings, Braille. Every item here is not yet verified on a Mantis Q40.

- Tether braille: automatically or to focus, so the display follows the terminal's cursor. To review leaves the display where you moved it.
- Show messages and Message timeout: NVDA's own messages cover the line for a few seconds. A short timeout, or disabled, keeps diroxide's status line in view.
- Word wrap: on, so a long line breaks between words when you pan.
- Focus context presentation: fill display for context changes is the default. Try it if the start of a line is not what you expect.
- Interrupt speech while scrolling: on, if you read by braille and do not want speech to continue while you pan.

JAWS braille settings to try, in Settings Center (Insert+6), Braille. Every item here is not yet verified on a Mantis Q40.

- Braille mode: Line mode shows the line at the cursor, which is what the layout is designed for. Structured mode adds control type words.
- Braille follows active cursor: on, so the display follows the terminal's cursor.
- Flash messages: JAWS shows its own messages for a while. A shorter time, or messages off, keeps the status line in view.
- Word wrap: on.
- Status cells: off on a 40-cell display, so all 40 cells show the line.

If the display does not follow the row, or a screen reader talks too much on every cursor move, the `cursor_fallback` setting parks the cursor on the status line instead.

## A check you can run

Run this once with NVDA and once with JAWS, in Windows Terminal and, if you like, the classic console. If you build from source, `cargo xtask sample-folder` followed by a folder name writes a folder to try it in, with a conflicting file, a large file and names with accents, Japanese and an emoji.

1. Start `diro` on that folder. You should hear the title and the first row, not the whole screen.
2. Press Down five times. Each row is spoken once, name first. The display shows the row from its first cell.
3. Press Tab. You hear one message. Press Tab again.
4. Press Space on three files. You hear "Selected report.pdf. 1 selected." each time. Press Up onto a marked row: is the word "selected" heard after the name?
5. Press Ctrl+D. You hear the count and the size of the selection.
6. Press Ctrl+F6, then Ctrl+F6 again, then Ctrl+F3. You hear each sort result. If a key does nothing, open the palette, type "sort" and choose the entry. That key is worth reporting.
7. Copy a file onto one with the same name. Press F5, press y, and walk the conflict list.
8. Copy a large file. How often does the progress speak? Press Ctrl+D for the sentence on demand. Press Escape partway and check that no half-written file remains.
9. Press Ctrl+P, type "layout", press Enter, and then Tab and Down. Is stacked quieter? Choose the layout you will keep.
10. Open the folder with the accented, Japanese and emoji names. Each should read cleanly, one row each.
11. Press Ctrl+P, type "test keys" and press Enter. Press Ctrl+F5, Alt+Left and Ctrl+V. Each key is said in words with what it sent and what it would run, or nothing is said and the terminal kept it. After ten seconds without a key it says so and ends. Escape ends it sooner.
12. Start a copy, press Ctrl+Q, answer n, press Escape to cancel the job, and press Ctrl+Q again. The shell prompt should come back with a visible cursor.

Questions to answer: Did Tab chatter? Was "selected" heard after the name? How often did progress speak? Which keys did not arrive (`diro keys test` lists them)? Which layout will you keep? Did your screen reader speak the terminal's window title when the folder changed? Was "Top of the list", pressed twice at the top, spoken twice?

diroxide sets the terminal's title to the active place. Whether NVDA or JAWS speak that title when it changes has not been verified.

## A check for archives, the viewer and the quick view

If you build from source, `cargo xtask demo` followed by a new or empty folder on your own drive writes a demo set into it. It holds `course.zip` (ten entries), `backup.7z`, `logs.tar.xz`, `locked.zip` (its password is printed once when the demo is made), `hostile.zip` (one member named `../escape.txt`), `big.bin` (60 MB), `main.rs`, `report.md`, `notes.org`, `big.log` and an empty folder, `out`. Put the left panel on the demo folder and the right panel on `out`, with screen-reader mode on. Where a key is not named, use the command's name in the palette. Run it with NVDA, then JAWS, then the Braille display, and on the display read the first 40 cells of each line.

1. Open `course.zip` with Enter. You hear "course.zip, archive, in" and the folder, then "Opened course.zip: 10 items". Press Down through the list: each row starts with the name, then "folder" or "text file", then a size, with the position last.
2. Press Enter on `week1`: "week1, in course.zip". Press F3 on `a.txt`: the viewer's title names the file and the archive. Press Escape. The cursor is back on the row.
3. Press Left until you are back in the demo folder. Open `locked.zip`. You hear "Password for locked.zip (hidden):". Type a wrong password: "Wrong password for locked.zip. Try again? y or n". Is anything you typed spoken?
4. Open `hostile.zip`. The listing has no `../escape.txt`. Unpack it into `out`: "Skipped ../escape.txt: name leaves the archive."
5. Mark `week1` inside `course.zip` and press F5. You hear the plan and a copy into `out` ending in "finished". Press Ctrl+Z: the copy goes to the trash.
6. Put the right panel inside `course.zip`, mark a file in the left panel and press F5: "Archives are read only here. Use Pack to make a new one."
7. Mark two files and run Pack. You hear "Pack 2 items into" and the format, level and place rows, then a question that starts on No. Answer y: "Pack finished" and "Checked ... all good". Run Test archive on the new zip: "Archive is sound".
8. Press F3 on `report.md`. Press Ctrl+D: "Line 1 of ...". Search for a word that occurs more than once: "Match 1 of ...". Press n. Search for a word that is not there: "No match for".
9. Press F3 on `big.bin`: "big.bin looks binary; hex mode". Read a hex row and ask for one byte with Say byte. Is eight bytes a row comfortable by ear?
10. Press F3 on `main.rs`. Nothing about color is spoken. Ask for Token kind at a function name, then Line by kind, then Outline.
11. Press Shift+F3 on `report.md`: "Quick view on" and one line, "Preview: report.md, 12 headings". Move quickly down and up through five files. Is it quiet, with about one "Preview:" line when you stop?
12. Press Tab. You hear "Preview, report.md". Move by heading, then to the table, and step through a cell: the header, a value and the row and column. Press Escape. Move to `notes.org`: "Preview: notes.org, Org".
13. Cursor on `big.bin`: run Split file and choose Email attachment. You hear the parts, the total and the free space, then a question. Answer y. Open `out`: `big.bin.001` and the other parts, then `big.bin.crc`.
14. Rename `big.bin.003` and run Join files from `big.bin.001`: "Part 003 is missing. Nothing was joined." Rename it back. Join from `big.bin.002`: "Joined big.bin: checksum matches." and the offer to trash the parts, which starts on No. Answer n.
15. Run Make symbolic link on `main.rs`. On Windows without Developer Mode: "Cannot link: needs developer mode."

Questions to answer: Was every line's meaning inside the first 40 cells? Did anything speak a password? Did the quick view chatter? Was eight bytes of hex right? Which command did you need the palette for?

## Keys that may clash

- The screen reader key. NVDA, JAWS and Orca use Insert, or Caps Lock if you choose it, as their own key. diroxide uses neither, so your screen reader's commands pass through to it.
- Single-letter keys. Typing a letter finds a row by name. Screen readers do not use their browse mode in a terminal, so these letters reach diroxide. Dictation software types letters; turn single-letter keys off with the `single_letters` setting, and every command stays reachable by the keys that are not letters, by the menus and by the palette. The viewer has single-letter keys too, such as w, l, q, n and p; with single-letter keys off, every viewer command, search included, is still in the palette, and Escape closes the viewer.
- The keypad and Num Lock, described above.
- Windows Terminal keeps some keys for itself. [Terminals](terminals.md) lists them and says how to give them back or avoid them.

## See also

- [The accessibility statement](accessibility.md)
- [Terminals](terminals.md)
- [The panels](panels.md)
- [Archives](archives.md)
- [The viewer](viewer.md)
- [The quick view](quick-view.md)
- [Split and join](split-join.md)
- [Troubleshooting](troubleshooting.md)
- [Settings and where the files live](settings.md)
- [The documentation index](README.md)
