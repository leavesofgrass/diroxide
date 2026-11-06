#!/usr/bin/env bash
# Runs the git plugin's tests against the development image's git (2.47.3
# in Debian's packages, R29 2.7), so the plugin's real-git tests run on
# Linux as they do on every CI runner: status, the trust scan's proof,
# commits, stashes, file:// remotes and clones.
#
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=<id> docker compose run --rm dev \
#       bash docker/git/run-tests.sh
#
# Nothing is downloaded and nothing of the owner's is read: every git the
# tests run has its own empty global configuration, no system one, a home
# of its own and a placeholder author, and every remote is a folder of the
# test's own. Extra arguments go to cargo nextest.
set -euo pipefail

if ! command -v git > /dev/null; then
    echo "run-tests.sh: git not found; the plugin's tests need it." >&2
    exit 1
fi
echo "run-tests.sh: $(git --version), at $(command -v git)."

cargo nextest run --manifest-path plugins/diro-git/Cargo.toml --all-features \
    --locked --no-fail-fast "$@"
