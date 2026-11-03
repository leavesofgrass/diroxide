# ADR-0007: Frontends over one core

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Saturday, October 10, 2026: amended at the owner's decision after R4's accessibility stack check (`research/accessibility-stack.md`, section 7). Three changes: the GUI's panel is a list box first, not a grid; the terminal's cursor sits at the focused row's first cell on every frame, and the layout is a setting; side by side is not offered in screen-reader mode. The original wording is in the git history.

Status update, Saturday, October 10, 2026, later the same evening: the owner decided that the two-panel side-by-side layout is the primary layout of the program, for screen-reader users too, and that the stacked layout is the fallback for screen-reader users and for narrow or limited terminal sessions. The terminal bullet and the fallback paragraph below say so; the sentence "side by side is not offered in screen-reader mode" no longer holds.

Status update, Saturday, October 17, 2026 (the pre-wiring contract step, P2b, from the requests pairs 1 to 3 held at their integrations):

- **Token classes are theme roles.** The theme's `PanelRole` gains seventeen token-class roles, `token_keyword` to `token_problem` (`PanelRole::TOKENS`), one for each class the preview crate's tokenizer gives, each a text color on the page with the attribute the class has by nature (bold keywords and headings, underlined links, a reversed problem, never italic). So `[colors]` overrides a class by its key, the contrast check covers it, and the verdict counts a weak pair once; no built-in theme's verdict changed. The terminal's viewer maps classes onto these roles (`diroxide_tui::viewer::class_role`) instead of borrowing the heading, link, code, quote, error and dim text colors, and `CodeStyles::with_colors` takes the `[colors]` table; classes the theme draws as plain text keep the row's own color unless the owner colors them (W2d's request 1).
- **Text on the clipboard is the frontend's.** `Effect::SetClipboardText(String)` joins the screen and speech effects a frontend carries out: the terminal can set the clipboard with its own escape sequence, the GUI through the platform; `SetClipboard` with places stays the runtime's. The viewer's `copy_path` and `copy_name` return it beside the sentence they say (W2d's request 3).

Status update, Saturday, October 17, 2026 (the pre-W3a contract step, P3b, from the requests the search, labels and tracker lanes held at their integrations):

- **Label colors are theme roles.** `PanelRole` gains six label roles, `label_blue`, `label_orange`, `label_purple`, `label_yellow`, `label_gray` and `label_brown` (`PanelRole::LABELS`, `is_label`), one for each color a label may have, with no red and no green. Each is a fixed hue whose lightness is moved, the hue kept, until it meets the theme's text floor on the page, so every built-in theme's label colors pass the contrast check and no verdict changed; `[colors]` overrides one by its key, checked like any role. The app maps a label's color to its role (`LabelColor::role`, `LabelStore::role_of`), so a frontend colors the label's word with it while the row's words stay as they are, the name first and the color's word after it (W3l's request 1).

diroxide has three frontends: the terminal, the GUI, and a headless server. A feature should exist once, in the core, and every frontend should read the same words for it. The input thread must never block, and tests must drive the app without a terminal or the GUI.

Status update, Monday, November 2, 2026 (Wave 6, lane L, plugin lists inside a panel): a plugin's lister opens as a **plugin list**, a per-panel overlay over the panel the plugin names, not a dialog over both panels and not a Place. `Panel::list()` stays the folder's own rows; frontends draw `Panel::shown_list()`, which is the plugin list while one is up and the folder's rows otherwise, with the list's title in the header. The folder beneath is untouched, quiet while the list is up, and never saved in the session or the history. The panel's keys act on the list's rows, Enter runs the lister's activate command as a job, every file command refuses with "This row is not a file.", and Left, Backspace, Alt+Up and Escape leave it (Back and Forward only leave it on the first press); leaving sends `lister/closed` and says "Plugin list closed (NAME)", then the folder's landing line. A plugin's `panel/set` with a place closes the list without a word. The GUI draws `shown_list()` the same way (lane G1).

Status update, Tuesday, November 3, 2026 (Wave 6, alpha 3, the GUI built by lanes G0 to G3 with the app lanes A4 to A6): the window is built as this record's GUI bullet says, in the same `diro` binary as the terminal, over textweaver's vendored Masonry, Vello, Parley and AccessKit stack with four small patches of diroxide's own (`docs/dev/vendoring.md`).

- **Each panel is one AccessKit list box over `Panel::shown_list()`.** An option's name is `ListRow::name()`, the label and details with no state words; the list's real size and each option's absolute position are set on the nodes, the focused row is the active descendant, and a marked or unavailable row carries the state on Windows and Linux and the word in its name on macOS, which reads no list states. Only a window of about sixty rows around the focus is nodes, so a hundred thousand rows cost the same tree as sixty. A screen reader hears the row's name first and its own position after it.
- **The live region keeps the three importances.** Normal is polite, High assertive, and Low is shown only, in the window as in the terminal; the start's lines are held until a screen reader asks for the window and said once; everything one step says is one announcement, in order. Whether the window should speak Low messages is a question for the owner's listening test.
- **The frontend is chosen by a table** in the CLI: `--tui` wins, then `--gui`, then the `frontend` setting, then automatic, which opens the window on a desktop except over SSH, with no display, with a pipe or a file in place of the terminal, or in a build without the `gui` feature. Automatic opens the terminal until the icons lane flips the default inside this alpha.
- **Every field is the app's.** The text field, the palette and the completion list are driven by the app's own words, caret and candidates, shown as a text field or a combo box, so the terminal and the window edit the same way; Masonry's own text field is not bridged in.
- **The dialogs take seven shapes** (Confirm with real buttons and No first, Question with its situation and a message of many lines as a list to read, Read, Choose as a modal list box, Secret with Show the password, Edit, and jobs with a progress indicator the app paces), only the top dialog live; the viewer and the quick view are list boxes; themes map by role with a word beside every color and the contrast verdict said; focus mode and full screen say nothing.
- **Menus are native on Windows** through the system's menu bar, hidden until Alt, F9, F10 or Alt with a menu's letter, and the list menus elsewhere and where the `menus` setting says so; macOS waits for an AppKit binding.
- Software drawing and the Windows console are ADR-0014. The consequences line below that says `--gui` opens the terminal no longer holds.

## Decision

- `diroxide_app::App` holds all state: both panels, the active side, the keymap, settings, dialogs, jobs, the undo log and the status line. `App::handle(AppEvent)` and `App::dispatch(Command)` change it and return `Effect`s; the app never does I/O.
- Effects split in two. A frontend carries out the screen and speech effects (`Redraw`, `Announce`, `OpenDialog`, `CloseDialog`, `Quit`). The `Runtime` carries out the I/O effects (`List`, `StartJob`, `CancelJob`, `RunPlugin`, `OpenExternally`, `SetClipboard`) and turns results into `AppEvent`s for the app. Listing requests carry ids, so a late answer to an old request is dropped.
- Announcements carry an importance. Low is shown and said only on Say Status (focus moves in the terminal, where the screen reader reads the line under the cursor itself); Normal is a polite live region; High is assertive.
- **The terminal.** The screen is a title line (the active place first), the panel rows, the status line (the key fact first) and the jobs line, with the hardware cursor on the focused row at its first cell, set on every frame in every state, and an asterisk on selected rows. Side by side is the primary layout, as in every orthodox manager since Norton Commander. In screen-reader mode a side-by-side line carries rows of both panels, so the stacked layout, one panel above the other with each line one row at full width, is the fallback offered to screen-reader users and to narrow or limited terminal sessions, and single draws only the active panel. Layout is a setting with three values: side by side (the default), stacked (the fallback) and single. In screen-reader mode the inactive panel is redrawn only on demand, whatever the layout. The plain theme draws no boxes; there are no spinners.
- **The GUI.** Each panel is one focusable AccessKit list box whose options are the rows. An option's name is the row as the list model speaks it (name first, details after), its one-based position and the list's real size are set on it, marked rows are selected, and the focused row is the list's active descendant. A grid with named columns replaces this only if AccessKit's adapters gain the Grid and Table patterns and interfaces, or the owner prefers it after a listening session (R4: `accessibility-stack.md`). `diroxide_gui::panel_grid` stays as the data both shapes are built from.
- **Headless.** `diro serve --stdio` exposes the core over the plugin protocol (ADR-0006).
- Phase 0's runtime is synchronous; `Runtime::run_until_idle` carries out effects until nothing is left, for tests and one-shot commands. W1d moves the work to worker threads with a channel back.

## Alternatives

- **Frontends owning state.** Simpler at first, but every feature would be built three times and read three ways.
- **Both panels side by side for screen readers too.** Matches sighted orthodox managers, but a screen reader reading a line, and a Braille display showing 40 cells of it, would mix the two panels.

## Fallback

R4 found that a side-by-side screen mixes both panels in every line a screen reader reads and in every 40 cells a display shows. The owner's decision (Saturday, October 10, 2026): side by side stays the primary layout for everyone, and the stacked layout is the fallback for screen-reader users and limited terminal sessions, reached by the setting and recommended by the screen-reader guide where the owner's ten-minute test says so. The core does not change.

## Consequences

- `diro PATH` opened one screen and returned in Phase 0; since W1d-2 (Friday, October 16, 2026) it opens the full terminal screen on the alternate screen, `--inline` is the fallback viewport, and `--gui` says the GUI comes later and opens the terminal (the plan's alpha.6 is the GUI).
- The scripted terminal test in Wave 1 drives `App` and `Runtime` and reads the rendered lines.

## See also

- [ADR-0002: The core model](0002-the-core-model.md)
- [ADR-0005: Commands, keys and menus from one model](0005-commands-keys-and-menus.md)
