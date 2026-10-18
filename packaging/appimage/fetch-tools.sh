#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Carried from textweaver's docker/appimage/fetch-tools.sh under the
# owner's relicensing (plan answer 41), the pins as it had them.
#
# Downloads the AppImage tools that packaging/appimage/build.sh uses, from
# their official GitHub releases, and checks each against the SHA-256
# digest GitHub publishes for that release asset (pinned here, so a changed
# file fails the build):
#
# - appimagetool 1.9.1 (github.com/AppImage/appimagetool), which packs the
#   AppDir into a squashfs image;
# - the type 2 runtime 20251108 (github.com/AppImage/type2-runtime), the
#   small program at the front of every AppImage. appimagetool would fetch
#   an unpinned "continuous" runtime by itself; passing this one keeps the
#   build reproducible.
#
#   packaging/appimage/fetch-tools.sh [DIR]     (default: /opt/appimage)
#
# Afterwards, DIR/appimagetool and DIR/runtime-ARCH exist.
set -eu

dir="${1:-/opt/appimage}"
arch="${ARCH:-$(uname -m)}"

APPIMAGETOOL_VERSION=1.9.1
RUNTIME_VERSION=20251108

case $arch in
  x86_64)
    tool_sha=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
    runtime_sha=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
    ;;
  aarch64)
    tool_sha=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
    runtime_sha=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
    ;;
  *)
    echo "No AppImage tools are pinned for $arch." >&2
    exit 1
    ;;
esac

fetch() {
  url="$1" out="$2" want="$3"
  echo "Downloading $url"
  curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$out.part" "$url"
  got="$(sha256sum "$out.part" | cut -d' ' -f1)"
  if [ "$got" != "$want" ]; then
    echo "Checksum mismatch for $url: expected $want, got $got." >&2
    exit 1
  fi
  mv "$out.part" "$out"
  echo "Verified $out (sha256 $got)"
}

mkdir -p "$dir"
fetch "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$arch.AppImage" \
  "$dir/appimagetool" "$tool_sha"
chmod 0755 "$dir/appimagetool"
fetch "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-$arch" \
  "$dir/runtime-$arch" "$runtime_sha"
