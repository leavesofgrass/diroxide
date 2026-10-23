"""Tests of the memory-files toy provider, standard library only: the
provider contract list of R22 section 8.3 that a toy can keep.

Run them in this folder with the isolated option:
    python3 -I -m unittest        (Linux and macOS)
    py -3 -I -m unittest          (Windows)
"""

import base64
import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
MAIN = os.path.join(HERE, "main.py")


class Provider:
    """main.py as a child, spoken to as diroxide's provider adapter does."""

    def __init__(self):
        self.child = subprocess.Popen(
            [sys.executable, "-X", "utf8", "-u", MAIN],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, cwd=HERE)
        self.next_id = 0
        answer, _ = self.call("initialize", {"protocol": 1, "plugin_id": "memfs"})
        self.capabilities = answer["result"]["plugin_capabilities"]
        self.send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    def send(self, message):
        self.child.stdin.write((json.dumps(message) + "\n").encode("utf-8"))
        self.child.stdin.flush()

    def call(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method,
                   "params": params})
        notes = []
        while True:
            line = self.child.stdout.readline()
            if not line:
                raise AssertionError("the provider ended its output")
            message = json.loads(line.decode("utf-8"))
            if message.get("id") == self.next_id and "method" not in message:
                return message, notes
            notes.append(message)

    def ok(self, method, **params):
        answer, _ = self.call("provider/" + method, params)
        if "error" in answer:
            raise AssertionError("%s: %s" % (method, answer["error"]))
        return answer["result"]

    def code(self, method, **params):
        answer, _ = self.call("provider/" + method, params)
        return answer.get("error", {}).get("code")

    def write(self, place, data, commit=True, chunks=2):
        session = self.ok("write_begin", place=place, conflict="fail")["write_id"]
        step = max(1, len(data) // chunks)
        for offset in range(0, len(data), step):
            self.ok("write_chunk", write_id=session, offset=offset,
                    data=base64.b64encode(data[offset:offset + step]).decode("ascii"))
        return self.ok("write_end", write_id=session, commit=commit)

    def read(self, place, offset=0, length=None):
        params = {"place": place, "offset": offset}
        if length is not None:
            params["length"] = length
        result = self.ok("read", **params)
        return base64.b64decode(result["data"]), result["eof"]

    def close(self):
        self.call("shutdown", {})
        self.send({"jsonrpc": "2.0", "method": "exit", "params": {}})
        self.child.stdin.close()
        self.child.wait(timeout=10)
        self.child.stdout.close()


class MemfsTest(unittest.TestCase):

    def setUp(self):
        self.p = Provider()

    def tearDown(self):
        if self.p.child.poll() is None:
            self.p.close()

    def test_it_confirms_its_methods_and_connects(self):
        provider = self.p.capabilities["providers"][0]
        self.assertEqual(provider["scheme"], "memfs")
        self.assertIn("write_begin", provider["methods"])
        self.assertEqual(self.p.ok("connect", place="memfs:/")["root"], "memfs:/")

    def test_lists_an_empty_folder_and_a_folder_of_ten(self):
        self.assertEqual(self.p.ok("list", place="memfs:/")["entries"], [])
        self.p.ok("create_dir", place="memfs:/ten", parents=False)
        for n in range(10):
            self.p.write("memfs:/ten/f%d.txt" % n, b"x")
        names = [e["name"] for e in self.p.ok("list", place="memfs:/ten")["entries"]]
        self.assertEqual(names, ["f%d.txt" % n for n in range(10)])

    def test_a_large_folder_comes_in_batches(self):
        self.p.ok("create_dir", place="memfs:/big", parents=False)
        for n in range(2500):
            self.p.write("memfs:/big/f%04d" % n, b"", chunks=1)
        answer, notes = self.p.call("provider/list", {"place": "memfs:/big",
                                                      "partial_token": "t1"})
        self.assertEqual(answer["result"], {"complete": True, "next": None})
        batches = [n["params"]["value"]["entries"] for n in notes
                   if n["method"] == "$/progress"]
        self.assertEqual([len(b) for b in batches], [1000, 1000, 500])

    def test_a_cancel_stops_a_large_listing(self):
        self.p.ok("create_dir", place="memfs:/many", parents=False)
        for n in range(3000):
            self.p.write("memfs:/many/f%04d" % n, b"", chunks=1)
        self.p.next_id += 1
        request_id = self.p.next_id
        self.p.send({"jsonrpc": "2.0", "id": request_id, "method": "provider/list",
                     "params": {"place": "memfs:/many", "partial_token": "t2"}})
        self.p.send({"jsonrpc": "2.0", "method": "$/cancel_request",
                     "params": {"id": request_id}})
        while True:
            message = json.loads(self.p.child.stdout.readline().decode("utf-8"))
            if message.get("id") == request_id:
                break
        # Cancelled between batches, or done before the cancel came.
        if "error" in message:
            self.assertEqual(message["error"]["code"], -32800)
        else:
            self.assertEqual(message["result"]["complete"], True)

    def test_stat_a_file_a_folder_and_a_missing_name(self):
        self.p.write("memfs:/a.txt", b"hello")
        self.assertEqual(self.p.ok("stat", place="memfs:/a.txt")["size"], 5)
        self.assertEqual(self.p.ok("stat", place="memfs:/")["kind"], "folder")
        self.assertEqual(self.p.code("stat", place="memfs:/nope"), -32010)

    def test_reads_whole_ranges_past_the_end_and_empty_files(self):
        self.p.write("memfs:/a.txt", b"0123456789")
        self.assertEqual(self.p.read("memfs:/a.txt"), (b"0123456789", True))
        self.assertEqual(self.p.read("memfs:/a.txt", 3, 4), (b"3456", False))
        self.assertEqual(self.p.read("memfs:/a.txt", 8, 10), (b"89", True))
        self.p.write("memfs:/empty", b"", chunks=1)
        self.assertEqual(self.p.read("memfs:/empty"), (b"", True))

    def test_an_aborted_write_leaves_nothing(self):
        self.assertIsNone(self.p.write("memfs:/gone.txt", b"half", commit=False))
        self.assertEqual(self.p.code("stat", place="memfs:/gone.txt"), -32010)

    def test_folders_renames_and_removes_keep_the_rules(self):
        self.p.ok("create_dir", place="memfs:/a/b/c", parents=True)
        self.assertEqual(self.p.code("create_dir", place="memfs:/x/y", parents=False), -32010)
        self.p.write("memfs:/a/b/c/f.txt", b"f")
        self.p.write("memfs:/taken.txt", b"t")
        self.assertEqual(self.p.code("rename", **{"from": "memfs:/a", "to": "memfs:/taken.txt"}), -32011)
        self.p.ok("rename", **{"from": "memfs:/a", "to": "memfs:/moved"})
        self.assertEqual(self.p.read("memfs:/moved/b/c/f.txt"), (b"f", True))
        self.assertEqual(self.p.code("remove", place="memfs:/moved"), -32015)
        self.p.ok("remove", place="memfs:/moved/b/c/f.txt")
        self.p.ok("remove", place="memfs:/moved/b/c")
        self.assertEqual(self.p.code("write_begin", place="memfs:/taken.txt", conflict="fail"), -32011)
        self.assertEqual(self.p.code("create_dir", place="memfs:/../up", parents=False), -32016)
        self.assertEqual(self.p.code("trash", place="memfs:/taken.txt"), -32017)

    def test_it_ends_when_its_input_ends(self):
        self.p.child.stdin.close()
        self.assertEqual(self.p.child.wait(timeout=10), 0)
        self.p.child.stdout.close()


if __name__ == "__main__":
    unittest.main()
