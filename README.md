# Directory Oxide (diroxide)

A lean, fast, memory-safe two-panel file manager in pure Rust, in the
style of Worker and Directory Opus, built accessibility-first for a
post-X11, multi-platform world. Its command is `diro`.

Status: Phase 0, the contract. The workspace builds and its tests pass;
the crates hold their public types with deliberately simple bodies that
the waves of the first alpha replace. `diro PATH` prints one screen of a
folder; nothing else is usable yet.

- The plan for the first alpha, with the architecture, the plugin
  system, the alpha milestones and the multi-agent waves:
  [docs/plan.md](docs/plan.md).
- The documentation index: [docs/README.md](docs/README.md).
- Building and checking: [docs/dev/building.md](docs/dev/building.md).
- Platforms: Windows, Linux (Wayland and X11 through winit, plus any
  terminal), macOS.

## License

Copyright (c) 2026 Jon Pielaet

diroxide is licensed under either of

- the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), or
- the MIT license ([LICENSE-MIT](LICENSE-MIT)),

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in diroxide by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

diroxide reimplements the behavior of Worker without using its code; see
[the clean-room rule](docs/dev/clean-room.md).
