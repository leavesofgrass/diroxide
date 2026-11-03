# The GUI

This guide covers the GUI, the window version of diroxide: how to open it, what the window holds in the order a screen reader meets it, its keys and menus, what it says and when, and what it does not do yet. Everything the other guides say about what diroxide does, such as copying, searching or undoing, is the same in the window, because the window and the terminal are two views of one program. This guide says only what is different.

What this guide quotes is what the program says, taken from the program's code and tests. The tests read the window's accessibility tree the way a screen reader does, with no window on a screen, and a scripted walk of the window checks the lines it hears against the first 40 cells of a Braille line. None of it has been heard yet on NVDA, JAWS or a Braille display. If your experience differs, that is a finding.

The window in this alpha is the foundation of the GUI. It shows the two panels, the status lines and the key bar, takes every key, and says what happens. Several parts are plain stand-ins that a later step replaces. [What the window does not do yet](#what-the-window-does-not-do-yet) names them, and it is worth reading before you rely on the window. The terminal is still what `diro` opens when you type it with nothing else.

## Opening the GUI

Type `diro --gui`. Folders work as they do for the terminal:

```bash
diro --gui
```

```bash
diro --gui LEFT RIGHT
```

### Which one opens

`diro` chooses between the window and the terminal at each start, in this order. The first rule that applies decides.

1. `--tui` opens the terminal, whatever else is asked.
2. `--gui` opens the window. If this `diro` was built without the window, or there is no display, one sentence says why and the terminal opens.
3. The `frontend` setting, if it is `gui`, is the same as `--gui`. If it is `terminal`, the terminal opens.
4. Otherwise the setting is `auto`, the default, which is the next section.

`--gui` is an explicit ask, so it is tried even over SSH and even when the output is piped or sent to a file. Where there is no display, the sentence below is said instead. Subcommands such as `diro ls`, `diro search` and `diro serve` never open the window.

### What automatic does today

Today automatic opens the terminal, every time. The window is not the default yet. The last step of this alpha is to flip the default, and this guide will change when it does.

The program's decision already holds the cases for that day. When the default flips, automatic opens the window on a desktop and keeps the terminal in four cases:

- over SSH, when `SSH_CONNECTION` or `SSH_TTY` is set;
- with no display, which on Linux means neither `WAYLAND_DISPLAY` nor `DISPLAY` is set (Windows and macOS always have one);
- when standard input or output is a pipe or a file;
- in a build made without the window.

The help text of the `frontend` setting already describes that future. Until the default flips, `diro --gui`, or the setting `frontend` set to `gui`, is how you get the window. [The settings reference](settings-reference.md#frontend) lists the values.

### The frontend setting

`frontend` is in the Look group of Settings (Alt+T, then S) under the name "Opens in". It holds `auto`, `gui` or `terminal`. It is read when `diro` starts, so a change takes effect at the next start.

### When the window cannot open

A window that cannot open never ends the program with a crash. It says one sentence, with the cause first, and the terminal opens instead. There are three sentences to know.

No display, or a `diro` built without the window, before the window is tried:

```text
There is no display for the GUI (no WAYLAND_DISPLAY or DISPLAY); the terminal opens instead.
This diro was built without the GUI; the terminal opens instead.
```

No graphics driver, or one that fails, after the window is tried. The part in brackets is the driver's own reason and changes from machine to machine:

```text
Could not open the GUI: the window could not start its graphics (no compatible WGPU device found); the terminal opens instead. Try another graphics API with diro --graphics, such as dx12, vulkan or gl. The terminal, diro --tui, needs none.
```

Other failures use the same shape: "Could not open the GUI:", the cause, "the terminal opens instead.", and what to try. The causes are a window system that refused the window, an event loop that failed, and an internal error that was caught and said instead of ending the program.

### Graphics

The window draws with your graphics driver. `--graphics NAME` chooses the programming interface for one start, and the `graphics` setting ("GUI graphics" in Settings) keeps the choice. The names are `auto`, `vulkan`, `dx12`, `metal`, `gl` and `software`.

- `auto` lets the graphics driver choose. It is the default.
- `vulkan`, `dx12` (Direct3D 12, on Windows), `metal` (on macOS) and `gl` (OpenGL) name one interface.
- `software` is drawing on the processor, for a machine with no graphics driver. It is not built yet. `--graphics software`, and the setting, say so in one sentence, and the terminal opens: "Could not open the GUI: drawing on the processor comes later in this alpha; the terminal opens instead. Set graphics to auto, or use the terminal with diro --tui."
- A name that is not on the list is a usage mistake. `diro` says "the graphics API glide is not known; use auto, vulkan, dx12, metal, gl or software" and exits with 2.
- If the `WGPU_BACKEND` variable is already set in your environment, it wins over `--graphics`.

### In the background

`diro --gui --background` opens the window off screen, never in front, with no taskbar button on Windows. It is for automated checks, which must not take the focus from the person at the machine. You will not need it to use the window.

### On Windows: the console

`diro.exe` is a console program, so a shell waits for it, `diro ls | findstr` works, and `diro --tui` draws in the terminal it was started from.

- Started from Explorer, a shortcut or the Run box, Windows gives `diro` a console of its own. When the window opens, `diro` lets that console go, so its window closes. It can show for a moment first.
- Started from a shell, the shell's console is kept. The shell waits for the window, messages print there, and Ctrl+C there ends the window as it ends any program run in the foreground.
- If the window cannot open after the console was let go, a message box says why, titled "diroxide could not open its window", and waits for Enter or Escape. Then the terminal opens in a new console.

### Closing the window

Closing the window runs the same rule as Ctrl+Q: the close button does it, and so does Alt+F4 on Windows. With nothing running, the window closes. With a job running or waiting, diroxide asks first, "Quit and stop them? No is the default. y or n", and the window stays until you answer. [The transfer queue](transfer-queue.md#quitting-with-jobs) says what happens to the jobs.

### Other options

`--portable`, `--no-plugins`, `--cwd-file FILE` and the two folders work in the window as in the terminal. `--inline` belongs to the terminal and the window does not use it. `--theme NAME` and the `theme` setting do not change the window yet: it has one fixed set of colors, described under [the look](#the-look).

## The window in reading order

A screen reader meets the window's parts in this order. Each part has a name and a role, so you can tell where you are.

1. The window. Its title is the active panel's place first, then the program: "Documents - diroxide". With a plugin list up it names the list.
2. The title line, a label with the same words as the terminal's: the place first, then the panel, then the parent. "Documents, left panel, in D:\work". A filter, and a sort that is not your default, are added as they are in the terminal.
3. The left panel's Sort toolbar, named "Sort the left panel". It has three buttons, "Sort by name", "Sort by size" and "Sort by date". Each button's key is its keyboard shortcut, not part of its name: Ctrl+F3, Ctrl+F6 and Ctrl+F5.
4. The left panel's list, named "Left panel, Documents".
5. The right panel's Sort toolbar, "Sort the right panel", and its list, "Right panel, Backup".
6. The status bar, with two lines: the status line, then the jobs line.
7. The key bar, a toolbar named "Key bar".

Pressing a panel's sort button makes that panel the active one and sorts it.

A live region that you cannot see sits beside these parts. It carries what the window announces. [What is said, and when](#what-is-said-and-when) describes it.

### The two lists

Each panel is one list, and each row is an option in it. The list is what the screen reader follows. The focused row is the list's active descendant, so the screen reader reads it when the focus moves, and the window adds no announcement of its own for a move.

- An option's name is the row as the terminal says it, with the name first: "report.pdf, PDF, 3 bytes, modified today". A folder says "folder" after its name: "Backup, folder, modified today" and then the time.
- The position is a property of the option, not part of its name, so a screen reader says "1 of 5" in its own way. The list carries the real number of rows, even though only the visible rows and two pages each way are built as options. A folder of 100,000 entries does not make 100,000 nodes.
- A marked row is selected. On Windows and Linux "selected" is the option's state, so it is said once. A row that is not marked has no selection state, so nothing says "not selected" on every row. On macOS, where VoiceOver reads a list's options as plain text, "selected" and "unavailable" stay in the name.
- An unavailable row is listed and says so.
- An empty folder is one label, "Empty folder", inside the list. While a folder is read it says "Reading folder".
- The header drawn above the other panel is for the eye. A screen reader gets the panel's name from its list.

### The status bar

The status bar holds two labels. The first is the status line, which shows the last message or the folder and its counts. The second is the jobs line, "No jobs" or what runs. Neither is a live region, so a screen reader does not read them when they change. They are there to be read when you look at them. Messages that matter are announced another way, below, and Ctrl+D says the picture whenever you ask.

### The key bar

The key bar is the terminal's key bar as buttons. Each is named by its label, "Rename", "View", "Edit", "Copy", "Move", "Folder", "Trash", "Menu" and "Quit", with its key as the shortcut. A button whose command has no key is left out. The pairs are the ones the terminal's key bar draws. Switch bank shows the next bank, and Say key bar reads the pairs in words. The `key_bar` setting turns the bar off.

The buttons do not take the keyboard focus. Tab is diroxide's, and it switches panels. A click on a button presses it, and so does a screen reader's Click on it.

## The keys

The window uses the same commands as the terminal and a keymap column of its own, the GUI column. [Every key](keyboard.md#in-the-gui) lists it, generated from the keymap. It differs from the terminal's column where a window can do more than a terminal can: F1 is help, Ctrl+Shift+P or Ctrl+P is the palette, F9 or F10 opens the menus, Ctrl+Comma is Settings, Ctrl+Shift+S is the Sort list, and Pack, Unpack and Test archive are Alt+F5, Alt+F9 and Alt+Shift+F9.

Every key goes to the program, including Tab, so the keys do what the other guides say. The Keys list is the same list. [Customizing keys](customizing-keys.md) says that the GUI column is changed by typing the key's name. Capturing a GUI key from a real key press in the window is for a later step.

How the window reads a key:

- A printable key arrives as the character your layout typed, so a character that is not a letter does not carry a Shift.
- AltGr is text. A character typed with AltGr goes to a field, or to find by name, as typed.
- The keypad's plus, minus, star and slash are told apart from the main row's.
- A held key repeats only where a repeat means something: the arrows, Page Up, Page Down, Home and End, and in a field Backspace, Delete and typed characters. Any other repeat is dropped, so a held F8 cannot open a second question.
- On macOS the GUI uses Command where the key lists say Ctrl, and a chord with Option is read by the key's place on the keyboard.
- F11 and F12 reach diroxide. They are not the toolkit's debugging keys.

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
- A plugin's commands are not in the native bar yet. The command palette and any key you gave them still run them.

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
- A click on a row of a plugin list does not move the focus to it yet, and a double click presses Enter on the row that has the keyboard focus, not the row you clicked. Use the keys in a plugin list.

## The palette

Ctrl+Shift+P or Ctrl+P opens the command palette. It is a dialog named "Command palette" whose field is an editable combo box. Type a few words and the candidates appear in a list under it, each starting with the command's name. Enter runs the chosen command. Escape closes it, and the focus returns to the row you were on.

## The dialogs, as they are now

Every question, list, form and text the program shows is a dialog. The window's dialogs are a first version, and this section says plainly what they are.

A dialog covers the window. While it is open, the window behind it is disabled and hidden from screen readers, so reading and the focus stay inside. Only the top dialog is shown. It has a name, which is its question, its list's title or its field's label. It may have lines to read first, such as the listing under a confirmation. Then it has one control, a text field or a list, and the focus is on it. Escape closes the dialog, and the focus goes back to the focused row of the panel.

The shapes, in the window:

- A confirmation, such as "Copy 3 items to Backup, 312 bytes?", is a stand-in. It is a list of two answers, "No, the default" and then "Yes", with the focus on the first. The keys y, n, Enter and Escape answer it as in the terminal. The next step replaces this list with real buttons, and this guide will change then.
- A question with choices, such as a name that is taken, and a list to choose from, such as a menu, Settings, the Jobs list or the search results, are lists. Their keys are the program's, as in the terminal.
- A text to read, such as help or details, is a list of lines.
- A field with a label, such as "Rename report.pdf to", is a text input. Its text is read as characters, and its caret is the text selection, so a screen reader reads each character as the caret moves over it. The caret starts before the file's extension, as in the terminal.
- A field with completion, such as the palette or Go to a path, is an editable combo box. Its candidates are a list under it.
- A password field is a password input. It holds only the mask, one asterisk for each key, or nothing when the `secret_echo` setting is `silent`. The password itself never enters the accessibility tree, and a test plants a password and checks that no name or value holds it. There is no Show button yet.

Some things the dialogs do not do yet:

- The lists in a dialog take no clicks. Use the keys.
- A field opens no input method session, so typing Chinese, Japanese or Korean through an input method is not supported yet.
- Text that the program copies, such as a file's path, stays inside the program and is not put on the system clipboard yet.
- When a dialog opens, its name may be said twice: once by the program's announcement and once by the dialog itself. Nobody has listened to this yet.

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
- A running job's progress is shown in the jobs line and is not spoken. Ctrl+D says where the job is. A progress node that speaks at the progress interval belongs to a later step.
- The program's start-up lines are held until a screen reader asks for the window, then said once, as one announcement: the interrupted jobs of the last session, a problem with a settings file, or on a first start the welcome, the key bar and the layout. On the one run measured, they arrived about two seconds after launch.
- The first start's line "The key bar on the last line names the keys." is the terminal's wording. In the window the key bar is in the Key bar toolbar.

Say status (Ctrl+D) and Say other panel (Alt+O) work as in the terminal.

## The mouse

The mouse has few jobs, because the keys do everything.

- A click on a row focuses the panel and the row. A double click on a focused row opens it, as Enter does.
- The wheel scrolls the rows.
- A click on a sort button or a key bar button presses it.
- A screen reader's Focus, Click or Scroll into view on a row moves the focus to that row, and its Click on a button presses it.
- There is no right-click menu. The right button acts like the main one. Shift+F10, the Menu key or Ctrl+Space opens the actions list, as in the terminal.
- There is no drag and drop.

## The look

The window has one fixed set of colors until themes come. It is a dark page with light text.

- The focused row is a filled blue bar in the active panel and an outline in the other. The panel with the keyboard focus has a ring.
- A marked row starts with an asterisk and says "selected", and its text is orange. Blue and orange stay apart for red-green color blindness. No state is carried by color alone.
- Text pairs meet 4.5 to 1 contrast, and the focus marks meet 3 to 1. Tests check it.
- The text is 16 pixels at your display's scale. There is no setting for the size or the font yet.

The window carries its own fonts, Atkinson Hyperlegible Next and Atkinson Hyperlegible Mono, regular and bold, so nothing has to be installed. On Linux it also opens Fontconfig when it is there, to find the system's fonts. Without Fontconfig only the two Atkinson families draw, and a character they lack, such as a Chinese or Japanese name or an emoji, is drawn as a missing glyph. That affects only what is drawn. The names a screen reader reads are the full names. [Installing diroxide](install.md#what-the-gui-needs) has the rest.

## What the window does not do yet

These are not in the window yet. None of them is dated: each is to come in a later step of this alpha or a later alpha.

- The viewer (F3) and the quick view (Shift+F3). Their keys reach the program, but the window does not draw what they show. Use the terminal for them.
- Dialogs with buttons. A confirmation is a list of two answers, and a password field has no Show button.
- Spoken progress for a running job.
- Clicks in a dialog's list, and a click on a plugin list's row.
- Themes. The colors are fixed, and the `theme` setting does nothing in the window.
- Drawing on the processor, for a machine with no graphics driver. Without a driver the window cannot open, and the terminal opens instead.
- Icons.
- Input methods, and putting the program's copied text on the system clipboard.
- A native menu bar on macOS, and plugin commands in the native bar on Windows.
- A name for winit's helper window. The toolkit that opens the window also makes a helper window, whose class is "Winit Thread Event Target". It has no title. It shows as a pane with no name in the desktop's window list, so a screen reader's window list can meet it.
- Thumbnails and focus mode, which the settings list as coming later.
- Low messages are shown and not announced, as described above. Nobody has yet listened to whether that is too quiet at the edges of a list.

[Known limits](known-limits.md#the-gui) repeats these as limits. [Using diroxide with a screen reader](screen-readers.md#the-gui) has a check to run with NVDA or JAWS.

## See also

- [Using diroxide with a screen reader](screen-readers.md)
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
