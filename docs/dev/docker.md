# Docker

Linux builds and tests run in a container from day one, beside the native Windows checks. The image is `diroxide-dev`, built from `docker/Dockerfile`; `compose.yaml` runs it with one target volume per agent.

## What the image has

- Debian trixie with Rust 1.98.1 (the official `rust` image), rustfmt and clippy.
- cargo-nextest, for the tests.
- ImageMagick, FFmpeg and pandoc, for the tools registry's real-tool tests.
- p7zip, for cross-checking archives the archive provider writes.
- An OpenSSH server, for the SFTP provider's tests (not started by default).
- A non-root user, `builder`, who runs everything.

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
