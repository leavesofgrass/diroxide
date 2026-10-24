Volume actions, an example diroxide plugin in Python.

It unmounts, ejects and mounts the volume on a row of the Volumes list,
with the system's own tool, and says the result in one sentence with the
volume's name first: "Backup is safe to remove." A busy volume is never
forced: "Could not eject Backup: files are open."

  Linux     Unmount, Eject and Mount, through udisksctl, with
            --no-user-interaction: a volume that needs a password says
            so instead of opening a window. List devices uses lsblk.
  macOS     Unmount, Eject and Mount, through diskutil. List devices
            uses diskutil list.
  Windows   Eject only, through the shell's Eject verb in PowerShell,
            started with no console window. Windows mounts and unmounts
            removable drives itself.

The volume arrives in the selection file: a device (/dev/sdb1), a mount
point, or a drive (E:\). Anything else is refused before a tool runs.
The tools run by an argument list, never through a shell, and never with
a force option. On Windows the drive reaches PowerShell through an
environment variable, never inside its command.

Check it:        diro plugin check volume-actions
Enable it:       diro plugin enable volume-actions
Preview first:   every command declares dry_run, so a preview says what
                 it would run and changes nothing.
Run its tests:   python3 -I -m unittest   (py -3 -I -m unittest on Windows)

The tests use fake tools on a test PATH; they never touch a real drive.
It needs Python 3 and nothing else, and uses no network.
