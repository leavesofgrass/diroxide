# ADR-0004: The job engine

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

## Context

Worker's copy runs on a thread, while its dirsize and delete block the interface. diroxide runs every operation that touches more than one entry, or can take more than a moment, as a job, so the interface never blocks and every operation reports, cancels and undoes the same way.

## Decision

- A `JobRequest` (in core) names the kind, the sources, the destination, a new name and the conflict policy. `Engine::plan` turns it into a `Plan` without touching anything: one `Step` per entry, each acting on one place.
- `Plan::listing` writes every step in words with hidden characters spelled out as `[U+XXXX]`; a delete, move or overwrite shows it before its confirmation. `Plan::summary` says the plan in one line, meaning first.
- `Engine::run` runs a plan on the caller's thread (the app's runtime puts it on a worker thread), reporting to a `JobSink`: progress as `JobProgress`, which reads "Copying 12 of 300, 1.2 of 4.0 GB, about 2 minutes left", and conflicts, which block until the user answers. The answer applies to the rest of the job; asking again counts as skip.
- `JobControl` cancels (between steps, and inside providers through the token) and pauses.
- The `JobReport` says what was done, skipped and failed, each with the reason in words, and whether the job was cancelled; its summary reads "Copy finished: 3 done, 1 skipped."
- Renames, moves and trashings go into an `UndoRecord`, kept by the `UndoLog`; Undo replays the moves backwards and restores trashed entries.
- Nothing in the engine runs a shell.

## Alternatives

- **Jobs on tokio.** The runtime is confined to the plugin host and SFTP (plan question 8); blocking jobs on worker threads keep the engine simple and the providers synchronous.
- **Conflicts answered up front.** Asking once, when the first conflict appears, keeps jobs that have none free of questions.

## Fallback

If one thread per job is too many for a large queue, the runtime runs jobs from a small pool; the engine's interface does not change.

## Consequences

- Phase 0's planning does not walk folders and its run copies single files only; W1b adds the walk, the listing of every path, conflicts across folders, the fake provider for tests, and the property test that a plan equals what ran.
- The app shows the jobs line from the engine's progress, at most twice a second.

## See also

- [ADR-0003: The provider trait](0003-the-provider-trait.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)
