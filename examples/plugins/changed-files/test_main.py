"""Tests of the changed-files lister, standard library only.

Run them in this folder with the isolated option:
    python3 -I -m unittest        (Linux and macOS)
    py -3 -I -m unittest          (Windows)

Each test starts main.py as diroxide does and plays diroxide's side: it
answers the plugin's own requests (panel/get, tools/run, lister/open,
ui/show_text) from a script, with canned git output, so no git and no
repository are needed.
"""

import base64
import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
MAIN = os.path.join(HERE, "main.py")

STATUS = (b" M src/lib.rs\0?? notes.txt\0R  new name.txt\0old name.txt\0"
          b"UU both.txt\0M  staged.txt\0")


class Host:
    """main.py as a child, with diroxide's side answered by `answers`, a
    function from a request to its result."""

    def __init__(self, answers):
        self.answers = answers
        self.asked = []
        self.child = subprocess.Popen(
            [sys.executable, "-X", "utf8", "-u", MAIN],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, cwd=HERE)
        self.next_id = 0

    def send(self, message):
        self.child.stdin.write((json.dumps(message) + "\n").encode("utf-8"))
        self.child.stdin.flush()

    def request(self, method, params=None):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method,
                   "params": params or {}})
        notes = []
        while True:
            line = self.child.stdout.readline()
            if not line:
                raise AssertionError("the plugin ended its output")
            message = json.loads(line.decode("utf-8"))
            if "method" in message and "id" in message:
                self.asked.append(message)
                self.send({"jsonrpc": "2.0", "id": message["id"],
                           "result": self.answers(message)})
            elif "method" in message:
                notes.append(message)
            elif message.get("id") == self.next_id:
                return message, notes

    def close(self):
        if self.child.poll() is None:
            self.child.stdin.close()
            self.child.wait(timeout=10)
        self.child.stdout.close()


def answers(exit_code=0, status=STATUS, diff=b"@@ -1 +1 @@\n-a\n+b\n"):
    def answer(request):
        method = request["method"]
        if method == "panel/get":
            return {"place": "/work/repo"}
        if method == "tools/run":
            output = status if request["params"]["args"][0] == "status" else diff
            return {"exit_code": exit_code, "truncated": False,
                    "stdout": base64.b64encode(output).decode("ascii"),
                    "stderr": ""}
        return {"applied": True}
    return answer


def row_context(*paths):
    return {"selection": [{"name": "Modified, " + p, "kind": "item",
                           "extra": {"state": "Modified", "path": p}}
                          for p in paths]}


class ChangedFilesTest(unittest.TestCase):

    def start(self, host):
        answer, _ = host.request("initialize", {"protocol": 1,
                                                "plugin_id": "changed-files"})
        self.assertEqual(answer["result"]["plugin_capabilities"]["listers"],
                         ["changes"])
        host.send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    def test_show_opens_a_lister_of_rows_state_first(self):
        host = Host(answers())
        try:
            self.start(host)
            answer, _ = host.request("command/run", {"command": "show",
                                                     "context": {}})
            self.assertEqual(answer["result"]["done"], 5)
            methods = [r["method"] for r in host.asked]
            self.assertEqual(methods, ["panel/get", "tools/run", "lister/open"])
            run = host.asked[1]["params"]
            self.assertEqual(run["tool"], "git")
            self.assertEqual(run["cwd"], "/work/repo")
            self.assertIn("-z", run["args"])
            opened = host.asked[2]["params"]
            self.assertEqual(opened["title"], "Changed files: 5, 1 untracked")
            names = [r["name"] for r in opened["rows"]]
            self.assertEqual(names, [
                "Modified, src/lib.rs", "Untracked, notes.txt",
                "Renamed, new name.txt", "Conflict, both.txt",
                "Staged, staged.txt"])
            self.assertTrue(all(r["kind"] == "item" for r in opened["rows"]))
        finally:
            host.close()

    def test_enter_on_a_row_shows_its_diff(self):
        host = Host(answers())
        try:
            self.start(host)
            host.request("command/run", {"command": "show", "context": {}})
            answer, _ = host.request(
                "command/run",
                {"command": "changed-files.diff",
                 "context": row_context("src/lib.rs")})
            self.assertEqual(answer["result"]["done"], 1)
            shown = host.asked[-1]
            self.assertEqual(shown["method"], "ui/show_text")
            self.assertEqual(shown["params"]["kind"], "diff")
            self.assertEqual(shown["params"]["title"], "Changes in src/lib.rs")
        finally:
            host.close()

    def test_a_folder_outside_a_repository_is_said(self):
        host = Host(answers(exit_code=128))
        try:
            self.start(host)
            answer, notes = host.request("command/run", {"command": "show",
                                                         "context": {}})
            self.assertEqual(answer["result"]["done"], 0)
            said = [n["params"]["text"] for n in notes
                    if n["method"] == "ui/announce"]
            self.assertEqual(said, ["This folder is not in a git repository."])
        finally:
            host.close()

    def test_stage_adds_the_marked_rows_then_lists_again(self):
        host = Host(answers())
        try:
            self.start(host)
            host.request("command/run", {"command": "show", "context": {}})
            answer, notes = host.request(
                "command/run",
                {"command": "stage", "context": row_context("a.txt", "b c.txt")})
            self.assertEqual(answer["result"]["done"], 2)
            add = [r for r in host.asked if r["method"] == "tools/run"][-2]
            self.assertEqual(add["params"]["args"], ["add", "--", "a.txt", "b c.txt"])
            self.assertEqual(host.asked[-1]["method"], "lister/open")
            said = [n["params"]["text"] for n in notes
                    if n["method"] == "ui/announce"]
            self.assertEqual(said, ["Staged 2 files"])
        finally:
            host.close()

    def test_a_line_break_in_a_name_stays_one_row(self):
        host = Host(answers(status=b" M line\nbreak.txt\0"))
        try:
            self.start(host)
            host.request("command/run", {"command": "show", "context": {}})
            rows = host.asked[-1]["params"]["rows"]
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["extra"]["path"], "line\nbreak.txt")
        finally:
            host.close()


if __name__ == "__main__":
    unittest.main()
