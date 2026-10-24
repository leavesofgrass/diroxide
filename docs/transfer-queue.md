# The transfer queue

This guide covers the Jobs list, the queue behind it, what happens when a server stops answering, retries, resume, the speed limit, and what quitting does while jobs are unfinished. It is about jobs that touch another computer, and it also says what the Jobs list shows for jobs on this computer.

What this guide quotes is what the program says. Each sentence is checked in tests against the 40 cells of a Braille line, and each is written so its meaning comes first. None of it has been heard yet on NVDA, JAWS or a Braille display. If your experience differs, that is a finding.

[Remote places over SFTP](remote-places.md) says how to open a server. [Operations and undo](operations.md) says how every job starts, with its plan and its question.

## The queue is the Jobs list

There is no second thing to learn. A job you start goes through the same steps as ever: the plan, the question that starts on No, then the run. The queue decides only when a confirmed job starts.

- A job on this computer starts at once, as before.
- A job that touches a server takes a place in line. Two jobs run at once for one server, and four run at once for all servers together. The rest wait their turn, in the order they came.
- A job that touches two servers, such as a copy from one to the other, takes a place on both or on neither.
- Downloads of optional components run one at a time.
- A job that stops to ask you something gives its place back, so one forgotten question cannot hold the line. When you answer, the job waits for a place again, ahead of jobs that never started.
- The queue lives in this session. It is not kept when you quit.

The two limits are settings, `jobs.per_host` and `jobs.remote`, below.

A job that waited an hour is as safe as one that started at once. diroxide checks each entry again before it acts on it, and skips one that changed after the plan was made.

## The jobs line

The line says "No jobs" when idle, and a lone job's own sentence, as before. With several jobs, the count comes first and then the front job's sentence:

```text
3 jobs, 1 waiting: Copying 40 percent
```

Only the front job's progress is said as it goes. The other jobs speak when their state changes. Ctrl+D (Say status) gives the whole picture, including each job that waits its turn.

A change of state is said at once. Progress is said at most every ten seconds in screen-reader mode, as before.

- "Queued: copy 12 items to build" when a job has to wait.
- "Copy started: 12 items to build" when it gets its place.
- "Copy waits for build: connection lost" when its server stops answering.
- "build is back. Copy goes on." when the server answers again.
- "Copy finished: 12 done." when it ends.

## The Jobs list

Open it from the Tools menu: Alt+T, then J. It is also in the palette (Ctrl+P) as Jobs. "Job history" in the palette opens the same list on the first finished job.

The list has up to three groups, each under a heading that gives its count:

```text
Running, 2 jobs
Waiting, 1 job
Finished, 5 jobs
```

Every row puts its state first, then the job:

```text
Copying 40 percent: 12 items to build
Waiting 2 of 5: copy 3 items to build
Paused at 30 percent: copy to build
Stalled at 30 percent: copy to build
Waiting for build: copy at 30 percent
Waiting for an answer: copy 3 items to build
Verifying 60 percent: move 2 items to build
Done: copy 12 items to build
Failed: copy to build, 2 items not done
Cancelled: copy 3 items to build, 1 done
```

- Running holds every job that started: running, paused, stalled, verifying, waiting for an answer, and waiting for the connection.
- Waiting holds the jobs that wait their turn, in the order they will start. "Waiting 2 of 5" says the place in line.
- Finished holds the jobs that ran, newest first. The list keeps the last 100.

With nothing to show, the list says "No jobs have run yet." An open list follows the jobs as they change, and your focus stays on the same job when it moves from one group to another.

Up and Down move. A heading row has nothing to do, so Enter on one moves to the next row. Escape closes the list.

### What Enter offers

Enter on a job opens a list named for that job. It offers, in this order, what fits the job.

A waiting job:

- Run now. Starts it past the limits.
- Move to the front, Move up, Move down, Move to the end.
- Set a speed limit.
- On errors, with the current choice: "On errors: ask about each error".
- Cancel this job.

A running job:

- Pause, or Resume if it is paused.
- Set a speed limit.
- Cancel this job.

A finished job:

- Show what failed. It lists each item and why.
- Retry the failed items. It is shown dimmed when nothing failed, or when the job was not a copy, a move, a trash or a delete.

Under any of them come the queue's own rows:

- Pause the queue, or Resume the queue. With the queue paused, no new job starts and you hear "Queue paused: nothing new starts." Jobs that run go on. New jobs wait, local ones too. "Queue goes on." ends it.
- Pause all jobs: "All jobs paused." Resume all jobs: "All jobs go on."

Every one of these is also in the palette, so no new key is needed.

Pausing one job says "Pausing copy 12 items to build", and Resume says "Resuming copy 12 items to build". Pause is not offered for a job that is only waiting its turn. Cancel it or move it instead.

### Cancel

- Delete, F8 and Shift+Delete in the list cancel the job under the focus.
- A waiting job is cancelled at once, with no question. It has not started, so there is nothing to undo.
- A running job asks first, and the question starts on No: "Cancel copy 12 items to build? No is the default. y or n". A yes says "Cancelling: copy 12 items to build", and a no says "Not cancelled."
- When other jobs run beside it, or when the job never started, the end names it: "Copy cancelled: report.iso to build, 0 done."

Escape on a panel cancels the job in front, as it always did.

## When a server stops answering

A copy of twelve files must not open twelve dialogs when the network goes. Each server has one gate that all its jobs share.

1. A call fails because the network is lost. diroxide checks the server with a quick look at its top folder, so a passing glitch costs nothing.
2. If the server is lost, every job on it says once that it waits: "Copy waits for build: connection lost". Its row reads "Waiting for build: copy at 30 percent".
3. The gate tries again after 5 seconds, then 15, then 45. These are the same waits that retries use, below. One job does the trying for all of them.
4. The first success opens the gate. Every job goes on, resuming its file where it can, and you hear once: "build is back. Copy goes on." With several jobs: "build is back. 3 jobs go on."
5. If the waits run out, one question opens for the server. It waits behind any dialog that is already open and never covers one.

```text
build is not answering.
3 jobs wait for it.
```

The choices, in this order:

- Keep waiting. This is where the focus starts, and what Escape chooses. It begins a new round of waits and says "Waiting for build."
- Try again now.
- Cancel these jobs. It says one line for each job it stops: "Copy cancelled: report.iso on build".

A job that you cancel while it waits leaves at once. If it was the one doing the trying, another waiting job takes over.

A connection that needed a typed password is not opened again quietly. Open the place again from its panel, answer the password question, and the waiting jobs go on.

A copy on a network share of this computer, such as a Windows share, counts as a local job. It has no gate, only the per-item retries below.

## Retries, and the error policy

A lost network is tried again without a word. The waits grow: 5 seconds, then 15, then 45. Each wait is three times the one before, and none is longer than ten minutes. There is no way to retry forever.

- The tries are counted and reported at the end, not spoken one by one: "done after 2 automatic retries".
- When the tries run out for an item, the usual four-answer list opens for it: Retry, Skip, Skip all like this, and Cancel the job.
- A stall is separate. When nothing moves for 20 seconds the line says "Copying stalled at 30 percent", and Escape gives up.

The number of tries and the first wait are settings, `jobs.network_retries` and `jobs.network_retry_seconds`. With 0 tries, the item asks at once.

### What a job does with an error

Each job has an error policy. It is the setting `jobs.on_errors` when the job is made, and you can change it for a job that is waiting.

- Ask about each error (the default). The four-answer list opens for each failed item.
- Continue and list errors. A failed item is skipped, and the failures are listed at the end. No question opens. It is for work you leave alone.
- Stop at the first error. The job stops, and says why.

In the Jobs list, a waiting job's row "On errors" opens the three choices, and says "On errors: continue and list errors" when you choose.

### Retry the failed items

On a finished job, Retry the failed items makes a new job from the items that failed. It is planned and asked like any job, and it skips what is already at the destination. The plan says "Retry: what is already there is skipped". The palette's Retry the failed items, with no job chosen, uses the newest job that had failures.

It works for copy, move, trash and delete. "Nothing to retry: no job failed." and "Retry is not offered for split jobs." say why it did not.

## Resume

A copy cut by a lost connection goes on where it stopped, but only when everything agrees. Resume never trusts the length alone.

- It applies to a copy of 4 MiB or more, to or from a place that can resume. SFTP places can. A smaller file simply starts again.
- When the network is lost, the part file stays. A part file is the file written under its temporary name, `.diro-part-` and eight hex digits. diroxide writes down what it knows about each one in `partials.json` in the state folder: the target, the part file, the source, the source's size and time, and the bytes written.
- Cancel removes the part file and its line. A pause, a lost connection or a crash keeps them.
- To resume, the target and the source must be the same, the source's size and time must be what they were, the part file must be there and no longer than the source, and the head and tail check must pass. The check compares the first 64 KiB and the last 1 MiB of the part file with the same ranges of the source.
- When it resumes you hear: "Resuming report.iso at 410 MB of 800 MB".
- When it does not, you hear why, and the file starts over: "Not resumed: the source changed." (or "Not resumed: the part file is too long.", "Not resumed: the part file differs.", "Not resumed: the part file is gone.") and then "Starting again from the start."
- After the last byte, the usual checks run: the size, then a read-back, then the atomic rename that makes it the real file.
- A move never trusts a resumed prefix. It reads the source's first part again and compares it before anything is removed. Resume saves writing the prefix, never reading it.
- A plain copy does not read every byte back, and the head and tail check does not see a bad byte in the middle. A move does read everything back before it removes the source.
- Split and join never resume.
- On a network share, answering Skip after a lost connection can leave a `.diro-part-` file behind, and it is listed in `partials.json`. diroxide never deletes leftover part files on its own. Delete one like any other file.

## The speed limit

There are two ways to set it.

- In the Jobs list, Set a speed limit on one job limits that job only.
- The palette's Speed limit command, or the setting below, limits every remote job.

Both ask for a number with a unit: bytes, KB, MB or GB, in thousands, with or without "a second". All of these work: `500 KB`, `1.5 MB a second`, `800kb/s`, `2 MB`. Type `off`, `0` or nothing to turn it off.

What you hear:

```text
Speed limit: 500 KB a second
Speed limit off
Speed limit unchanged.
Not a speed: type 500 KB, 2 MB or off.
```

"Speed limit unchanged." is what Escape says. The last line answers a value that is not a speed, and the field stays open.

A limit changed while a copy runs takes hold on the next chunk. Pause and Cancel still answer between chunks, so Escape is never held up by the limit.

The global limit applies to remote jobs only, unless the setting `jobs.speed_limit_scope` is `all`.

## Quitting with jobs

Ctrl+Q asks when any job runs or waits, because the queue is not kept:

```text
2 jobs are not finished.
Quit and stop them? No is the default. y or n
```

With one job it says "1 job is not finished." A no goes back. A yes stops the jobs and quits. The setting `confirm.quit_with_jobs` turns this question off.

## Settings

The Settings list has a group named Jobs and transfers, with F1 help on every row. [The settings reference](settings-reference.md) lists them with their ranges.

- `jobs.per_host` (2; 1 to 8): jobs at once per server.
- `jobs.remote` (4; 1 to 16): remote jobs at once, in all.
- `jobs.speed_limit_kb` (0; 0 to 10000000): the most a job may read and write in a second, in KB. 0 is off.
- `jobs.speed_limit_scope` (`remote`): which jobs the limit slows, `remote` or `all`.
- `jobs.network_retries` (3; 0 to 10): tries on a lost network before the job asks. 0 is off.
- `jobs.network_retry_seconds` (5; 1 to 600): the first wait. 5 gives waits of 5, 15 and 45 seconds.
- `jobs.on_errors` (`ask`): what a new job does when an item fails, `ask`, `continue_and_list` or `stop_at_first`.

Local jobs always start at once, whatever `jobs.per_host` says.

## What is not built

[Known limits](known-limits.md) has the full list.

- The queue is not saved across a restart.
- Pause is not offered for a job that is waiting its turn.
- A job's error policy can be changed only while the job waits.
- Retry covers copy, move, trash and delete.
- Several connections for one big file are not built.
- A lost connection is final only when the plugin is turned off for good. A plugin cannot yet tell diroxide that a loss is final.

## See also

- [Remote places over SFTP](remote-places.md)
- [Operations and undo](operations.md)
- [Hooks](hooks.md)
- [Settings and where the files live](settings.md)
- [The settings reference](settings-reference.md)
- [Known limits](known-limits.md)
- [Using diroxide with a screen reader](screen-readers.md)
- [The documentation index](README.md)
