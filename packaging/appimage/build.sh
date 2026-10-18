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
# a tag's source with the packaging of its own commit). The AppDir holds
# the program under usr/bin, the menu entry and the icon, and the README,
# the changelog and the licenses under usr/share/doc. The AppImage is run
# once with its own extract-and-run option, so a runner without FUSE
# still checks that it starts.
set -eu

bin="$1"
tag="$2"
out="$3"
tools="$4"
here="$(cd "$(dirname "$0")" && pwd)"
src="${5:-$(cd "$here/../.." && pwd)}"
arch="${ARCH:-$(uname -m)}"
appdir="$out/diro.AppDir"
icon="$src/assets/logo/png/diroxide-mark-256.png"

mkdir -p "$out" "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps" "$appdir/usr/share/doc/diroxide"
install -m 0755 "$bin" "$appdir/usr/bin/diro"
cp "$here/diro.desktop" "$appdir/diro.desktop"
cp "$here/diro.desktop" "$appdir/usr/share/applications/diro.desktop"
cp "$icon" "$appdir/diroxide.png"
cp "$icon" "$appdir/usr/share/icons/hicolor/256x256/apps/diroxide.png"
cp "$icon" "$appdir/.DirIcon"
ln -s usr/bin/diro "$appdir/AppRun"
cp "$src/README.md" "$src/CHANGELOG.md" "$src/LICENSE-APACHE" "$src/LICENSE-MIT" "$src/NOTICE" "$appdir/usr/share/doc/diroxide/"

file="$out/diro-$tag-linux-$arch.AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tools/appimagetool" --runtime-file "$tools/runtime-$arch" "$appdir" "$file"
chmod 0755 "$file"
echo "The AppImage reports:"
"$file" --appimage-extract-and-run --version
ls -l "$file"
