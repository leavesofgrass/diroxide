# ADR-0001: Workspace and dependencies

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Friday, October 16, 2026 (the Wave 1 contract, P0b): the plugin boundary is the language boundary (the owner, Sunday, October 11, 2026; plan section 5.8, item 3). The `diro` binary and every crate of the main workspace are pure Rust and build with no C compiler, with one exception, the owner's (answer 40): the optional-components downloader's client, `ureq` 3.4 with rustls, whose `ring` provider compiles C, behind the default `fetch` feature of `diro`; `--no-default-features` builds with no C compiler, and the downloader then uses `curl` or `wget` from PATH.

- `crates/diroxide-sftp` moved to `plugins/diro-sftp/`, a workspace of its own that the root's `exclude` keeps out. The `lua` module and feature left `diroxide-plugins`. `liblzma`, `mlua`, `russh`, `russh-sftp` and `tokio` left `[workspace.dependencies]`. There are no `sftp`, `lua` or `xz-c` features.
- A new gate, `cargo xtask native --check`, runs `cargo tree` inverted on `cc`, `cmake`, `cxx` and `cxx-build` over every feature, target and edge, and fails on any path into the workspace but its one allow entry: `cc` under `ring`, reached through `rustls` and `ureq`. CI runs it beside `deps --check`.
- The theme crate ported from textweaver joins as `diroxide-theme`, the seventeenth crate of the plan's count, depending on no workspace crate. The main workspace has 17 packages (15 `diroxide-*` crates, `diro` and `xtask`); the compare crate of plan section 6, `diroxide-diff`, will be the eighteenth when alpha.3 adds it.
- Declared for Wave 1 and after: `zip` repinned to 9.0.1 exactly (9.0.4 exists; a bump is the orchestrator's); `lzma-rust2` on the 0.21 line that `zip` and `sevenz-rust2` share; `sevenz-rust2`, `tar` and `bzip2` without C; `rustix`, `windows-sys` and `reflink-copy`; `trash` pinned to 5.2.9; `toml_edit`, `opener`, `sha2`, `process-wrap`, `secrecy`, `zeroize` and `regex`; `ignore` 0.4.33 and `grep-searcher` 0.1.17; `similar`, `two-face` with the fancy-regex engine, and `pulldown-cmark`.
- `blake3` always has `cc` as a build dependency, even with its `pure` feature, so the gate fails the day a crate uses it. Before the duplicate finder (alpha.8) uses it, the owner decides: a second allow entry, or a pure-Rust hash in its place.
- `deny.toml` gains `CDLA-Permissive-2.0` (webpki-roots, under `fetch`), the `libbz2-rs-sys` exception, and three advisory ignores with their reasons. The regex crates build at `opt-level = 3` in debug builds.

Status update, Friday, October 16, 2026 (the alpha.2 contract step, P2): pure Rust is a goal, not a rule (the owner, answer 46), so the native gate's allow list is now the inventory, each entry with its reason in words, added in the same commit as its dependency.

- **carta carries C.** `carta` 0.0.11 depends on `stacker`, whose `psm` builds a small assembly file, and `stacker` itself compiles one C file, both for growing the stack on deeply nested documents. No pure-Rust reader covers Org, reStructuredText, MediaWiki, DokuWiki and Jira (answer 27), so the inventory gains two entries: `cc` under `psm` through `stacker` and `carta`, and `cc` under `stacker` through `carta`. The machine's C compiler (MSVC on Windows, the base image's gcc in the container) builds them. Fallback: carta behind the plugin boundary, as answer 27 already allows.
- **Depended on, not only declared.** So the license, duplicate and native gates read them, the crates that will use the alpha.2 dependencies now name them: `diroxide-archive` takes `zip` 9.0.1 (with `aes-crypto`), `sevenz-rust2`, `tar`, `flate2`, `lzma-rust2` and `bzip2`; `diroxide-ops` takes `crc32fast`; `diroxide-preview` takes `encoding_rs`, `syntect` (parsing with the fancy-regex engine), `two-face`, `pulldown-cmark`, `carta` (five readers, default features off) and `serde`. `cargo tree -d` shows one `lzma-rust2`, 0.21.0. `scraper` waits for spike C1's path A. `syntect` and `fancy-regex` build at `opt-level = 3` in debug builds.
- **The container** installs the archivers its header already named (`xz-utils`, `bzip2`, `zip`, `unzip`, `libarchive-tools`) and `curl` and `wget`, for W2b's cross-checks and the downloader's fallback tests.

Status update, Saturday, October 17, 2026 (the pre-wiring contract step, P2b, from the requests pairs 1 to 3 held at their integrations):

- **A slim `diro`.** The root manifest declares `diroxide-preview` with `default-features = false`, and the syntax set (syntect with bat's grammars, about 3 MB) is a `syntax-set` feature at each layer: `diro`'s, on by default beside `fetch`, turns on the CLI's, the CLI's the app's, the app's the preview crate's. The app and the CLI leave it off by default, so `cargo build -p diro --no-default-features` builds without it (`cargo check -p diro --no-default-features` is the check); the app's and the terminal's tests turn it on through their dev-dependencies. `docs/dev/building.md` says how (W2d's request 2).

## Context

diroxide is built by waves of agents, one or two at a time, each owning crates. The workspace has to keep their work apart, keep builds lean on the owner's machine (shared with textweaver), and make the license and dependency rules checkable by a machine rather than by memory.

## Decision

- One Cargo workspace: edition 2024, resolver 3, members `crates/*` and `xtask`.
- Fifteen `diroxide-*` crates as named in plan section 5.1, the binary crate `diro`, and `xtask`: seventeen packages. The plan's `diroxide-cli` ("the diro binary") is split into the library `diroxide-cli`, which holds the command line and is tested as a library, and a thin binary crate `diro`, so that `cargo install diro` is the install line the plan names.
- The toolchain is pinned in `rust-toolchain.toml` to 1.96 (the toolchain installed in the project's rustup home and shared with textweaver; Phase 0 first pinned 1.98.1, and the orchestrator repinned at integration on Saturday, October 10, 2026), with rustfmt and clippy. No `rust-version` is set; resolver 3 then picks dependency versions the running toolchain can build.
- `[workspace.package]` carries version `0.1.0-alpha.1`, license `Apache-2.0 OR MIT`, homepage `https://diroxide.app`, and `publish = false` until the owner says otherwise.
- Every dependency is declared once in `[workspace.dependencies]`, and members take it with `workspace = true`. The heavy crates of plan section 6 are declared there but used by no crate yet, so the first build stays small; versions follow textweaver's pins where it uses the crate, so the two projects share a cache. Versions that were not in the local cache on the Phase 0 day are marked "unverified" and are checked by the wave that first uses them.
- Internal dependencies point one way, by the table in `xtask/src/deps.rs` and `docs/dev/architecture.md`. `cargo xtask deps --check` fails on a dependency without `workspace = true`, on one that points the wrong way, on a member missing from the table, and on any dependency on `xtask` or `diro`.
- Lints: `missing_docs` and `unsafe_code` denied workspace-wide (denied, not forbidden, so the local provider's `sys` module can allow `unsafe` for platform calls, reviewed at integration); `dbg_macro`, `todo` and `unimplemented` denied; `unwrap_used` and `expect_used` warned (so denied in CI), allowed in tests by `clippy.toml`.
- Profiles from plan section 7: `dev` and `test` with line-tables-only debug info and no incremental compilation, dependencies at `opt-level = 1` without debug info, `release` without LTO, `dist` with fat LTO, one codegen unit and stripped symbols.

## Alternatives

- **Explicit member list.** Clearer, but every new crate would also need a root edit; the glob plus the dependency table gives the same control, because an unknown crate fails `deps --check`.
- **`diroxide-cli` as the binary itself.** Fewer packages, but then `cargo install diro` does not work, and the command line is harder to test without spawning a process.
- **Forbid `unsafe_code`.** Stronger, but the local provider needs a few platform calls (the drive bitmask, attributes); a forbidden lint cannot be allowed in one module.
- **Depending on the heavy crates now.** Shows the real build cost early, but makes the first build slow and fetches crates the stubs do not use.

## Fallback

If a crate boundary proves wrong, two crates merge, or a module moves, by a contract change request resolved at integration. If a heavy crate fails, plan section 6 names its fallback (liblzma or lzma-rust2, russh or openssh-sftp-client, the vendored window stack or a fresh vendor).

## Consequences

- The first clean build compiles only serde, thiserror, toml, serde_json and anyhow with their own dependencies.
- Each wave adds its heavy dependency to its own crate's manifest with `workspace = true`; changing a version in the root is the orchestrator's.
- A crate is added only when a wave's brief says so, in three places: the folder, the table in `xtask`, and the architecture page.

## See also

- [Architecture](../dev/architecture.md)
- [Building](../dev/building.md)
