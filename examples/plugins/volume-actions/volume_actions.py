#!/usr/bin/env python3
"""Volume actions: Unmount, Eject and Mount for diroxide's Volumes list.

An example diroxide plugin of manifest-only commands, standard library
only. diroxide starts it with an argument list (see plugin.toml) and the
volume's identity in the selection file: a device node (/dev/sdb1), a
disk identifier's node (/dev/disk2s1), a mount point, or on Windows a
drive (E:\\). It runs the system's own tool for the job, by an argument
list and never through a shell:

    Linux    udisksctl unmount, power-off and mount, with
             --no-user-interaction, so a password question fails in words
             instead of opening a window; lsblk lists the devices
    macOS    diskutil unmount, eject and mount; diskutil list lists them
    Windows  the shell's Eject verb, through PowerShell with no console
             window; Windows mounts removable drives itself

Rules it keeps:

- Never a force option. A busy volume says "Could not eject Backup:
  files are open." and nothing is forced.
- Words first: each result is one sentence with the volume's name, said
  through diroxide's directives file ("Backup is safe to remove.").
- DIRO_DRY_RUN=1 changes nothing and says what it would run.
- The identity is checked before any tool runs, and reaches PowerShell
  through an environment variable, never inside its command text.

Exit codes: 0 done; 1 failed, nothing changed; 2 not a volume; 4 busy,
nothing forced; 5 the system asked for a password; 6 the tool is missing.
"""

import base64
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys

DONE, FAILED, WRONG, BUSY, AUTH, NO_TOOL = 0, 1, 2, 4, 5, 6

# How long a tool may take: macOS gives up on a busy unmount after about
# a minute.
TOOL_TIMEOUT = 75

# What the tools say when a volume is busy or a password is needed.
BUSY_WORDS = re.compile(
    r"target is busy|device ?busy|resource busy|in use|dissented|"
    r"could not be unmounted|failed to unmount",
    re.IGNORECASE)
AUTH_WORDS = re.compile(r"not ?authori[sz]ed|authentication|polkit",
                        re.IGNORECASE)

LINUX_DEVICE = re.compile(r"^/dev/[A-Za-z0-9][A-Za-z0-9._/-]*$")
MAC_DEVICE = re.compile(r"^/dev/r?disk[0-9]+(s[0-9]+)*$")
WINDOWS_DRIVE = re.compile(r"^([A-Za-z]):\\?$")

# The shell's Eject verb for one drive, which PowerShell runs from its
# encoded command; the drive comes from the environment and is checked
# again here. 0 ejected; 2 not a drive; 3 not found; 4 still there.
EJECT_SCRIPT = r"""
$ErrorActionPreference = 'Stop'
$drive = $env:DIRO_VOLUME_DRIVE
if ($drive -notmatch '^[A-Za-z]:$') { exit 2 }
$shell = New-Object -ComObject Shell.Application
$item = $shell.Namespace(17).ParseName($drive + '\')
if ($null -eq $item) { exit 3 }
$item.InvokeVerb('Eject')
$deadline = (Get-Date).AddSeconds(15)
while ((Get-Date) -lt $deadline) {
  if (-not (Test-Path -LiteralPath ($drive + '\'))) { exit 0 }
  Start-Sleep -Milliseconds 250
}
exit 4
"""


class Context:
    """What one run has: the system, the environment, where directives go."""

    def __init__(self, system=None, env=None):
        self.system = system or {"win32": "windows", "darwin": "macos"}.get(
            sys.platform, "linux")
        self.env = dict(os.environ if env is None else env)
        self.dry_run = self.env.get("DIRO_DRY_RUN") == "1"
        self.said = []

    def say(self, verb, text):
        """Writes one directive: announce, warn or error."""
        line = "%s %s" % (verb, " ".join(text.split()))
        self.said.append(line)
        path = self.env.get("DIRO_DIRECTIVES")
        if path:
            with open(path, "a", encoding="utf-8", newline="\n") as out:
                out.write(line + "\n")

    def tool(self, name):
        """The tool's full path on the PATH, or None."""
        return shutil.which(name, path=self.env.get("PATH", ""))

    def run(self, argv, extra_env=None):
        """Runs a tool by its argument list, never a shell; on Windows
        with no console window. Returns (code, stdout, stderr)."""
        env = dict(self.env)
        env.update(extra_env or {})
        flags = 0
        if os.name == "nt":
            flags = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
        try:
            done = subprocess.run(
                argv, env=env, stdin=subprocess.DEVNULL,
                capture_output=True, timeout=TOOL_TIMEOUT,
                creationflags=flags, check=False)
        except subprocess.TimeoutExpired:
            return 124, "", "it did not answer in time"
        except OSError as error:
            return 127, "", str(error)
        text = lambda b: b.decode("utf-8", errors="replace")
        return done.returncode, text(done.stdout), text(done.stderr)


def selection(env):
    """The volumes chosen: the NUL-separated selection file's names."""
    path = env.get("DIRO_SELECTION_FILE0")
    if not path:
        return []
    with open(path, "rb") as source:
        raw = source.read()
    return [part.decode("utf-8", errors="replace")
            for part in raw.split(b"\0") if part]


def why(code, stderr):
    """The reason in words, and the exit code, for a tool that failed."""
    if BUSY_WORDS.search(stderr):
        return "files are open", BUSY
    if AUTH_WORDS.search(stderr):
        return "the system asked for a password", AUTH
    if code == 124:
        return "it did not answer in time", FAILED
    return "the system refused", FAILED


# ----- Linux: udisksctl and lsblk -----

def mounts_table(env):
    """Mount point to device, from the kernel's table."""
    path = env.get("DIRO_MOUNTS_FILE", "/proc/self/mounts")
    table = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as source:
            for line in source:
                fields = line.split()
                if len(fields) >= 2:
                    point = re.sub(r"\\([0-7]{3})",
                                   lambda m: chr(int(m.group(1), 8)), fields[1])
                    table[point] = fields[0]
    except OSError:
        pass
    return table


def linux_target(ctx, target):
    """The device node for a device or a mount point, or None."""
    if LINUX_DEVICE.match(target):
        return target
    device = mounts_table(ctx.env).get(target.rstrip("/") or "/")
    if device and LINUX_DEVICE.match(device):
        return device
    return None


def linux_name(ctx, tool, device):
    """The volume's label, from udisksctl info, or the device's name."""
    code, out, _ = ctx.run([tool, "info", "-b", device])
    if code == 0:
        match = re.search(r"^\s*IdLabel:\s*(\S.*)$", out, re.MULTILINE)
        if match:
            return match.group(1).strip()
    return os.path.basename(device)


def linux_action(ctx, action, target):
    tool = ctx.tool("udisksctl")
    if not tool:
        ctx.say("error", "Could not %s %s: udisksctl was not found." %
                (action, target))
        return NO_TOOL
    device = linux_target(ctx, target)
    if not device:
        ctx.say("error", "Could not %s %s: it is not a volume." % (action, target))
        return WRONG
    name = linux_name(ctx, tool, device)
    if action == "mount":
        steps = [[tool, "mount", "-b", device, "--no-user-interaction"]]
        plan = "would mount %s" % device
    elif action == "unmount":
        steps = [[tool, "unmount", "-b", device, "--no-user-interaction"]]
        plan = "would unmount %s" % device
    else:
        steps = [[tool, "unmount", "-b", device, "--no-user-interaction"],
                 [tool, "power-off", "-b", device, "--no-user-interaction"]]
        plan = "would unmount %s, then power it off" % device
    if ctx.dry_run:
        ctx.say("announce", "%s %s (preview): %s." % (action.capitalize(), name, plan))
        return DONE
    mounted_at = None
    for index, argv in enumerate(steps):
        code, out, err = ctx.run(argv)
        if code != 0:
            not_mounted = re.search(r"not mounted", err, re.IGNORECASE)
            if action == "eject" and index == 0 and not_mounted:
                continue
            reason, exit_code = why(code, err)
            ctx.say("error", "Could not %s %s: %s." % (action, name, reason))
            return exit_code
        found = re.search(r"Mounted \S+ at (.+?)\.?$", out.strip())
        if found:
            mounted_at = found.group(1)
    if action == "mount":
        where = " at %s" % mounted_at if mounted_at else ""
        ctx.say("announce", "%s is mounted%s." % (name, where))
    elif action == "unmount":
        ctx.say("announce", "%s is unmounted." % name)
    else:
        ctx.say("announce", "%s is safe to remove." % name)
    return DONE


def linux_devices(ctx):
    """One JSON line a volume, from lsblk's own JSON."""
    tool = ctx.tool("lsblk")
    if not tool:
        return NO_TOOL
    code, out, _ = ctx.run([tool, "--json", "--bytes", "--output",
                            "NAME,PATH,LABEL,SIZE,RM,HOTPLUG,MOUNTPOINT,TYPE,FSTYPE"])
    if code != 0:
        return FAILED
    try:
        devices = json.loads(out).get("blockdevices", [])
    except ValueError:
        return FAILED

    def walk(nodes, removable):
        for node in nodes:
            gone = removable or bool(node.get("rm")) or bool(node.get("hotplug"))
            if node.get("fstype") and node.get("type") in ("part", "disk", "crypt"):
                print(json.dumps({
                    "name": node.get("label") or node.get("name") or "",
                    "device": node.get("path") or "",
                    "size": node.get("size"),
                    "removable": gone,
                    "mounted_at": node.get("mountpoint") or None,
                }, ensure_ascii=False))
            walk(node.get("children") or [], gone)

    walk(devices, False)
    return DONE


# ----- macOS: diskutil -----

def mac_info(ctx, tool, target):
    """diskutil's plist about a volume, or an empty dictionary."""
    code, out, _ = ctx.run([tool, "info", "-plist", target])
    if code != 0:
        return {}
    try:
        return plistlib.loads(out.encode("utf-8"))
    except Exception:  # A plist diskutil did not write: nothing known.
        return {}


def mac_action(ctx, action, target):
    tool = ctx.tool("diskutil")
    if not tool:
        ctx.say("error", "Could not %s %s: diskutil was not found." %
                (action, target))
        return NO_TOOL
    if not (MAC_DEVICE.match(target) or target.startswith("/Volumes/")):
        ctx.say("error", "Could not %s %s: it is not a volume." % (action, target))
        return WRONG
    info = mac_info(ctx, tool, target)
    name = info.get("VolumeName") or os.path.basename(target.rstrip("/"))
    if ctx.dry_run:
        ctx.say("announce", "%s %s (preview): would run diskutil %s." %
                (action.capitalize(), name, action))
        return DONE
    code, _, err = ctx.run([tool, action, target])
    if code != 0:
        reason, exit_code = why(code, err)
        ctx.say("error", "Could not %s %s: %s." % (action, name, reason))
        return exit_code
    words = {"mount": "is mounted", "unmount": "is unmounted",
             "eject": "is safe to remove"}[action]
    ctx.say("announce", "%s %s." % (name, words))
    return DONE


def mac_devices(ctx):
    """One JSON line a volume of the external disks, from diskutil."""
    tool = ctx.tool("diskutil")
    if not tool:
        return NO_TOOL
    code, out, _ = ctx.run([tool, "list", "-plist", "external", "physical"])
    if code != 0:
        return FAILED
    try:
        listed = plistlib.loads(out.encode("utf-8"))
    except Exception:  # Not diskutil's plist: nothing to list.
        return FAILED
    for disk in listed.get("AllDisksAndPartitions", []):
        for part in disk.get("Partitions", []) or [disk]:
            identifier = part.get("DeviceIdentifier")
            if not identifier:
                continue
            print(json.dumps({
                "name": part.get("VolumeName") or identifier,
                "device": "/dev/" + identifier,
                "size": part.get("Size"),
                "removable": True,
                "mounted_at": part.get("MountPoint") or None,
            }, ensure_ascii=False))
    return DONE


# ----- Windows: the shell's Eject verb through PowerShell -----

def windows_label(letter):
    """The drive's label, or "Drive E"; no colon, so the sentence after
    it reads cleanly ("Could not eject Drive E: files are open.")."""
    try:
        import ctypes
        buffer = ctypes.create_unicode_buffer(261)
        ok = ctypes.windll.kernel32.GetVolumeInformationW(
            "%s:\\" % letter, buffer, 261, None, None, None, None, 0)
        if ok and buffer.value.strip():
            return buffer.value.strip()
    except (AttributeError, OSError):
        pass
    return "Drive %s" % letter


def windows_action(ctx, action, target):
    if action != "eject":
        ctx.say("error", "Windows mounts and unmounts removable drives itself.")
        return WRONG
    match = WINDOWS_DRIVE.match(target)
    if not match:
        ctx.say("error", "Could not eject %s: it is not a drive." % target)
        return WRONG
    letter = match.group(1).upper()
    name = (ctx.env.get("DIRO_VOLUME_NAME") or
            (windows_label(letter) if os.name == "nt" else "Drive %s" % letter))
    tool = ctx.tool("powershell")
    if not tool:
        ctx.say("error", "Could not eject %s: PowerShell was not found." % name)
        return NO_TOOL
    if ctx.dry_run:
        ctx.say("announce", "Eject %s (preview): would ask Windows to eject "
                "%s: and wait until it is gone." % (name, letter))
        return DONE
    encoded = base64.b64encode(EJECT_SCRIPT.encode("utf-16-le")).decode("ascii")
    code, _, err = ctx.run(
        [tool, "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
        extra_env={"DIRO_VOLUME_DRIVE": "%s:" % letter})
    if code == DONE:
        ctx.say("announce", "%s is safe to remove." % name)
        return DONE
    if code == 4:
        ctx.say("error", "Could not eject %s: files are open." % name)
        return BUSY
    if code == 3:
        ctx.say("error", "Could not eject %s: Windows does not list it." % name)
        return FAILED
    reason, exit_code = why(code, err)
    ctx.say("error", "Could not eject %s: %s." % (name, reason))
    return exit_code


# ----- the commands -----

ACTIONS = {"linux": linux_action, "macos": mac_action, "windows": windows_action}
DEVICES = {"linux": linux_devices, "macos": mac_devices}


def main(argv, ctx=None):
    """Runs one command on the selection: unmount, eject, mount or
    devices. Returns the exit code."""
    ctx = ctx or Context()
    command = argv[1] if len(argv) > 1 else ""
    if command == "devices":
        lister = DEVICES.get(ctx.system)
        return lister(ctx) if lister else DONE
    if command not in ("unmount", "eject", "mount"):
        sys.stderr.write("volume_actions.py unmount|eject|mount|devices\n")
        return WRONG
    targets = selection(ctx.env)
    if not targets:
        ctx.say("error", "Choose a volume first.")
        return WRONG
    worst = DONE
    for target in targets:
        code = ACTIONS[ctx.system](ctx, command, target)
        worst = code if worst == DONE else worst
    return worst


if __name__ == "__main__":
    sys.exit(main(sys.argv))
