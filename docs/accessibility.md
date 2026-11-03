# Accessibility statement

This page says what diroxide promises about accessibility, how each promise is checked, what has been checked so far, and what has not. It was written on Friday, October 16, 2026, and revised on Saturday, October 17, 2026, to cover everything in alpha 1. Nothing in alpha 1 had been heard on a screen reader at either date, because the listening tests come after the build. The page is updated after each listening check.

## Who diroxide is built for

diroxide is built first for people who use a screen reader or a Braille display, and for people who are red-green color blind. Accessibility is the product, not a feature added later. When looks, speed or convenience conflict with accessibility, accessibility wins. If an accessible way cannot be found for something, the report for that release says so, and nothing inaccessible ships quietly.

## The promises

- A row, a label and a message put their meaning in their first 40 characters, because a 40-cell Braille display shows one line of that length at a time.
- Status is words, such as "selected", "Pass" and "Fail". It is never a color or a symbol alone. A symbol can be an extra, never the only signal, because symbols often come out as noise on a Braille display.
- Color never carries meaning alone, above all red against green. Every highlight also has an attribute (bold, underline or reverse) or a text marker, such as the word "selected" on a marked row.
- Everything works from the keyboard. The terminal frontend has no mouse. Every command can be reached by name from the command palette and by the menus, so a key that a terminal keeps for itself never costs you a command.
- Single-letter keys can be turned off, so dictation software and speech recognition do not trigger commands (WCAG success criterion 2.1.4).
- The hardware cursor is always on the focused row, in every state: a panel, a dialog, a menu, the palette, a prompt and an empty folder. A screen reader or a Braille display that follows the cursor follows the row.
- Every change you need to know about can be announced, once. A repeated message is heard again. Progress changes slowly in screen-reader mode, and Ctrl+D gives it on demand.
- There are no spinners, and the default look draws no boxes.
- A question lists what it will touch and starts on No. Hidden characters in a name are spelled out.
- The documentation is plain Markdown with real headings and lists, no tables, and a "See also" list at the end of every guide.

## How the promises are checked

Automated tests run with every change.

- A walk through a session, the Braille walk test, checks that the lines diroxide writes carry their key fact within 40 cells. The message catalogs are tested the same way. A catalog of every message, generated with a build failure on a line whose meaning starts after cell 40, is not built yet.
- A reachability test checks that every command has a terminal key or is palette-only, and that the palette reaches every command.
- A cursor test reads the cursor's position from a test terminal in every state.
- A test sends a key press and its release and expects one action.
- Tests draw names with accents, emoji, Chinese, Japanese and Korean characters, and names with escape sequences in them.
- A check compares the keymap with the keys NVDA, JAWS and Orca use, and with the keys terminals cannot send.
- A documentation check looks for broken links, tables and guides without a "See also".

Listening checks are done by hand, because no test hears what a screen reader says. After the build is complete, the first check takes ten minutes with NVDA, then JAWS, then a 40-cell Braille display. Longer checks for archives, the viewer and the quick view, and for search, labels, compare, keys and watching, follow in [Using diroxide with a screen reader](screen-readers.md). Their findings are filed as the first inputs of the next alpha, or fixed at once if they stop daily use.

## What has been checked

When this page was written, nothing in alpha 1 had been heard on a screen reader. What exists is the design and the automated tests above. The "you should hear" lines in [Using diroxide with a screen reader](screen-readers.md) are what the build is meant to say. They become checked statements only when a listening check has confirmed them, and this page will say so, with the date, the screen reader and its version, and the terminal.

## What has not been checked

- NVDA in Windows Terminal is the main target and has not been heard on this project.
- JAWS in Windows Terminal is unknown territory.
- Orca on Linux and VoiceOver on macOS are best effort and have not been tried.
- The classic Windows console, Konsole and the graphics-card terminals have not been tried.
- A Braille display other than a 40-cell HumanWare Mantis Q40, and contracted braille, have not been tried.
- Whether the side by side layout reads acceptably with each screen reader. The stacked layout is recommended until it has been checked.
- The settings listed as not yet verified in the screen reader guide.

## Color

- The logo uses rust orange and steel blue, a pair that red-green color blindness leaves intact, and no red or green.
- The 24 themes are each checked for contrast and report the result in words when you choose one: "meets AA", or "below AA: 2 checks fall short".
- With `NO_COLOR` set, diroxide uses attributes alone.

## Known limits

[Known limits](known-limits.md) lists what does not work yet, including the keys some terminals will not send.

## The graphical interface (GUI)

The GUI is the window that `diro` opens on a desktop in alpha 3. It is built on the platform accessibility interfaces: UI Automation on Windows and AT-SPI on Linux. The macOS interface, NSAccessibility, has not been tried. Each panel is a list box whose options are the rows, and each dialog is a named, modal part whose keyboard focus is on a named control. [The GUI](gui.md) describes it, and [Using diroxide with a screen reader](screen-readers.md) has a listening check for it.

What checks it so far:

- Tests read the window's accessibility tree the way a screen reader does, with no screen reader and no window on a screen, on Windows, Linux and macOS. They cover the panels, the status lines, the key bar, the live region, the viewer, the quick view and every dialog the program can open, 77 of them.
- A scripted walk of the window, the owner's ten-minute check done by a test, checks the lines it hears against the first 40 cells of a Braille line.
- A UI Automation report on Windows and an AT-SPI report on Linux list what a real window gives a screen reader: names, roles, states, the menu bar, and every window of the process, which must have a name.
- A test plants a password and checks that it is in no name, value, description, drawing or announcement while it is hidden.
- A test checks that every one of the 24 themes draws no pair of colors below its floor: 4.5 to 1 for text, 7 to 1 in a high-contrast theme, and 3 to 1 for the focus marks.

What has not been checked: the window has not been used with NVDA, JAWS, Orca or VoiceOver, or read on a Braille display. The dialogs have been read only in the tests' harness, because the UI Automation report cannot open a dialog without taking the foreground. macOS has not been tried.

## Tell us

If something is hard to use with your screen reader, display or eyes, that is a bug, not a preference. [Troubleshooting](troubleshooting.md) says what to include when you report it.

## See also

- [Using diroxide with a screen reader](screen-readers.md)
- [The GUI](gui.md)
- [Themes and colors](themes.md)
- [Terminals](terminals.md)
- [Known limits](known-limits.md)
- [Troubleshooting](troubleshooting.md)
- [The documentation index](README.md)
