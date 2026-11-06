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

FROM rust:1.96-slim-trixie AS dev

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
# The base image names its toolchain by the full release (1.96.1), and
# rust-toolchain.toml asks for "1.96", so until alpha 4's contract step
# (P4) every container run downloaded a second toolchain and its
# components before it built anything. The toolchain is installed here by
# the toolchain file's own name and made the default, so a run starts
# building at once.
RUN rustup toolchain install 1.96 --profile minimal --component rustfmt,clippy \
 && rustup default 1.96 \
 && rustup component add rustfmt clippy \
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

# The GUI's accessibility checks on Linux (the GUI alpha's foundation
# lane, G1; the owner's answer 107, Monday, November 2, 2026: these tools
# in a separate image stage). The dev image above stays as it is; this
# stage adds what the AT-SPI check (tools/a11y/atspi-check.sh) needs: a
# virtual display (Xvfb), a D-Bus session and the AT-SPI bus, Mesa's
# software Vulkan (lavapipe) and GL for the renderer, Python's AT-SPI
# binding, the X11 libraries winit opens at run time, and
# accessibility-cli (DioxusLabs, MIT or Apache-2.0; not on crates.io)
# built from a pinned commit with its own lock file, as textweaver builds
# it (its ADR-0039). The -dev packages are accessibility-cli's build's.
#
#   MSYS_NO_PATHCONV=1 docker compose build gui
FROM dev AS gui

ARG A11Y_CLI_REV=f64c036915095fbb9187b6fdf0f039879618a159

USER root
RUN apt-get update \
 && apt-get install --yes --no-install-recommends \
      xvfb \
      xauth \
      dbus \
      dbus-x11 \
      at-spi2-core \
      python3-pyatspi \
      python3-gi \
      gir1.2-atspi-2.0 \
      mesa-vulkan-drivers \
      libvulkan1 \
      libgl1-mesa-dri \
      libegl1 \
      libxkbcommon-x11-0 \
      libxcursor1 \
      libxrandr2 \
      libxi6 \
      libx11-xcb1 \
      libdbus-1-dev \
      libatspi2.0-dev \
      libx11-xcb-dev \
 && rm -rf /var/lib/apt/lists/*
# Built with a cargo home and a target folder of its own inside the
# image, both gone after, so the builder's cache stays the builder's and
# the build output is not left in the image; the binary lands in
# /usr/local/bin.
RUN CARGO_HOME=/tmp/a11y-cli-cargo CARGO_TARGET_DIR=/tmp/a11y-cli-target cargo install --locked \
      --git https://github.com/DioxusLabs/accessibility-cli \
      --rev "${A11Y_CLI_REV}" accessibility-cli --root /usr/local \
 && rm -rf /tmp/a11y-cli-cargo /tmp/a11y-cli-target
USER builder

# The helper programs of alpha 4 (tools, previews and conversions; the
# owner's answer 118, Thursday, November 5, 2026, for this stage and its
# tests only, nothing on the owner's computer): a stage of its own beside
# the GUI's, so the dev image above stays as it is. The dev stage already
# carries ImageMagick, FFmpeg, pandoc, poppler-utils (pdftotext, pdfinfo)
# and git. This stage adds Tesseract for "Read text in picture", ExifTool
# for the metadata readers' fallback, GnuPG, age and minisign for
# encryption and signatures, the file-system makers for the disk-image
# plugin's fixtures (dosfstools and mtools for FAT, squashfs-tools,
# ntfs-3g for mkntfs, xorriso for ISO 9660) and qemu-utils for qemu-img.
#
#   MSYS_NO_PATHCONV=1 docker compose build tools
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=p4 docker compose run --rm tools bash
FROM dev AS tools

USER root
RUN apt-get update \
 && apt-get install --yes --no-install-recommends \
      tesseract-ocr \
      libimage-exiftool-perl \
      gnupg \
      gpgv \
      age \
      minisign \
      dosfstools \
      mtools \
      squashfs-tools \
      ntfs-3g \
      xorriso \
      qemu-utils \
 && rm -rf /var/lib/apt/lists/*
USER builder
