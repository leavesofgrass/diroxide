# ADR-0013: The job queue, the connection gate and resume

Status: Accepted, Friday, October 23, 2026 (alpha 2, Wave 4, lanes Q-E and A2, from research/alpha2-combined-plan.md, R25, part 1, and the owner's answers 80, 87, 96, 97 and R25's defaults N1 and N4 to N9).

## Context

Until alpha 2 every job started at once on its own thread, with no waiting state, no resume and three fixed retries. Remote places change that: a host takes a few connections well and many badly, a lost link would raise one dialog per job, and a transfer cut at 90 percent should not start from zero. WinSCP and FileZilla keep a transfer queue beside their file lists; the owner asked that diroxide learn from them (answer 80) without a second list.

## Decision

- **The queue is the jobs list, not a second thing.** Waiting jobs are data in the engine's queue, not threads. The Jobs list shows Running, Waiting and Finished, each row state first; the jobs line gives the count first. Local jobs start at once as before; remote jobs run two per host and four in all; component downloads one at a time. A job that needs two hosts takes both slots or neither; a question releases the slot; cancel works from any state; the order is stable.
- **Retries are settings:** silent retries at 5, 15 and 45 seconds, counted in the report, with an error policy per job and as a default for waiting jobs: ask (the default), continue and list, or stop at the first.
- **One connection gate per host.** When a host is lost, every job on it says once that it waits; after the retry budget one question per host, with Keep waiting first; the first success opens the gate; a plugin that says its loss is final stops the waiting jobs with its reason.
- **Resume:** a copy cut by a lost connection continues at the offset when the ledger of partial files, a fingerprint of the source, and a check of the first 64 KiB and the last 1 MiB agree; otherwise it starts again and says why. Cancel removes the partial file; a lost connection, a pause or a crash keeps it. A resumed move re-reads the prefix, and a remote move reads the destination back and hashes it before the source goes (answer 96). The plugin's side is `resume` on `write_begin`, which reopens the partial file and answers its length.
- **A speed limit per job and a global one,** in plain units, changeable while a job runs, the global one for remote jobs only unless chosen, off by default.
- **Hooks run in the engine:** `before_*` hooks after the plan and before the confirmation, on a worker thread, with the manifest's timeout and `on_timeout`; a veto stops the job with the reason, said with the plugin's name last; `after_*` once with the report.
- **The queue is not saved across restarts in this alpha;** quit asks when jobs are waiting or running, and the next start reports an interrupted job.

## Alternatives

- A separate transfer queue with its own list and keys, as the two managers have: a second thing to learn and to hear; refused.
- Pausing every job at once on a lost host and never asking: simpler, but a flaky link would then need the owner's hand for every blip; the gate asks once after the budget.
- Trusting the acknowledgements for a remote move instead of reading the destination back: half the traffic and a weaker guard; the owner chose the read-back.

## Fallback

If a provider cannot resume, its capabilities say so and a cut transfer starts again, as before alpha 2.

## Consequences

- Copies of 4 MiB or more on resumable places keep their part file after a lost network, listed in the ledger; answering Skip on a network share leaves one.
- Split and join never resume. Network shares count as local: no gate, only per-item retries.
- The engine changed in three crates at once (core, vfs, ops); the app wires its events, states and the Jobs list.

## See also

- [The transfer queue](../transfer-queue.md), the guide.
- [ADR-0004: The job engine](0004-the-job-engine.md) and [ADR-0012: SFTP over the OpenSSH client](0012-sftp-over-the-openssh-client.md).
