#!/bin/sh
# Builds the Linux AppImage of diro from a built binary (plan section 8:
# one file per platform, an AppImage on Linux as textweaver ships).
#
#   packaging/appimage/build.sh BINARY TAG OUTDIR TOOLS
#
# BINARY is the diro program built with the dist profile, TAG the version
# tag, OUTDIR where the AppImage lands, TOOLS the folder that
# packaging/appimage/fetch-tools.sh filled (appimagetool and runtime-ARCH).
# The AppDir holds the program under usr/bin, the menu entry and the icon,
# and the README, the changelog and the licenses under usr/share/doc.
# The AppImage is run once with its own extract-and-run option, so a
# runner without FUSE still checks that it starts.
set -eu

bin="$1"
tag="$2"
out="$3"
tools="$4"
arch="${ARCH:-$(uname -m)}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
appdir="$out/diro.AppDir"

mkdir -p "$out" "$appdir/usr/bin" "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/256x256/apps" "$appdir/usr/share/doc/diroxide"
install -m 0755 "$bin" "$appdir/usr/bin/diro"
cp "$root/packaging/appimage/diro.desktop" "$appdir/diro.desktop"
cp "$root/packaging/appimage/diro.desktop" "$appdir/usr/share/applications/diro.desktop"
cp "$root/assets/logo/png/diroxide-mark-256.png" "$appdir/diroxide.png"
cp "$root/assets/logo/png/diroxide-mark-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/diroxide.png"
cp "$appdir/diroxide.png" "$appdir/.DirIcon"
ln -s usr/bin/diro "$appdir/AppRun"
cp "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE-APACHE" "$root/LICENSE-MIT" "$root/NOTICE" \
  "$appdir/usr/share/doc/diroxide/"

file="$out/diro-$tag-linux-$arch.AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$tools/appimagetool" \
  --runtime-file "$tools/runtime-$arch" "$appdir" "$file"
chmod 0755 "$file"
echo "The AppImage reports:"
"$file" --appimage-extract-and-run --version
ls -l "$file"
