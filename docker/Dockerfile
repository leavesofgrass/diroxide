# The diroxide development image: Debian trixie with the pinned Rust
# toolchain, the external tools the tools registry drives, 7-Zip for
# cross-checking archives (trixie's 7zip package, the successor of
# p7zip: 7-Zip 25.01, with the binaries 7z, 7za and 7zr), the classic
# archivers for the archive cross-checks (xz-utils, bzip2, zip, unzip and
# libarchive-tools for bsdtar), curl and wget for the components
# downloader's fallback tests (approved by the owner on Sunday, October
# 11, 2026), and an OpenSSH server for the SFTP tests. The archivers, curl
# and wget were named here but missing from the package list until the
# alpha.2 contract step (P2, Friday, October 16, 2026), which adds them for
# W2b's cross-check suite and the demo set. The C compiler of the base
# image builds the inventory's psm and stacker (through carta) and ring.
#
# Built once per toolchain change by the orchestrator:
#   MSYS_NO_PATHCONV=1 docker compose build dev
# See docs/dev/docker.md.

FROM rust:1.96-slim-trixie

ARG NEXTEST_VERSION=0.9.148

RUN apt-get update \
 && apt-get install --yes --no-install-recommends \
      ca-certificates \
      git \
      pkg-config \
      imagemagick \
      ffmpeg \
      pandoc \
      7zip \
      xz-utils \
      bzip2 \
      zip \
      unzip \
      libarchive-tools \
      curl \
      wget \
      python3 \
      poppler-utils \
      openssh-server \
 && rm -rf /var/lib/apt/lists/*

# rustfmt and clippy for the checks; nextest for the tests. Installed as
# root into the image's cargo home, which stays on PATH for the builder.
RUN rustup component add rustfmt clippy \
 && cargo install cargo-nextest --version "${NEXTEST_VERSION}" --locked \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

# The SSH server is installed but not started; the SFTP tests start it
# with their own host keys and configuration (docker/sftp/, Wave 3).
RUN mkdir -p /run/sshd

# Builds run as an ordinary user with its own cargo home, so the shared
# registry volumes are not owned by root.
RUN useradd --create-home --uid 1000 --shell /bin/bash builder \
 && mkdir -p /target /home/builder/.cargo/registry /home/builder/.cargo/git \
 && chown -R builder:builder /target /home/builder/.cargo
USER builder
ENV CARGO_HOME=/home/builder/.cargo \
    PATH=/usr/local/cargo/bin:/home/builder/.cargo/bin:$PATH \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=never \
    CARGO_TARGET_DIR=/target
WORKDIR /work
