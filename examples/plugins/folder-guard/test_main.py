"""Tests of the folder guard, standard library only.

Run them in this folder with the isolated option:
    python3 -I -m unittest        (Linux and macOS)
    py -3 -I -m unittest          (Windows)

Each test starts main.py as diroxide does, plays diroxide's side of the
conversation over its pipes, and checks the answers. No file is read or
written; the places are names only.
"""

import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
MAIN = os.path.join(HERE, "main.py")


def place(*parts):
    """A full path on this system, for names only."""
    root = "D:\\" if os.name == "nt" else "/"
    return os.path.join(root, *parts)


class Plugin:
    """main.py as a child process, spoken to one message a line."""

    def __init__(self):
        self.child = subprocess.Popen(
            [sys.executable, "-X", "utf8", "-u", MAIN],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, cwd=HERE)
        self.next_id = 0

    def send(self, message):
        line = json.dumps(message, ensure_ascii=False) + "\n"
        self.child.stdin.write(line.encode("utf-8"))
        self.child.stdin.flush()

    def receive(self):
        line = self.child.stdout.readline()
        if not line:
            raise AssertionError("the plugin ended its output")
        return json.loads(line.decode("utf-8"))

    def request(self, method, params=None):
        """Sends a request; returns its answer and the notifications that
        came first."""
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method,
                   "params": params or {}})
        notes = []
        while True:
            message = self.receive()
            if message.get("id") == self.next_id and "method" not in message:
                return message, notes
            notes.append(message)

    def notify(self, method, params=None):
        self.send({"jsonrpc": "2.0", "method": method, "params": params or {}})

    def start(self, protected):
        answer, _ = self.request("initialize", {
            "protocol": 1, "plugin_id": "folder-guard",
            "settings": {"protected": protected}})
        return answer

    def close(self):
        self.request("shutdown")
        self.notify("exit")
        self.child.stdin.close()
        self.child.wait(timeout=10)
        self.child.stdout.close()


def delete(*places):
    """hook/before's parameters for a delete of these places."""
    return {"event": "before_delete", "job": 7, "count": len(places),
            "summary": "Move %d items to the trash" % len(places),
            "items": [{"from": p, "kind": "file"} for p in places]}


class FolderGuardTests(unittest.TestCase):

    def test_a_delete_inside_a_protected_folder_is_stopped(self):
        plugin = Plugin()
        try:
            answer = plugin.start(place("work", "keep") + "; " + place("other"))
            self.assertEqual(answer["result"]["protocol"], 1)
            answer, notes = plugin.request("hook/before", delete(
                place("work", "notes.txt"), place("work", "keep", "a.txt")))
            self.assertEqual(answer["result"],
                             {"action": "cancel", "reason": "folder is protected"})
            self.assertEqual(notes[0]["method"], "log")
        finally:
            plugin.close()

    def test_a_delete_elsewhere_goes_on_and_is_logged_after(self):
        plugin = Plugin()
        try:
            plugin.start(place("work", "keep"))
            answer, _ = plugin.request(
                "hook/before", delete(place("work", "keeper.txt")))
            self.assertEqual(answer["result"], {"action": "continue"})
            plugin.notify("hook/after", {
                "event": "after_delete", "job": 7, "status": "done",
                "items": [{"from": place("work", "keeper.txt"),
                           "kind": "file", "result": "done"}]})
            # The next answer comes after the log line of the notification.
            answer, notes = plugin.request("hook/before", delete())
            self.assertEqual(answer["result"], {"action": "continue"})
            self.assertTrue(notes[0]["params"]["message"].startswith(
                "Delete done: "))
        finally:
            plugin.close()

    def test_a_changed_setting_protects_at_once(self):
        plugin = Plugin()
        try:
            plugin.start("")
            target = place("photos", "crow.jpg")
            answer, _ = plugin.request("hook/before", delete(target))
            self.assertEqual(answer["result"]["action"], "continue")
            plugin.notify("settings/changed",
                          {"settings": {"protected": place("photos")}})
            answer, _ = plugin.request("hook/before", delete(target))
            self.assertEqual(answer["result"]["action"], "cancel")
        finally:
            plugin.close()


if __name__ == "__main__":
    unittest.main()
