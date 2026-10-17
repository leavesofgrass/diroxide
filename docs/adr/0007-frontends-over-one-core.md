# ADR-0007: Frontends over one core

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Saturday, October 10, 2026: amended at the owner's decision after R4's accessibility stack check (`research/accessibility-stack.md`, section 7). Three changes: the window's panel is a list box first, not a grid; the terminal's cursor sits at the focused row's first cell on every frame, and the layout is a setting; side by side is not offered in screen-reader mode. The original wording is in the git history.

Status update, Saturday, October 10, 2026, later the same evening: the owner decided that the two-panel side-by-side layout is the primary layout of the program, for screen-reader users too, and that the stacked layout is the fallback for screen-reader users and for narrow or limited terminal sessions. The terminal bullet and the fallback paragraph below say so; the sentence "side by side is not offered in screen-reader mode" no longer holds.

Status update, Saturday, October 17, 2026 (the pre-wiring contract step, P2b, from the requests pairs 1 to 3 held at their integrations):

- **Token classes are theme roles.** The theme's `PanelRole` gains seventeen token-class roles, `token_keyword` to `token_problem` (`PanelRole::TOKENS`), one for each class the preview crate's tokenizer gives, each a text color on the page with the attribute the class has by nature (bold keywords and headings, underlined links, a reversed problem, never italic). So `[colors]` overrides a class by its key, the contrast check covers it, and the verdict counts a weak pair once; no built-in theme's verdict changed. The terminal's viewer maps classes onto these roles (`diroxide_tui::viewer::class_role`) instead of borrowing the heading, link, code, quote, error and dim text colors, and `CodeStyles::with_colors` takes the `[colors]` table; classes the theme draws as plain text keep the row's own color unless the owner colors them (W2d's request 1).

diroxide has three frontends: the terminal, the window, and a headless server. A feature should exist once, in the core, and every frontend should read the same words for it. The input thread must never block, and tests must drive the app without a terminal or a window.

## Decision

- `diroxide_app::App` holds all state: both panels, the active side, the keymap, settings, dialogs, jobs, the undo log and the status line. `App::handle(AppEvent)` and `App::dispatch(Command)` change it and return `Effect`s; the app never does I/O.
- Effects split in two. A frontend carries out the screen and speech effects (`Redraw`, `Announce`, `OpenDialog`, `CloseDialog`, `Quit`). The `Runtime` carries out the I/O effects (`List`, `StartJob`, `CancelJob`, `RunPlugin`, `OpenExternally`, `SetClipboard`) and turns results into `AppEvent`s for the app. Listing requests carry ids, so a late answer to an old request is dropped.
- Announcements carry an importance. Low is shown and said only on Say Status (focus moves in the terminal, where the screen reader reads the line under the cursor itself); Normal is a polite live region; High is assertive.
- **The terminal.** The screen is a title line (the active place first), the panel rows, the status line (the key fact first) and the jobs line, with the hardware cursor on the focused row at its first cell, set on every frame in every state, and an asterisk on selected rows. Side by side is the primary layout, as in every orthodox manager since Norton Commander. In screen-reader mode a side-by-side line carries rows of both panels, so the stacked layout, one panel above the other with each line one row at full width, is the fallback offered to screen-reader users and to narrow or limited terminal sessions, and single draws only the active panel. Layout is a setting with three values: side by side (the default), stacked (the fallback) and single. In screen-reader mode the inactive panel is redrawn only on demand, whatever the layout. The plain theme draws no boxes; there are no spinners.
- **The window.** Each panel is one focusable AccessKit list box whose options are the rows. An option's name is the row as the list model speaks it (name first, details after), its one-based position and the list's real size are set on it, marked rows are selected, and the focused row is the list's active descendant. A grid with named columns replaces this only if AccessKit's adapters gain the Grid and Table patterns and interfaces, or the owner prefers it after a listening session (R4: `accessibility-stack.md`). `diroxide_gui::panel_grid` stays as the data both shapes are built from.
- **Headless.** `diro serve --stdio` exposes the core over the plugin protocol (ADR-0006).
- Phase 0's runtime is synchronous; `Runtime::run_until_idle` carries out effects until nothing is left, for tests and one-shot commands. W1d moves the work to worker threads with a channel back.

## Alternatives

- **Frontends owning state.** Simpler at first, but every feature would be built three times and read three ways.
- **Both panels side by side for screen readers too.** Matches sighted orthodox managers, but a screen reader reading a line, and a Braille display showing 40 cells of it, would mix the two panels.

## Fallback

R4 found that a side-by-side screen mixes both panels in every line a screen reader reads and in every 40 cells a display shows. The owner's decision (Saturday, October 10, 2026): side by side stays the primary layout for everyone, and the stacked layout is the fallback for screen-reader users and limited terminal sessions, reached by the setting and recommended by the screen-reader guide where the owner's ten-minute test says so. The core does not change.

## Consequences

- `diro PATH` opened one screen and returned in Phase 0; since W1d-2 (Friday, October 16, 2026) it opens the full terminal screen on the alternate screen, `--inline` is the fallback viewport, and `--gui` says the window comes with alpha.6 and opens the terminal.
- The scripted terminal test in Wave 1 drives `App` and `Runtime` and reads the rendered lines.

## See also

- [ADR-0002: The core model](0002-the-core-model.md)
- [ADR-0005: Commands, keys and menus from one model](0005-commands-keys-and-menus.md)
