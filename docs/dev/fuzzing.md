# Fuzzing

diroxide's fuzz targets feed random input to the code that reads what other people make: file names, filters and patterns, archives, the highlighter, the components list, the plugin protocol and manifests, script headers, and the SFTP plugin's packets. A target may refuse its input with an error, but it must never panic, hang, leak, or run out of memory. The targets are built with cargo-fuzz, which links libFuzzer and AddressSanitizer, and need a nightly Rust toolchain.

They live in two Cargo workspaces of their own, `fuzz/` and `plugins/diro-sftp/fuzz/`, so the main build, `cargo test` and `cargo install diro` never compile them: libFuzzer is C++, and the main workspace keeps to the pure-Rust goal (plan section 4). Most targets are a few lines that call a plain function in the crate's own `fuzzing` module, so the same code also runs on its seeds in the crate's normal tests.

## The targets

In `fuzz/`:

- `names`: name checks, display names and the name rules for Windows and the rest.
- `filter_pattern`: globs, regular expressions with their limits, and the first rung's patterns.
- `filter_parse`: the filter language's parser; a filter that reads must read back the same.
- `archive_names`: member names and places inside archives.
- `archive_sniff`: the archive signature sniffer and its agreement with the name.
- `archive_zip`: zip reading under the strict limits.
- `archive_tar`: plain tar reading under the strict limits.
- `archive_stream`: compressed tar streams under the output cap.
- `archive_xz`: xz decoding under the memory cap, and the provider's checks of an xz file.
- `archive_7z`: 7z reading, with the coders' pre-scan and passwords.
- `archive_roundtrip`: diroxide's archive writers against its readers.
- `highlight_hang`: the highlighter, which must never hang; the first byte picks the language.
- `component_manifest`: the optional components list, `components.toml`.
- `plugin_framing`: the plugin protocol's line reader and message parser.
- `manifest_toml`: the plugin manifest reader, `plugin.toml`, for each system.
- `script_header`: the header of a one-file script command.

In `plugins/diro-sftp/fuzz/`:

- `sftp_packet`: the SFTP codec, as a stream of packets in both directions.

## Run a target on your computer

Install the nightly toolchain and cargo-fuzz once. The repository's `rust-toolchain.toml` stays as it is: `+nightly` picks the nightly toolchain for one command.

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
```

Then, from the repository root, run a target for a minute:

```bash
cargo +nightly fuzz run --fuzz-dir fuzz names -- -max_total_time=60
```

It runs until it finds a problem, the time is up, or you press Control C. To start from the target's seeds, give two folders: the first, which collects new finds and which git ignores, and the seeds folder, which is only read:

```bash
cargo +nightly fuzz run --fuzz-dir fuzz archive_zip fuzz/corpus/archive_zip fixtures/archives/seeds/archive_zip -- -max_len=1048576 -max_total_time=60
```

The SFTP target runs the same way from its own folder:

```bash
cargo +nightly fuzz run --fuzz-dir plugins/diro-sftp/fuzz sftp_packet plugins/diro-sftp/fuzz/corpus/sftp_packet fixtures/sftp/seeds/sftp_packet -- -max_total_time=60
```

`cargo +nightly fuzz list --fuzz-dir fuzz` lists the targets. Some take limits, written in the comment at the top of each target and used by the nightly run as well: `archive_zip`, `archive_7z` and `archive_roundtrip` take `-max_len=1048576`; `archive_stream` and `archive_xz` take `-max_len=262144`; `highlight_hang` takes `-timeout=2`, the viewer's deadline.

The seeds folders: `fixtures/archives/seeds/TARGET` for the archive targets, `fixtures/plugins/seeds/plugin_framing`, `fixtures/plugins/manifests`, `fixtures/scripts/headers`, `fixtures/components/manifests`, `fixtures/viewer/code` for the highlighter, and `fixtures/sftp/seeds/sftp_packet`.

### On Windows

cargo-fuzz works with the MSVC toolchain, with two differences:

- For the targets in `fuzz/`, add `--no-include-main-msvc` after `run` or `build`. One of the archive readers' dependencies is also built as a DLL, which cannot link with the argument cargo-fuzz otherwise gives every crate; `fuzz/build.rs` gives it to the targets alone. The SFTP target needs no flag.
- Run from a Developer PowerShell for Visual Studio, or put the folder holding `clang_rt.asan_dynamic-x86_64.dll` on PATH (it is in Visual Studio's MSVC tools, under `bin\HostX64\x64`). Without it a target stops at once.

A panic on Windows ends the run without saving its input: Rust's abort is a fast fail there, which libFuzzer cannot catch, and the exit code is 0xC0000409 (Git Bash shows 127). The log still names the panic and its line. Run again with the seed the log printed at the start (`-seed=N`), or reproduce on Linux, where every crash is saved; [the development container](docker.md) is Linux. Leaks are found only on Linux.

## Where a crash input goes

When a target finds a problem, cargo-fuzz saves the input that caused it in `fuzz/artifacts/TARGET/` (or `plugins/diro-sftp/fuzz/artifacts/TARGET/`), named for the kind of problem and a hash: `crash-` for a panic or a memory error, `timeout-` for an input that ran too long, `leak-` for memory never freed, and `oom-` for running out of memory. It prints the command that runs that input again. Git ignores both `artifacts` folders, and the `corpus` and `target` folders beside them.

To run one input again:

```bash
cargo +nightly fuzz run --fuzz-dir fuzz archive_xz fuzz/artifacts/archive_xz/crash-1234abcd
```

Then fix the bug, and keep the input as a regular test in the crate it came from, so the bug stays fixed. For the archive targets that is a case in `crates/diroxide-archive/tests/it/`: the seeds under `fixtures/archives/seeds/` are generated and checked against their generators, so a file dropped there is not replayed. For the plugin protocol's framing, a file in `fixtures/plugins/seeds/plugin_framing/` replays in the crate's normal tests.

## The nightly run

`.github/workflows/fuzz.yml` runs every night at 09:17 UTC, and by hand from the Actions tab (Fuzz, then Run workflow), where it asks how many seconds each target runs. It never runs on a push.

- One job builds every target once, on Linux, with the nightly toolchain, AddressSanitizer and debug assertions, so arithmetic overflow is a crash too. It starts from the main workspace's `Cargo.lock` and the plugin's, so the versions fuzzed are the versions that ship. The targets are read from the two manifests, so a new target is fuzzed the next night with no change to the workflow.
- One job per target runs it for ten minutes, from its corpus of earlier nights and its seeds. Each target's corpus is cached under its own name, and saved after a failure too.
- A target that finds a problem fails its job and uploads the artifact `fuzz-crash-TARGET`: the input, the run's log `fuzz.log` with the stack trace near its end, and `README.txt` with the reproduce line in words.
- The last job writes the run's summary: one line per target, failures first, the meaning first, such as "Pass: archive_zip, 10 minutes, 41,200 runs" or "Fail: archive_7z, crash, input uploaded, after 1,234 runs".

To replay a nightly crash, download its artifact from the run's page, or with `gh run download RUN-ID --name fuzz-crash-TARGET`, save the input in the target's `artifacts` folder, and run the command in its `README.txt`.

## By day

A change that stops a target compiling is caught on its push, not at night: the CI job "Fuzz targets compile" in `ci.yml` checks both fuzz crates with the pinned toolchain. The same check on your computer:

```bash
cargo check --manifest-path fuzz/Cargo.toml --bins
cargo check --manifest-path plugins/diro-sftp/fuzz/Cargo.toml --bins
```

## Add a target

1. Write the body as a plain function in the crate's `fuzzing` module, with a test that replays its seeds, so normal CI runs it with no fuzzer.
2. Add a file under `fuzz_targets/` that calls it, and a `[[bin]]` entry in the manifest with `test = false`, `doc = false` and `bench = false`.
3. Put its seeds in `fixtures/AREA/seeds/TARGET/`; the nightly run finds a seeds folder named for the target by itself. Any limits go in the target's comment and in the `case` lines of `fuzz.yml`'s run step.

## See also

- [Building](building.md)
- [Docker](docker.md)
- [Security](../security.md)
- [The plan, section 6](../plan.md)
