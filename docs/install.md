# Installing diroxide

This guide says how to get diroxide running, what is available now, and what comes later. In alpha.1 there are no download files yet, so you build the program from its source.

## Two ways in, and where each stands

diroxide will have two ways in, and only two.

- One file for each platform, from the project's releases page: `diro.exe` on Windows, one `diro` for macOS, and an AppImage on Linux. There is no archive to open and no installer. Each file comes with a line in a SHA256SUMS list and a provenance attestation, so you can check it. The Windows x86_64 file and the Linux x86_64 AppImage start with alpha.6. The macOS file, the Windows ARM64 file and the Linux aarch64 AppImage come with alpha.9.
- `cargo install diro`, for anyone who builds from the published crate, and the way to any other shape of install on Linux. It comes at beta.1.

Package manager entries, such as winget, Scoop, Homebrew and the AUR, come with alpha.9 and point at the same files.

Until then, the way in is to build from source.

## Build from source

What you need:

- A Rust toolchain from rustup. The repository pins the exact version in `rust-toolchain.toml`, so rustup downloads the right one the first time you build.
- The linker Rust needs on your system. On Windows that is the Microsoft C++ Build Tools. On Linux it is a package such as build-essential or gcc. On macOS it is the Xcode command line tools. In alpha.1 diroxide compiles no C or C++ code, so no C compiler is needed for it; Rust only asks the system for a linker.

In the source folder, build the program:

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

## The first run of a downloaded file

This applies from alpha.6. The builds are not code-signed, so each platform needs one step the first time. A program you built yourself needs none of this.

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

If your system cannot run AppImages, install with `cargo install diro` once it is available.

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
