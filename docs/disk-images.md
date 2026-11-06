# Disk images

This guide covers disk images: opening an ISO, a raw disk or a virtual disk like a folder, its partitions, copying files out, what the plugin reads itself and what it hands to 7-Zip, the limits that protect you, and what it does not do.

Disk images are read by the disk image plugin (its id is `disk`, its name "Disk images"), an optional component. Get it from Manage optional components. Without it, Enter on `fedora.iso` says "Could not open fedora.iso: it needs the disk image plugin or 7-Zip. Get it from Manage optional components, then try again."

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## What opens, and how

A disk image opens by its name: `.iso`, `.vhd`, `.vhdx`, `.vdi`, `.vmdk`, `.qcow2`, `.squashfs` and `.sfs` are listed as archives, and Enter or Right opens one in the panel. Raw images are named too loosely to open by name (`.img`, `.raw`, `.bin`, `.dd` are as often pictures or firmware), so they open with Open as archive, which reads the file's first bytes: on `card.img` it opens the disk image.

What the plugin opens is decided by the bytes, never the name:

- **Containers:** VHD (fixed and dynamic), VHDX, VDI (fixed and dynamic), VMDK (a sparse extent, or a descriptor whose flat extents sit beside it) and QCOW2 (version 2 and 3, compressed clusters included). A differencing VHD or VHDX, and a QCOW2 with a backing file, need their parent; see "7-Zip and qemu-img" below.
- **Partition tables:** MBR, with its logical partitions, and GPT.
- **File systems:** ISO 9660 with Joliet and Rock Ridge, FAT12, FAT16 and FAT32 with long names, ext2, ext3 and ext4, NTFS, and squashfs compressed with gzip or LZ4.
- **Through 7-Zip, when it is installed:** UDF, exFAT, HFS+, APFS, squashfs compressed with xz or zstd, and anything a built-in reader refuses. An image on a local drive goes to 7-Zip as it is; a partition, or an image inside a container, an archive or on an SFTP site, is copied into the plugin's folder in the spool first.

A hybrid ISO, one that also carries a partition table so it boots from a stick, opens as the ISO.

## Partitions are folders

A disk with a partition table lists one folder per partition, `partition-1`, `partition-2`, in table order, even when there is only one, so a path never changes its meaning. The plugin describes each partition in words, number first:

- "Partition 2, home, ext4, 40.0 GB": the label first, then the file system and the size.
- "Partition 1, EFI System, FAT32, 537 MB": with no label, the table's type in words.
- "Partition 3, BitLocker, 40.0 GB, locked": a BitLocker or LUKS partition is shown and never opened.

In this alpha the panel's row says the folder's name, "partition-2, folder", and the title inside it says "partition-2, in disk.img". The volumes list says each partition's words with its size and free space ("Partition 2, home, ext4", then the space), so it is the place to hear which partition is which. A disk with no table, such as an ISO or a floppy, is its file system: `fedora.iso!EFI/BOOT` is a folder in it, and the title says "EFI, in fedora.iso".

When an image opens, the status line says what it holds, said on Say Status: "Opened disk.vhdx: 3 partitions, 120 GB".

## What a row says

A file's row is like any other: its name first, then its kind and size. What is unusual comes right after the name:

- A link in an image is a link row with its target as text. It is never followed, and reading it gives the target's text, not the file it names.
- A name with a control character or bytes that are not text is shown with stand-ins and says "name needs care". A name that would leave the image (`..`), or one longer than the limits, is left out, and the status line says how many after the listing: "3 names skipped".
- A file NTFS compressed or encrypted is listed; reading a compressed one says "Could not read report.txt: NTFS compressed it, and only 7-Zip reads that." The plugin does not yet hand such a file to 7-Zip; 7-Zip itself reads it from the image. An NTFS file's other data streams are never rows, because a colon in a name is reserved on Windows.
- A folder that leads back to one it is inside is left out, and the status line says "Skipped a loop in the image". No path goes more than 64 folders deep.

## Copy a file out

Images are read only. Mark files and press F5 with the other panel on a folder; the files are read straight from the image, nothing is unpacked first, and nothing in the image ever changes. F3 views a file in place. Copy into an image, rename, delete and make folder are refused.

An image inside an image opens the same way, `disk.img!partition-1/boot.iso!EFI`, and an image inside a zip or on an SFTP site too: `bundle.zip!disk.iso!EFI` reads the image's bytes through the zip, a block at a time. Images nest at most four levels deep, counted with the archives on the way: "it passes the limit for nesting: 4 archives deep".

## 7-Zip and qemu-img

The plugin uses two programs when they are there and works without them. Settings names them; nothing turns off when they are missing.

- **7-Zip** reads what no built-in reader opens. The status line says "Read with 7-Zip: no built-in reader" once, and the opened line names the format with "read with 7-Zip". Without 7-Zip such an image or partition says "it needs 7-Zip to read squashfs with xz", with Get 7-Zip from Manage optional components as the next step. A file up to about 6 MB comes back through a pipe; a larger one is unpacked into the plugin's folder in the spool, only by 7-Zip 25.01 or later, which fixed two link bugs that could write outside that folder. An older 7-Zip says "7-Zip 24.08 is too old: needs 25.01" for such a file. 7-Zip shows names in an ISO that are not plain ASCII as underscores; the built-in ISO reader reads them right.
- **qemu-img** converts a container nothing else reads, such as a QCOW2 with a backing file, into a raw disk in the spool, and only when the size the container claims fits the spool limit.

An encrypted image through 7-Zip says "it is encrypted, and this plugin asks no password". A password never goes on a command line.

## The limits that protect you

The archive limits of `config.toml` apply to images too, by their defaults:

- A sparse container (dynamic VHD, VHDX, VDI, VMDK, QCOW2) may claim far more than its file holds; that is normal, and the status line names both figures when it opens: "Sparse: claims 2.2 TB, holds 1.1 GB". Nothing is ever made at the claimed size: a listing reads the tables and the blocks it needs. A copy into the spool (for 7-Zip or qemu-img) that would pass 4 GiB is refused: "it is too large to unpack here: claims 2.2 TB, holds 1.1 GB".
- A folder with more than 1,000,000 entries is refused; a name over 4,096 bytes, or a part over 255, is left out and counted.
- A table, folder record, block map or region that claims more than 256 MiB is refused before it is read: "its block map claims 3.0 GB, over the 268 MB limit".
- If the image file changes while it is open (it grows, or its time changes), the plugin says "Image changed since it was opened", and the next listing opens it again.

## Not in this alpha

- Writing to an image, making one, and formatting are the plugin's writing half, in a later alpha.
- Open as raw disk image, which would show a hybrid ISO's partitions, is not built.
- An encrypted image or archive through 7-Zip is not asked a password.
- ext4 images made by `mke2fs` are not among the plugin's test images yet; small ext2 and ext4 volumes written by the tests are.

## See also

- [Archives](archives.md), which diroxide reads itself
- [Optional components](components.md), to get the plugin or 7-Zip
- [Plugins](plugins.md), to turn a plugin on or off
- [Remote places](remote-places.md), for an image on an SFTP site
- [Known limits](known-limits.md)
- [The documentation index](README.md)
