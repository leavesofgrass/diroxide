# Using diroxide with a screen reader

This guide is for people who use a screen reader such as NVDA, JAWS, Orca or VoiceOver, with or without a Braille display. It says how diroxide works with a screen reader, what you should hear for each key, the settings to check in NVDA and JAWS, how to read it on a 40-cell display, and a short check you can run yourself.

What this guide says about diroxide comes from its design. A setting that comes from a screen reader's own documentation, or that has not been tried here, is marked not yet verified. Nothing in alpha.1 had been heard on a screen reader when this guide was written, so every "you should hear" line below is what diroxide is built to say, not a report of what was heard. If your experience differs, that is a finding. [The accessibility statement](accessibility.md) says what has been checked.

## How diroxide works with your screen reader

diroxide has no voice of its own. Everything it would say is text on the screen, and your screen reader speaks it. It gives your screen reader two signals.

- The hardware cursor. It is always on the first cell of the focused row, so a screen reader or a Braille display that follows the cursor follows the row. In a prompt it sits where the next character will go. In an empty folder it sits on the line that says "Empty folder". Screen readers follow the cursor, not the highlight, which is why diroxide does not rely on a highlight.
- The status line. Messages, moves and questions go to one line that a terminal screen reader speaks when it changes. The same message twice in a row would be silent, so diroxide blanks the line for 150 milliseconds before a repeated message, and you hear it again.

Other things you can rely on:

- Meaning comes first. Every row, label and message puts its meaning in the first 40 characters, which is what a 40-cell Braille display shows.
- Words come before symbols. Status is words such as "selected", "Pass" and "Fail", never a color or a symbol alone. A selected row also starts with an asterisk, but the word is there too.
- There are no spinners. Progress is a sentence that changes slowly.
- The default look draws no boxes. Box drawing often comes out as noise on a Braille display.
- diroxide never uses Insert or Caps Lock, which NVDA, JAWS and Orca use as their own key. Its default keys use no Control plus Alt chords, which VoiceOver uses.
- Control and invisible characters in a file name are shown as a question mark, so a hostile name cannot send commands to your terminal or confuse your screen reader.

### Screen-reader mode

The `screen_reader_mode` setting turns on the behavior a screen reader needs. diroxide does not try to detect your screen reader, so check this setting yourself in Settings. In screen-reader mode:

- the jobs line changes at once when a job changes state, and otherwise at most every ten seconds (`progress_interval_seconds`). Outside screen-reader mode it can change twice a second;
- the other panel is redrawn only when you ask, so a change there does not interrupt you;
- the key bar is hidden and outside the order you move through;
- while you type in a prompt, nothing is announced until you press Tab, which says how many matches there are;
- highlights carry an attribute and a text marker, never only color.

## What you should hear

These lines show what diroxide says for each key. The status line says the same words that a menu or the palette shows.

- Starting: the title line first, "dirocheck, left panel, in D:\", and then the first row. You do not hear the whole screen. The status line names the version and the layout.
- Down and Up in the stacked or single layout: the cursor moves one row and your screen reader speaks the row: "Photos, folder, modified Tuesday". No message is written for a focus move.
- Down in the side by side layout with the left panel active: your screen reader's current line is the whole screen line, which also holds the right panel's row, so it may read the left row and then the right. With the right panel active, the cursor sits near column 41 and a 40-cell display shows the left row first. This is why stacked is recommended.
- Tab: one message and nothing else, "Right panel: dirocheck, 31 items". Chatter on Tab is the first thing to report.
- Space: "Selected report.pdf. 1 selected." and then the next row is read. The row you moved onto says "selected" if it is marked.
- Ctrl+D (Say Status): "3 selected, 4.2 MB. dirocheck, 31 items". With no marks: "dirocheck, 31 items, no selection", then the focus: "on report.pdf, 3 of 31".
- Alt+O (Say Other Panel): "Other panel: Backup, 12 items", then its focused row.
- Ctrl+F6 (sort by size): "Sorted by size, largest first". Again: "Sorted by size, smallest first".
- F5 (copy): "Copy 3 files to Backup? y or n". The question starts on No.
- A name taken: a list opens, "Name taken: report.pdf", "New is newer and larger", "In Backup, other panel, file 1 of 3", the two fact lines, then "Skip this one, 1 of 9" with the focus on it.
- A copy running: "Copy started: 1 file to Backup", then "Copying 30 percent, 2 minutes left" at most every ten seconds, and once at the end, "Copy finished: 1 done".
- F2 (rename): "Rename report.pdf to:" with the name in the field. A name that needs care: "Name needs care: ends with a space".
- F8 (trash): "Move report-2026.pdf to the trash? y or n". Ctrl+Z: "Undid trash: 1 item is back".
- Ctrl+Q with a job running: "Quit? 1 job is running. y or n".

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
- A row: the name, the kind, the size. "report.pdf, PDF, 2.1 MB, modified Tuesday". A selected row: "* report.pdf, selected, PDF, 2.1 MB,".
- After a move or a mark: "Selected report.pdf. 1 selected." and "At the top already".
- Say Status: "3 selected, 4.2 MB. Documents, 40 items".
- A sort: "Sorted by size, largest first".
- A question: the action, the count and the target before the answer keys. "Move 3 files to Backup? y or n".
- Progress: "Copying 30 percent, 2 minutes left". The percent and the estimate sit inside the first 40 cells.

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

Run this once with NVDA and once with JAWS, in Windows Terminal and, if you like, the classic console. If you build from source, `cargo xtask sample-folder` writes a folder to try it in, with a conflicting file, a large file and names with accents, Japanese and an emoji.

1. Start `diro` on that folder. You should hear the title and the first row, not the whole screen.
2. Press Down five times. Each row is spoken once, name first. The display shows the row from its first cell.
3. Press Tab. You hear one message. Press Tab again.
4. Press Space on three files. You hear "Selected report.pdf. 1 selected." each time. Is the word "selected" heard on the row, or only the asterisk?
5. Press Ctrl+D. You hear the count and the size of the selection.
6. Press Ctrl+F6, then Ctrl+F6 again, then Ctrl+F3. You hear each sort result. If a key does nothing, open the palette, type "sort" and choose the entry. That key is worth reporting.
7. Copy a file onto one with the same name. Press F5, press y, and walk the conflict list.
8. Copy a large file. How often does the progress speak? Press Ctrl+D for the sentence on demand. Press Escape partway and check that no half-written file remains.
9. Press Ctrl+P, type "layout", press Enter, and then Tab and Down. Is stacked quieter? Choose the layout you will keep.
10. Open the folder with the accented, Japanese and emoji names. Each should read cleanly, one row each.

Five things to report: whether Tab chattered, whether "selected" was heard, how often progress spoke, which keys did not arrive (`diro keys test` lists them), and which layout you kept.

## Keys that may clash

- The screen reader key. NVDA, JAWS and Orca use Insert, or Caps Lock if you choose it, as their own key. diroxide uses neither, so your screen reader's commands pass through to it.
- Single-letter keys. Typing a letter finds a row by name. Screen readers do not use their browse mode in a terminal, so these letters reach diroxide. Dictation software types letters; turn single-letter keys off with the `single_letters` setting, and every command stays reachable by the keys that are not letters, by the menus and by the palette.
- The keypad and Num Lock, described above.
- Windows Terminal keeps some keys for itself. [Terminals](terminals.md) lists them and says how to give them back or avoid them.

## See also

- [The accessibility statement](accessibility.md)
- [Terminals](terminals.md)
- [The panels](panels.md)
- [Troubleshooting](troubleshooting.md)
- [Settings and where the files live](settings.md)
- [The documentation index](README.md)
