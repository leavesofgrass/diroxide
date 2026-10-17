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

`diro` has two default features, and both can be left out:

- `syntax-set`: the viewer's syntax set, syntect with bat's grammars, about 3 MB of the binary. Without it the viewer says "Highlighting off: not in this build", and the commands that need no grammar keep working.
- `fetch`: the optional-components downloader's HTTP client, whose `ring` compiles C. Without it the downloader uses `curl` or `wget` from PATH. (carta's stack growth still compiles a little C and assembly, so a C compiler is needed either way; see the inventory in `xtask/src/native.rs`.)

The syntax set reaches the preview crate through each layer's own `syntax-set` feature: `diro` turns on the CLI's, the CLI the app's, the app the preview crate's. The root manifest declares `diroxide-preview` with `default-features = false`, and the app and the CLI leave the feature off by default, so only `diro`'s default turns it on. The app's and the terminal's tests turn it on through their dev-dependencies, since they read highlighted code.

A build with neither:

```bash
cargo build --profile dist -p diro --no-default-features
```

A build without the syntax set that keeps the downloader's client:

```bash
cargo build --profile dist -p diro --no-default-features --features fetch
```

The check every agent runs after touching these features:

```bash
cargo check -p diro --no-default-features
```

## The tasks

`cargo xtask` runs the workspace's own tasks (the alias is in `.cargo/config.toml`, so run it from the repository folder):

- `cargo xtask deps --check`: every dependency is inherited from the workspace and points the allowed way (see the architecture page).
- `cargo xtask keyboard`: writes `docs/keyboard.md` from the keymap's defaults; `--check` fails when the page is out of date.
- `cargo xtask regen`: will rebuild every generated file after a merge; today it lists what it will rebuild.

## Phase 0 measurements

The first clean build of the workspace, its time and its target folder's size are recorded in the Phase 0 report. Phase 0 ran its checks with Rust 1.96.1; it had pinned 1.98.1, which was not installed in the project's rustup home, so the orchestrator repinned to 1.96 at integration, the toolchain textweaver also uses.

## See also

- [Docker](docker.md)
- [Architecture](architecture.md)
- [ADR-0001: Workspace and dependencies](../adr/0001-workspace-and-dependencies.md)
