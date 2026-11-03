#!/usr/bin/env bash
# The AT-SPI check for diroxide's window on Linux, with no display of its
# own and no screen reader: Xvfb, a private D-Bus session, the AT-SPI bus,
# and Mesa's software Vulkan (lavapipe), which is a software adapter, so
# the window draws on the processor through softbuffer on X11 (R27's
# commit 31). It opens diro --gui on a sample folder and runs
# atspi-dump.py against it; the report is Markdown on standard output,
# with the drawing road from diro's --log.
#
# Needs the GUI image stage (docker/Dockerfile, target gui; the owner's
# answer 107):
#
#   MSYS_NO_PATHCONV=1 docker compose build gui
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=g1 docker compose run --rm gui bash tools/a11y/atspi-check.sh
#
# Environment: EXE (default $CARGO_TARGET_DIR/debug/diro, built when
# missing), SECONDS_TO_WATCH (default 6). Plays no audio. Adapted from
# textweaver's crates/textweaver-xilem/tools/atspi-check.sh (the owner's
# own project).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
target="${CARGO_TARGET_DIR:-$repo/target}"
exe="${EXE:-$target/debug/diro}"
seconds="${SECONDS_TO_WATCH:-6}"

if [[ "${1:-}" == "--inner" ]]; then
  inner=1
else
  inner=0
fi

if [[ $inner == 0 && -z "${EXE:-}" && ! -x "$exe" ]]; then
  (cd "$repo" && cargo build -p diro)
fi

run() {
  export NO_AT_BRIDGE=0
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
  /usr/libexec/at-spi-bus-launcher --launch-immediately >/dev/null 2>&1 &
  sleep 1
  # AccessKit's AT-SPI adapter gives the tree while assistive technology
  # is said to be on (textweaver's ADR-0039).
  for prop in IsEnabled ScreenReaderEnabled; do
    dbus-send --session --print-reply --dest=org.a11y.Bus /org/a11y/bus \
      org.freedesktop.DBus.Properties.Set string:org.a11y.Status "string:$prop" \
      variant:boolean:true >/dev/null 2>&1 || true
  done
  local work
  work="$(mktemp -d)"
  mkdir -p "$work/sample/Backup"
  for name in notes.txt report.pdf budget.xlsx raccoon.txt; do
    printf 'diroxide\n' >"$work/sample/$name"
  done
  printf 'old\n' >"$work/sample/Backup/old.txt"
  # Its own state and config folders, never the user's.
  export XDG_CONFIG_HOME="$work/config" XDG_STATE_HOME="$work/state" HOME="$work/home"
  mkdir -p "$HOME"
  "$exe" --gui --log "$work/gui.log" "$work/sample" "$work/sample/Backup" >"$work/diro.log" 2>&1 &
  local gui=$!
  local status=0
  python3 "$here/atspi-dump.py" --pid "$gui" --seconds "$seconds" || status=$?
  # The check's own window: ended once the report is written.
  kill "$gui" 2>/dev/null || true
  wait "$gui" 2>/dev/null || true
  echo
  echo "## The drawing road (--log)"
  echo
  sed 's/^/- /' "$work/gui.log" 2>/dev/null || echo "- (no log)"
  echo
  echo "## diro's output"
  echo
  echo '```'
  cat "$work/diro.log" || true
  echo '```'
  return $status
}

if [[ $inner == 1 ]]; then
  run
else
  export EXE="$exe" SECONDS_TO_WATCH="$seconds"
  xvfb-run -a -s "-screen 0 1280x900x24" dbus-run-session -- bash "$here/atspi-check.sh" --inner
fi
