# ADR-0007: Frontends over one core

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Saturday, October 10, 2026: amended at the owner's decision after R4's accessibility stack check (`research/accessibility-stack.md`, section 7). Three changes: the window's panel is a list box first, not a grid; the terminal's cursor sits at the focused row's first cell on every frame, and the layout is a setting; side by side is not offered in screen-reader mode. The original wording is in the git history.

## Context

diroxide has three frontends: the terminal, the window, and a headless server. A feature should exist once, in the core, and every frontend should read the same words for it. The input thread must never block, and tests must drive the app without a terminal or a window.

## Decision

- `diroxide_app::App` holds all state: both panels, the active side, the keymap, settings, dialogs, jobs, the undo log and the status line. `App::handle(AppEvent)` and `App::dispatch(Command)` change it and return `Effect`s; the app never does I/O.
- Effects split in two. A frontend carries out the screen and speech effects (`Redraw`, `Announce`, `OpenDialog`, `CloseDialog`, `Quit`). The `Runtime` carries out the I/O effects (`List`, `StartJob`, `CancelJob`, `RunPlugin`, `OpenExternally`, `SetClipboard`) and turns results into `AppEvent`s for the app. Listing requests carry ids, so a late answer to an old request is dropped.
- Announcements carry an importance. Low is shown and said only on Say Status (focus moves in the terminal, where the screen reader reads the line under the cursor itself); Normal is a polite live region; High is assertive.
- **The terminal.** The screen is a title line (the active place first), the panel rows, the status line (the key fact first) and the jobs line, with the hardware cursor on the focused row at its first cell, set on every frame in every state, and an asterisk on selected rows. In screen-reader mode only the active panel is drawn, full width, so a line never mixes rows of both panels; sighted mode draws both side by side. Layout is a setting with three values: single (the screen-reader default), stacked (one panel above the other) and side by side (sighted mode only). The plain theme draws no boxes; there are no spinners.
- **The window.** Each panel is one focusable AccessKit list box whose options are the rows. An option's name is the row as the list model speaks it (name first, details after), its one-based position and the list's real size are set on it, marked rows are selected, and the focused row is the list's active descendant. A grid with named columns replaces this only if AccessKit's adapters gain the Grid and Table patterns and interfaces, or the owner prefers it after a listening session (R4: `accessibility-stack.md`). `diroxide_gui::panel_grid` stays as the data both shapes are built from.
- **Headless.** `diro serve --stdio` exposes the core over the plugin protocol (ADR-0006).
- Phase 0's runtime is synchronous; `Runtime::run_until_idle` carries out effects until nothing is left, for tests and one-shot commands. W1d moves the work to worker threads with a channel back.

## Alternatives

- **Frontends owning state.** Simpler at first, but every feature would be built three times and read three ways.
- **Both panels side by side for screen readers too.** Matches sighted orthodox managers, but a screen reader reading a line, and a Braille display showing 40 cells of it, would mix the two panels.

## Fallback

R4 found that a side-by-side screen mixes both panels in every line a screen reader reads and in every 40 cells a display shows, so it is not offered in screen-reader mode. If the owner hears speech on Tab in the single-panel layout, the terminal makes the stacked layout the default; the core does not change.

## Consequences

- `diro PATH` in Phase 0 prints one screen of the terminal frontend and returns; the window says it is not built yet.
- The scripted terminal test in Wave 1 drives `App` and `Runtime` and reads the rendered lines.

## See also

- [ADR-0002: The core model](0002-the-core-model.md)
- [ADR-0005: Commands, keys and menus from one model](0005-commands-keys-and-menus.md)
