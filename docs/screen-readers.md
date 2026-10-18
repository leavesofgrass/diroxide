# Using diroxide with a screen reader

This guide is for people who use a screen reader such as NVDA, JAWS, Orca or VoiceOver, with or without a Braille display. It says how diroxide works with a screen reader, what you should hear for each key, the settings to check in NVDA and JAWS, how to read it on a 40-cell display, and a short check you can run yourself.

What this guide says about diroxide comes from its design. A setting that comes from a screen reader's own documentation, or that has not been tried here, is marked not yet verified. Nothing in alpha 1 had been heard on a screen reader when this guide was written, because the listening tests come after the build, so every "you should hear" line below is what diroxide is built to say, not a report of what was heard. If your experience differs, that is a finding. [The accessibility statement](accessibility.md) says what has been checked.

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
- the other panel is redrawn only when you ask, so a change there does not interrupt you, and the quick view shows only "Preview" there until you press Tab. Its one-line announcement still arrives;
- while you type in a prompt, one line says the count of matches, and the list is drawn only when you move into it;
- highlights carry an attribute and a text marker, never only color;
- code in the viewer is not colored, and a hex row is eight bytes, so it fits 40 cells.

The key bar is the same in every mode. It is the last line of the screen, and it names the function keys in words: "F2 Rename  F3 View  F4 Edit  F5 Copy  F6 Move  F7 Folder  F8 Trash  F9 Menu  Ctrl+Q Quit". It is on by default, in screen-reader mode too, and the `key_bar` setting (Key bar, a switch in the Look group) turns it off. The width of the terminal decides how many pairs fit, with the most common operations first; at 80 columns the pairs have one space between them so that all nine fit. Say key bar, in the Help menu (Alt+H, then K) and the palette, reads the line to you whether it is drawn or not. Switch bank, in the View menu and the palette, shows the next bank of buttons. While the viewer is open, the key bar shows the viewer's own keys.

The `quiet_screen` setting goes further and draws as little as possible, for a screen reader that reads every change.

## What you should hear

These lines show what diroxide says for each key. The status line says the same words that a menu or the palette shows.

- Starting: on the very first start the status line says "diroxide alpha 1. Shift+F1 for help. Ctrl+Q quits.", then "The key bar on the last line names the keys.", then "Layout: side by side. Ctrl+P, Layout." The alpha number is the build's own. The title line comes first, "dirocheck, left panel, in D:\", and then the first row. You do not hear the whole screen.
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

- Enter on an archive: the title puts the kind first, "course.zip, archive, in D:\diro-demo", and the status line says "Opened course.zip: 10 items, 1.8 KB." A row inside says its name, then its kind, size and position: "notes.txt, text file, 3 KB, 5 of 10". A flag comes right after the name: "secret.docx, locked, Word, 40 KB". Enter on a folder inside: "week1, 3 items, in course.zip".
- A big tar.gz or tar.xz: nothing is said while it is listed, and then "Opened logs.tar.xz: 3 items, 8.5 KB." There is no progress line for this yet.
- A locked zip lists without asking. F3 or F5 on a locked member brings the prompt "Password for locked.zip (hidden):", with an asterisk for each key you type or nothing, as `secret_echo` says. A wrong password: "Wrong password for locked.zip. Try again? No is the default. y or n".
- A limit: "Expands to 40 GB from 40 KB. Read huge.bin? No is the default. y or n". The fact is first and the question starts on No.
- F5 out of an archive: "Unpack 4 members into out? No is the default. y or n", then the job lines, then "Unpack finished: 4 done." A skipped member has a line, the name first: "Skipped ../escape.txt: name leaves the archive."
- Pack, from the palette or File, Archive, Pack: a list, "Pack, Pack 2 items into demo.zip, 1 of 9", then the question "Pack 2 items into demo.zip? No is the default. y or n", the job lines, and "Pack finished: 2 done. Checked 2 of 2 entries, all good."
- Test archive: "Test started: demo.zip", then "Archive is sound: 2 members."
- A change inside an archive: "Archives are read only here. Use Pack to make a new one."

### The viewer

- F3: the title names the file, "notes.txt, in D:\diro-demo", or for a member "a.txt, in course.zip!week1", and the cursor is on the first line. Ctrl+D says "Line 1 of 3, 33 percent". Escape says "Viewer closed: notes.txt".
- Search: the slash key opens "Search for", then "Match 1 of 2, line 1", and N steps to "Match 2 of 2, line 2". Nothing found: "No match for zebra."
- Hex mode, H: "Hex mode, 8 bytes a row", then eight bytes a row, `000010: 48 65 6c 6c 6f 20 77 6f Hello wo`. That is 40 characters, so a 40-cell display shows a whole row, and a screen reader reads it slowly. B (Say byte) speaks one byte: "Byte 0 is 32 hex, the digit 2". Whether eight bytes is the right width is for the listening check to decide, and the `viewer.hex_row` setting changes it.
- Code is silent: no color is spoken. Right on a word says it: "main". Ask for what the colors would show. K (token kind): "Function name: main, line 4". Shift+K (the line by kind): "Line 4: keyword, function name". O (the outline): "Outline: 2 symbols in main.rs", then rows that put the kind first, "function main, line 4, 1 of 2". [The viewer](viewer.md) lists the commands.
- A large file: "Showing the first 64 MB of 800 MB." A binary file: "big.bin looks binary; hex mode".

### The quick view

- Shift+F3: "Quick view on", then one line, never the whole document: "Preview: report.md, 12 headings". Move up through five files quickly: each row is read, then one line arrives when the cursor stops, such as "No preview: locked.zip, archive, 197 bytes".
- In screen-reader mode the other panel shows only "Preview" and "Tab goes into the preview" until you go into it. Tab then draws the document and says "Preview, report.md, Report, heading 1, 1 of 25". H: "Heading 2: Part 1, 2 of 12". T: "Table 1 of 1, 2 rows, 2 columns". Right: "Age, header, row 1 of 2, column 2". Shift+Down: "Age: 36, row 2 of 2, column 2".
- Slash: "Find in the preview", then "Match 1 of 22 in the preview", and N: "Match 2 of 22 in the preview". Escape returns to the panel, with the cursor on your row and nothing read twice.
- A file changed in another program: "Preview updated: report.md changed", once. Shift+F3 again: "Quick view off".

### Split, join, links and permissions

- Split, from File, Split file: a list, "Split into, Email attachment, 18 MB, Parts of 18,000,000 bytes, 1 of 9". The plan says "4 parts of 18 MB, the last 6 MB", "60 MB in all" and "Enough space: 12 GB free on D:", then the question. The job says "Split started: 1 item to out", then "Splitting 4 of 4 parts, 60.0 of 60.0 MB", then "Split finished: 5 done."
- Join, from any part: "Join 4 parts into big.bin, 60.0 MB? No is the default. y or n". A gap is named first, "Part 003 is missing. Nothing was joined." The verdict: "Joined big.bin: checksum matches." Then "Move the 4 parts to the trash? No is the default. y or n". Answer n and it says "The parts stay where they are."
- Details, Alt+Enter on an archive: "Details of course.zip, Type: zip archive, 1 of 8", then the size, then "Entries: 10 items".
- Change permissions, from File: "Permissions, Change permissions of main.rs, 1 of 4". Permissions in words first: "Owner can read and write, group can read, others can read (644)". The octal number is second.
- Make symbolic link, from File: "Make link, Make a link to main.rs in out, 1 of 4". The job says "Link finished: 1 done." On Windows without Developer Mode: "Cannot link: needs developer mode."
- Shift+F5 (Duplicate): "Duplicate main.rs as: main (2).rs".

### Search, filters and select by pattern

- Ctrl+S and typing: nothing is spoken for each key. About 400 milliseconds after you stop, one line says the count and the first match: "37 matches, first report.pdf", or "1 match, big.log". With none: "No matches". Enter keeps the filter: "Filter on, 4 of 9 shown", and the title line reads it: "filter: size over 1 KB and name matches *.log". Escape in the panel: "Filter off, 9 shown".
- A problem is said in two parts, each once: after the pause, "Filter problem: size needs a number", and after one more pause, "Word 3. Try size over 10 MB." It is not said on every key.
- Say filter, in the View menu (Alt+V, then Y) and the palette: the filter in one sentence with the real days, "modified last week, October 4 to 10".
- Plus (select by pattern): "Select by pattern: *". Enter on a pattern: "4 entries will be selected. No is the default. y or n", and after y, "4 selected".
- Ctrl+F opens the Search dialog: "Search, Name or filter: every entry, 2 of 11". Type a letter, give the words, and press Enter: "Searching work for *.txt", or "Searching work for *.txt, containing total due". Then at the progress interval, not on every change: "Searching: 214 found, 3,120 folders". At the end: "Done: 4 found in 3 folders." and then the skips, such as "Hidden files skipped." Escape: "Stopped at 214 found." F2 pauses: "Paused at 214 found."
- A result row: "notes.txt, in work, 4 KB". A content hit: "invoice.txt, line 3: total due now". A folder: "q3, folder, in Documents". The position comes last. The cursor never moves while results arrive.
- F3 on a content hit opens the viewer at the match: "At line 3 of 4". Escape: "Back in the results, invoice.txt". A file that has gone since the list was made: "notes.txt, gone since this list was made". Escape once the search has ended: "Results closed. Search results opens them again."
- The Search menu (F9, then R): Save this search, "Search saved as Text files. Saved searches runs it." Saved searches: "Saved searches, 1 search, Text files, saved search, runs on open, 1 of 1". Enter: "Running Text files: Searching work for *.txt", then "Done: 4 found in 3 folders."
- The Space list: "Counting the space in work", then "Total 66.4 GB. Largest 3 hold 81 percent", then rows such as "Videos, folder, 41.2 GB, 62 percent", ending "And 340 smaller items, 2.1 GB, 3 percent".

### Labels, bookmarks and views

- Alt+L: "Set label on 3 items, empty", with the labels listed under the field. Type `urg` and one line says how many labels match. Enter: "Label urgent added to 3 items". Ctrl+Z: "Undid label urgent on 3 items".
- A labeled row: "report.pdf, label: urgent, 2.1 MB". The label's word is drawn in its color too; speech and Braille say the word only.
- Labels list, from the palette: "Labels, 6 labels", then rows such as "Urgent, label, 14 items, checking, orange, Ctrl+1". When the count is ready the row says "0 missing" and the status line says "Checked 14 labeled items: 0 missing". The row's change is not spoken. Enter: "Looking for 14 items labeled urgent", then "Done: 14 items labeled urgent."
- Ctrl+B: "Bookmarks, no bookmarks, Add this folder, sample, in Drive D:\, 1 of 1". Enter on the first row: "Bookmarked sample, in Drive D:\". Ctrl+B again: the first row says "Remove this folder", and the bookmark's row says "sample, bookmark, in Drive D:\, not used yet".
- Delete on a bookmark: "Remove bookmark Reports? No is the default. y or n", then "Removed bookmark Reports".
- History list, in the Go menu (Alt+G, then R): rows such as "reports, in work, used 12 times, today". Delete: "Forgot Backup. 2 places left."
- Save view for this folder, in the View menu (Alt+V, then W): "View saved for work: sorted by size, largest first". When you enter that folder later: the folder's line, then "Sorted by size, largest first; saved for this folder."

### Compare files

- Compare files, from the palette: first "Comparing old.txt with report.txt", or "Comparing report.txt here with old.txt there" for one file in each panel. Then the report in one message: "5 changes between old.txt and report.txt. Changed 3, added 1, removed 1." and the direction, "From old.txt to report.txt." Then "Ignoring changes in spaces." when it applies.
- F7 and Shift+F7, or N and P, move between changes and say each row with its position: "Changed line 39, was: total due, now: total owed, 2 of 5". Enter on a row: "Change 1 of 5, line 12 before, none after", then "Before, line 12: Copyright 2025." S swaps: "Swapped. From report.txt to old.txt."
- Two files that are the same: "Same: report.txt and old.txt". The same file on both sides: "Same file: report.txt".
- A difference you cannot hear says its reason: "Changed line 3, hidden character U+200B added".

### The Keys list and presets

- Settings, then Keys list: "Keys list, every command and its key, ...". Enter: "Keys, preset diroxide defaults, 0 changed". Type `copy` to narrow the list: "Copy path, no key. 4 matches" or a row such as "Copy to other panel, F5 in both".
- Change key on a row: "Press the new key now. Escape cancels." Press a key: "Got Control plus F9", then "Ctrl+F9 saved for Sort by size." A key that is taken: "F5 already copies to the other panel. Take it, swap, or cancel?" with the focus on Cancel. Escape: "Nothing changed." The screen reader key: "Insert is the screen reader key."
- After 10 seconds with no key: "No key heard. The terminal may keep it." and then "Type the key's name, such as ctrl+f5, empty".
- Choose a preset, then Enter on one: "Apply Total Commander keys? No is the default. y or n". After y: "Total Commander keys applied." The defaults again: "The diroxide default keys applied."
- Test keys: "Got Control plus F5. Runs Sort by modified. Sent F5 with Control, a press."

### Watching a folder

- Watch this folder, from the palette: "Watching in for new files. Escape stops." (The folder here is called in.) A file copied in slowly: nothing while it grows, then about 2 seconds after it stops, "Arrived: scan-004.pdf, 1.2 MB". Several within ten seconds: "Watching in: 3 arrived". Escape: "Stopped watching in: 3 arrived".
- The status line shows "Waiting for scan-004.pdf to settle" but diroxide does not say it.
- In a shell, `diro watch in`: "Watching in for new files. Ctrl+C stops."

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
- A password prompt: "Password for course.zip (hidden):". A wrong one: "Wrong password for course.zip. Try again? No is the default. y or n".
- A hex row, eight bytes: `000010: 48 65 6c 6c 6f 20 77 6f Hello wo`, exactly 40 characters.
- The quick view's line: "Preview: report.md, 12 headings".
- A split plan: "12 parts of 20 MB, the last 3.1 MB". A gap: "Part 004 is missing. Nothing was joined."
- A filter problem: "Filter problem: size needs a number" (35 cells). The count after a filter: "37 matches, first report.pdf" (28).
- A search: "Searching: 214 found, 3,120 folders" (35), "Done: 214 found in 3,120 folders" (32), "Stopped at 214 found." (21). A result: "notes.txt, in Documents/q3, 4 KB" (32), "report.txt, line 42: total due" (30).
- A label: "Label urgent added to 3 items" (29), "Urgent, label, 14 items, 2 missing" (34). A bookmark: "Reports, alias r, in work" (25), "Bookmarked Reports, in work" (27); a bookmark row, "Reports, bookmark, in work, used 12 times", is 41 cells, with the kind and the place inside the first 26.
- Compare: "5 changes between old.txt and report.txt" (40), "Changed 3, added 1, removed 1." (30). A change row starts with the kind and the line: "Removed line 12, Copyright 2025" (31). A long change is cut to a window in the row.
- The Keys list: "Keys, preset Total Commander, 3 changed" (39), "Copy to other panel, F5 in both" (31), "Press the new key now. Escape cancels." (38), "Ctrl+F5 already sorts by modified time." (39).
- Watching: "Watching in for new files. Escape stops." (40), "Arrived: scan-004.pdf, 1.2 MB" (29), "Watching in: 3 arrived" (22).
- The Space list: "Total 66.4 GB. Largest 3 hold 81 percent" (40).
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

1. Open `course.zip` with Enter. You hear the title, "course.zip, archive, in" and the folder, then "Opened course.zip: 10 items, 1.8 KB." Press Down through the list: each row starts with the name, then "folder" or "text file", then a size, with the position last.
2. Press Left. The title starts with the demo folder's name and "left panel". Press Right on `course.zip` to enter it again. Press Enter on `week1`: "week1, 3 items, in course.zip".
3. Press F3 on `a.txt`: the title is "a.txt, in course.zip!week1". Press Escape: "Viewer closed: a.txt", and the cursor is back on the row.
4. Go back to the demo folder. Mark `week1` inside `course.zip` and press F5. You hear "Unpack 4 members into out? No is the default. y or n". Answer y: "Unpack finished: 4 done." Press Ctrl+Z: the files go to the trash.
5. Open `backup.7z`: "Opened backup.7z: 6 items". Open `logs.tar.xz`: "Opened logs.tar.xz: 3 items". There is no "Reading" line yet.
6. Open `locked.zip`. It lists at once. Press F3 on a member: "Password for locked.zip (hidden):". Type a wrong password: "Wrong password for locked.zip. Try again? No is the default. y or n". Answer y and type the right one. Is anything you typed spoken?
7. Open `hostile.zip`. It lists two items, without `../escape.txt`. Unpack it into `out` with Unpack from the File menu, Archive: "Unpack finished: 2 done, 1 skipped. Skipped ../escape.txt: name leaves the archive."
8. Mark two files and run Pack from the palette. You hear "Pack, Pack 2 items into" and the folder's name, "1 of 9". Press Enter on the first row: "Pack 2 items into" and the name, "? No is the default. y or n". Answer y: "Pack finished: 2 done. Checked 2 of 2 entries, all good." Run Test archive on the new zip: "Test started" and the name, then "Archive is sound: 2 members."
9. Put the right panel inside `course.zip`, mark a file in the left panel and press F5: "Archives are read only here. Use Pack to make a new one."
10. Press F3 on `report.md`. Press Ctrl+D: "Line 1 of ...". Press the slash key, type a word that occurs more than once and press Enter: "Match 1 of ...". Press N. Search for a word that is not there: "No match for".
11. Press F3 on `big.bin`: "big.bin looks binary; hex mode". Press B to hear one byte: "Byte 0 is" and a value. Is eight bytes a row comfortable by ear?
12. Press F3 on `main.rs`. Nothing about color is spoken. Press Right on a word, then K, Shift+K and O: the token, "Function name:", "Line 4:" and "Outline:".
13. Press Shift+F3 on `report.md`: "Quick view on", then one line, "Preview: report.md, 12 headings". Hold Down through five files. Each row is read, then you hear one "Preview:" or "No preview:" line when you stop, such as "No preview: locked.zip, archive, 197 bytes". Is it quiet?
14. Press Tab. You hear "Preview, report.md, Report, heading 1, 1 of 25". Press H: "Heading 2: Part 1, 2 of 12". Press T: "Table 1 of 1, 2 rows, 2 columns". Press Right: "Age, header, row 1 of 2, column 2". Press Shift+Down: "Age: 36, row 2 of 2, column 2".
15. Press L, then Enter on a web link: the question starts on No ("Open https://example.org in the browser? No is the default. y or n"). Answer n: "Not opened". Press slash, type `part` and press Enter: "Match 1 of 22 in the preview". Press N: "Match 2 of 22 in the preview". Press Escape: the panel, with the cursor on `report.md`, and nothing read twice.
16. Move to `notes.org`: "Preview: notes.org, Org, 2 headings". In the panel its row says "file". Edit `report.md` in another program and save: "Preview updated: report.md changed", once. Press Shift+F3: "Quick view off".
17. Put the cursor on `big.bin`, with the right panel on `out`. Run Split file from the File menu. You hear "Split into, Email attachment, 18 MB, Parts of 18,000,000 bytes, 1 of 9". Press Enter: the plan lists "4 parts of 18 MB, the last 6 MB" and "Enough space", and the question starts on No. Answer y: "Split started: 1 item to out", then "Split finished: 5 done." In `out` are the four parts, then `big.bin.crc`. Press F3 on the `.crc` file: three lines, "filename=big.bin", "size=60000000" and a "crc32=" line.
18. Rename `big.bin.003` and run Join files from `big.bin.001`: "Part 003 is missing. Nothing was joined." Rename it back. Join from `big.bin.002`: "Join 4 parts into big.bin, 60.0 MB? No is the default. y or n". Answer y: "Joined big.bin: checksum matches." and "Move the 4 parts to the trash? No is the default. y or n". Answer n: "The parts stay where they are."
19. Press Alt+Enter on `course.zip`: "Details of course.zip, Type: zip archive", then the size, then "Entries: 10 items".
20. Run Change permissions on `main.rs`: "Permissions, Change permissions of main.rs, 1 of 4".
21. Run Make symbolic link on `main.rs`: "Make link, Make a link to main.rs in out, 1 of 4". Answer y: "Link finished: 1 done." On Windows without Developer Mode: "Cannot link: needs developer mode."
22. Press Shift+F5 on `main.rs`: "Duplicate main.rs as: main (2).rs". Press Escape.
23. Press F9, then Enter for File, then A for Archive, then P for Pack: "Pack, Pack 1 item into" and a name. Press Escape. Is the last line of the screen the key bar, "F2 Rename  F3 View ..."?
24. In Settings, turn Single letters off: "Single letters: single keys do nothing". Press F3 on a text file, then H: nothing is said. Run Search in file from the palette: "Search for, empty". Type a word and press Enter: "Match 1 of ...". The viewer commands are also in the View menu's Viewer submenu.
25. Press Ctrl+Q. The program quits and the shell is where you left it.

Questions to answer: Was every line's meaning inside the first 40 cells? Did anything speak a password? Did the quick view chatter? Was eight bytes of hex right? Which command did you need the palette for?

## A check for search, labels, compare, keys and watching

Run this once with NVDA, once with JAWS, and then read the first 40 cells of each line on the Braille display. It uses the demo set from the check above (`cargo xtask demo` followed by a new or empty folder on your own drive). For the compare steps, copy `old.txt` and `report.txt` from `fixtures\diff\report` in the source folder into the demo folder. Use its empty folder, `out`, to watch. For a search with thousands of results, `cargo xtask search-tree` followed by a folder name and `--small` writes a tree. Steps 1 to 3 run in a shell. The others use the program. The words below are the ones the project's scripted run of this check expects; your reader may say them slightly differently.

1. In a shell, run `diro search` on the demo folder with `ext md`. You hear "Searching diro-demo for ext md", then "report.md, in diro-demo, 649 bytes", then "1 found." Run it again with `modified last week --explain`. You hear "Reads as: modified last week" and the real days. Run it with `size over`. You hear "Filter problem: size needs a number", then "Word 3. Try size over 10 MB.", and the exit code is 2.
2. Run `diro search` with `--content crow --extension md`. You hear a row such as "report.md, line 7: The crow count rose again this quarter., 4 more, in diro-demo". Is the line number easy to find in the first 40 cells?
3. Run `diro diff old.txt report.txt`. You hear "5 changes between old.txt and report.txt", the counts, "From old.txt to report.txt.", then five rows. Does the long line's window, "...in Portland opened early and the crows...", read well? Run it with `--format unified` and listen to "Same", "Added" and "Removed" rows and "9 same lines left out". Then run `diro watch out`, copy a file into `out` slowly, and listen for "Arrived:" about 2 seconds after the copy stops. Press Ctrl+C: "Stopped watching out: 1 arrived".
4. Start `diro` on the demo folder. The title starts with the folder's name.
5. Press Ctrl+F. You hear "Search, Name or filter: every entry, 2 of 11". The dialog is a list of labeled settings. Type `*.md` and press Enter. You hear "Searching diro-demo for *.md", then "Done:" and a count. Press Down: the row starts with the name, then "in" and the folder, and ends with its position.
6. Press Ctrl+F, Down to the "Contains text" row, type `crow` and press Enter. You hear "Searching diro-demo for *.md, containing crow" and a row "report.md, line 7: ...". Press F3. The viewer opens at that line: "At line 7 of". Press Escape: "Back in the results, report.md".
7. Start a search that matches thousands of files in the tree from `search-tree`, and press Escape while it runs. You hear "Stopped at" and a count, or "Done:" if the walk was faster than your key. The cursor must not have moved while results arrived. Press Escape again: "Results closed. Search results opens them again."
8. Press Ctrl+S and type `size over 1 KB and name *.log` slowly. Is it silent while you type? About a second after you stop, do you hear one line with a count, such as "1 match, big.log"? Does the title line read "filter: size over 1 KB and name matches *.log"?
9. Clear the field and type `size over`. You should hear "Filter problem: size needs a number" once, after the pause, and the second sentence after a second pause.
10. Clear the field and type `modified last week`. Run Say filter from the View menu (Alt+V, then Y). You hear the real days. On Saturday, October 17, 2026, with weeks that start on Sunday, it is "modified last week, October 4 to 10". Press Enter: "Filter on". Press Escape: "Filter off, N shown".
11. Press Plus and type `*.md`. You hear "Select by pattern: *", then after Enter "N entries will be selected. No is the default. y or n". Press y: "N selected".
12. Press Alt+L, type `urg` and press Enter: "Label urgent added to N items". Press Ctrl+Z: "Undid label urgent on N items". Label again. Open the Labels list from the palette. You hear "Labels, 6 labels, Urgent, label, N items". Is the row change from "checking" to "0 missing" noticed? Press Enter: "Looking for N items labeled urgent", then "Done: N items labeled urgent."
13. Press Ctrl+B. The list says "Add this folder". Press Enter: "Bookmarked diro-demo, in" and the drive. Press Ctrl+B again: the first row says "Remove this folder", and the new bookmark's row starts with its name and "bookmark".
14. Run Save this search from the Search menu (F9, then R, then S). Type a name, `Text files`, and press Enter: "Search saved as Text files. Saved searches runs it." Run Saved searches (F9, then R, then V): "Saved searches, 1 search, Text files, saved search, runs on open, 1 of 1". Press Enter: "Running Text files:" and the search.
15. Run Space list from the Search menu (F9, then R, then C). You hear "Total", a size and "Largest 3 hold" and a percent. Rows follow: "big.bin" with its size and percent. The last row begins "And".
16. Mark `old.txt` and `report.txt`. Run Compare files. You hear "Comparing old.txt with report.txt", then the verdict, the counts and the direction. Press F7 and Shift+F7, or N and P: each change is one row ending with its position, such as "2 of 5". Press Enter on one: "Change 1 of 5, line" and the lines before and after. Press Escape. Press S: "Swapped. From report.txt to old.txt." Compare a file with itself: "Same file: report.txt".
17. Open Settings and press K, then Enter on "Keys list, every command and its key". You hear "Keys, preset diroxide defaults, 0 changed". Type `copy` to narrow the list.
18. Choose Sort by size, then Enter, then Change key, then press Ctrl+F9: "Got Control plus F9", then "Ctrl+F9 saved for Sort by size." Repeat with F5: "F5 already copies to the other panel. Take it, swap, or cancel?" Press Escape: "Nothing changed."
19. Choose Change key and wait 10 seconds. You hear "No key heard. The terminal may keep it." and "Type the key's name, such as ctrl+f5".
20. Run Test keys. Press Ctrl+V and Ctrl+Enter. In Windows Terminal, Ctrl+V pastes text and does not arrive, and the Paste row of the Keys list says "check terminal". Tell the project which keys did not arrive.
21. In the Keys list, choose Choose a preset, then Total Commander: "Apply Total Commander keys? No is the default. y or n". Answer y: "Total Commander keys applied." Press Shift+F6 on a file: the rename prompt. Press Escape. Choose the diroxide defaults again: "The diroxide default keys applied."
22. In a shell, run `diro keys check`. You hear "Preset diroxide defaults: OK." and one line like it for each preset, then a line about `keymap.toml`.
23. In the panel on `out`, run Watch this folder from the palette: "Watching out for new files. Escape stops." Copy a file in slowly. Nothing while it grows, then "Arrived: name, size". Press Escape: "Stopped watching out: 1 arrived".

Questions to answer: Was the meaning of every line inside the first 40 cells? Did the count after the filter pause help, or interrupt? Is 400 milliseconds too short? Did "Waiting for ... to settle" get read aloud even though diroxide does not say it? Was the compare verdict and its counts enough before the change list? Does "check terminal" on every Shift plus function key row help or only add noise? Did the label's color help when someone sighted was nearby, and did the row read the same without it? Which keys does your terminal not deliver?

## Keys that may clash

- The screen reader key. NVDA, JAWS and Orca use Insert, or Caps Lock if you choose it, as their own key. diroxide uses neither, so your screen reader's commands pass through to it.
- Single-letter keys. Typing a letter finds a row by name. Screen readers do not use their browse mode in a terminal, so these letters reach diroxide. Dictation software types letters; turn single-letter keys off with the `single_letters` setting, and every command stays reachable by the keys that are not letters, by the menus and by the palette. The viewer has single-letter keys too, such as w, l, q, n and p; with single-letter keys off, every viewer command, search included, is still in the palette, and Escape closes the viewer.
- The keypad and Num Lock, described above. Some key presets use the keypad's plus, minus, star and slash for the selection. They work under NVDA and JAWS only with Num Lock on.
- Ctrl with a digit. The favorite labels are on Ctrl+1 to Ctrl+7, and many Unix terminals do not send them. The Edit menu's Labels submenu (Alt+E, then B) is the sure way.
- Ctrl+B for the bookmarks. Inside tmux it is tmux's own prefix key and arrives only when pressed twice. Alt+B and the Go menu (Alt+G, then K) open the same list.
- Shift with a function key. Some terminals send Shift plus a function key, Shift+F7 among them, as F13 to F24. The Keys list marks each of them "check terminal". Inside a compare, P and the actions list (Shift+F10) are the sure way back.
- The vim-style key preset runs letters as commands, so it turns type-ahead off. Dictation software types letters, so use Single letters, off, with dictation.
- Windows Terminal keeps some keys for itself. [Terminals](terminals.md) lists them and says how to give them back or avoid them.

## See also

- [The accessibility statement](accessibility.md)
- [Terminals](terminals.md)
- [The panels](panels.md)
- [Archives](archives.md)
- [The viewer](viewer.md)
- [The quick view](quick-view.md)
- [Split and join](split-join.md)
- [Search and the filter language](search.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Compare files](compare.md)
- [Customizing keys](customizing-keys.md)
- [Watching a folder](watching.md)
- [Troubleshooting](troubleshooting.md)
- [Settings and where the files live](settings.md)
- [The documentation index](README.md)
