#!/usr/bin/env bash
# Runs the SFTP plugin's container tests (R25 commit 76; answer 99) in a
# throwaway copy of the development image, locally or on the Linux CI job:
#
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=<id> docker compose run --rm --user root \
#       dev bash docker/sftp/run-tests.sh
#
# As root it does only the two things a password sign-in needs and a
# throwaway container allows: a password for the container's own user,
# made now and kept in a file only that user can read, and the small PAM
# stack of pam-sshd. Then, as that user, it runs the tests, which start
# and stop their own sshd on a loopback port, with keys they make. Nothing
# listens outside the container; nothing of the owner's is read.
#
# Extra arguments go to cargo nextest. DIRO_SFTP_CAPTURE=1 also writes
# ssh's real standard-error texts and prompts into fixtures/sftp/.
set -euo pipefail

if [ "$(id -u)" != 0 ]; then
    echo "run-tests.sh: start it as root in the container (docker compose run --user root)." >&2
    exit 2
fi
if [ ! -f /.dockerenv ]; then
    echo "run-tests.sh: only in a throwaway container; it changes PAM and a password." >&2
    exit 2
fi

user="${DIRO_SFTP_USER:-builder}"
id "$user" >/dev/null
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d /tmp/diro-sftp-run-XXXXXX)"
chown "$user" "$work"
chmod 700 "$work"

password_file="$work/password"
head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 24 > "$password_file"
echo "$user:$(cat "$password_file")" | chpasswd
chown "$user" "$password_file"
chmod 600 "$password_file"
cp "$here/pam-sshd" /etc/pam.d/sshd

runuser -u "$user" -m -- env \
    HOME="/home/$user" USER="$user" LOGNAME="$user" \
    DIRO_SFTP_TEST_PASSWORD_FILE="$password_file" \
    DIRO_SFTP_CAPTURE="${DIRO_SFTP_CAPTURE:-}" \
    cargo nextest run --manifest-path plugins/diro-sftp/Cargo.toml --all-features \
        --run-ignored only -E 'test(/^server::/)' --no-fail-fast "$@"
