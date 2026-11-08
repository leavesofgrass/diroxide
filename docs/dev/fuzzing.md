# Fuzzing

diroxide's fuzz targets feed random input to the code that reads what other people make: file names, filters and patterns, archives, the highlighter, picture headers and document containers, the metadata readers, the components list, the plugin protocol and manifests, script headers, the SFTP plugin's packets, the disk image plugin's partition tables and file systems, and the git plugin's reading of git's output. A target may refuse its input with an error, but it must never panic, hang, leak, or run out of memory. The targets are built with cargo-fuzz, which links libFuzzer and AddressSanitizer, and need a nightly Rust toolchain.

They live in five Cargo workspaces of their own, `fuzz/`, `plugins/diro-sftp/fuzz/`, `plugins/diro-disk/fuzz/`, `plugins/diro-git/fuzz/` and `plugins/diro-crypt/fuzz/`, so the main build, `cargo test` and `cargo install diro` never compile them: libFuzzer is C++, and the main workspace keeps to the pure-Rust goal (plan section 4). Most targets are a few lines that call a plain function in the crate's own `fuzzing` module, so the same code also runs on its seeds in the crate's normal tests.

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
- `archive_slt`: the 7z tool backend's listing parser. The input is a `7z l -slt` listing, as 7-Zip prints it or as a hostile member name might twist it, parsed and indexed under the strict limits. Nothing may panic, every record must point at an item that exists, and no indexed path may climb out. The body is `diroxide_archive::fuzzing::slt`. It has no seeds folder: its samples are `SLT_SAMPLES` in `crates/diroxide-archive/src/fuzzing.rs`, replayed by the crate's tests. Its own comment asks for `-max_len=262144`.
- `highlight_hang`: the highlighter, which must never hang; the first byte picks the language.
- `preview_image_header`: the picture words' header readers, with small limits; a header that reads must give words.
- `preview_container`: the previews' read path for zip-based documents, through its member checks and its nesting cut, with small caps.
- `component_manifest`: the optional components list, `components.toml`.
- `plugin_framing`: the plugin protocol's line reader and message parser.
- `manifest_toml`: the plugin manifest reader, `plugin.toml`, for each system.
- `script_header`: the header of a one-file script command.
- `meta_file_bytes`: every built-in metadata reader, the Exif reader and the internet mark's readers, on any bytes as a file.
- `meta_checksum_list`: the checksum-list parser (`SHA256SUMS`, BSD tagged lists, `.sfv`); a name that climbs out of its folder is never taken, and a list written from what was read reads back the same.
- `meta_strip`: the strip into a copy, on any bytes as a picture; the copy holds nothing the strip takes out.

In `plugins/diro-sftp/fuzz/`:

- `sftp_packet`: the SFTP codec, as a stream of packets in both directions.

In `plugins/diro-disk/fuzz/`, nine targets for the disk image plugin. Each body is a plain function in the plugin's `fuzzing` module:

- `partition_table`: the MBR and GPT readers, every count and size bounded, a chain of logical partitions that points back ended.
- `iso`: the ISO 9660 reader, the record-claim guard, Joliet and Rock Ridge names, relocated folders.
- `fat`: the FAT reader behind its read-only wrapper: boot records, folders and cluster chains.
- `ext`: the ext reader: superblock, group descriptors, inodes, extents and block maps.
- `ntfs`: the NTFS reader: the boot sector, file records, indexes and data runs.
- `squashfs`: the squashfs reader: the superblock checks, then the tables.
- `container_header`: the headers of VHD, VHDX, VDI, VMDK and QCOW2, the claims bounded before any table is read.
- `seven_listing`: the parser of `7z l -slt` output, on any text.
- `image`: the whole open, container, table or file system, the root and one level listed.

In `plugins/diro-git/fuzz/`, four targets for the git plugin, each a plain function in its `fuzzing` module:

- `status_v2`: the porcelain version 2 status reader and the words of what it reads: renames, conflicts, submodules, names that are not UTF-8.
- `log_record`: the log record reader and a commit's name-status reader.
- `config_scan`: the trust scan over a configuration listing with scopes, what a hostile repository's own configuration could hold.
- `other_readers`: blame, the branch and stash lists, and progress lines.

In `plugins/diro-crypt/fuzz/`, three targets for the encryption plugin, each a plain function in its `fuzzing` module (`plugins/diro-crypt/src/fuzzing.rs`). They read what a hostile file or the host could hand the plugin, and none may panic, loop or allocate by a number it read:

- `recipients`: a recipients file or a pasted key, and the words of what it refuses: an SSH key, a plugin key, a hardware key, a line that is no key.
- `form_answer`: a form's answer as the host could send it, the passphrase form's and the recipient form's, with values of the wrong kind or missing.
- `header`: an age file's header, armored or not, how much work its passphrase asks for, and the names made from a file's name.

The encryption targets have no seeds folder. The plugin's tests in `tests/it/fuzz.rs` make their seeds at test time, replay every cut of each and copies with bytes bent, and add random bytes, so `cargo test` in the plugin runs them with no fuzzer.

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

`cargo +nightly fuzz list --fuzz-dir fuzz` lists the targets. Some take limits, written in the comment at the top of each target and used by the nightly run as well: `archive_zip`, `archive_7z` and `archive_roundtrip` take `-max_len=1048576`; `archive_stream`, `archive_xz` and `archive_slt` take `-max_len=262144`; `preview_image_header` and `preview_container` take `-max_len=1048576`; `highlight_hang` takes `-timeout=2`, the viewer's deadline.

The disk and git targets run the same way from their plugin's folder, and each plugin's own tests replay them. The git targets' seeds are in `plugins/diro-git/fixtures/seeds/TARGET`:

```bash
cd plugins/diro-git
cargo +nightly fuzz run status_v2 fuzz/corpus/status_v2 fixtures/seeds/status_v2
```

The disk targets' seeds are the gzipped fixture images in `plugins/diro-disk/fixtures`, made by `docker/disk/make-images.sh`. The plugin's tests in `tests/it/fuzz.rs` replay every target on each fixture and on cut and bent copies of it, so `cargo test` in the plugin runs them with no fuzzer. `cargo +nightly fuzz run iso` from `plugins/diro-disk` starts from an empty corpus.

The seeds folders: `fixtures/archives/seeds/TARGET` for the archive targets, `fixtures/plugins/seeds/plugin_framing`, `fixtures/plugins/manifests`, `fixtures/scripts/headers`, `fixtures/components/manifests`, `fixtures/viewer/code` for the highlighter, and `fixtures/sftp/seeds/sftp_packet`.

### On Windows

cargo-fuzz works with the MSVC toolchain, with two differences:

- For the targets in `fuzz/`, add `--no-include-main-msvc` after `run` or `build`. One of the archive readers' dependencies is also built as a DLL, which cannot link with the argument cargo-fuzz otherwise gives every crate; `fuzz/build.rs` gives it to the targets alone. The SFTP target needs no flag.
- Run from a Developer PowerShell for Visual Studio, or put the folder holding `clang_rt.asan_dynamic-x86_64.dll` on PATH (it is in Visual Studio's MSVC tools, under `bin\HostX64\x64`). Without it a target stops at once.

A panic on Windows ends the run without saving its input: Rust's abort is a fast fail there, which libFuzzer cannot catch, and the exit code is 0xC0000409 (Git Bash shows 127). The log still names the panic and its line. Run again with the seed the log printed at the start (`-seed=N`), or reproduce on Linux, where every crash is saved; [the development container](docker.md) is Linux. Leaks are found only on Linux.

## Where a crash input goes

When a target finds a problem, cargo-fuzz saves the input that caused it in `fuzz/artifacts/TARGET/` (or the same folder under `plugins/diro-sftp/fuzz/`, `plugins/diro-disk/fuzz/`, `plugins/diro-git/fuzz/` or `plugins/diro-crypt/fuzz/`), named for the kind of problem and a hash: `crash-` for a panic or a memory error, `timeout-` for an input that ran too long, `leak-` for memory never freed, and `oom-` for running out of memory. It prints the command that runs that input again. Git ignores both `artifacts` folders, and the `corpus` and `target` folders beside them.

To run one input again:

```bash
cargo +nightly fuzz run --fuzz-dir fuzz archive_xz fuzz/artifacts/archive_xz/crash-1234abcd
```

Then fix the bug, and keep the input as a regular test in the crate it came from, so the bug stays fixed. For the archive targets that is a case in `crates/diroxide-archive/tests/it/`: the seeds under `fixtures/archives/seeds/` are generated and checked against their generators, so a file dropped there is not replayed. For the plugin protocol's framing, a file in `fixtures/plugins/seeds/plugin_framing/` replays in the crate's normal tests.

## The nightly run

`.github/workflows/fuzz.yml` runs every night at 09:17 UTC, and by hand from the Actions tab (Fuzz, then Run workflow), where it asks how many seconds each target runs. It never runs on a push.

- One job builds every target once, on Linux, with the nightly toolchain, AddressSanitizer and debug assertions, so arithmetic overflow is a crash too. It starts from the main workspace's `Cargo.lock` and each plugin's own, so the versions fuzzed are the versions that ship. The targets are read from the five manifests, so a new target is fuzzed the next night with no change to the workflow, and a target name used by two folders fails the run. The nightly run covers all five folders: the 22 targets of `fuzz/`, the SFTP plugin's one, the disk image plugin's nine, the git plugin's four and the encryption plugin's three, 39 in all.
- One job per target runs it for ten minutes, from its corpus of earlier nights and its seeds. The git plugin's seeds are found by the same rule as the main fixtures. The disk and encryption targets have no seeds folder, so they start from the corpus of earlier nights. Each target's corpus is cached under its own name, and saved after a failure too.
- A target that finds a problem fails its job and uploads the artifact `fuzz-crash-TARGET`: the input, the run's log `fuzz.log` with the stack trace near its end, and `README.txt` with the reproduce line in words.
- The last job writes the run's summary: one line per target, failures first, the meaning first, such as "Pass: archive_zip, 10 minutes, 41,200 runs" or "Fail: archive_7z, crash, input uploaded, after 1,234 runs".

To replay a nightly crash, download its artifact from the run's page, or with `gh run download RUN-ID --name fuzz-crash-TARGET`, save the input in the target's `artifacts` folder, and run the command in its `README.txt`.

## By day

A change that stops a target compiling is caught on its push, not at night: the CI job "Fuzz targets compile" in `ci.yml` checks all five fuzz crates with the pinned toolchain. The same check on your computer:

```bash
cargo check --manifest-path fuzz/Cargo.toml --bins
cargo check --manifest-path plugins/diro-sftp/fuzz/Cargo.toml --bins
cargo check --manifest-path plugins/diro-disk/fuzz/Cargo.toml --bins
cargo check --manifest-path plugins/diro-git/fuzz/Cargo.toml --bins
cargo check --manifest-path plugins/diro-crypt/fuzz/Cargo.toml --bins
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
