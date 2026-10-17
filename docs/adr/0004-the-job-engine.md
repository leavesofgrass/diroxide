# ADR-0004: The job engine

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Friday, October 16, 2026 (the Wave 1 contract, P0b, from R8 C4 and C5, R20, R23 and plan section 5.4):

- **Create, verify, then destroy.** A replacement is written and committed before anything it replaces is removed; a clone is tried before the destination is touched; a trash failure never falls through to a permanent delete; every job's results are reconciled against a fresh read (`Reconciliation`: planned, present and missing, the count first).
- **Conflicts one at a time.** `Conflict` carries its kind, both entries and the keep-both name; `lines()` says the situation first, then the difference and a fact line for each; `choices()` gives the nine verb-first choices, Skip first. The sink answers a `ConflictAnswer` with apply-to-rest, which replaces "the answer applies to the rest of the job" above.
- **Errors.** An error on an item goes to `JobSink::error` as an `ItemError`, answered with Retry, Skip, Skip all like this or Cancel; Retry comes first only where it helps.
- **Words.** Progress is percent first ("Copying 30 percent, 2 minutes left"), the counts on a detail line, with a `JobState` on every update and `bytes_per_second`. The report puts problems first ("Copy had 8 failures: 290 done, 2 skipped.") and gains warnings.
- **Undo.** `UndoRecord` gains the job's kind, its origin and the created entries, so a copy, a made folder, a split and a join undo too; it is serializable for the journal W1b writes before the first step.
- **Tests.** The fake provider has the switches these tests need: place rules, latency and a stall, the lies (a commit that writes nothing, a clone that fails late, a trash that does nothing) and faults after N bytes.

Status update, Friday, October 16, 2026 (the alpha.2 contract step, P2, from `research/alpha2-plan.md` commits 2 to 4 and W1d-1's held request 3):

- **Undo of what a job made.** The created entries W1b records are held by tests: a property test copies a random tree on the fake provider, undoes it, and finds the tree as it was with every new file in the fake trash exactly once; the undo of a split trashes the sidecar and the parts, newest first, and of a join the whole.
- **Links.** `Step::Link` plans and runs links; the planner first asks the destination's capabilities and refuses with `OpsError::CannotLink`, in the capability's words. The fake provider gains a `no_links` switch.
- **The journal** returns every finished job, those that left nothing to undo included, so an undo after a restart is matched with the record it reversed (an undo of a trash restores and records nothing). The app pairs them and drops the empty ones, as the undo log does in the session.
- **Words.** The interrupted-job sentence names a pack, an unpack, an archive test, a split and a join; the jobs-line estimate test covers Testing.
- **Dependencies.** The ops crate takes `crc32fast` for the split sidecar (W2c).

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
