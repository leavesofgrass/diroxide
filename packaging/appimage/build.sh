#!/bin/sh
# Builds the Linux AppImage of diro from a built binary (plan section 8:
# one file per platform, an AppImage on Linux as textweaver ships).
#
#   packaging/appimage/build.sh BINARY TAG OUTDIR TOOLS [SOURCE]
#
# BINARY is the diro program built with the dist profile, TAG the version
# tag, OUTDIR where the AppImage lands, TOOLS the folder that
# packaging/appimage/fetch-tools.sh filled (appimagetool and runtime-ARCH),
# and SOURCE the checkout whose README, changelog, licenses and logo go in
# (default: the repository this script is in; the release workflow builds
# a tag's source with the packaging of its own commit). TOOLS given as
# `none` lays out the AppDir and stops, for a check with no tools at hand
# (packaging/appimage/check.sh takes the AppDir as well as the image).
#
# The AppDir holds the program under usr/bin, with AppRun a link to it
# and no wrapper script, so the program sees the environment it was
# started with: the D-Bus session and the AT-SPI bus, and the speech
# socket under XDG_RUNTIME_DIR, stay reachable from inside the image. The
# menu entries: diro.desktop opens the GUI (diro --gui; the image's own
# entry), diro-terminal.desktop the terminal (diro --tui, in a terminal).
# The icon: the mark at every hicolor size and as the scalable SVG
# (cargo xtask icons draws the sizes), the 256-pixel PNG at the top. The
# README, the changelog and the licenses go under usr/share/doc. The
# AppImage is run once with its own extract-and-run option, so a runner
# without FUSE still checks that it starts.
set -eu

bin="$1"
tag="$2"
out="$3"
tools="$4"
here="$(cd "$(dirname "$0")" && pwd)"
src="${5:-$(cd "$here/../.." && pwd)}"
arch="${ARCH:-$(uname -m)}"
appdir="$out/diro.AppDir"
icons="$src/assets/logo/png"
icon="$icons/diroxide-mark-256.png"
mark="$src/assets/logo/diroxide-mark.svg"

mkdir -p "$out" "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/doc/diroxide"
install -m 0755 "$bin" "$appdir/usr/bin/diro"
cp "$here/diro.desktop" "$appdir/diro.desktop"
cp "$here/diro.desktop" "$appdir/usr/share/applications/diro.desktop"
cp "$here/diro-terminal.desktop" "$appdir/usr/share/applications/diro-terminal.desktop"
cp "$icon" "$appdir/diroxide.png"
cp "$icon" "$appdir/.DirIcon"
# The hicolor sizes (xtask/src/icons.rs, HICOLOR_SIZES). A size a tag's
# source does not have yet is left out, and said.
for size in 16 22 24 32 48 64 128 256 512; do
  png="$icons/diroxide-mark-$size.png"
  if [ -f "$png" ]; then
    mkdir -p "$appdir/usr/share/icons/hicolor/${size}x${size}/apps"
    cp "$png" "$appdir/usr/share/icons/hicolor/${size}x${size}/apps/diroxide.png"
  else
    echo "No $size-pixel icon in this source; that size is left out."
  fi
done
mkdir -p "$appdir/usr/share/icons/hicolor/scalable/apps"
cp "$mark" "$appdir/usr/share/icons/hicolor/scalable/apps/diroxide.svg"
ln -sf usr/bin/diro "$appdir/AppRun"
cp "$src/README.md" "$src/CHANGELOG.md" "$src/LICENSE-APACHE" "$src/LICENSE-MIT" "$src/NOTICE" "$appdir/usr/share/doc/diroxide/"

if [ "$tools" = none ]; then
  echo "The AppDir is laid out in $appdir; no AppImage was packed (TOOLS is none)."
  exit 0
fi

file="$out/diro-$tag-linux-$arch.AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tools/appimagetool" --runtime-file "$tools/runtime-$arch" "$appdir" "$file"
chmod 0755 "$file"
echo "The AppImage reports:"
"$file" --appimage-extract-and-run --version
ls -l "$file"
