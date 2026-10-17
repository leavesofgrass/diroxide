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

## What it measures

Each line of the output starts with the scenario and its median, then the budget where there is one. The next line gives every run and one more fact.

- `list`: list a folder of 100,000 empty files through the local provider. R8's starting budget is 300 ms on Linux and macOS and 800 ms on Windows, warm.
- `stat`: read the details of each of the 100,000 entries by its place, without a listing. It starts from the same budgets.
- `list-stat`: list the folder, then read each entry's details, as a panel that fills its columns late does.
- `copy`: copy a 1 GB file through the job engine, planned and run, with the default options. The second line gives the rate.
- `copy-system`: copy the same file with the system's own copy call, for reference. The gap between the two is the engine's cost.
- `slow-place`: copy 40 files from the slow place on the fake provider: every call waits 2 ms and one call, 80 calls into the run, stalls for 400 ms. The second line says in how many runs the job said it had stalled; it should be every run.
- `archive-zip`, `archive-tar` and `archive-7z`: list a zip, a tar and a 7z of 100,000 empty members through the archive provider, timed from the open to the last entry: opening the archive, building its index and listing every member. Each run opens a new provider, so no run reads another's cached index. The archives are packed first by diroxide's own pack runner, outside the timing.
- `unpack`: unpack a zip holding one 200 MB file through the unpack runner, planned and run, into a new folder each run. The second line gives the rate.
- `viewer`: open a 256 MB text log in the viewer and draw its first screen of 24 rows, in screen-reader mode, through the app's viewer itself.
- `search-name`: search the 1 GB search tree (100,000 files in 5,000 folders, written once per run from `bench/search-tree.toml`, the small one with `--quick`) for the 1,000 files whose names hold its hit word, through the search engine itself, as the app runs it. The plan's starting budget is 0.6 seconds warm on Linux and 1.5 seconds on Windows.
- `search-content`: search the same tree's contents for the hit word, the same way: 1,000 files, the binaries skipped. The starting budget is 1.5 seconds on Linux and 3.75 seconds on Windows.

The search scenarios call the search engine directly, since xtask may depend on the search crate (W3s's request 2); each checks that it found exactly the tree's hits.

The archive scenarios call the archive provider and its runners directly, since xtask may depend on the archive crate (P2b, W2g's request 1); before that they ran the `diro` binary, so the first figures below include starting `diro` and writing every path, and the next baseline is the one to compare with.

The sizes and budgets are in `bench/params.toml`; `--quick` uses its small sizes. The new scenarios have no budgets yet: they report, and the ratchet compares them with the baseline once one is saved.

## Where the numbers go

Every run writes `target/bench/result.json` (under `CARGO_TARGET_DIR` when it is set): the platform, the day from the clock in UTC, whether the sizes were quick, whether the build was optimized, and each scenario's runs, median, budget and note.

`bench/baseline.json` keeps one optimized full run per platform (`windows`, `linux`, `macos`). To make the current run the baseline for this platform:

```bash
cargo run --release -p xtask -- bench --save-baseline
```

A baseline is saved only from an optimized, full run of every scenario.

## The archive scenarios' first figures

Measured on the owner's Windows machine on Saturday, October 17, 2026, in an optimized full run, with another builder at work beside it; the median of three runs each:

- List 100,000 entries in a zip: 265 ms.
- List 100,000 entries in a tar: 266 ms.
- List 100,000 entries in a 7z: 263 ms.
- Unpack a zip of 200 MB: 500 ms, 400 MB a second.
- View the first screen of a 256 MB log: 8.6 ms.

The same run put the Wave 1 scenarios between 12 and 39 percent off the saved Windows baseline in both directions, which is the other builder's noise, so the baseline was not saved from it. Save it on a quiet machine, with nothing else building:

```bash
cargo run --release -p xtask -- bench --save-baseline
```

## The ratchet

After the run, each median is compared with this platform's baseline, when one exists for the same sizes and build: "Baseline 380 ms: 8 percent slower, inside the 20 percent band." The band is `tolerance_percent` in `bench/params.toml`.

In Wave 1 the ratchet only reports. From Wave 2, once two baselines exist, CI runs the bench with `--gate`, which fails when a median leaves the band in either direction: slower is a regression to fix, faster is a new baseline to save.

```bash
cargo run --release -p xtask -- bench --gate
```

## The trees

The listing's folder and the copied file are written in a work folder under `target/bench/` and removed when the run ends; generated trees are never committed. The same generators write the owner's check folder and the search tree, each only below a folder the caller names and never on drive C:

```bash
cargo xtask sample-folder D:\dirocheck
```

```bash
cargo xtask search-tree D:\searchtree
```

The search tree is the 1 GB, 100,000-file tree of alpha.3's search spike, with a known number of hits; its parameters are in `bench/search-tree.toml`, and `--small` writes a 300-file one.

The demo set of the alpha.2 check goes in a new or empty folder on drive D: with a plain name, and nowhere else:

```bash
cargo xtask demo D:/demo
```

It writes `course.zip`, `backup.7z` (solid), `logs.tar.xz`, `locked.zip` (AES-256, written by diroxide's own pack runner with a password made up as it runs and said once on its output), `hostile.zip` (one name in it would leave the archive; it is built in memory), `big.bin` (60 MB), `main.rs`, `report.md`, `notes.org`, `big.log` and an empty `out` folder, then lists each with its size and reads every archive back with `diro archive test` and with the unzip, xz, tar and 7z programs it finds on PATH.

## The search spike

The search lane's spike (alpha.3) times the walkers and the search on the 1 GB search tree, compares them with `rg` when `rg` is on PATH, and lists a slow place, a stand-in for a network share, at 1, 2, 4 and 8 threads. Write the tree once, then run the spike on it in an optimized build:

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
