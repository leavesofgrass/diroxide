# The GUI

This guide covers the GUI, the window version of diroxide: how to open it, what the window holds in the order a screen reader meets it, its keys and menus, its dialogs, what it says and when, and what it does not do yet. Everything the other guides say about what diroxide does, such as copying, searching or undoing, is the same in the window, because the window and the terminal are two views of one program. This guide says only what is different.

What this guide quotes is what the program says, taken from the program's code and tests. The tests read the window's accessibility tree the way a screen reader does, with no window on a screen, and a scripted walk of the window checks the lines it hears against the first 40 cells of a Braille line. None of it has been heard yet on NVDA, JAWS or a Braille display. If your experience differs, that is a finding.

The window in this alpha takes every key and says what happens. It shows the two panels, the status lines and the key bar. It has dialogs with real buttons, the viewer and the quick view, 24 themes, a way to work without a graphics card, and focus mode. [What the window does not do yet](#what-the-window-does-not-do-yet) names what is still missing, and it is worth reading before you rely on the window. On a desktop, typing `diro` with nothing else opens the window now, and `diro --tui` opens the terminal.

## Opening the GUI

On a desktop, type `diro`. The window opens. Folders work as they do for the terminal:

```bash
diro
```

```bash
diro LEFT RIGHT
```

`diro --gui` asks for the window outright. You need it only where automatic would choose the terminal, such as over SSH. `diro --tui` asks for the terminal.

### Which one opens

`diro` chooses between the window and the terminal at each start, in this order. The first rule that applies decides.

1. `--tui` opens the terminal, whatever else is asked.
2. `--gui` opens the window. If this `diro` was built without the window, or there is no display, one sentence says why and the terminal opens.
3. The `frontend` setting, if it is `gui`, is the same as `--gui`. If it is `terminal`, the terminal opens.
4. Otherwise the setting is `auto`, the default, which is the next section.

`--gui` is an explicit ask, so it is tried even over SSH and even when the output is piped or sent to a file. Where there is no display, the sentence below is said instead. Subcommands such as `diro ls`, `diro search` and `diro serve` never open the window.

### What automatic does today

Automatic opens the window on a desktop. It keeps the terminal in four cases:

- over SSH, when `SSH_CONNECTION` or `SSH_TTY` is set;
- with no display, which on Linux means neither `WAYLAND_DISPLAY` nor `DISPLAY` is set (Windows and macOS always have one);
- when standard input or output is a pipe or a file;
- in a build made without the window.

In those four cases the terminal opens with no sentence first. A sentence comes only when you asked for the window and it cannot open, or when the window starts and then fails, as the next sections show.

So `diro` typed in Windows Terminal, in a shell on a Linux or macOS desktop, or a double click on `diro.exe`, opens the window. This is a change from the earlier alphas, where a bare `diro` opened the terminal. To get the terminal, type `diro --tui` for one start, or set `frontend` to `terminal` and a bare `diro` opens the terminal from then on. The shell function that `diro init` prints, `dcd`, always asks for the terminal. [The settings reference](settings-reference.md#frontend) lists the values.

### The frontend setting

`frontend` is in the Look group of Settings (Alt+T, then S) under the name "Opens in". It holds `auto`, `gui` or `terminal`, and `auto` is the default. It is read when `diro` starts, so a change takes effect at the next start. `--gui` beats the value `terminal`, and `--tui` beats the value `gui`.

### When the window cannot open

A window that cannot open never ends the program with a crash. It says one sentence, with the cause first, and the terminal opens instead. There are three sentences to know.

The first two are for no display, and for a `diro` built without the window. They are said before the window is tried, and only when you asked for the window with `--gui` or the setting `gui`. Automatic chooses the terminal in those cases without a sentence.

```text
There is no display for the GUI (no WAYLAND_DISPLAY or DISPLAY); the terminal opens instead.
This diro was built without the GUI; the terminal opens instead.
```

Graphics that cannot start in any way, after the window is tried. A machine with no graphics card does not end here, because the window then draws on the processor. This sentence is for a machine where every way of drawing failed. The part in brackets is the reason and changes from machine to machine:

```text
Could not open the GUI: the window could not start its graphics (no compatible WGPU device found); the terminal opens instead. Try another graphics API with diro --graphics, such as software, dx12, vulkan or gl. The terminal, diro --tui, needs none.
```

Other failures use the same shape: "Could not open the GUI:", the cause, "the terminal opens instead.", and what to try. The causes are a window system that refused the window, an event loop that failed, and an internal error that was caught and said instead of ending the program.

### Graphics

The window draws with your graphics card when it has one, and on the processor when it has none. `--graphics NAME` chooses for one start, and the `graphics` setting ("GUI graphics" in Settings) keeps the choice. The names are `auto`, `vulkan`, `dx12`, `metal`, `gl` and `software`.

- `auto` lets the program choose how to draw. It is the default. The next section says how.
- `vulkan`, `dx12` (Direct3D 12, on Windows), `metal` (on macOS) and `gl` (OpenGL) name one interface for the graphics card.
- `software` is drawing on the processor, even where a graphics card is there. It needs no graphics driver.
- A name that is not on the list is a usage mistake. `diro` says "the graphics API glide is not known; use auto, vulkan, dx12, metal, gl or software" and exits with 2.
- If the `WGPU_BACKEND` variable is already set in your environment, it wins over `--graphics`.

### How the window draws

When it draws its first frame, the window first asks for a graphics adapter. Then it uses the first of these ways of drawing that works.

1. The graphics card, when the adapter is a real one.
2. The processor. The program draws each frame itself and hands it to the system's window, so no graphics driver is involved. The window draws this way when no adapter is found, when the only adapter is one that draws on the processor anyway (Windows' own software adapter, or llvmpipe or lavapipe on Linux), when the graphics card cannot draw, and when you ask for it with `--graphics software`.
3. The hybrid renderer on a software adapter, when drawing on the processor cannot start, for a display system it does not know.
4. The terminal, after the sentence above, when no way works.

The way of drawing changes how the window looks and nothing that a screen reader reads. The accessibility tree is built the same for every way. Drawing on the processor is slower than the graphics card, and the whole window is drawn again for each change.

When the window is not drawn by the graphics card, it says so once when it opens, politely, and the status line shows it until another message replaces it. The reason follows a colon, and there are three forms:

```text
Drawing on the processor: no graphics adapter was found.
Drawing on the processor, as the graphics setting asks.
Drawing on a software adapter: no graphics adapter was found, and the processor's presenter could not start.
```

Other reasons after the colon are "the only graphics adapter is a software one" with the adapter's name in brackets, and "the graphics card" with its name and "could not draw". The first words say how the window draws, so the line is clear in the first 40 cells. Nothing is said when the graphics card draws.

To keep a record, start with `--log FILE`:

```bash
diro --gui --log gui.log
```

The program appends lines that begin with "graphics:" to the file. They say how the window drew and why, the adapter found first, any way that failed before it, the graphics device the card made, and, when the window closes, how many frames were drawn and the average time of each. Use a plain file name: a name with a control character is refused. The log is for diagnosis, and a file that cannot be written is left alone.

### In the background

`diro --gui --background` opens the window off screen, never in front, with no taskbar button on Windows. It is for automated checks, which must not take the focus from the person at the machine. A window opened this way never goes full screen. You will not need it to use the window.

### On Windows: the console

`diro.exe` is a console program, so a shell waits for it, `diro ls | findstr` works, and `diro --tui` draws in the terminal it was started from.

- Started from Explorer, a shortcut or the Run box, Windows gives `diro` a console of its own. When the window opens, `diro` lets that console go, so its window closes. It can show for a moment first.
- Started from a shell, the shell's console is kept. The shell waits for the window, messages print there, and Ctrl+C there ends the window as it ends any program run in the foreground.
- If the window cannot open after the console was let go, a message box says why, titled "diroxide could not open its window", and waits for Enter or Escape. Then the terminal opens in a new console.

### Closing the window

Closing the window runs the same rule as Ctrl+Q: the close button does it, and so does Alt+F4 on Windows. With nothing running, the window closes. With a job running or waiting, diroxide asks first, "Quit and stop them? No is the default. y or n", and the window stays until you answer. The question has two buttons, No and Yes, and the focus starts on No. [The transfer queue](transfer-queue.md#quitting-with-jobs) says what happens to the jobs.

### Other options

`--portable`, `--no-plugins`, `--cwd-file FILE` and the two folders work in the window as in the terminal. `--theme NAME` and the `theme` setting choose the window's colors, as [the themes guide](themes.md) says. `--inline` belongs to the terminal and the window does not use it.

## The window in reading order

A screen reader meets the window's parts in this order. Each part has a name and a role, so you can tell where you are.

1. The window. Its title is the active panel's place first, then the program: "Documents - diroxide". With a plugin list up it names the list.
2. The title line, a label with the same words as the terminal's: the place first, then the panel, then the parent. "Documents, left panel, in D:\work". A filter, and a sort that is not your default, are added as they are in the terminal.
3. The left panel's Sort toolbar, named "Sort the left panel". It has three buttons, "Sort by name", "Sort by size" and "Sort by date". Each button's key is its keyboard shortcut, not part of its name: Ctrl+F3, Ctrl+F6 and Ctrl+F5.
4. The left panel's list, named "Left panel, Documents".
5. The right panel's Sort toolbar, "Sort the right panel", and its list, "Right panel, Backup".
6. The status bar, with two lines: the status line, then the jobs line. While a job runs, a progress indicator named "Job progress" comes after them.
7. The key bar, a toolbar named "Key bar".

Pressing a panel's sort button makes that panel the active one and sorts it.

A live region that you cannot see sits beside these parts. It carries what the window announces. [What is said, and when](#what-is-said-and-when) describes it.

Some screens replace parts of this order. A dialog covers the window, and [the dialogs](#the-dialogs) say how. The viewer takes the place of the two panels, and [the viewer and the quick view](#the-viewer-and-the-quick-view) say how.

On Windows the toolkit that opens the window also makes a helper window. It is named "diroxide background helper", so a screen reader's window list does not meet a pane with no name.

### The two lists

Each panel is one list, and each row is an option in it. The list is what the screen reader follows. The focused row is the list's active descendant, so the screen reader reads it when the focus moves, and the window adds no announcement of its own for a move.

- An option's name is the row as the terminal says it, with the name first: "report.pdf, PDF, 3 bytes, modified today". A folder says "folder" after its name: "Backup, folder, modified today" and then the time.
- The position is a property of the option, not part of its name, so a screen reader says "1 of 5" in its own way. The list carries the real number of rows, even though only the visible rows and two pages each way are built as options. A folder of 100,000 entries does not make 100,000 nodes.
- A marked row is selected. On Windows and Linux "selected" is the option's state, so it is said once. A row that is not marked has no selection state, so nothing says "not selected" on every row. On macOS, where VoiceOver reads a list's options as plain text, "selected" and "unavailable" stay in the name.
- An unavailable row is listed and says so.
- An empty folder is one label, "Empty folder", inside the list. While a folder is read it says "Reading folder".
- The header drawn above the other panel is for the eye. A screen reader gets the panel's name from its list.

### The status bar

The status bar holds two labels and, while a job runs, a progress indicator. The first label is the status line, which shows the last message or the folder and its counts. The second is the jobs line, "No jobs" or what runs. Neither is a live region, so a screen reader does not read them when they change. They are there to be read when you look at them. Messages that matter are announced another way, below, and Ctrl+D says the picture whenever you ask.

The progress indicator is named "Job progress". Its value is the percent of the job in front, such as "40 percent", and it is drawn as a bar with the percent in words beside it, so the amount is never shown by a length or a color alone. Its value changes only when the jobs line changes, and the program paces that line: at once when a job changes state, at most every ten seconds in screen-reader mode (`progress_interval_seconds`), and twice a second otherwise. The indicator is not a live region. Whether your screen reader speaks a progress bar's changes is its own setting. With no job running it is not in the tree, and a job that only waits its turn has no value.

### The key bar

The key bar is the terminal's key bar as buttons. Each is named by its label, "Rename", "View", "Edit", "Copy", "Move", "Folder", "Trash", "Menu" and "Quit", with its key as the shortcut. A button whose command has no key is left out. The pairs are the ones the terminal's key bar draws. Switch bank shows the next bank, and Say key bar reads the pairs in words. The `key_bar` setting turns the bar off.

The buttons do not take the keyboard focus. Tab is diroxide's, and it switches panels. A click on a button presses it, and so does a screen reader's Click on it.

## The keys

The window uses the same commands as the terminal and a keymap column of its own, the GUI column. [Every key](keyboard.md#in-the-gui) lists it, generated from the keymap. It differs from the terminal's column where a window can do more than a terminal can: F1 is help, Ctrl+Shift+P or Ctrl+P is the palette, F9 or F10 opens the menus, Ctrl+Comma is Settings, Ctrl+Shift+S is the Sort list, and Pack, Unpack and Test archive are Alt+F5, Alt+F9 and Alt+Shift+F9.

Every key goes to the program, including Tab, so the keys do what the other guides say. The Keys list is the same list. [Customizing keys](customizing-keys.md) says how to change a key. In the window, Change key takes a real key press, as [Settings and the Keys list](#settings-and-the-keys-list) says.

How the window reads a key:

- A printable key arrives as the character your layout typed, so a character that is not a letter does not carry a Shift.
- AltGr is text. A character typed with AltGr goes to a field, or to find by name, as typed.
- The keypad's plus, minus, star and slash are told apart from the main row's.
- A held key repeats only where a repeat means something: the arrows, Page Up, Page Down, Home and End, and in a field Backspace, Delete and typed characters. Any other repeat is dropped, so a held F8 cannot open a second question.
- On macOS the GUI uses Command where the key lists say Ctrl, and a chord with Option is read by the key's place on the keyboard.
- F11 and F12 reach diroxide. They are not the toolkit's debugging keys. F11 is the key of the Full screen command, which turns full screen on and off, as [focus mode and full screen](#focus-mode-and-full-screen) says.

### Keys that cannot arrive

Some keys never reach the window, or are dropped on purpose. No default in the GUI column uses one of them, and a test checks that. The list is in `gui-unsafe.toml` in the program's source, and it is:

- Alt+F4. Windows closes the window with it, and the quit rule runs from the close.
- Alt+Space. It opens the window's system menu.
- Alt+Tab, Alt+Shift+Tab and Ctrl+Alt+Tab. They switch between windows.
- Ctrl+Escape. It opens the Start menu.
- Ctrl+Alt+Delete. It is the system's secure attention key.
- Insert. It is your screen reader's key.
- Chords of the Windows key, such as the Windows key with E, D, L or R. They are the system's, and the window drops them on Windows and Linux.

With the native menu bar on, F9, F10, Alt pressed alone and Alt with a menu's letter go to the bar, not to the list menus. [The menus](#the-menus) says how.

## The menus

The window has the same nine menus as the terminal: File, Edit, View, Go, Search, Select, Tools, Plugins and Help. Each item is a command with its access key and its key. An item that the place does not allow is left out, as it is in the list menus.

On Windows the menus are the system's menu bar, which is the default (the `menus` setting, "GUI menus", is `native`). UI Automation shows the bar as a menu bar of menus and menu items. The access key is the letter marked in each name, and an item's key is its accelerator, so a screen reader can say it.

- The setting "Hide the menu bar" (`auto_hide_menu`) is on by default. With it on, the bar is hidden until you ask for it. Alt pressed alone shows it, and so do F9, F10 and Alt with a menu's letter, such as Alt+F for File. Search's letter is R. When the menu closes, the bar hides again, saying nothing. Turn the setting off and the bar stays shown.
- Alt with another key between is not Alt alone. Alt+Left goes back, as always.
- The Plugins menu in the bar holds Manage plugins and the commands of the plugins you turned on, as the list menus do. A plugin that is off leaves no commands in the bar.

The menu keys enter the bar only when no dialog is open and the key test is not running. Then the keys belong to the dialog or the test. A key that you bound to something else stays that command's.

On Linux, on macOS, and on any system when `menus` is `list`, the menus are lists in the window, as in the terminal. F9 opens "Menu bar", a list with File first, and Alt with a menu's letter opens that menu. macOS has no native bar yet, so its menus are the list.

## The layouts

The three layouts of [the panels](panels.md#the-two-panels-and-the-three-layouts) are all in the window, and the window follows a change at once. Change layout, in the View menu and the palette, goes to the next one.

- Side by side puts the panels next to each other. It is the default.
- Stacked puts one above the other.
- Single shows the active panel alone. The other panel is out of the window and out of the accessibility tree until Tab brings it.

In the terminal, side by side puts a row of each panel on one screen line, which is why stacked is recommended there. In the window each panel is its own list, so a screen reader reads one panel's row at a time in every layout. Which layout suits a screen reader best in the window has not been decided by listening.

## Plugin lists in a panel

A plugin's list of rows that are not files, such as the changed files of a git folder or the Sites list, is drawn in its panel. [The panels](panels.md#plugin-lists) says what it does. In the window:

- The panel's list is named for what it shows, "Left panel, Changed files". The title line and the window's title name the list too.
- The list is announced when it opens, with its meaning first and the plugin's name last: "Changed files, 2 rows, left panel (Changed files)".
- Up, Down, Space, Enter and the other keys work on the list's rows as they do in the terminal.
- A click on a row of a plugin list moves the focus to that row, and a double click presses Enter on the row you clicked.

## The palette

Ctrl+Shift+P or Ctrl+P opens the command palette. It is a dialog named "Command palette" whose field is an editable combo box. Type a few words and the candidates appear in a list under it, each starting with the command's name. Enter runs the chosen command. Escape closes it, and the focus returns to the row you were on.

## The viewer and the quick view

The viewer (F3) and the quick view (Shift+F3) are drawn in the window. Their keys are the ones [the viewer](viewer.md) and [the quick view](quick-view.md) list.

The viewer takes the place of the two panels. While it shows, the panels are out of the accessibility tree.

- The viewer is one list, named "Viewer, big.txt", and "Viewer, rows.txt, hex" in hex mode. Each row on screen is an option, named by its text, as the terminal says the line.
- The list carries the file's line count as its size, and each row carries its line's place, so your screen reader says the position in its own way. Only the rows on screen are built, so a file of 100,000 lines is a screen of nodes, fewer than 60, and not 100,000. The count grows while the file is counted, so End on a big file reaches the last line a moment after the key.
- Jump and Search ask in a prompt, as dialogs. Follow shows what is added to a growing file, at the progress interval in screen-reader mode.
- Code is colored as the `highlighting` setting says, which by default means colors off in screen-reader mode, as in the terminal. A screen reader hears the kinds of words through the commands that say them. Where code is colored, each class is drawn with the theme's role for it and with a shape as well (bold keywords, underlined links), so no class shows by color alone. Search matches are underlined.
- Escape closes the viewer. The panels come back, with the focus on the file's row.
- The viewer takes no mouse yet.

The quick view turns the other panel into a preview of the file under the cursor.

- The other panel's list is named for what it is: "Right panel, Preview, notes.md".
- The focus stays on the file's row. Tab goes into the preview, and Tab again comes back to the panel's row. The preview stays on while you are back in the panel.
- In the preview the quick view's own keys work: the next heading, the links list and Find. The links list is a dialog, Find is a field, and Escape returns to the preview.
- A click on a row of the preview goes into it, and Escape comes back.
- Shift+F3 turns the quick view off, and the panel lists a folder again.

## The dialogs

Every question, list, form and text the program shows is a dialog. The window draws seven shapes of dialog, and a test opens every purpose the program has, 77 of them, and checks that each has a name and that the keyboard focus is on a named control.

A dialog covers the window. While it is open, the window behind it is disabled and hidden from screen readers, so reading and the focus stay inside. Only the top dialog is in the accessibility tree. When it closes, the one beneath shows. When the last one closes, the focus goes back to the focused row of the panel. Escape walks back down the same way: from a list of values to the Settings list, and from the Settings list to the panel.

A dialog has a name, which is its question, its list's title or its field's label. Your screen reader says the name and the focused control as the focus moves in. The program does not say them again: its own line for the same words is left out of the live region, so the name is heard once. A line of the program's own that is not the dialog's name, such as "Searching work", is still said.

The shapes, in the window:

- A confirmation, such as "Copy 3 items to Backup, 312 bytes?", is two buttons.
  - The dialog's name is the question, ending "No is the default. y or n".
  - The buttons are "No" and "Yes", No first, and the focus starts on No. A button's key is its keyboard shortcut, not part of its name: N for No and Y for Yes.
  - Tab goes from No to Yes, to the listing when there is one, and back to No. Shift+Tab goes the other way. The arrow keys move between the two buttons.
  - Enter or Space presses the focused button. The keys y, n and Escape answer as they do in the terminal. A click presses a button, and so does a screen reader's Click. A screen reader's Focus moves the focus to the button and answers nothing.
  - Yes waits out the dialog settle time (`dialog_settle_ms`, 250 milliseconds by default). A key that comes inside it cannot answer, so a held key cannot say yes, and a held F5 does not open a second question.
  - When the question lists what it touches, the listing is a list to read, named with its count: "Listing, 3 lines". Its lines are the plan: what is copied and where, and each file. Up, Down, Page Up, Page Down, Home and End move through it. A screen reader meets the listing before the buttons in the tree, and Tab reaches the buttons first.
  - The answer closes the dialog, and the focus returns to the panel's row.
- A question with choices, such as a name that is taken, is named by its first line: "Name taken: report.pdf". The rest of what it says is read after the name, as the dialog's description, and drawn under it.
  - The choices are a list, and the focus starts on the safe one. For a name that is taken it is "Skip this one", and Escape answers with it, so the old file stays.
  - When the question has two lines or more after its first, they are a list to read, named "Message, 11 lines" for eleven. Tab moves between the choices and the message, and the focus starts on the choices. A new server key is one of these: the host, the key's type, and the fingerprint one part of four groups a line, "Part 1 of 4: 2134 141b 8436 8a94", so each part fits a Braille line.
- A list to choose from, such as a menu, Settings, the Jobs list or the search results, is one list named by its title. Its help is drawn above it as a line. Its keys are the program's, as in the terminal, and a letter jumps to the next row that starts with it. A click on a row moves the focus there, and a double click chooses it, as Enter does. A screen reader's Focus or Click on an option moves the focus there too.
- A text to read, such as help or details, is a list of lines. Up, Down, Page Up, Page Down, Home and End move through it, and Escape closes it.
- A field with a label, such as "Rename report.pdf to", is a text input. Its text is read as characters, and its caret is the text selection, so a screen reader reads each character as the caret moves over it. The caret starts before the file's extension, as in the terminal.
- A field with completion, such as the palette or Go to a path, is an editable combo box. Its candidates are a list under it, named "Candidates". Tab completes a path from the folder you are in.
- A password field is a password input. It holds only the mask, one asterisk for each key, or nothing when the `secret_echo` setting is `silent`. The password itself never enters the accessibility tree, the drawing or the live region. A test plants a password and checks each of them, and the program's debugging output too.

### Show the password

After a password field there is a check box, "Show the password", or "Show the passphrase" when the secret is a passphrase. Its key, Alt+S, is its keyboard shortcut. It is off unless the `secret_show_toggle` setting ("Passwords shown") starts password fields with the text shown.

- Tab goes from the field to the check box. Space or a click turns it on. Alt+S turns it on or off from the field.
- Turning it on says "Showing the password. Anyone who can see or hear your screen can read it." The field then holds the text as a plain text input, so a screen reader reads it as you type.
- Turning it off says "Hidden." The mask is back, and the text is gone from the tree.
- Typing while the focus is on the check box types into the field, and the focus goes back to it.

### Typing, paste and input methods

- Every field except a password field opens an input method session when it takes the focus, so what an input method commits, such as Chinese, Japanese or Korean text, is typed at the caret. A password field opens none.
- A paste into a field types the first line of the pasted text. The system's paste key does it.
- On a panel, a paste is refused in words, because its letters would run as keys: "Text paste ignored here. Paste files with Paste in the Edit menu."
- Text the program copies, such as Copy path and Copy name, goes to the system clipboard: "Name copied: notes.txt". On Windows, if another program holds the clipboard, you hear "Could not copy to the clipboard: another program is using it. Try again in a moment." The window goes on.

## Jobs in the window

A job in the window is the same job as in the terminal. These are the differences.

- The progress is shown, never said. The jobs line and the "Job progress" indicator change at the program's pace, and no percent reaches the live region. Ctrl+D says where the job is. A job's start line and its result are said once each: "Copy finished: 2 done, 1 skipped." A job that ends in the step it starts says only its result.
- Escape on a panel, with no dialog open, stops the job in front and says so.
- The Jobs list is a dialog (Alt+T, then J). A job that waits its turn offers Pause. A paused one says "Held: copy notes.txt to Backup", and its row reads "Held 1 of 1: copy notes.txt to Backup". [The transfer queue](transfer-queue.md) has the rows and the words.
- After a quit that stopped jobs, the next start says one sentence, and the Jobs list has an Interrupted group first. Enter on a job there offers Retry and Forget. Retry asks the usual question, with the focus on No, and Yes copies.

## Settings and the Keys list

Settings opens with Ctrl+Comma, or from the Tools menu (Alt+T, then S). It is a list of settings.

- A setting with choices opens its values as a list over the Settings list, and the current value is marked "current". Escape goes back to the Settings list with the focus on the setting you left, and Escape again goes back to the panel.
- A change is said, such as "Layout: stacked". A theme chosen is said with its contrast verdict, as [the themes guide](themes.md) says.

The Keys list is a field over a list of commands. Type to narrow the list. Enter on a command opens its actions.

- Change key waits for any key press. The prompt is a text to read, named "Press the new key now. Escape cancels." The window takes the key exactly as it comes, including a chord a terminal never sends, such as Ctrl+Shift+K. It says "Ctrl+Shift+K saved for Compare files." and writes the line to `keymap.toml`, so the next start has it.
- The key you press in the window goes in the GUI column. The terminal's column is changed by typing a key's name, with the action "Change the terminal key, type its name". [Customizing keys](customizing-keys.md) has the steps.

## Programs the window starts

The window has no terminal to hand over, so it starts programs in windows of their own and goes on. It waits for none of them.

- Shell here opens the system's terminal in the active folder.
- Run a command asks "Run in work", runs your line in a terminal, and leaves the terminal open when the command ends.
- F4 starts the editor on the file directly, with no terminal.

The program says only that something started, in its own words, with the folder or file last: "Started a shell in work", "Started the command in work", "Started the editor on notes.txt". It does not say when the program ends, and it does not read the panels again. The terminal's "Shell here: exit the shell to come back." and "Back in diroxide" are not said in the window.

When a program cannot start, the line says why and what to try: "Could not start a shell in work: no terminal program was found. Set the TERMINAL variable to a terminal program, such as xterm." For any other cause the next step is "Check the command, or the shell and editor settings."

How the terminal is found depends on the system.

- Windows: each program gets a console of its own, which is the system's terminal (Windows Terminal where it is the default). A program with a window of its own, such as Notepad, simply opens. A command line keeps its window open after the command.
- Linux: the terminal is the one in the `TERMINAL` variable, or else the first of `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal` and `xterm` that is found on the path. A command line keeps its shell open after it.
- macOS: Terminal opens in the folder for the shell, and runs a command line through `osascript`. This has not been tried by a person.

The folder and the environment are the ones the terminal's handover gives: `DIRO_ACTIVE` and the rest, as [operations](operations.md#open-edit-and-run) says.

## What is said, and when

The window has a live region, a part you cannot see that a screen reader reads when its content changes. The program puts a message there when it has something to say. Every message is a new node, so the same words twice are two announcements and the second is not lost. That matters when you press Right on a file twice and the program says "Not a folder" each time.

Each message has an importance, and it decides how it is said.

- Normal is polite. It is said when the screen reader is idle. "Selected report.pdf. 1 selected.", "Copy finished: 3 done." and the panel you switched to are normal.
- High is assertive. It interrupts. Errors, warnings and questions are high.
- Low is only shown. It goes to the status line and is not announced. The edges of a list, "Top of the list" and "Bottom of the list", "At the top already" at the top of a drive, and a running job's progress are low.

What this means for listening:

- A move of the focus says nothing of its own. The screen reader reads the focused row, because the row is the list's active descendant.
- Pressing Up at the first row, or Down at the last, is silent. The status line shows "Top of the list" or "Bottom of the list", and the screen reader does not read it.
- Tab says one thing, the panel you moved to: "Right panel: Backup, 1 item".
- The lines said in one step are one announcement, in their order. The same line twice is said once. A job's start line is left out when more came after it in the same step, such as the result of a quick job, so "Copy finished: 1 done" is not preceded by "Copy started: 1 file to Backup". The announcement is as urgent as its most urgent line.
- A dialog is said once, by itself, as [the dialogs](#the-dialogs) say.
- A running job's progress is shown in the jobs line and the "Job progress" indicator and is not spoken by the program. Ctrl+D says where the job is.
- A theme chosen is said once with its verdict. Drawing in a way other than the graphics card is said once. Full screen and focus mode say what they did in a few words, "Full screen on." and "Focus mode on: the bars fade after three seconds.", and the bars fading and coming back say nothing.
- The program's start-up lines are held until a screen reader asks for the window, then said once, as one announcement: the interrupted jobs of the last session, a problem with a settings file, or on a first start the welcome, the key bar and the layout. On the one run measured, they arrived about two seconds after launch.
- The first start's line about the key bar is "The key bar names the keys." In the terminal it is "The key bar on the last line names the keys."

Say status (Ctrl+D) and Say other panel (Alt+O) work as in the terminal.

## The mouse

The mouse has few jobs, because the keys do everything.

- A click on a row focuses the panel and the row. A double click on a focused row opens it, as Enter does. This holds for the folder's rows, for a plugin list's rows and for the quick view's rows.
- The wheel scrolls the rows.
- A click on a sort button or a key bar button presses it. In a dialog, a click presses a button and a double click chooses a row.
- A screen reader's Focus, Click or Scroll into view on a row moves the focus to that row, and its Click on a button presses it.
- There is no right-click menu. The right button acts like the main one. Shift+F10, the Menu key or Ctrl+Space opens the actions list, as in the terminal.
- There is no drag and drop.
- The viewer takes no mouse yet.

## The look

The window is drawn in one of 24 themes. The default is Galaxy, a dark page with light text and purple accents. [The themes guide](themes.md) lists them with what each is like for contrast, how to choose one, and what the window does for a theme that falls short.

- The focused row is a filled band in the active panel and an outline in the other. The panel with the keyboard focus has a ring, with a line just inside it, so the ring shows against a dark page and a light band alike.
- A marked row starts with an asterisk and says "selected". No state is carried by color alone: every role also keeps its bold, underline or italic, and the words say what a row is.
- Every pair of colors the window draws meets 4.5 to 1 contrast for text, or 7 to 1 in a high-contrast theme, and the focus marks meet 3 to 1. Where a theme's own color for a part falls short, the window draws that part in the text color. Tests check every theme.
- On Windows, High Contrast (Contrast themes) wins over every theme: while it is on, the window draws only with the system's own colors. Turning it on while the window is open is said: "System high contrast colors in use." When you turn it off, your theme comes back.
- With the `follow_system_appearance` setting on, the window takes the light or dark partner of your theme to match the system's setting. It reads that setting once, when it starts.
- The text is 16 pixels at your display's scale. There is no setting for the size or the font yet.
- The window has an icon, the diroxide mark, and it changes nothing a screen reader reads. On Windows, Explorer shows it for `diro.exe` and for a shortcut to it, and the taskbar and the title bar show it while the window is open. On Linux, the AppImage carries the mark in every size a desktop's icon theme asks for, and in two menu entries: "diroxide", which opens the window, and "diroxide in a terminal", which opens the terminal in a terminal of its own. A build from source has no menu entry of its own; the two entry files are in the source, under `packaging/appimage`. macOS shows no icon in this alpha.

The window carries its own fonts, Atkinson Hyperlegible Next and Atkinson Hyperlegible Mono, regular and bold, so nothing has to be installed. On Linux it also opens Fontconfig when it is there, to find the system's fonts. Without Fontconfig only the two Atkinson families draw, and a character they lack, such as a Chinese or Japanese name or an emoji, is drawn as a missing glyph. That affects only what is drawn. The names a screen reader reads are the full names. [Installing diroxide](install.md#what-the-gui-needs) has the rest.

## Focus mode and full screen

Focus mode gives the panels the room. After 3 seconds with no key and no mouse move, the title line, the status bar and the key bar fade under a veil. Any key, or a mouse move, brings them back. Full screen makes the window fill the screen, and while it does, the bars fade in the same way.

Each is a command, in the View menu and in the command palette, and each says what it did.

- Full screen is on F11, in the View menu as Full screen (Alt+V, then U) and in the palette. It says "Full screen on." or "Full screen off.". Full screen is a command like the others, so [customizing keys](customizing-keys.md) applies to its key.
- Focus mode is in the View menu (Alt+V, then M) and in the palette. It has no key. It turns the `focus_mode` setting ("Focus mode", in the Look group of Settings) on or off and writes it, and says "Focus mode on: the bars fade after three seconds." or "Focus mode off.". The switch in the Settings list says the same words. The setting is off by default.
- Both lines are polite, so a screen reader reads them when it is idle.
- With a dialog open, both commands do nothing and the status line shows "Close this dialog first."
- A window opened with `--background` never goes full screen.
- The veil is paint only. The accessibility tree is the same before and after, and so is the viewer's, and nothing is said when the bars fade or come back. A screen reader reads the window as it always does.
- The veil does not come down while a dialog is open.
- The panels' sort buttons and the native menu bar are not under the veil yet.

The same two commands are in the terminal's menus and palette, and they only explain themselves there. Full screen says "Full screen is the window's; the terminal program may have its own." and changes nothing, because full screen in a terminal is the terminal program's, often on F11, and that key never reaches diroxide. Focus mode turns the setting on for the window and says "Focus mode on, for the window; the terminal keeps its bars."

## What the window does not do yet

These are not in the window yet. None of them is dated: each is to come in a later alpha.

- An icon on macOS. The mark's smallest sizes, 16 to 24 pixels, have not been looked at by a sighted person.
- A native menu bar on macOS. Its menus are the list.
- Your own theme files in the window. It draws the 24 built-in themes, and Galaxy for a name it does not know, without saying that the name is unknown. The program finds the theme files in your `themes` folder by name, so Settings lists them, Next theme steps through them, the verdict is said, and the terminal draws them. [Themes and colors](themes.md#your-own-themes) has the details.
- A setting for the text size or the font.
- Mouse support in the viewer.
- Thumbnails, which the settings list as coming later.
- Low messages are shown and not announced, as described above. Nobody has yet listened to whether that is too quiet at the edges of a list.

[Known limits](known-limits.md#the-gui) repeats these as limits, with what has not been tried. [Using diroxide with a screen reader](screen-readers.md#the-gui) has a check to run with NVDA or JAWS.

## See also

- [Using diroxide with a screen reader](screen-readers.md)
- [Themes and colors](themes.md)
- [Installing diroxide](install.md)
- [The panels](panels.md)
- [Every key](keyboard.md)
- [Customizing keys](customizing-keys.md)
- [Settings and where the files live](settings.md)
- [The settings reference](settings-reference.md)
- [Plugins](plugins.md)
- [The transfer queue](transfer-queue.md)
- [Troubleshooting](troubleshooting.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
