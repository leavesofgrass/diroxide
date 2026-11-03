# Building

diroxide builds with Cargo alone. The toolchain is pinned in `rust-toolchain.toml` (Rust 1.96, with rustfmt and clippy), so rustup installs and uses the right one.

## On the owner's Windows machine

The project settings put everything on drive D:

- `CARGO_HOME` is `D:\rust\cargo` and `RUSTUP_HOME` is `D:\rust\rustup`.
- `RUSTC_WRAPPER` is sccache, with its cache in `D:\sccache`.
- `CARGO_INCREMENTAL` is 0, and the dev and test profiles use line-tables-only debug info.

Each building agent sets its own target folder, so builders never share a build lock:

```bash
export CARGO_TARGET_DIR=D:/diroxide/target/<agent-id>
```

If a build fails because of the sccache wrapper, run it once with the wrapper empty and say so in the report:

```bash
RUSTC_WRAPPER= cargo build --workspace
```

## The checks

These are the checks every agent runs before reporting, on the crates it changed. The orchestrator runs them on the whole workspace once per merge batch.

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```bash
cargo nextest run --workspace --all-features
```

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

```bash
cargo xtask deps --check
```

```bash
cargo xtask keyboard --check
```

`scripts/dev-check.sh` and `scripts/dev-check.ps1` run all of them in order and stop at the first failure.

cargo-nextest is pre-approved; install it into the D: cargo home when it is missing:

```bash
CARGO_HOME=D:/rust/cargo cargo install cargo-nextest --locked
```

## The profiles

- `dev` and `test`: line-tables-only debug info, no incremental compilation; dependencies at `opt-level = 1` with no debug info.
- `release`: optimized, without LTO, for contributors.
- `dist`: fat LTO, one codegen unit, stripped, for packages. Build it with `cargo build --profile dist -p diro`.

## A slim `diro`

`diro` has four default features (`default = ["fetch", "syntax-set", "gui", "tui"]` in `crates/diro/Cargo.toml`). Three can be left out, and the fourth is a name:

- `syntax-set`: the viewer's syntax set, syntect with bat's grammars, about 3 MB of the binary. Without it the viewer says "Highlighting off: not in this build", and the commands that need no grammar keep working.
- `fetch`: the optional-components downloader's HTTP client, whose `ring` compiles C. Without it the downloader uses `curl` or `wget` from PATH. (carta's stack growth still compiles a little C and assembly, so a C compiler is needed either way; see the inventory in `xtask/src/native.rs`.)
- `gui`: the window, built on the vendored Masonry, Vello, wgpu and AccessKit stack (`docs/dev/vendoring.md`). Without it `diro --gui` says "This diro was built without the GUI; the terminal opens instead." The stack compiles no C. Drawing on the processor, for a machine with no graphics card, is part of it.
- `tui`: a marker for the terminal frontend, which is always built. It leaves nothing out; it lets a command line name what a build holds, as in `--no-default-features --features tui`.

The syntax set reaches the preview crate through each layer's own `syntax-set` feature: `diro` turns on the CLI's, the CLI the app's, the app the preview crate's. The root manifest declares `diroxide-preview` with `default-features = false`, and the app and the CLI leave the feature off by default, so only `diro`'s default turns it on. The app's and the terminal's tests turn it on through their dev-dependencies, since they read highlighted code.

A build with none of the three, the smallest:

```bash
cargo build --profile dist -p diro --no-default-features
```

A build without the syntax set and the window that keeps the downloader's client:

```bash
cargo build --profile dist -p diro --no-default-features --features fetch
```

A build without the syntax set that keeps the window and the client:

```bash
cargo build --profile dist -p diro --no-default-features --features gui,fetch
```

The check every agent runs after touching these features:

```bash
cargo check -p diro --no-default-features
```

The GUI crate builds its model without the `gui` feature too, and `.github/workflows/gui.yml` runs its tests with and without the feature.

## The tasks

`cargo xtask` runs the workspace's own tasks (the alias is in `.cargo/config.toml`, so run it from the repository folder):

- `cargo xtask deps --check`: every dependency is inherited from the workspace and points the allowed way (see the architecture page).
- `cargo xtask keyboard`: writes `docs/keyboard.md` from the keymap's defaults; `--check` fails when the page is out of date.
- `cargo xtask regen`: writes every generated page and the icon files; `--check` fails when one is out of date.
- `cargo xtask icons`: draws the icon files from the mark (see "The icon" below); `--check` compares them byte for byte.
- `cargo xtask bench --gui`: the bench with the GUI's budgets (see "The GUI's budgets" below); `--only gui` measures them alone.

## The icon

`cargo xtask icons` draws every icon file from the approved mark, `assets/logo/diroxide-mark.svg`, with resvg and no font, the same bytes on every run: the mark's PNGs in `assets/logo/png/` at 16 to 512 pixels, and in `assets/logo/icons/` the Windows `.ico` (16, 20, 24, 32, 40, 48, 64 and 256 pixels), the macOS `.icns`, the compiled Windows resource `diroxide.res`, and the 64-pixel mark as plain RGBA for the window. `cargo xtask regen --check` compares them on x86_64; another processor may round a few pixels differently, so the check there may report the PNGs.

- **Windows, the Microsoft toolchain:** `crates/diro/build.rs` gives the linker `diroxide.res` with `cargo:rustc-link-arg-bins`, so `diro.exe` carries the icon for Explorer, shortcuts and the taskbar, with no resource compiler and no C. The window loads the same icon group by its id, 1. A test in `crates/diro/tests/it/icon.rs` reads the group back from `diro.exe` by its id.
- **Windows, the GNU toolchain, and Linux:** the program has no icon of its own; the window uses the 64-pixel mark compiled in.
- **Linux desktops:** the AppImage installs the mark at every hicolor size and as the scalable `diroxide.svg`; Wayland takes the icon from the desktop file.
- **macOS:** a bare program has no icon, and winit sets no window icon there; the `.icns` waits for an app bundle.

After a change to the mark, run `cargo xtask icons` and commit what it writes.

## The AppImage

`packaging/appimage/build.sh BINARY TAG OUTDIR TOOLS` lays out the AppDir and packs it with the tools `packaging/appimage/fetch-tools.sh` downloads; with `none` for TOOLS it lays out the AppDir only. `AppRun` is a link to the program, so the program keeps the environment it was started with: the D-Bus session, the AT-SPI bus and the speech socket stay reachable. `diro.desktop` opens the GUI (`diro --gui`) and `diro-terminal.desktop` the terminal (`diro --tui`, in a terminal).

`packaging/appimage/check.sh IMAGE` checks an AppImage or an AppDir in the GUI container: the layout, the GUI drawn under Xvfb, and the AT-SPI check against the program inside the image. With no tools at hand, on the owner's machine:

```bash
MSYS_NO_PATHCONV=1 DIRO_AGENT=g4 docker compose run --rm gui bash -c 'cargo build -p diro && sh packaging/appimage/build.sh /target/debug/diro check /target/appimage none && bash packaging/appimage/check.sh /target/appimage/diro.AppDir'
```

The GUI workflow (`.github/workflows/gui.yml`) packs a real AppImage and runs the same check on it.

## The GUI's budgets

`cargo run --release -p xtask -- bench --gui` records, beside the bench's scenarios and never gated: the size of the `diro` it built, the time from the start of `diro --gui --background` to the window's first frame (the `--log` line "graphics: drawing with"), and the peak working set a second after it; `--cold-build` adds a cold release build of `diro` into `target/bench/cold-build`, a folder that must not exist yet and is left for `.claude/tools/safe-remove.ps1`. Each start is one window, off screen and never activated.

The first numbers, on the owner's Windows 11 machine with a graphics card (an NVIDIA GeForce RTX 3060), on Tuesday, November 3, 2026, from `bench --only gui --cold-build`; the bench lane records the baseline on a quiet machine:

- The cold release build of `diro`: 5 minutes 6 seconds. Earlier the same morning, with two other lanes building beside it, a cold debug build (`cargo build -p diro`, an empty target folder, no compiler cache) took 7 minutes 40 seconds and a cold release build 7 minutes 52 seconds; the `dist` build after the release one took 12 minutes 54 seconds.
- The binary: 60.8 MB in the debug profile, 44.1 MB in the release profile, and 39.5 MB in the `dist` profile, which the release files use.
- From the start to the first frame: 1.20 seconds, the median of three starts (1.67, 1.20 and 1.16 seconds), drawn on the graphics card by the release build.
- The working set at the first frame: 189.0 MB, the median of the same three starts.

## Phase 0 measurements

The first clean build of the workspace, its time and its target folder's size are recorded in the Phase 0 report. Phase 0 ran its checks with Rust 1.96.1; it had pinned 1.98.1, which was not installed in the project's rustup home, so the orchestrator repinned to 1.96 at integration, the toolchain textweaver also uses.

## See also

- [Docker](docker.md)
- [Architecture](architecture.md)
- [ADR-0001: Workspace and dependencies](../adr/0001-workspace-and-dependencies.md)
