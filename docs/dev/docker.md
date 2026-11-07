# Docker

Linux builds and tests run in a container from day one, beside the native Windows checks. The image is `diroxide-dev`, built from `docker/Dockerfile`; `compose.yaml` runs it with one target volume per agent.

## What the image has

- Debian trixie with Rust 1.96 (the official `rust` image), rustfmt and clippy.
- cargo-nextest, for the tests.
- ImageMagick, FFmpeg, pandoc and poppler-utils (`pdftotext` and `pdfinfo`), for the tools registry's real-tool tests and the previews'.
- git, for the git plugin's tests, and Python 3.
- 7-Zip (Debian's `7zip` package, the successor of p7zip: 7-Zip 25.01, with `7z`, `7za` and `7zr`), for cross-checking archives the archive provider writes.
- The classic archivers, `xz-utils`, `bzip2`, `zip`, `unzip` and `libarchive-tools` (`bsdtar`), for the same cross-checks.
- `curl` and `wget`, the components downloader's fallbacks, for their tests (approved by the owner on Sunday, October 11, 2026).
- An OpenSSH server, for the SFTP plugin's tests (not started by default).
- A non-root user, `builder`, who runs everything.

## The three stages

One Dockerfile builds three stages, and `compose.yaml` has a service for each. The `dev` stage above is the base of the other two.

- `dev`: the image above. The service `dev` runs it.
- `gui`: the window's accessibility checks on Linux (the owner's answer 107, Monday, November 2, 2026). It adds a virtual display (Xvfb), a D-Bus session and the AT-SPI bus, Mesa's software Vulkan and GL, Python's AT-SPI binding and `accessibility-cli` built from a pinned commit. The service `gui` runs it.
- `tools`: the helper programs of alpha 4, for tests only (the owner's answer 118, Thursday, November 5, 2026). Nothing in it is on the owner's computer. It adds Tesseract, for Read text in picture; ExifTool, for the metadata readers' fallback; GnuPG with `gpgv`, age and minisign, for encryption and signatures; `dosfstools` and `mtools` for FAT, `squashfs-tools`, `ntfs-3g` for `mkntfs` and `xorriso` for ISO 9660, which make the disk image plugin's fixtures; and `qemu-utils`, for `qemu-img`. The image is `diroxide-tools`, and the service `tools` has the same mounts and limits as `dev`.

Build and use the tools stage like the dev image:

```bash
MSYS_NO_PATHCONV=1 docker compose build tools
MSYS_NO_PATHCONV=1 DIRO_AGENT=p4 docker compose run --rm tools scripts/dev-check.sh
```

Two scripts run a plugin's own tests in the image, with nothing downloaded and nothing of the owner's read. `docker/disk/run-tests.sh` runs the disk image plugin's tests in the tools stage, where 7-Zip 25.01 and `qemu-img` are, so the tests that only check words elsewhere run the real programs here. `docker/git/run-tests.sh` runs the git plugin's tests in the dev stage, against Debian's git 2.47.3.

## Building the image

The orchestrator builds the image once per toolchain change; agents do not. In Git Bash, path conversion must be off for Docker commands:

```bash
MSYS_NO_PATHCONV=1 docker compose build dev
```

## Running the checks in the container

Each agent has its own target volume, named by `DIRO_AGENT`, so containers never share a build lock:

```bash
MSYS_NO_PATHCONV=1 DIRO_AGENT=w1a docker compose run --rm dev scripts/dev-check.sh
```

The defaults are 6 GB of memory, 4 CPUs and 4 cargo jobs per container (`DIRO_MEM`, `DIRO_CPUS` and `DIRO_JOBS` change them). At most three containers build at once on the owner's machine.

## Care of the machine

- Never purge Docker data. The owner keeps licensed and personal volumes and images there.
- Check D:'s free space and `docker system df` before heavy runs; D: never goes below 200 GB free.
- Restart Docker Desktop from its own menu between waves; never force it to stop.
- The registry and git caches are shared volumes (`diro-cargo-registry`, `diro-cargo-git`), so the crates download once.

## See also

- [Building](building.md)
- [The plan, section 7](../plan.md)
