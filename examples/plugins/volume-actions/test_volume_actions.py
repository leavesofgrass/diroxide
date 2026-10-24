"""Tests of the volume actions, standard library only.

Run them in this folder with the isolated option:
    python3 -I -m unittest        (Linux and macOS)
    py -3 -I -m unittest          (Windows)

Every system tool is a fake on a test PATH made in a temporary folder:
udisksctl, lsblk, diskutil and powershell each record their arguments
and answer as the test says. No real tool runs, and no drive, mount or
device is touched; the places are names only. The systems' paths are
tested on any computer.
"""

import contextlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import volume_actions as va  # noqa: E402

# One fake for every tool: it appends its name and arguments to the log,
# then answers from the scenario file, keyed by its first arguments.
FAKE = r'''
import json, os, sys
tool = sys.argv[1]
args = sys.argv[2:]
with open(os.environ["FAKE_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps([tool] + args) + "\n")
with open(os.environ["FAKE_SCENARIO"], encoding="utf-8") as source:
    scenario = json.load(source)
answer = None
for key in (" ".join([tool] + args[:2]), " ".join([tool] + args[:1]), tool):
    if key in scenario:
        answer = scenario[key]
        break
answer = answer or {}
if tool == "powershell":
    drive = os.environ.get("DIRO_VOLUME_DRIVE", "")
    with open(os.environ["FAKE_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps(["drive", drive]) + "\n")
sys.stdout.write(answer.get("out", ""))
sys.stderr.write(answer.get("err", ""))
sys.exit(answer.get("code", 0))
'''

TOOLS = ("udisksctl", "lsblk", "diskutil", "powershell")

BUSY_LINUX = ("Error unmounting /dev/sdb1: GDBus.Error:org.freedesktop."
              "UDisks2.Error.DeviceBusy: Error unmounting /dev/sdb1: "
              "target is busy\n")
AUTH_LINUX = ("Error unmounting /dev/sdb1: GDBus.Error:org.freedesktop."
              "UDisks2.Error.NotAuthorizedCanObtain: Not authorized to "
              "perform operation\n")
BUSY_MAC = ("Volume Backup on disk2s1 failed to unmount: dissented by PID "
            "412 (/usr/bin/vim)\nUnmount failed for /dev/disk2s1\n")
MAC_INFO = ("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">"
            "<dict><key>VolumeName</key><string>Backup</string>"
            "<key>DeviceIdentifier</key><string>disk2s1</string></dict></plist>\n")


class Bench(unittest.TestCase):
    """A temporary folder with the fake tools, the selection file, the
    directives file and the scenario."""

    def setUp(self):
        self.folder = tempfile.mkdtemp(prefix="volume-actions-")
        self.addCleanup(shutil.rmtree, self.folder, True)
        bin_dir = os.path.join(self.folder, "bin")
        os.mkdir(bin_dir)
        fake = os.path.join(self.folder, "fake_tool.py")
        with open(fake, "w", encoding="utf-8") as out:
            out.write(FAKE)
        for tool in TOOLS:
            if os.name == "nt":
                with open(os.path.join(bin_dir, tool + ".cmd"), "w") as out:
                    out.write('@"%s" "%s" %s %%*\n' % (sys.executable, fake, tool))
            else:
                path = os.path.join(bin_dir, tool)
                with open(path, "w") as out:
                    out.write('#!/bin/sh\nexec "%s" "%s" %s "$@"\n'
                              % (sys.executable, fake, tool))
                os.chmod(path, 0o755)
        self.log = os.path.join(self.folder, "log.jsonl")
        self.scenario_file = os.path.join(self.folder, "scenario.json")
        self.directives = os.path.join(self.folder, "directives.txt")
        self.selected = os.path.join(self.folder, "selection0")
        self.mounts = os.path.join(self.folder, "mounts")
        with open(self.mounts, "w") as out:
            out.write("/dev/sda1 / ext4 rw 0 0\n"
                      "/dev/sdb1 /media/ada/My\\040Backup vfat rw 0 0\n")
        self.env = {
            "PATH": bin_dir,
            "PATHEXT": os.environ.get("PATHEXT", ".COM;.EXE;.BAT;.CMD"),
            "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
            "FAKE_LOG": self.log,
            "FAKE_SCENARIO": self.scenario_file,
            "DIRO_DIRECTIVES": self.directives,
            "DIRO_SELECTION_FILE0": self.selected,
            "DIRO_MOUNTS_FILE": self.mounts,
        }
        self.scenario({})

    def scenario(self, answers):
        with open(self.scenario_file, "w", encoding="utf-8") as out:
            json.dump(answers, out)

    def run_on(self, system, command, *targets, dry_run=False):
        with open(self.selected, "wb") as out:
            out.write(b"".join(t.encode("utf-8") + b"\0" for t in targets))
        env = dict(self.env)
        if dry_run:
            env["DIRO_DRY_RUN"] = "1"
        ctx = va.Context(system=system, env=env)
        code = va.main(["volume_actions.py", command], ctx)
        return code, ctx.said

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log, encoding="utf-8") as source:
            return [json.loads(line) for line in source]

    def assert_never_forced(self):
        for call in self.calls():
            for arg in call[1:]:
                self.assertNotIn(arg.lower(), ("-f", "--force", "force"), call)


class Linux(Bench):
    def test_an_idle_volume_ejects_and_is_said_safe_to_remove(self):
        self.scenario({"udisksctl info": {"out": "  IdLabel:   Backup\n"}})
        code, said = self.run_on("linux", "eject", "/dev/sdb1")
        self.assertEqual(code, va.DONE)
        self.assertEqual(said, ["announce Backup is safe to remove."])
        verbs = [c[1] for c in self.calls() if c[0] == "udisksctl"]
        self.assertEqual(verbs, ["info", "unmount", "power-off"])
        for call in self.calls():
            if call[1] in ("unmount", "power-off"):
                self.assertIn("--no-user-interaction", call)
        self.assert_never_forced()
        with open(self.directives, encoding="utf-8") as source:
            self.assertEqual(source.read(), "announce Backup is safe to remove.\n")

    def test_a_busy_volume_says_files_are_open_and_is_never_forced(self):
        self.scenario({
            "udisksctl info": {"out": "  IdLabel:   Backup\n"},
            "udisksctl unmount": {"code": 1, "err": BUSY_LINUX},
        })
        code, said = self.run_on("linux", "eject", "/dev/sdb1")
        self.assertEqual(code, va.BUSY)
        self.assertEqual(said, ["error Could not eject Backup: files are open."])
        self.assertLessEqual(len(said[0]) - len("error "), 40)
        verbs = [c[1] for c in self.calls() if c[0] == "udisksctl"]
        self.assertNotIn("power-off", verbs, "nothing goes on after a refusal")
        self.assert_never_forced()

    def test_a_password_question_fails_in_words(self):
        self.scenario({"udisksctl unmount": {"code": 1, "err": AUTH_LINUX}})
        code, said = self.run_on("linux", "unmount", "/dev/sdb1")
        self.assertEqual(code, va.AUTH)
        self.assertEqual(
            said, ["error Could not unmount sdb1: the system asked for a password."])

    def test_a_mount_point_is_found_in_the_mounts_table(self):
        code, said = self.run_on("linux", "unmount", "/media/ada/My Backup")
        self.assertEqual(code, va.DONE)
        unmount = [c for c in self.calls() if c[1:2] == ["unmount"]]
        self.assertEqual(unmount[0][2:4], ["-b", "/dev/sdb1"])
        self.assertEqual(said, ["announce sdb1 is unmounted."])

    def test_mount_says_where(self):
        self.scenario({
            "udisksctl info": {"out": "  IdLabel:   CARD\n"},
            "udisksctl mount": {"out": "Mounted /dev/sdc1 at /media/ada/CARD\n"},
        })
        code, said = self.run_on("linux", "mount", "/dev/sdc1")
        self.assertEqual(code, va.DONE)
        self.assertEqual(said, ["announce CARD is mounted at /media/ada/CARD."])

    def test_a_preview_changes_nothing_and_says_what_it_would_run(self):
        self.scenario({"udisksctl info": {"out": "  IdLabel:   Backup\n"}})
        code, said = self.run_on("linux", "eject", "/dev/sdb1", dry_run=True)
        self.assertEqual(code, va.DONE)
        self.assertEqual(said, [
            "announce Eject Backup (preview): would unmount /dev/sdb1, "
            "then power it off."])
        verbs = [c[1] for c in self.calls()]
        self.assertEqual(verbs, ["info"], "only the label is read")

    def test_what_is_not_a_volume_runs_nothing(self):
        for target in ("/etc/passwd", "../dev/sdb1", "/dev/sdb1;reboot", "-f"):
            code, said = self.run_on("linux", "eject", target)
            self.assertEqual(code, va.WRONG, target)
            self.assertTrue(said[-1].startswith("error Could not eject"), said)
        self.assertEqual(self.calls(), [])

    def test_a_missing_tool_is_said(self):
        self.env["PATH"] = os.path.join(self.folder, "nowhere")
        code, said = self.run_on("linux", "eject", "/dev/sdb1")
        self.assertEqual(code, va.NO_TOOL)
        self.assertEqual(said, ["error Could not eject /dev/sdb1: udisksctl was not found."])

    def test_devices_are_listed_as_json_lines(self):
        listed = {"blockdevices": [
            {"name": "sda", "path": "/dev/sda", "type": "disk", "rm": False,
             "hotplug": False, "children": [
                 {"name": "sda1", "path": "/dev/sda1", "type": "part",
                  "fstype": "ext4", "mountpoint": "/", "size": 500}]},
            {"name": "sdb", "path": "/dev/sdb", "type": "disk", "rm": True,
             "hotplug": True, "children": [
                 {"name": "sdb1", "path": "/dev/sdb1", "type": "part",
                  "fstype": "vfat", "label": "Backup", "size": 931,
                  "mountpoint": None}]},
        ]}
        self.scenario({"lsblk": {"out": json.dumps(listed)}})
        with open(self.selected, "wb"):
            pass
        ctx = va.Context(system="linux", env=self.env)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = va.main(["volume_actions.py", "devices"], ctx)
        printed = out.getvalue().splitlines()
        self.assertEqual(code, va.DONE)
        rows = [json.loads(line) for line in printed]
        self.assertEqual(rows[1], {"name": "Backup", "device": "/dev/sdb1",
                                   "size": 931, "removable": True,
                                   "mounted_at": None})
        self.assertFalse(rows[0]["removable"])


class Mac(Bench):
    def test_a_busy_volume_is_never_forced(self):
        self.scenario({
            "diskutil info": {"out": MAC_INFO},
            "diskutil unmount": {"code": 1, "err": BUSY_MAC},
        })
        code, said = self.run_on("macos", "unmount", "/dev/disk2s1")
        self.assertEqual(code, va.BUSY)
        self.assertEqual(said, ["error Could not unmount Backup: files are open."])
        self.assert_never_forced()

    def test_eject_says_safe_to_remove(self):
        self.scenario({"diskutil info": {"out": MAC_INFO}})
        code, said = self.run_on("macos", "eject", "/dev/disk2s1")
        self.assertEqual(code, va.DONE)
        self.assertEqual(said, ["announce Backup is safe to remove."])
        self.assertEqual(self.calls()[-1], ["diskutil", "eject", "/dev/disk2s1"])

    def test_what_is_not_a_volume_runs_nothing(self):
        code, _ = self.run_on("macos", "eject", "/Users/ada")
        self.assertEqual(code, va.WRONG)
        self.assertEqual(self.calls(), [])


class Windows(Bench):
    def test_an_idle_drive_ejects_with_the_drive_passed_outside_the_command(self):
        self.env["DIRO_VOLUME_NAME"] = "Backup"
        code, said = self.run_on("windows", "eject", "E:\\")
        self.assertEqual(code, va.DONE)
        self.assertEqual(said, ["announce Backup is safe to remove."])
        calls = self.calls()
        powershell = [c for c in calls if c[0] == "powershell"][0]
        self.assertEqual(powershell[1:4], ["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        self.assertNotIn("E:", " ".join(powershell), "the drive is not in the command")
        self.assertIn(["drive", "E:"], calls)

    def test_a_busy_drive_says_files_are_open(self):
        self.env["DIRO_VOLUME_NAME"] = "Backup"
        self.scenario({"powershell": {"code": 4}})
        code, said = self.run_on("windows", "eject", "e:")
        self.assertEqual(code, va.BUSY)
        self.assertEqual(said, ["error Could not eject Backup: files are open."])

    def test_only_a_drive_is_ejected_and_nothing_is_mounted(self):
        for target in ("E:\\photos", "\\\\server\\share", "C:\\..\\D:", "E"):
            code, _ = self.run_on("windows", "eject", target)
            self.assertEqual(code, va.WRONG, target)
        code, said = self.run_on("windows", "mount", "E:\\")
        self.assertEqual(code, va.WRONG)
        self.assertEqual(said, ["error Windows mounts and unmounts removable drives itself."])
        self.assertEqual(self.calls(), [])

    def test_a_preview_runs_nothing(self):
        self.env["DIRO_VOLUME_NAME"] = "Drive E"
        code, said = self.run_on("windows", "eject", "E:\\", dry_run=True)
        self.assertEqual(code, va.DONE)
        self.assertTrue(said[0].startswith("announce Eject Drive E (preview)"), said)
        self.assertEqual(self.calls(), [])

    def test_the_eject_script_has_no_force_and_reads_its_drive_from_the_environment(self):
        script = va.EJECT_SCRIPT.lower()
        self.assertIn("$env:diro_volume_drive", script)
        self.assertNotIn("force", script)
        self.assertIn("invokeverb('eject')", script)


if __name__ == "__main__":
    unittest.main()
