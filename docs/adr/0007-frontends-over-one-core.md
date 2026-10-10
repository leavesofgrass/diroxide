# ADR-0007: Frontends over one core

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0). The terminal's screen-reader layout is to be reviewed against R4's report (`research/accessibility-stack.md`).

## Context

diroxide has three frontends: the terminal, the window, and a headless server. A feature should exist once, in the core, and every frontend should read the same words for it. The input thread must never block, and tests must drive the app without a terminal or a window.

## Decision

- `diroxide_app::App` holds all state: both panels, the active side, the keymap, settings, dialogs, jobs, the undo log and the status line. `App::handle(AppEvent)` and `App::dispatch(Command)` change it and return `Effect`s; the app never does I/O.
- Effects split in two. A frontend carries out the screen and speech effects (`Redraw`, `Announce`, `OpenDialog`, `CloseDialog`, `Quit`). The `Runtime` carries out the I/O effects (`List`, `StartJob`, `CancelJob`, `RunPlugin`, `OpenExternally`, `SetClipboard`) and turns results into `AppEvent`s for the app. Listing requests carry ids, so a late answer to an old request is dropped.
- Announcements carry an importance. Low is shown and said only on Say Status (focus moves in the terminal, where the screen reader reads the line under the cursor itself); Normal is a polite live region; High is assertive.
- **The terminal.** The screen is a title line (the active place first), the panel rows, the status line (the key fact first) and the jobs line, with the hardware cursor on the focused row and an asterisk on selected rows. In screen-reader mode only the active panel is drawn, full width, so a line never mixes rows of both panels; sighted mode draws both side by side. The plain theme draws no boxes; there are no spinners.
- **The window.** Each panel is an AccessKit grid with named columns, row and cell nodes, a selection and a focus; `diroxide_gui::panel_grid` computes that shape from the app today.
- **Headless.** `diro serve --stdio` exposes the core over the plugin protocol (ADR-0006).
- Phase 0's runtime is synchronous; `Runtime::run_until_idle` carries out effects until nothing is left, for tests and one-shot commands. W1d moves the work to worker threads with a channel back.

## Alternatives

- **Frontends owning state.** Simpler at first, but every feature would be built three times and read three ways.
- **Both panels side by side for screen readers too.** Matches sighted orthodox managers, but a screen reader reading a line, and a Braille display showing 40 cells of it, would mix the two panels.

## Fallback

If R4 finds that screen readers handle a side-by-side layout well with the cursor's position, the terminal offers it as an option; the core does not change.

## Consequences

- `diro PATH` in Phase 0 prints one screen of the terminal frontend and returns; the window says it is not built yet.
- The scripted terminal test in Wave 1 drives `App` and `Runtime` and reads the rendered lines.

## See also

- [ADR-0002: The core model](0002-the-core-model.md)
- [ADR-0005: Commands, keys and menus from one model](0005-commands-keys-and-menus.md)
