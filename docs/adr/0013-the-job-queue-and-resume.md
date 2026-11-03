# ADR-0013: The job queue, the connection gate and resume

Status: Accepted, Friday, October 23, 2026 (alpha 2, Wave 4, lanes Q-E and A2, from research/alpha2-combined-plan.md, R25, part 1, and the owner's answers 80, 87, 96, 97 and R25's defaults N1 and N4 to N9). Amended Monday, November 2, 2026 (alpha 3, Wave 6, lane Q2): a hold for one waiting job, and the queue saved across restarts, below. Amended Tuesday, November 3, 2026 (alpha 3, Wave 6, lane A4): a state word for the hold in core, and a quit that keeps part files for a resume, below.

## Context

Until alpha 2 every job started at once on its own thread, with no waiting state, no resume and three fixed retries. Remote places change that: a host takes a few connections well and many badly, a lost link would raise one dialog per job, and a transfer cut at 90 percent should not start from zero. WinSCP and FileZilla keep a transfer queue beside their file lists; the owner asked that diroxide learn from them (answer 80) without a second list.

## Decision

- **The queue is the jobs list, not a second thing.** Waiting jobs are data in the engine's queue, not threads. The Jobs list shows Running, Waiting and Finished, each row state first; the jobs line gives the count first. Local jobs start at once as before; remote jobs run two per host and four in all; component downloads one at a time. A job that needs two hosts takes both slots or neither; a question releases the slot; cancel works from any state; the order is stable.
- **Retries are settings:** silent retries at 5, 15 and 45 seconds, counted in the report, with an error policy per job and as a default for waiting jobs: ask (the default), continue and list, or stop at the first.
- **One connection gate per host.** When a host is lost, every job on it says once that it waits; after the retry budget one question per host, with Keep waiting first; the first success opens the gate; a plugin that says its loss is final stops the waiting jobs with its reason.
- **Resume:** a copy cut by a lost connection continues at the offset when the ledger of partial files, a fingerprint of the source, and a check of the first 64 KiB and the last 1 MiB agree; otherwise it starts again and says why. Cancel removes the partial file; a lost connection, a pause or a crash keeps it. A resumed move re-reads the prefix, and a remote move reads the destination back and hashes it before the source goes (answer 96). The plugin's side is `resume` on `write_begin`, which reopens the partial file and answers its length.
- **A speed limit per job and a global one,** in plain units, changeable while a job runs, the global one for remote jobs only unless chosen, off by default.
- **Hooks run in the engine:** `before_*` hooks after the plan and before the confirmation, on a worker thread, with the manifest's timeout and `on_timeout`; a veto stops the job with the reason, said with the plugin's name last; `after_*` once with the report.
- **A hold for one waiting job** (alpha 3): Pause on a job that waits its turn keeps it in its place; the scheduler passes over it and it holds back no later job; Resume lets it go in the same place. It counts among the waiting. Pause all jobs holds every waiting job with the queue, and Resume all jobs lets them go. Core has the state word for a hold (`JobState::Held`, "held in line"), which the ops queue gives a held job and the app's jobs table reads, so the words are the same wherever they are said.
- **The queue is saved across restarts** (alpha 3; it was not in alpha 2): after every change the waiting, held and running jobs, with the request each was planned from, go to `queue.json` in the state folder beside the ledger, written atomically by one writer thread so the input thread never waits, and the file is removed when the queue is empty. Quit asks when jobs are waiting or running, with the same words, and a yes writes the queue as it stands before it stops them. The next start lists every job found as Interrupted, first in the Jobs list, and starts none: Retry plans its request again with the usual question, Forget drops it, and it stays listed until one of those. One sentence says how many, once. A file that does not read is renamed aside, never deleted. A lock beside the file keeps a second session on the same state folder out of it.

## Alternatives

- A separate transfer queue with its own list and keys, as the two managers have: a second thing to learn and to hear; refused.
- Pausing every job at once on a lost host and never asking: simpler, but a flaky link would then need the owner's hand for every blip; the gate asks once after the budget.
- Starting the saved queue's jobs again on their own at the next start, as some transfer managers do: a job that waited a day may no longer be wanted, and a start nobody asked for is a surprise heard late; refused. Saving the plans themselves: a day-old plan is stale, while its request plans again against what is there now.
- A quit that keeps part files for a resume: left out of alpha 2, because the quit's stop was a cancel, which removes them, and a second kind of stop was needed in the engine. Done in alpha 3, below.
- Trusting the acknowledgements for a remote move instead of reading the destination back: half the traffic and a weaker guard; the owner chose the read-back.

## Fallback

If a provider cannot resume, its capabilities say so and a cut transfer starts again, as before alpha 2.

## Consequences

- Copies of 4 MiB or more on resumable places keep their part file after a lost network, listed in the ledger; answering Skip on a network share leaves one.
- Split and join never resume. Network shares count as local: no gate, only per-item retries.
- The engine changed in three crates at once (core, vfs, ops); the app wires its events, states and the Jobs list.
- Alpha 3: the hold lives in the ops queue; the saved queue lives in the app's runtime, which knows each job's request. Two sessions on one state folder: the first keeps the queue. A quit stops a running job with a second kind of stop (`JobControl::interrupt`), which does what a lost connection does: a copy's part file and its ledger line stay, so Retry on the interrupted job resumes the copy at its offset when the checks above agree ("Resuming big.bin at 2.1 MB of 5.2 MB"). Cancel still removes the part file. The question at quit keeps its words. Where a place cannot resume, a copy cut by a quit starts its file again: the local provider keeps part files, and an SFTP place does not yet, since its provider has no `as_resumable`, so a copy to a server starts again.

## See also

- [The transfer queue](../transfer-queue.md), the guide.
- [ADR-0004: The job engine](0004-the-job-engine.md) and [ADR-0012: SFTP over the OpenSSH client](0012-sftp-over-the-openssh-client.md).
