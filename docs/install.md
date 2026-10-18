# Installing diroxide

This guide says how to get diroxide running, what is available now, and what comes later. Alpha 1 has one download for each platform. `cargo install diro` comes with the first beta, and you can always build the program from its source.

## Two ways in, and where each stands

diroxide will have two ways in, and only two.

- One file for each platform, from the project's [releases page](https://github.com/leavesofgrass/diroxide/releases): `diro.exe` on Windows, one `diro` for macOS (Apple silicon and Intel in one universal file), and an AppImage on Linux. There is no archive to open and no installer. Each file comes with a line in a SHA256SUMS list and a provenance attestation, so you can check it. Alpha 1 has the Windows x86_64 file, the macOS universal file and the Linux x86_64 AppImage. The Windows ARM64 file and the Linux aarch64 AppImage come in a later alpha.
- `cargo install diro`, for anyone who builds from the published crate, and the way to any other shape of install on Linux. It comes with the first beta.

Package manager entries, such as winget, Scoop, Homebrew and the AUR, come in a later alpha and point at the same files.

## Download the one file

On the [releases page](https://github.com/leavesofgrass/diroxide/releases), each release lists its files. The file's name carries the version and the platform, such as `diro-0.1.0-alpha.1-windows-x86_64.exe`, `diro-0.1.0-alpha.1-macos-universal` and `diro-0.1.0-alpha.1-linux-x86_64.AppImage`. Download the one for your system, rename it to `diro` (or `diro.exe`) if you like, and put it in a folder on your PATH. Then read [the first run of a downloaded file](#the-first-run-of-a-downloaded-file) below: each platform asks for one step the first time.

Build from source when you want the newest commit, or a smaller diro.

## Build from source

What you need:

- A Rust toolchain from rustup. The repository pins the exact version in `rust-toolchain.toml`, so rustup downloads the right one the first time you build.
- A C compiler, and the linker Rust needs. A build of the workspace needs a C compiler. carta, the converter the quick view reads Org and other formats with, brings two small crates, psm and stacker, that compile a little C and assembly so that deeply nested documents cannot overflow the stack. The downloader's secure connection library, ring, compiles C too. `cargo install diro` needs the same when it comes with the first beta. The tools are the ones that also give Rust its linker, so one install covers both:
  - Windows: the Microsoft C++ Build Tools, with the "Desktop development with C++" workload. They hold the C compiler, the assembler and the linker.
  - Linux: a C compiler and make. On Debian and Ubuntu that is the `build-essential` package, on Fedora the `gcc` and `make` packages, and on Arch the `base-devel` group. `clang` works in place of gcc.
  - macOS: the Xcode command line tools, installed with `xcode-select --install`. They hold clang.

Everything else diroxide uses is pure Rust. A C compiler is needed only for those crates, and `cargo xtask native --check` checks that nothing else in the build asks for one.

Get the source with git (`git clone https://github.com/leavesofgrass/diroxide`) or as the tagged archive from the repository's releases page, then, in its folder, build the program:

```bash
cargo build --release -p diro
```

The first build takes a while. The program is `target/release/diro`, or `target\release\diro.exe` on Windows. Copy it to a folder on your PATH.

To have cargo install it into its own program folder instead:

```bash
cargo install --path crates/diro --locked
```

Check that it runs:

```bash
diro --version
```

The first line is the program's name and version, `diro 0.1.0-alpha.1`. The next lines name the project and its license.

### A smaller diro

The viewer colors code with a syntax set that adds about 3 MB to the program. A build can leave it out, and the downloader's secure connection library with it:

```bash
cargo build --release -p diro --no-default-features
```

In that build the viewer says "Highlighting off: not in this build" for every file, and the commands that need no grammar keep working, such as search, hex mode, the bracket jump and indentation. The downloader falls back to `curl` or `wget` on your PATH. To leave out only the syntax set and keep the downloader's client:

```bash
cargo build --release -p diro --no-default-features --features fetch
```

A smaller build still needs a C compiler, for carta. [Building, for contributors](dev/building.md) has the details.

## The first run of a downloaded file

The builds are not code-signed, so each platform needs one step the first time. A program you built yourself needs none of this.

- Windows: SmartScreen warns when you first start `diro.exe`. Choose More info, then Run anyway. Put the file in a folder on your PATH.
- macOS: the system quarantines a downloaded file. Remove the mark once, then make the file executable:

```bash
xattr -d com.apple.quarantine diro
```

```bash
chmod +x diro
```

If you prefer, choose Open Anyway under Privacy and Security in System Settings after the first refusal.

- Linux: make the AppImage executable, then run it. Use the name of the file you downloaded in place of `diro.AppImage`:

```bash
chmod +x diro.AppImage
```

If your system cannot run AppImages, build from source, or install with `cargo install diro` once it is available.

## Checking a download

Every release file has a line in the SHA256SUMS list. Compute the file's SHA-256 and compare it with that line. On Windows, in PowerShell:

```powershell
Get-FileHash -Algorithm SHA256 diro.exe
```

On Linux:

```bash
sha256sum diro.AppImage
```

On macOS:

```bash
shasum -a 256 diro
```

The two values must match exactly. If they do not, do not run the file.

Each file also has a provenance attestation, GitHub's signed record that this workflow built it from this repository. With the GitHub command-line tool signed in, check it with the file's name:

```bash
gh attestation verify diro.exe --owner leavesofgrass
```

## Set up your shell

Two optional pieces make diroxide pleasant at a prompt.

- Leave your shell in the folder you were in when you quit. `diro init` prints a small function named `dcd` for your shell, and you add it to your shell's startup file; then you start diroxide with `dcd`. [The quick start](quickstart.md) shows the command. For cmd, the function is a file named `dcd.cmd` that you put on your PATH.
- Complete diroxide's subcommands and flags with Tab. `diro completions` writes the completion script for bash, zsh, fish or PowerShell to standard output. Save it where your shell loads completions; your shell's documentation says where. For example:

```bash
diro completions zsh
```

## Where diroxide keeps its files

Settings, keys and menus live in a config folder, and what diroxide remembers lives in a state folder. [The settings guide](settings.md) says where they are on each system. `diro --portable` keeps both beside the program instead, which is handy on a USB drive.

## Updating and removing

To update, build again from newer source. Your settings and state stay as they are.

To remove diroxide, delete the program file. The config and state folders stay until you delete them yourself, so you can reinstall later without losing your settings. diroxide never removes them.

## See also

- [Quick start](quickstart.md)
- [Settings and where the files live](settings.md)
- [Troubleshooting](troubleshooting.md)
- [Building, for contributors](dev/building.md)
- [The documentation index](README.md)
