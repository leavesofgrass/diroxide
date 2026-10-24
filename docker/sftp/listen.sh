#!/usr/bin/env bash
# Starts an OpenSSH server in a throwaway copy of the development image for
# the owner's listening test of the SFTP places (alpha 2, stage 2), with a
# test account, a 50 MB file, a folder of 5,000 files and a small volume
# for the room check. Nothing of the owner's is read; everything lives in
# the container and is gone when it stops. From D:\diroxide, in Git Bash:
#
#   MSYS_NO_PATHCONV=1 docker run --rm -it -p 2222:22 \
#       --tmpfs /home/builder/small:size=20m,uid=1000,gid=1000 \
#       -v "$PWD:/work" --workdir /work diroxide-dev bash docker/sftp/listen.sh
#
# Then, in diro: Ctrl+L, sftp://builder@localhost:2222/ and the password
# this script prints. Ctrl+C in the server's terminal stops it. To change
# the host key for the changed-key check, stop the server and start it
# again with DIRO_SFTP_NEW_KEY=1.
set -euo pipefail

if [ "$(id -u)" != 0 ]; then
    echo "listen.sh: start it as root in the container (docker run --user root)." >&2
    exit 2
fi
if [ ! -f /.dockerenv ]; then
    echo "listen.sh: only in a throwaway container; it changes PAM and a password." >&2
    exit 2
fi

user="${DIRO_SFTP_USER:-builder}"
id "$user" >/dev/null
here="$(cd "$(dirname "$0")" && pwd)"
home="/home/$user"

password="$(head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 12)"
echo "$user:$password" | chpasswd
cp "$here/pam-sshd" /etc/pam.d/sshd

keys=/tmp/diro-sftp-listen
if [ -n "${DIRO_SFTP_NEW_KEY:-}" ] || [ ! -d "$keys" ]; then
    rm -rf "$keys"
    mkdir -p "$keys"
    ssh-keygen -q -t ed25519 -N '' -f "$keys/host_ed25519"
fi
mkdir -p /run/sshd

data="$home/listen"
if [ ! -d "$data" ]; then
    mkdir -p "$data/many" "$data/small"
    head -c 52428800 /dev/urandom > "$data/big.bin"
    for i in $(seq -w 1 5000); do echo "file $i" > "$data/many/file-$i.txt"; done
    chown -R "$user:$user" "$data"
fi
if mountpoint -q "$home/small"; then chown "$user:$user" "$home/small"; fi

echo "SFTP server ready."
echo "Account: $user"
echo "Password: $password"
echo "Place: sftp://$user@localhost:2222/"
echo "Data: $data (big.bin 50 MB, many/ 5,000 files); small volume at $home/small"
echo "Stop: Ctrl+C here."

exec /usr/sbin/sshd -D -e -f "$here/sshd_config" \
    -o "HostKey=$keys/host_ed25519" -o Port=22 -o ListenAddress=0.0.0.0 \
    -o PasswordAuthentication=yes -o KbdInteractiveAuthentication=yes -o PubkeyAuthentication=yes \
    -o "Subsystem=sftp internal-sftp" -o PidFile=none
