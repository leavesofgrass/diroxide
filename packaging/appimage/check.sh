#!/usr/bin/env bash
# Checks that the Linux AppImage starts its GUI and that a screen reader
# reaches it, on a virtual display with no screen reader of its own:
#
#   packaging/appimage/check.sh IMAGE
#
# IMAGE is the AppImage that packaging/appimage/build.sh packed, or the
# AppDir it laid out (TOOLS given as none). Needs the GUI image stage
# (docker/Dockerfile, target gui: Xvfb, D-Bus, the AT-SPI bus, Mesa's
# software Vulkan, pyatspi; the owner's answer 107), where there is no
# FUSE, so the image runs with its extract-and-run option:
#
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=g4 docker compose run --rm gui bash packaging/appimage/check.sh /target/appimage/diro.AppDir
#
# What it checks, each said as Pass or Fail:
#
# 1. The image's layout: AppRun is a link to usr/bin/diro, no wrapper
#    script, so the program sees the environment it was started with (the
#    D-Bus session, the AT-SPI bus, and the speech socket under
#    XDG_RUNTIME_DIR); the two menu entries and the icons are there.
# 2. The image starts the GUI on the virtual display: diro --gui draws,
#    and its --log names the drawing road, within 60 seconds.
# 3. The AT-SPI check (tools/a11y/atspi-check.sh) against the program
#    inside the image: its window and both panels on the AT-SPI bus, as
#    Orca reads them.
#
# Its own config and state folders, never the user's; plays no audio.
set -euo pipefail

# The start on the virtual display: xvfb-run is a POSIX shell script and
# passes no shell function on, so the check calls itself there:
#   check.sh --start WORK PROGRAM [ARGUMENTS]
# setsid, so the image's runtime and the program it started end together.
if [[ "${1:-}" == "--start" ]]; then
  work="$2"
  shift 2
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
  setsid "$@" --gui --log "$work/start.log" "$work/sample" >"$work/start.out" 2>&1 &
  group=$!
  drew=1
  for _ in $(seq 1 240); do
    if grep -q '^graphics: drawing with ' "$work/start.log" 2>/dev/null; then
      drew=0
      break
    fi
    kill -0 "$group" 2>/dev/null || break
    sleep 0.25
  done
  kill -- "-$group" 2>/dev/null || true
  wait "$group" 2>/dev/null || true
  exit $drew
fi

image="${1:?Usage: packaging/appimage/check.sh IMAGE (an AppImage or an AppDir)}"
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
status=0
pass() { echo "Pass: $*"; }
fail() { echo "Fail: $*"; status=1; }

# The AppDir: the one given, or the image's own files, extracted.
if [[ -d "$image" ]]; then
  appdir="$(cd "$image" && pwd)"
  run=("$appdir/AppRun")
else
  image="$(cd "$(dirname "$image")" && pwd)/$(basename "$image")"
  (cd "$work" && "$image" --appimage-extract >/dev/null)
  appdir="$work/squashfs-root"
  run=(env APPIMAGE_EXTRACT_AND_RUN=1 "$image")
fi

echo "## 1. The image's layout"
if [[ -L "$appdir/AppRun" && "$(readlink "$appdir/AppRun")" == usr/bin/diro ]]; then
  pass "AppRun is a link to usr/bin/diro, so the program keeps the environment it is started with."
else
  fail "AppRun is not a link to usr/bin/diro."
fi
grep -qx 'Exec=diro --gui' "$appdir/diro.desktop" && grep -qx 'Terminal=false' "$appdir/diro.desktop" \
  && pass "diro.desktop opens the GUI, with no terminal." || fail "diro.desktop does not open the GUI."
grep -qx 'Exec=diro --tui' "$appdir/usr/share/applications/diro-terminal.desktop" \
  && grep -qx 'Terminal=true' "$appdir/usr/share/applications/diro-terminal.desktop" \
  && pass "diro-terminal.desktop opens the terminal, in a terminal." || fail "diro-terminal.desktop is missing or wrong."
missing=""
for size in 16 22 24 32 48 64 128 256 512; do
  [[ -f "$appdir/usr/share/icons/hicolor/${size}x${size}/apps/diroxide.png" ]] || missing="$missing $size"
done
[[ -f "$appdir/usr/share/icons/hicolor/scalable/apps/diroxide.svg" ]] || missing="$missing scalable"
[[ -z "$missing" ]] && pass "the icon at every hicolor size and as the SVG." || fail "icons missing:$missing."
if command -v desktop-file-validate >/dev/null; then
  for entry in "$appdir/diro.desktop" "$appdir/usr/share/applications/diro-terminal.desktop"; do
    desktop-file-validate "$entry" && pass "$(basename "$entry") validates." || fail "$(basename "$entry") does not validate."
  done
fi

# The image's start, with its own folders.
mkdir -p "$work/sample/Backup" "$work/home"
printf 'diroxide\n' >"$work/sample/notes.txt"
export XDG_CONFIG_HOME="$work/config" XDG_STATE_HOME="$work/state" HOME="$work/home"

echo
echo "## 2. The GUI starts from the image"
if xvfb-run -a -s "-screen 0 1280x900x24" dbus-run-session -- bash "$here/check.sh" --start "$work" "${run[@]}"; then
  pass "the image opened the GUI and drew: $(grep '^graphics: drawing with ' "$work/start.log" | head -n 1)"
else
  fail "the image did not draw within 60 seconds."
  sed 's/^/    /' "$work/start.out" "$work/start.log" 2>/dev/null || true
fi

echo
echo "## 3. A screen reader reaches the program inside the image"
if EXE="$appdir/AppRun" bash "$repo/tools/a11y/atspi-check.sh" >"$work/atspi.md"; then
  pass "the AT-SPI check found the window and its panels."
else
  fail "the AT-SPI check did not pass."
fi
sed 's/^/    /' "$work/atspi.md"

echo
if [[ $status == 0 ]]; then
  echo "Pass: the AppImage starts its GUI and a screen reader reaches it."
else
  echo "Fail: see the lines above."
fi
exit $status
