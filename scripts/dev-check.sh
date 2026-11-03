#!/usr/bin/env bash
# Runs the full native check of the workspace, in order, and stops at the
# first failure. Each step says its name first; the last line says Pass or
# Fail in words. Works from any folder; runs in the repository's root so
# the pinned toolchain and the cargo xtask alias apply.
#
#   scripts/dev-check.sh
#
# Set CARGO_TARGET_DIR first when several builders share the machine.
#
# The full check runs the slow tests: it sets DIRO_SLOW=1 unless it is
# already set (docs/dev/building.md, "Slow tests"). DIRO_SLOW=0 runs the
# same check without them, as a builder's quick run does.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export DIRO_SLOW="${DIRO_SLOW:-1}"
if [ "$DIRO_SLOW" = 1 ]; then
    printf 'Slow tests: run, DIRO_SLOW is 1.\n'
else
    printf 'Slow tests: skipped, DIRO_SLOW is not 1.\n'
fi

step() {
    local name="$1"
    shift
    printf 'Check: %s\n' "$name"
    if ! "$@"; then
        printf 'Fail: %s\n' "$name"
        exit 1
    fi
}

step "format" cargo fmt --all --check
step "clippy, warnings denied" cargo clippy --workspace --all-targets --all-features -- -D warnings
step "tests" cargo nextest run --workspace --all-features
step "rustdoc, warnings denied" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
step "dependency direction" cargo xtask deps --check
step "pure Rust, no C build" cargo xtask native --check
step "keyboard page" cargo xtask keyboard --check
step "generated files" cargo xtask regen --check

printf 'Pass: every check.\n'
