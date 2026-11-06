#!/usr/bin/env bash
# Makes the disk-image plugin's test images with the real tools (R29
# 1.10, R30 commit 96), in the development image's tools stage, which
# carries them (the owner's answer 118):
#
#   MSYS_NO_PATHCONV=1 DIRO_AGENT=<id> docker compose run --rm tools \
#       bash docker/disk/make-images.sh plugins/diro-disk/fixtures
#
# Nothing is downloaded: every image is made here from a small tree of
# files written by this script. Each image is written gzip-compressed
# into OUT (they are mostly zeros, so each is a few kilobytes), with the
# SHA-256 of the image itself in OUT/SHA256SUMS, so the tests on Windows
# and macOS read real NTFS, squashfs, xorriso ISOs and qemu-img's
# containers without the tools. The raw images live only in the
# container's own temporary folder, which goes with the container.
#
# Not made: ext4, because the image has no mke2fs (e2fsprogs is not in
# the tools stage; the plugin's tests write an ext2 volume by hand), and
# exFAT (no mkfs.exfat).
set -euo pipefail

out="${1:?usage: make-images.sh OUT_DIR}"
mkdir -p "$out"
work="$(mktemp -d /tmp/diro-disk-images-XXXXXX)"
src="$work/src"

# The tree every image holds, as far as its file system allows. Every
# file made here has a plain name; the one whose name inside the images
# has spaces ("A Long File Name.txt") is made as long.txt beside the tree
# and named in each image by the tool that writes it, so no file with a
# space in its name is ever made, even in this throwaway folder.
mkdir -p "$src/docs" "$src/deep/a/b/c/d/e/f/g/h/i/j"
printf 'Hello, world.' > "$src/hello.txt"
printf 'inside' > "$src/docs/inner.txt"
printf 'a long name' > "$work/long.txt"
spaced='A Long File Name.txt'
printf 'deep down' > "$src/deep/a/b/c/d/e/f/g/h/i/j/deep.txt"
ln -s hello.txt "$src/link"
touch -d '2026-01-02 03:04:05 UTC' "$src/hello.txt" "$src/docs/inner.txt"

pack() {
    local name="$1"
    gzip -9 -n -c "$work/$name" > "$out/$name.gz"
    (cd "$work" && sha256sum "$name") >> "$work/sums"
    printf 'made %s: %s bytes, %s gzipped\n' "$name" \
        "$(stat -c %s "$work/$name")" "$(stat -c %s "$out/$name.gz")"
}

# NTFS: mkntfs on a plain file, the files copied in with ntfscp (it
# cannot make folders), one with a second data stream.
truncate -s 2M "$work/ntfs.img"
mkntfs -F -Q -q -s 512 -L ntfsvol "$work/ntfs.img"
ntfscp -q "$work/ntfs.img" "$src/hello.txt" hello.txt
ntfscp -q "$work/ntfs.img" "$work/long.txt" "$spaced"
ntfscp -q -N secret "$work/ntfs.img" "$src/docs/inner.txt" hello.txt
# The $UpCase table is 128 KiB that no compressor shrinks (each entry
# differs from the next) and that the plugin's reader never reads, so it
# is zeroed to keep the fixture under 64 KB gzipped; Windows would not
# mount the result, and no test asks it to.
python3 - "$work/ntfs.img" <<'PY'
import sys
path = sys.argv[1]
data = bytearray(open(path, "rb").read())
start = bytes(b for i in range(64) for b in i.to_bytes(2, "little"))
at = data.find(start)
assert at > 0, "no $UpCase table found"
data[at:at + 128 * 1024] = bytes(128 * 1024)
open(path, "wb").write(data)
PY
pack ntfs.img

# squashfs, with the codec the plugin reads itself and one it gives to
# 7-Zip.
long="\"$spaced\" f 644 0 0 cat $work/long.txt"
mksquashfs "$src" "$work/gzip.sqfs" -comp gzip -noappend -all-root -quiet -p "$long"
pack gzip.sqfs
mksquashfs "$src" "$work/xz.sqfs" -comp xz -noappend -all-root -quiet -p "$long"
pack xz.sqfs
mksquashfs "$src" "$work/lz4.sqfs" -comp lz4 -noappend -all-root -quiet -p "$long"
pack lz4.sqfs
# A member larger than one answer of the host's (7 MB of repeated text),
# which 7-Zip must unpack into the spool rather than pipe.
mkdir -p "$work/big"
python3 - "$work/big/big.txt" <<'PY'
import sys
line = b"Seven megabytes of the same line, for the spool path.\n"
open(sys.argv[1], "wb").write((line * 140000)[:7340032])
PY
mksquashfs "$work/big" "$work/big-xz.sqfs" -comp xz -noappend -all-root -quiet
pack big-xz.sqfs

# An ISO from xorriso with Rock Ridge and Joliet: a long name, a link,
# and a folder deeper than ISO 9660's eight levels, which Rock Ridge
# relocates.
xorriso -as mkisofs -quiet -R -J -V HARDISO -graft-points -o "$work/xorriso.iso" \
    /="$src" "$spaced=$work/long.txt"
pack xorriso.iso

# FAT made by mkfs.fat and mtools, then the same disk in each container
# qemu-img writes.
mkfs.fat -C -n FATVOL "$work/fat.img" 2048 > /dev/null
mcopy -i "$work/fat.img" "$src/hello.txt" ::hello.txt
mmd -i "$work/fat.img" ::docs
mcopy -i "$work/fat.img" "$src/docs/inner.txt" ::docs/inner.txt
pack fat.img
qemu-img convert -q -f raw -O qcow2 "$work/fat.img" "$work/fat.qcow2"
pack fat.qcow2
qemu-img convert -q -c -f raw -O qcow2 "$work/fat.img" "$work/deflate.qcow2"
pack deflate.qcow2
qemu-img convert -q -f raw -O vmdk "$work/fat.img" "$work/fat.vmdk"
pack fat.vmdk
qemu-img convert -q -f raw -O vhdx "$work/fat.img" "$work/fat.vhdx"
pack fat.vhdx
qemu-img convert -q -f raw -O vdi "$work/fat.img" "$work/fat.vdi"
pack fat.vdi
qemu-img convert -q -f raw -O vpc "$work/fat.img" "$work/dynamic.vhd"
pack dynamic.vhd
qemu-img convert -q -f raw -O vpc -o subformat=fixed "$work/fat.img" "$work/fixed.vhd"
pack fixed.vhd
# A QCOW2 whose data is all in its backing file, named plainly beside it:
# only qemu-img reads it (the plugin's reader and 7-Zip refuse a backing
# file), so the tests copy fat.qcow2 next to it.
(cd "$work" && qemu-img create -q -f qcow2 -b fat.qcow2 -F qcow2 overlay.qcow2)
pack overlay.qcow2

sort -k 2 "$work/sums" > "$out/SHA256SUMS"
echo "make-images.sh: wrote $(wc -l < "$out/SHA256SUMS") images to $out"
