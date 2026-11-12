# The bench

The bench measures what a two-panel file manager does most: list a large folder, read every entry's details, and copy. It is the gate of the plan's principle "measure, then optimize". It writes its own trees under the target folder, runs each scenario three times, says the median in words, and keeps the numbers in a JSON file. It never touches a real folder, the trash or the network, and plays no sound.

## Running it

The budgets are for an optimized build, so run the bench in release:

```bash
cargo run --release -p xtask -- bench
```

`cargo xtask bench` runs it in the debug build. The numbers are slower and are never compared with an optimized baseline; it says so on its second line.

A quick run on small trees, for a first look and for the tests:

```bash
cargo xtask bench --quick
```

One scenario only:

```bash
cargo run --release -p xtask -- bench --only copy
```

Several, with commas and no spaces:

```bash
cargo run --release -p xtask -- bench --only copy,copy-system
```

The GUI's budgets beside the scenarios (see "The GUI's budgets" below), or alone:

```bash
cargo run --release -p xtask -- bench --gui
```

```bash
cargo run --release -p xtask -- bench --only gui
```

`--cold-build` adds a cold release build of `diro` to the GUI's budgets, and implies `--gui`:

```bash
cargo run --release -p xtask -- bench --gui --cold-build
```

The full run needs about 20 GB free under the target folder while it copies (the 10 GB source and one copy at a time), and removes its trees when it ends; a cold build leaves a full release build in `target/bench/cold-build`, for `.claude/tools/safe-remove.ps1`.

## What it measures

Each line of the output starts with the scenario and its median, then the budget where there is one. The next line gives every run and one more fact.

- `list`: list a folder of 100,000 empty files through the local provider. R8's starting budget is 300 ms on Linux and macOS and 800 ms on Windows, warm.
- `stat`: read the details of each of the 100,000 entries by its place, without a listing. It starts from the same budgets.
- `list-stat`: list the folder, then read each entry's details, as a panel that fills its columns late does.
- `copy`: copy a 10 GB file through the job engine, planned and run, with the default options, the plan's "copy 10 GB" (it was 1 GB until the alpha 3 baseline). The second line gives the rate.
- `copy-system`: copy the same file with the system's own copy call, for reference. The gap between the two is the engine's cost.
- `slow-place`: copy 40 files from the slow place on the fake provider: every call waits 2 ms and one call, 80 calls into the run, stalls for 400 ms. The second line says in how many runs the job said it had stalled; it should be every run.
- `archive-zip`, `archive-tar` and `archive-7z`: list a zip, a tar and a 7z of 100,000 empty members through the archive provider, timed from the open to the last entry: opening the archive, building its index and listing every member. Each run opens a new provider, so no run reads another's cached index. The archives are packed first by diroxide's own pack runner, outside the timing.
- `unpack`: unpack a zip holding one 200 MB file through the unpack runner, planned and run, into a new folder each run. The second line gives the rate.
- `viewer`: open a 256 MB text log in the viewer and draw its first screen of 24 rows, in screen-reader mode, through the app's viewer itself.
- `search-name`: search the 1 GB search tree (100,000 files in 5,000 folders, written once per run from `bench/search-tree.toml`, the small one with `--quick`) for the 1,000 files whose names hold its hit word, through the search engine itself, as the app runs it. The plan's starting budget is 0.6 seconds warm on Linux and 1.5 seconds on Windows.
- `search-content`: search the same tree's contents for the hit word, the same way: 1,000 files, the binaries skipped. The starting budget is 1.5 seconds on Linux and 3.75 seconds on Windows.

- `details`: read the details of 1,000 files, half small pictures and half text, through the app's own details read, as the details list reads them, with no helper program found. The second line counts the rows read.
- `checksum`: the SHA-256 of a file of 1 GB, as Checksum and `diro sum` compute it. The second line gives the rate.
- `convert-plan`: plan converting 100 Markdown files to plain text with carta's built-in recipe, without running it: the plan `diro convert --dry-run` says.
- `columns`: the app opening a folder of 100,000 empty files with three content columns (`text.lines`, `audio.artist`, `image.dimensions`), from its start until it is idle: the listing, the rows with their "reading" cells, and the cells read near the focus.

The search scenarios call the search engine directly, since xtask may depend on the search crate (W3s's request 2); each checks that it found exactly the tree's hits. The four alpha 4 scenarios go through the app and the job engine, which xtask may depend on, and so measure the program's own code.

## The GUI's budgets

`--gui` adds four measures of the window (R27's commit 35, `xtask/src/budgets.rs`), said and recorded beside the scenarios and compared with the baseline in words, never gated:

- `binary-size`: the size of the `diro` the bench built, the release profile for an optimized bench.
- `gui-start`: from the start of `diro --gui --background` to the window's first frame, which the window says in its `--log` file ("graphics: drawing with"); the median of the bench's three starts. Each start is one real window, off screen, never activated and with no taskbar button, on a copy of the program in the bench's work folder with `--portable`, so its settings and state are the run's own; the program is ended once the frame is there.
- `gui-working-set`: the program's peak working set a second after its first frame, the median of the same starts.
- `cold-build`, with `--cold-build` only: `cargo build --release -p diro` into `target/bench/cold-build`, with no compiler cache. The folder must not exist yet, so the build is cold.

Where there is no display (Linux with neither `WAYLAND_DISPLAY` nor `DISPLAY`), the start and the working set are skipped, and said.

The archive scenarios call the archive provider and its runners directly, since xtask may depend on the archive crate (P2b, W2g's request 1); before that they ran the `diro` binary, so the first figures below include starting `diro` and writing every path, and the next baseline is the one to compare with.

The sizes and budgets are in `bench/params.toml`; `--quick` uses its small sizes. The new scenarios have no budgets yet: they report, and the ratchet compares them with the baseline once one is saved.

## Where the numbers go

Every run writes `target/bench/result.json` (under `CARGO_TARGET_DIR` when it is set): the platform, the day from the clock in UTC and the minute the run started, the machine, the versions, whether the sizes were quick, whether the build was optimized, each scenario's runs, median, budget and note, and the GUI's budgets when `--gui` measured them. Its first lines say the same:

```text
bench: started 2026-11-03 17:38 UTC.
bench: machine AMD Ryzen 5 5600G with Radeon Graphics, 6 cores, 12 threads, 64 GB, Windows 11 Home, build 26200; the trees on Samsung SSD 990 EVO 2TB, NVMe.
bench: built diroxide 0.1.0-alpha.2 at 160467c, rustc 1.96.1 (31fca3adb 2026-06-26).
```

The machine is named by its models only: the processor, its cores and threads, the memory, the system with its build or kernel, and the disk that holds the trees (its model and bus on Windows, its kind and file system elsewhere). Never the host name, an account or a volume label, since the baseline is committed and published. The versions are the workspace's, the commit the bench was built from ("with changes" when tracked files differed from it) and the compiler of the pinned toolchain.

`bench/baseline.json` keeps one optimized full run per platform (`windows`, `linux`, `macos`). To make the current run the baseline for this platform, with the GUI's budgets and a cold build, as the alpha 3 baseline was saved:

```bash
cargo run --release -p xtask -- bench --save-baseline --gui --cold-build
```

A baseline is saved only from an optimized, full run of every scenario, on a quiet machine: nothing else building, no check running, one bench at a time. A run saved without `--gui` keeps the GUI's budgets of the baseline it replaces, and says so, so the scenarios can be saved again without opening a window.

## The alpha 4 baseline

Saved on Saturday, November 7, 2026, from 5:24 to 5:31 PM Pacific time (the run's clock said 2026-11-08 01:24 UTC as it started), with `cargo run --release -p xtask -- bench --save-baseline`, the GUI's budgets kept from the alpha 3 baseline (no window was opened). The machine was quiet: no cargo or rustc ran, the processor was 1 to 10 percent busy before the start, and the bench ran alone. An earlier full run that afternoon, from 4:38 to 4:46 PM, was not saved: just after it a background download and the browser kept the processor a quarter to a third busy, and it put listing at 168 ms and the name search at 619 ms.

- The machine: the owner's Windows machine, an AMD Ryzen 5 5600G with Radeon Graphics, 6 cores and 12 threads, 64 GB of memory, Windows 11 Home build 26200, the trees on a Samsung SSD 990 EVO 2TB on NVMe.
- The versions: diroxide 0.1.0-alpha.3, alpha 4's work before its version bump, at commit 9a88c25 with this page's changes only, built with rustc 1.96.1 in the release profile.

The medians of three runs each:

- List 100,000 entries: 134 ms, within the 800 ms budget.
- Stat 100,000 entries: 6.66 s, over the 800 ms budget.
- List, then stat each of 100,000: 6.81 s.
- Copy 10 GB through the engine: 21.7 s, 461 MB a second, and with the system call 24.3 s, 411 MB a second: each the median of the three full runs of "The band" below, as R30's commit 115 asks. The saved run's own copies took 16.1 s (runs of 11.3, 16.1 and 28.2 s) and 24.8 s (24.8, 25.8 and 13.6 s).
- Copy 40 files from a slow place: 1.37 s, the stall said in 3 of 3 runs.
- List 100,000 entries in a zip: 222 ms; in a tar: 229 ms; in a 7z: 180 ms.
- Unpack a zip of 200 MB: 429 ms, 466 MB a second.
- View the first screen of a 256 MB log: 9.4 ms.
- Search 100,000 files by name: 72 ms, within the 1,500 ms budget.
- Search 995 MB of files by contents: 1.27 s, within the 3,750 ms budget.
- Read the details of 1,000 files: 334 ms, 3,000 rows.
- The SHA-256 of a file of 1 GB: 610 ms, 1.6 GB a second.
- Plan converting 100 files, a dry run: 13 ms.
- List 100,000 entries with three columns, the app from its start until idle: 1.53 s.

Against the alpha 3 baseline every scenario was within its band but the system's copy, 46 percent slower than its 17.0 s of November 3, which "The band" explains; listing was 11 percent slower and stat 1 percent.

## The alpha 3 baseline

Saved on Tuesday, November 3, 2026, from 9:38 to 9:51 AM Pacific time (the run's clock said 17:38 UTC as it started), with `cargo run --release -p xtask -- bench --save-baseline --gui --cold-build`. The machine was quiet: the orchestrator's checks on `main` were done, no other build or check ran, and the bench ran alone. As the run recorded them:

- The machine: the owner's Windows machine, an AMD Ryzen 5 5600G with Radeon Graphics, 6 cores and 12 threads, 64 GB of memory, Windows 11 Home build 26200, the trees on a Samsung SSD 990 EVO 2TB on NVMe. The window drew on the graphics card, an NVIDIA GeForce RTX 3060.
- The versions: diroxide 0.1.0-alpha.2 at commit 160467c, alpha 3's work before its version bump, built with rustc 1.96.1 in the release profile.

The medians of three runs each:

- List 100,000 entries: 121 ms, within the 800 ms budget.
- Stat 100,000 entries: 6.57 s, over the 800 ms budget.
- List, then stat each of 100,000: 6.71 s.
- Copy 10 GB through the engine: 19.9 s, 504 MB a second; the runs took 8.86, 19.9 and 24.5 s.
- Copy 10 GB with the system call: 17.0 s, 589 MB a second; the runs took 17.0, 27.7 and 14.4 s.
- Copy 40 files from a slow place: 1.41 s, the stall said in 3 of 3 runs.
- List 100,000 entries in a zip: 216 ms; in a tar: 217 ms; in a 7z: 173 ms.
- Unpack a zip of 200 MB: 365 ms, 547 MB a second.
- View the first screen of a 256 MB log: 8.2 ms.
- Search 100,000 files by name: 77 ms, within the 1,500 ms budget.
- Search 995 MB of files by contents: 1.44 s, within the 3,750 ms budget.

The GUI's budgets, recorded and never gated:

- The binary: 44.1 MB, the release build.
- Start to the first frame: 1.19 s; the starts took 1.71, 1.19 and 1.15 s.
- The working set at the first frame: 188.0 MB.
- A cold release build of `diro`: 5 minutes 6 seconds.

A second run straight after it, from 9:51 to 9:58 AM, not saved, put every scenario within 9 percent of this baseline but the two copies: the copy through the engine took 14.7 s (26 percent faster) and the system's copy 21.4 s (26 percent slower). A 10 GB copy outruns the disk's own write cache and the system's file cache, so its single runs spread from under 5 to almost 38 seconds on this machine; its median moves further than the 20 percent band from one run to the next, and `--gate` would fail on it by chance. The GUI's budgets moved by 2 percent or less.

The baseline it replaced, saved on Friday, October 16, 2026, had the 1 GB copy and none of the archive, viewer and search scenarios. Against it, the listing was 27 percent slower (95 to 121 ms), reading every entry's details 29 percent slower (5.08 to 6.57 s), listing and then reading them 8 percent faster, and the slow place 9 percent faster.

## The band

The ratchet's band is 20 percent (`tolerance_percent` in `bench/params.toml`): a median further than that from the baseline is said, and fails a run with `--gate`. A scenario may have a band of its own under `[band_percent]`.

The two 10 GB copies have 40 percent. On Saturday, November 7, 2026, from 4:24 to 4:33 PM Pacific time, three full runs of both copies in a row (`--only copy,copy-system`, each the median of three) on a quiet machine, nothing else building, gave:

- Through the engine: medians of 21.7, 27.5 and 21.7 s; single runs from 17.3 to 36.2 s.
- With the system call: medians of 28.2, 22.2 and 24.3 s; single runs from 17.4 to 29.5 s.

With the alpha 3 runs of Tuesday, November 3 (19.9 and 14.7 s through the engine, 17.0 and 21.4 s with the system call), every quiet median lies within 40 percent of the median of the three, 21.7 and 24.3 s, and none within 20 percent of all the others. A 10 GB copy outruns the disk's write cache and the system's file cache, so where a run lands depends on what the caches held when it started. Every other scenario keeps 20 percent.

## The November 3 slowdown, found

The alpha 3 baseline found listing 27 percent slower and stat 29 percent slower than the first baseline of October 16, 2026, and left it. It is the code, not the machine. On Saturday, November 7, 2026, from 4:20 to 4:24 PM Pacific time, the bench of October 16's tree (commit 20e331e, built in release from its own lock) and today's ran one after the other, twice each, on the same disk with nothing else building:

- List 100,000 entries: October 16's code 97 and 97 ms; today's 128 and 128 ms.
- Stat 100,000 entries: October 16's code 4.39 and 4.41 s; today's 6.67 and 6.66 s.

The October 16 numbers were the Phase 0 local provider's, before W1a's provider replaced it the same day. Today's stat opens each file twice: `std::fs::symlink_metadata` opens it once, then `fill_from_handle` opens it again to read the volume and file id, the number of links and the change time, three more system calls (about 23 microseconds a file on this machine). Today's listing builds each entry's full path for the link check, checks the name against the unfinished copies' pattern, and reads the platform's attributes and the online-only fact, about 0.3 microseconds more an entry. Both are what the details view and the copy ladder use; making the second open lazy, or the path only for links, is the next alpha's to weigh.

## The listing and stat costs, removed

Alpha 5's PF lane removed both causes above on Thursday, November 12, 2026, with nothing said or shown differently:

- A stat opens each entry once on Windows. The one handle gives the metadata, as `symlink_metadata` reads it, and the identity, the link count and the change time, which a second open read before. Every stat still gives the identity, so the engine's same-entry checks, case-only renames and hard links read what they read before. Where the entry cannot be opened for its attributes, the standard library's lookup answers. Unix keeps its one `lstat`.
- A listing builds an entry's full path only for a link's target. The name checks a listing makes of every entry (needs care, an archive extension) allocate nothing for an ASCII name, and give the same answers, tested name by name against the full check.

The tests in `crates/diroxide-vfs/src/local/mod.rs` count, through the provider, what each entry costs: a listing opens nothing and builds no path but a link's, and a stat opens each entry once on Windows and reads one `lstat` elsewhere.

No quiet minute could be found that day: seven other lanes were building, 6 to 17 cargo and 3 to 20 rustc processes at once. So the old and new code ran back to back, from 10:32 AM to 11:53 AM Pacific time, the old one built at 57bb7d8, each `cargo run --release -p xtask -- bench --only list,stat`, the median of three runs, with the load read before each:

- Old, 8 cargo and 5 rustc: list 648 ms, stat 59.0 s.
- New, 6 cargo and 14 rustc: list 516 ms, stat 30.8 s.
- Old, 15 cargo and 6 rustc: list 1.05 s, stat 65.0 s.
- New, 17 cargo and 5 rustc: list 194 ms, stat 20.6 s.
- Old, 17 cargo and 3 rustc: list 328 ms, stat 18.2 s.
- New, 15 cargo and 6 rustc: list 161 ms, stat 12.1 s.

Each new run, beside the old one just before it, listed in 80, 18 and 49 percent of the old time, and read every entry's details in 52, 32 and 66 percent; the load moved under them, which is the spread. A smaller timing on one tree of 10,000 files, the two ways of a stat in alternating rounds under the same load, put one open at 57 to 85 percent of two (a median of 60). The absolute numbers say only the load: the quiet baseline of November 7 was 134 ms and 6.66 s. Whether listing and stat are back at October 16's 97 ms and 4.4 s is for a quiet machine to say, with the next saved baseline.

## The archive scenarios' first figures

Measured on the owner's Windows machine on Saturday, October 17, 2026, in an optimized full run, with another builder at work beside it; the median of three runs each:

- List 100,000 entries in a zip: 265 ms.
- List 100,000 entries in a tar: 266 ms.
- List 100,000 entries in a 7z: 263 ms.
- Unpack a zip of 200 MB: 500 ms, 400 MB a second.
- View the first screen of a 256 MB log: 8.6 ms.

The same run put the Wave 1 scenarios between 12 and 39 percent off the saved Windows baseline in both directions, which is the other builder's noise, so the baseline was not saved from it. It was saved on a quiet machine on Tuesday, November 3, 2026 (see "The alpha 3 baseline" above).

## The ratchet

After the run, each median is compared with this platform's baseline, when one exists for the same build and the quick or full sizes: "Baseline 380 ms: 8 percent slower, inside the 20 percent band." The band is `tolerance_percent` in `bench/params.toml`. A scenario's label holds its sizes, so a baseline taken at other sizes is not compared: "Baseline not compared: it was for Copy 1.0 GB through the engine." The GUI's budgets are compared in words and never gated.

Without `--gate` the ratchet only reports. With it, a median outside the band in either direction fails the run: slower is a regression to fix, faster is a new baseline to save.

```bash
cargo run --release -p xtask -- bench --gate
```

CI does not run the bench: a shared runner's noise is wider than the band, and the baseline is this machine's. Run `--gate` on the machine the baseline came from, quiet.

## The trees

The listing's folder and the copied file are written in a work folder under `target/bench/` and removed when the run ends; generated trees are never committed. The same generators write the owner's check folder and the search tree, each only below a folder the caller names and never on drive C:

```bash
cargo xtask sample-folder D:\dirocheck
```

```bash
cargo xtask search-tree D:\searchtree
```

The search tree is the 1 GB, 100,000-file tree of the plan's alpha.3 search spike, with a known number of hits; its parameters are in `bench/search-tree.toml`, and `--small` writes a 300-file one.

The demo set of the plan's alpha.2 check goes in a new or empty folder on drive D: with a plain name, and nowhere else:

```bash
cargo xtask demo D:/demo
```

It writes `course.zip`, `backup.7z` (solid), `logs.tar.xz`, `locked.zip` (AES-256, written by diroxide's own pack runner with a password made up as it runs and said once on its output), `hostile.zip` (one name in it would leave the archive; it is built in memory), `big.bin` (60 MB), `main.rs`, `report.md`, `notes.org`, `big.log` and an empty `out` folder, then lists each with its size and reads every archive back with `diro archive test` and with the unzip, xz, tar and 7z programs it finds on PATH.

## The search spike

The search lane's spike (the plan's alpha.3) times the walkers and the search on the 1 GB search tree, compares them with `rg` when `rg` is on PATH, and lists a slow place, a stand-in for a network share, at 1, 2, 4 and 8 threads. Write the tree once, then run the spike on it in an optimized build:

```bash
cargo xtask search-tree D:\searchtree
```

```bash
cargo run --release -p diroxide-search --example spike -- D:\searchtree --json target/bench/search-spike.json
```

The slow place is the local provider with a pause before every call (5 ms by default, `--delay-ms` to change it); no network share was reachable, and its lines say "a fake network place". The spike reads the tree and writes only the JSON file it is given.

Its figures are kept in `bench/search-spike.toml`, one `[[run]]` for each machine and each state of the search engine. The first, measured on the owner's Windows machine on Saturday, October 17, 2026, in an optimized build, warm, with nothing changed in Windows Security:

- Walk the 105,000 entries with `ignore`'s parallel walker, filters off: 45 ms, about 2.3 million a second.
- Walk them through the local provider on one thread: 348 ms.
- Search names for `*quokkaline*` through Phase 0's walk and the new query types: 382 ms, 1,000 found, the first in under a millisecond.
- Search `ext txt and size over 1 KB and modified within 3 years` the same way: 411 ms, 90,000 found.
- A slow place with 5 ms before every call: 3,859 items a second on 1 thread, 7,732 on 2, 15,423 on 4 and 30,646 on 8; a stop answered in 2.5 ms.
- Make 100,000 hits into a results list: 171 ms; 1,000,000: 1.8 seconds.

`rg` was not on PATH, so nothing was compared with it. The plan's starting budgets (a warm name search in 0.6 seconds on Linux and 2.5 times that on Windows, the first hit within 100 ms, 100,000 rows in under a second, the stop answered within a second) already hold; the parallel walker is the one to compare with them next.

The second, on Saturday, October 17, 2026, with the search engine of the search lane (the parallel walker, the evaluator, the content reader), the same tree and build, warm:

- Search names for `*quokkaline*`: 72 ms, the first hit at 1.3 ms; `ext txt and size over 1 KB and modified within 3 years`: 104 ms, 90,000 found.
- Read every file whole on the parallel walk, nothing matched: 1.5 seconds, the ceiling of a content search on this machine.
- Search contents for a word in 1,000 files: 1.7 seconds, the first hit at 3 ms; for a word in none: 1.7; for a word in every file: 2.1; a regular expression: 1.8; any case: 1.6. Opening files is what is left: the reader itself runs within the ceiling's reach.
- The search engine over the slow place, 5 ms before every call: 7,550 entries a second on 2 threads and 29,976 on 8, the first hit at about 29 ms; a stop answered in 0.8 ms.
- 100,000 hits made into a results list: 308 ms.

Every starting budget of the plan holds on Windows: names well under 1.5 seconds, contents under 3.75 seconds, the first hit under 100 ms, the stop under a second, and a network place over 1,000 items a second on 2 threads. `rg` was again not on PATH; the container run is where it is compared.

## See also

- [Building](building.md)
- [Architecture](architecture.md)
- [The plan](../plan.md)
