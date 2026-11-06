#!/usr/bin/env bash
# Runs the disk-image plugin's tests in the development image's tools
# stage (R29 1.10), where 7-Zip 25.01 and qemu-img are, so the tests that
# skip or check words elsewhere run the real tools here: the 7-Zip
# fallback unpacks a member larger than one answer, and qemu-img converts
# a QCOW2 whose data is in its backing file.
#
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=<id> docker compose run --rm tools \
#       bash docker/disk/run-tests.sh
#
# Nothing is downloaded and nothing of the owner's is read: the images
# are the committed fixtures (made by make-images.sh) and the ones the
# tests write. Extra arguments go to cargo nextest.
set -euo pipefail

for tool in 7z qemu-img; do
    if command -v "$tool" > /dev/null; then
        echo "run-tests.sh: $tool found: $(command -v "$tool")."
    else
        echo "run-tests.sh: $tool not found; its tests check words instead." >&2
    fi
done

cargo nextest run --manifest-path plugins/diro-disk/Cargo.toml --all-features \
    --no-fail-fast "$@"
