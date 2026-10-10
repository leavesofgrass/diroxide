# diroxide documentation

diroxide (Directory Oxide) is a two-panel file manager in pure Rust, keyboard first and accessible first. Its command is `diro`. This folder holds the guides for people who use it, the notes for people who build it, and the decisions behind it.

The project is in its first alpha series. Most guides arrive with alpha.1; until then, the plan and the developer notes are the place to start.

## For people who use diroxide

- [Keyboard](keyboard.md): every default key, generated from the keymap.
- More guides (quickstart, panels, operations, settings, screen readers, troubleshooting) come with alpha.1.

## For people who build diroxide

- [The plan for the first alpha](plan.md): what diroxide is, its architecture, the alphas and the waves.
- [Architecture](dev/architecture.md): the crates, what each does, and which way dependencies point.
- [Building](dev/building.md): the toolchain, the checks, the profiles and the tasks.
- [Docker](dev/docker.md): the Linux container for builds and tests.
- [The clean-room rule](dev/clean-room.md): how diroxide stays a reimplementation of Worker's behavior, never a port of its code.
- [Decisions](adr/README.md): the architecture decision records.

## See also

- [The project README](../README.md)
- [The changelog](../CHANGELOG.md)
