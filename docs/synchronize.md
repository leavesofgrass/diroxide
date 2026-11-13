# Synchronize

This guide covers Synchronize: making two folders match, from a plan you read first. It says what the five modes do, how the plan is said, how to leave a row out or change its direction, what the question before the run says, what the run does and checks, how to undo it, and `diro sync` for a shell.

The short version: Compare folders finds what differs. Synchronize turns that result into a plan, one row for each change, and shows the plan before anything happens. Nothing is copied, replaced or removed until you answer a question that starts on No. A file that would be replaced or removed goes to the trash first, where the place has a trash.

What this guide says comes from the design and from the tests that check each sentence. Nothing here had been heard on a screen reader or read on a Braille display when it was written; if your experience differs, that is a finding.

## Before you start

Synchronize works on the folder in this panel and the folder in the other panel. Rows say "here" for this panel and "there" for the other, and sentences say "this panel" and "the other panel", never left and right.

Synchronize is in the Tools menu (Alt+T, then Y) and in the palette. It has no key of its own. It starts from a compare. From a panel it compares the two folders with the options from Settings, without the criteria list, and then lists the modes. Over an open result of [Compare folders](compare.md#compare-two-folders) it lists the modes at once, and "Synchronize: plan the changes" is the action at the end of that result.

## The modes

The modes are a list titled "Synchronize how:", and Enter on one makes the plan. The first is the one to take when you are not sure. A mode that removes says so in its details, "trashes there what is not on the other side".

- "Update the other panel": copy what is missing there, or newer here, to the other panel. Nothing is removed.
- "Mirror to the other panel": make the other panel the same as this one. Everything that differs is copied, and what is only there is removed, every removal listed.
- "Both ways, newer wins": copy each way and the newer file wins. When two files have the same time and different sizes, that is a problem row for you to settle. diroxide does not guess.
- "Update this panel": the same as the first, the other way. Nothing is removed.
- "Mirror to this panel": the same as the second, the other way.
- "Compare only": make no plan and go back to the result.

## The plan

The plan is a list. Its name is short, the mode and the number of changes: "Update the other panel: 2 changes". The whole summary is said once as the plan opens, mode first and then counts, in the form "Update the other panel: 2 problems, 4 to copy there, 2 skipped, 1 same; 5 bytes to copy". When the plan has nothing to do it says so: "Mirror to the other panel: nothing to do, 1 same".

Each row starts with its action, then the name, then the reason. Problems come first, then the removals are listed so you read them before the end:

- "Copy there: plan.md, only here"
- "Replace there: notes.txt, newer here"
- "Trash there: old.txt, only there"
- "Delete for good there: NAME, no trash there", where the place has no trash
- "Skip: ..." and "Left out: ..." for rows that do nothing
- "Choose a direction: ..." where a both-ways row has the same time and different sizes
- "Type differs: ..." and "Unreadable ...: ...", which are never resolved for you

Words come first and no color or arrow carries meaning. The rows are meant to be read down the list as they are.

Two notes can follow the summary. "6 changes cannot be undone: no trash there." says how many steps would delete for good. "The compare was cancelled: this plan is partial." says the compare did not finish.

### Changing one row

Space on a row leaves it out, or takes it back in, and says the row's new line once: "Left out: plan.md, would copy there". Enter on a row opens a short list of what can change on it:

- "Leave out", which skips the row, and "Take back in", which restores it
- "Copy toward the other panel" and "Copy toward this panel", to choose a direction. These two commands have no default keys, since no pair of keys is free in all five presets. They are in this list and the palette, and [customizing keys](customizing-keys.md) can give them keys (`sync.toward-here`, `sync.toward-there`).
- "Put back the suggestion", to undo your change on that row

The new line of the row is read back after each change. The plan's last rows are the actions:

- "Synchronize now: 2 items" runs the plan.
- "Mode: Update the other panel" opens the modes again.

Changing the tolerance or the hour shift setting after the compare blocks the run until you Compare again: "Compare again: the options changed".

When the panels' folder watchers report a change under either compared folder, the result and the plan show a first row, "Result is old: folders changed", and it is said once. Synchronize now then refuses: "Result is old: folders changed. Compare again first." Choose Compare again, then plan again. Only the changes the panels' own watchers see count; a change in a folder neither panel shows is not noticed, and the run's own check still skips a step whose file changed.

## The question

"Synchronize now" first asks the plugins' hooks, if any are on: a `before_copy` hook is asked about the copies and a `before_delete` hook about the removals ([hooks](hooks.md#synchronize-and-batch-rename) says how). Then comes the question. It starts on No, and it lists the removals first when there are any:

"Synchronize 3 items? No is the default. y or n"

- n or Escape: "Nothing was synchronized."
- y: "Synchronize started: 3 items".
- With nothing to do: "Nothing to synchronize."

## The run

The run is one job in the Jobs list, with one undo record. Cancel stops it and the rest is left as it was. Ctrl+Z undoes the whole synchronize in one step, and what went to the trash comes back from it.

Before it writes, the job reads again every place it will write over or remove. A step whose file changed after the compare is skipped, and says why: "changed since the compare; compare again". A name that is taken since then is skipped too, so a synchronize never overwrites what the compare did not see.

Replacing a file sends the old one to the trash first. A removal goes to the trash. Where the place has no trash, such as some remote places, the row says "Delete for good there" before you answer, and the notes count them.

Nothing here is a hidden step: the plan is what runs, in this order: the copies there, the copies here, each folder before what is inside it, and then the removals.

## From a shell

`diro sync HERE THERE` compares two folders and synchronizes them from the plan. The first place is here and the second is there.

```bash
diro sync --dry-run photos backup
```

- `--mode MODE`: `update` (the default) copies what is missing or newer here to there. `both` copies each way and the newer wins. `mirror` makes there the same as here, with every removal listed and sent to the trash.
- `--by WHAT`: `time` (size and time, the default), `size` or `content`.
- `--tolerance SECONDS`: times this close count as the same. 2 by default.
- `--dry-run` or `-n`: print the plan and change nothing. The plan is lines, problems first, in the same short words as the window: "Copy there: plan.md, only here", "Replace there: notes.txt, newer here", "Trash there: old.txt, only there".
- `--yes` or `-y`: run without asking. Without it the question starts on No: "Synchronize 2 items? y or n". With nobody to answer: "Nothing synchronized. Add --yes to run."

Nothing is replaced or removed for good from a shell. A plan that would change a place with no trash is refused before anything runs: "Refused: some changes cannot be undone." The run reads every place again first, as above, and Ctrl+C stops it.

The exit codes are 0 for done or a dry run, 1 for could not run, refused, a No, or nothing done, 2 for wrong use and 3 for partly done, where the output says what was not.

## Limits

- The compare and the run start at once. They do not wait in the queue behind other jobs.
- Compare progress is spoken but does not update the Jobs list's line.
- "By contents" is covered by the engine's tests, and has not been tried by hand on a large tree.
- No rsync engine, no saved synchronize tasks and no Save as script yet.
- The staleness notice depends on the panels' folder watchers, as said above.
- The two "copy toward" commands have no default keys.

[Known limits](known-limits.md#compare-folders-and-synchronize) has the same list.

## See also

- [Compare files and folders](compare.md)
- [Duplicates](duplicates.md)
- [Hooks](hooks.md)
- [Volumes and the trash](volumes-trash.md)
- [Customizing keys](customizing-keys.md)
- [Every key](keyboard.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
