"""Tests of the line counter, standard library only.

Run them in this folder with the isolated option:
    python3 -I -m unittest        (Linux and macOS)
    py -3 -I -m unittest          (Windows)

Each test starts main.py as diroxide does, plays diroxide's side of the
conversation over its pipes, and checks the answers. Files go in a
temporary folder and nowhere else.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
MAIN = os.path.join(HERE, "main.py")


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
        """Sends a request; returns its answer and the notifications
        that came first."""
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

    def start(self, settings=None):
        answer, _ = self.request("initialize", {
            "protocol": 1, "plugin_id": "line-counter",
            "settings": settings or {}})
        self.notify("initialized")
        return answer

    def stop(self):
        answer, _ = self.request("shutdown")
        self.notify("exit")
        self.child.stdin.close()
        self.child.wait(timeout=10)
        self.child.stdout.close()
        return answer


def context(places):
    return {"protocol": 1, "command": "line-counter.count_lines",
            "selection": [{"name": os.path.basename(p), "kind": "file",
                           "place": p} for p in places]}


class LineCounterTest(unittest.TestCase):

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.plugin = Plugin()

    def tearDown(self):
        if self.plugin.child.poll() is None:
            self.plugin.child.kill()
            self.plugin.child.wait()
        self.plugin.child.stdout.close()
        self.folder.cleanup()

    def write(self, name, text):
        path = os.path.join(self.folder.name, name)
        with open(path, "w", encoding="utf-8", newline="") as handle:
            handle.write(text)
        return path

    def test_it_starts_counts_and_stops(self):
        answer = self.plugin.start()
        self.assertEqual(answer["result"]["protocol"], 1)
        self.assertEqual(answer["result"]["plugin_capabilities"]["commands"],
                         ["count_lines"])
        path = self.write("a.txt", "one\ntwo\n\nfour\n")
        answer, notes = self.plugin.request(
            "command/run",
            {"command": "count_lines", "context": context([path])})
        self.assertEqual(answer["result"],
                         {"done": 1, "skipped": 0, "failed": []})
        said = [n["params"]["text"] for n in notes
                if n["method"] == "ui/announce"]
        self.assertEqual(said, ["Lines: 4, files: 1"])
        self.assertEqual(self.plugin.stop()["result"], None)
        self.assertEqual(self.plugin.child.returncode, 0)

    def test_settings_change_the_count(self):
        self.plugin.start({"skip_blank": True})
        path = self.write("b.txt", "one\r\n\r\nthree\r\n")
        _, notes = self.plugin.request(
            "command/run",
            {"command": "count_lines", "context": context([path])})
        self.assertEqual(notes[-1]["params"]["text"], "Lines: 2, files: 1")
        self.plugin.notify("settings/changed", {"settings": {"skip_blank": False}})
        _, notes = self.plugin.request(
            "command/run",
            {"command": "count_lines", "context": context([path])})
        self.assertEqual(notes[-1]["params"]["text"], "Lines: 3, files: 1")
        self.plugin.stop()

    def test_a_long_list_reports_progress_and_a_missing_file_fails(self):
        self.plugin.start()
        places = [self.write("f%02d.txt" % n, "x\n") for n in range(25)]
        places.append(os.path.join(self.folder.name, "missing.txt"))
        answer, notes = self.plugin.request(
            "command/run",
            {"command": "count_lines", "context": context(places)})
        kinds = [n["params"]["value"]["kind"] for n in notes
                 if n["method"] == "$/progress"]
        self.assertEqual(kinds[0], "begin")
        self.assertEqual(kinds[-1], "end")
        self.assertIn("report", kinds)
        self.assertEqual(answer["result"]["done"], 25)
        self.assertEqual(len(answer["result"]["failed"]), 1)
        self.plugin.stop()

    def test_unknown_requests_get_method_not_found(self):
        self.plugin.start()
        answer, _ = self.plugin.request("diroxide/probe")
        self.assertEqual(answer["error"]["code"], -32601)
        self.plugin.notify("some/notification")
        answer, _ = self.plugin.request(
            "command/run", {"command": "dance", "context": context([])})
        self.assertEqual(answer["error"]["code"], -32601)
        self.plugin.stop()

    def test_a_cancel_is_seen_between_files(self):
        self.plugin.start()
        places = [self.write("c%03d.txt" % n, "x\n" * 50) for n in range(400)]
        self.plugin.next_id += 1
        request_id = self.plugin.next_id
        self.plugin.send({"jsonrpc": "2.0", "id": request_id,
                          "method": "command/run",
                          "params": {"command": "count_lines",
                                     "context": context(places)}})
        self.plugin.notify("$/cancel_request", {"id": request_id})
        while True:
            message = self.plugin.receive()
            if message.get("id") == request_id:
                break
        # Cancelled, or done before the cancel arrived: both are answers.
        if "error" in message:
            self.assertEqual(message["error"]["code"], -32800)
        else:
            self.assertEqual(message["result"]["done"], 400)
        self.plugin.stop()

    def test_a_long_file_counts_every_line(self):
        self.plugin.start()
        path = self.write("long.txt", "line\n" * 20000)
        _, notes = self.plugin.request(
            "command/run",
            {"command": "count_lines", "context": context([path])})
        self.assertEqual(notes[-1]["params"]["text"], "Lines: 20000, files: 1")
        self.plugin.stop()

    def test_it_ends_when_its_input_ends(self):
        self.plugin.start()
        self.plugin.child.stdin.close()
        self.assertEqual(self.plugin.child.wait(timeout=10), 0)


if __name__ == "__main__":
    unittest.main()
