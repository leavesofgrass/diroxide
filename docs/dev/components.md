# Components: the definitions and the pins

The optional components are written down in three files, and two commands keep them in step. This page is for the person who adds or changes a component. [Optional components](../components.md) is the guide for the person who uses them.

## The three files

- `packaging/components/components.toml` holds the definitions, typed by hand: each component's words (title, kind, version, license, features, action) and, for each file, where it comes from. Its header comment lists every field.
- `packaging/components/pins.toml` holds the pins of the helper files a publisher makes: their sizes and SHA-256 digests. It is written by `cargo xtask components --pin`, which the pin job runs, and nobody types it.
- `crates/diroxide-components/components.toml` is the public list that the `diro` binary embeds. `cargo xtask components` writes it from the two files above and from the real assets. Never type a size or a hash in it: `cargo xtask components --check` and `cargo xtask regen --check` fail when it is stale.

A plugin's files are packed by the xtask from folders git tracks, and from prebuilt programs the release workflow hands in with `--assets FOLDER`. A component whose assets are not there is left out of the list with a line, so a list built without a release stays current. A helper's file is not packed at all: it is the publisher's own file at the publisher's own address, and this project hosts no copy.

## The commands

```bash
cargo xtask components [--check] [--assets FOLDER] [--pin ID ...] [--addresses]
```

- With no flag it builds the components and writes the embedded list.
- `--check` builds in memory and fails when the embedded list on disk differs.
- `--assets FOLDER` reads prebuilt plugin programs from `FOLDER/TARGET/program`, as the release workflow lays them out.
- `--pin ID` pins one helper component, and `--pin all` pins every one. It downloads each of the component's files, so it runs on the project's own runner (the owner's answer 117, Thursday, November 5, 2026). On a development computer it runs only with the owner's go.
- `--addresses` asks each pinned address for its headers and fails when one answers 404 or 410. Any other answer, or no answer, is said and not counted. The workspace job of `ci.yml` runs it on every push.

The one-line usage text in `xtask/src/main.rs` names both.

## A component's fields

Per component, in `[[component]]`:

- `id`, `title` (what it is, meaning first), `kind` (`plugin`, `helper` or `data`), `version` and `license` (an SPDX expression).
- `license_note`, `credit`, `features`, `needs_tools`, `needs_components`, `protocol` and `min_diro`.
- `action`: `place`, where the checked files are the component, or `unpack`, where each archive among the files is unpacked into the component's folder and a receipt takes its place.
- `folder`: the id by default.
- `tools`, for a helper: the tools registry's ids of the programs it holds, such as `ffmpeg` and `ffprobe`. The version line said after an install, and the package command said where the helper has no build, come from them.
- `notice`: a sentence the ask-first question says after the size and the license. Meaning first within 40 cells. Often empty. 7-Zip's is the unRAR sentence.
- `held`: why the whole component stays out of the list for now. The reason is printed each run. The owner settles it.

Per file, in `[[component.file]]`:

- `name`: a plain name (letters, digits, `-`, `_` and `.`), at most 120 bytes.
- `target`: a Rust target triple, `any`, or `universal-apple-darwin`, which matches both Apple architectures.
- `exec`: the program's path inside the component's folder. Only this file gets the executable bit on Unix.
- `also_exec`: more programs, by plain path, each given the executable bit and offered to the tools registry (`bin/ffprobe` beside `bin/ffmpeg`).
- `strip`: how many leading folder levels an unpack drops from each member's path.
- `keep`: which members to unpack, as patterns. Empty means every member. A member no pattern keeps is not part of the component, and nothing is said about it.
- `rename`: pairs of a member's path and a new plain path, for the few names a publisher makes that are not plain.
- `url`: the publisher's address, `https` only. A file with a `url` and no `pack` is a helper's file, pinned by `pins.toml`.
- `checksums`: the address of the publisher's own checksums file, for a file whose digest GitHub does not show.
- `held`: why this file alone stays out of the list for now.
- `pack`: for a file this project builds, the parts that go in its zip: `{ from = "folder or tracked file", as = "path in the zip", optional = true }` or `{ asset = "target/program", as = "..." }`.

### Patterns: keep, rename and also_exec

A pattern is matched against a member's path after `strip`, part by part, without regard to ASCII case. `*` stands for any run of characters within one part. `**` stands for any number of whole parts. So `bin/*` keeps the files directly in `bin`, `exiftool_files/**` keeps a folder with everything under it, and `LICENSE*` keeps `LICENSE.txt` at the top.

The limits keep matching quick whatever a list says:

- at most 32 patterns, 32 renames and 16 extra programs for one file;
- a pattern or path of at most 240 bytes;
- at most two `**` parts in one pattern;
- only the characters of a plain name and `*` in a pattern, and never `.` or `..` as a part.

A rename gives a member its plain name before the name rule is applied. Everything else a publisher's archive holds still has to be plain, which is why `exiftool(-k).exe` is renamed and a name with a plus sign, such as `libstdc++-6.dll`, is a reason to hold a file: it cannot be renamed when a program's import table loads it by that name.

## The pin job

A helper's pin is the file's size and SHA-256. A pin applies while the component, the file's name and its address stay the same, so a new version, which has a new address, needs a new pin.

`.github/workflows/pin.yml` runs `cargo xtask components --pin` on CI's runner, by hand and on the 3rd of each month at 06:23 UTC. By hand it takes a list of helper ids, or `all`. For each file it:

1. downloads it into a work folder under the build output, with the project's neutral User-Agent and nothing else;
2. compares the bytes with the publisher's own digest where one is shown: GitHub's, read from GitHub's API for a release asset, or a line of the file `checksums` names. A digest that differs, or that cannot be read, fails the pin;
3. unpacks it the way `diro` will, with `strip`, `keep` and `rename` applied, says each member left out and why, and fails the pin when one of the file's programs is not among what was unpacked;
4. writes the size, the digest, the bytes its kept members unpack to and what it was compared with into `pins.toml`.

The job's token goes to GitHub's API alone, for the release digests. The files themselves are fetched without it. The job never pushes. It uploads `pins.toml`, the regenerated embedded list and `pin-log.txt` as the artifact `pins`, and writes what changed to the run's summary, so the orchestrator commits the result. The log uses plain words and no color, so it reads well in a screen reader.

The monthly run exists so that a pin a publisher dropped is noticed: BtbN keeps a daily FFmpeg build for 14 days and the last build of each month for two years, and exiftool.org keeps only its newest version. The `--addresses` check on each push says the same thing sooner, with a 404.

### `pins.toml`

The file starts with `format = 1` and holds one `[[pin]]` for each file:

- `component` and `name`: the file.
- `url`: the address it was downloaded from.
- `size` and `sha256`: the pin.
- `unpacked`: the bytes its kept members unpack to, and 0 for a file placed as it is.
- `checked`: what it was compared with, in words: "GitHub's digest matches" or "the publisher's checksum matches".

## What is pinned today

The first run of the pin job (Friday, November 6, 2026) downloaded 23 files and wrote 22 pins, the 7-Zip extra archive being one file for two targets. Every digest matched GitHub's or the publisher's checksums file.

- gh 2.103.3: 6 pins, Windows, Linux and macOS, two architectures each.
- pandoc 3.13: 5 pins.
- 7-Zip 26.05: 4 pins, the extra archive and the Linux and macOS archives. The extra archive's files for Windows are held, because its `7za` reads no RAR, ISO or disk image.
- FFmpeg n9.0.2, BtbN's LGPL builds of October 31, 2026: 4 pins for Windows and Linux. The Linux files are held, because the shared builds' libraries are links, which an unpack never makes. The job names fourteen left-out members.
- ImageMagick 7.1.2-35, the portable build: 2 pins, for Windows on x86-64 and on ARM. `keep` takes `magick.exe`, the `.xml` files, `sRGB.icc` and the two notice files, and leaves the portable build's other copies of the program.
- ExifTool 13.61: 1 pin, and the component is held, because `perl532.dll` loads `libstdc++-6.dll`, a name that is not plain.

The three held entries wait for the owner's decisions. [Optional components](../components.md) says what each means for a person using them.

## See also

- [Optional components](../components.md)
- [Tools and diro doctor](../tools.md)
- [ADR-0010: The optional components](../adr/0010-the-optional-components.md)
- [Building](building.md)
- [The plugin protocol reference](plugin-protocol.md)
