#!/usr/bin/env python3
"""Memory files (toy): an example diroxide provider, standard library only.

A file system kept in a dictionary: the provider contract in one file.
diroxide asks for places such as memfs:/photos/a.jpg; this plugin lists,
reads ranges, writes in sessions that commit whole or leave nothing,
makes folders, renames and removes. Quitting diroxide loses the data.
The parts to change for a provider of your own are marked CHANGE.
"""

import base64
import datetime
import json
import os
import queue
import sys
import threading
import time

PROTOCOL = 1
VERSION = "0.1.0"  # CHANGE: keep it equal to plugin.toml's version.
SCHEME = "memfs"
BATCH = 1000

sys.stdin.reconfigure(encoding="utf-8", errors="replace")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")

_incoming = queue.Queue()
_set_aside = []
_lock = threading.Lock()

settings = {"batch_lists": True}
cancelled = set()
state_of = {"initialized": False}

# The error codes of the provider outcomes (the protocol's -32010 range).
NOT_FOUND = (-32010, "not_found")
ALREADY_EXISTS = (-32011, "already_exists")
NOT_A_DIRECTORY = (-32013, "not_a_directory")
IS_A_DIRECTORY = (-32014, "is_a_directory")
NOT_EMPTY = (-32015, "not_empty")
INVALID_NAME = (-32016, "invalid_name")
UNSUPPORTED = (-32017, "unsupported")


class Problem(Exception):
    """A provider outcome, said as the error of a request."""

    def __init__(self, outcome, message):
        super().__init__(message)
        self.code, self.kind = outcome


def send(message):
    line = json.dumps(message, ensure_ascii=False, separators=(",", ":"))
    with _lock:
        sys.stdout.write(line + "\n")
        sys.stdout.flush()


def reply(request_id, result=None):
    send({"jsonrpc": "2.0", "id": request_id, "result": result})


def fail(request_id, code, message, kind="failed"):
    send({"jsonrpc": "2.0", "id": request_id,
          "error": {"code": code, "message": message, "data": {"kind": kind}}})


def notify(method, params):
    send({"jsonrpc": "2.0", "method": method, "params": params})


def _read_input():
    """Reads messages on a thread of its own, so a command can look for a
    cancel while it works. A batch is refused (-32600) and a line that is
    not JSON answered (-32700), as the protocol asks; other stray lines
    are passed over."""
    for line in sys.stdin:
        line = line.strip()
        if line.startswith("["):
            send({"jsonrpc": "2.0", "id": None, "error": {
                "code": -32600, "message": "batches are not taken",
                "data": {"kind": "invalid_request"}}})
            continue
        if not line.startswith("{"):
            continue
        try:
            message = json.loads(line)
        except ValueError:
            send({"jsonrpc": "2.0", "id": None, "error": {
                "code": -32700, "message": "a line is not JSON",
                "data": {"kind": "parse_error"}}})
            continue
        if isinstance(message, dict):
            _incoming.put(message)
    _incoming.put(None)


def _watch_parent(process_id):
    if os.name != "posix" or not process_id or os.getppid() != process_id:
        return

    def watch():
        while True:
            time.sleep(2)
            if os.getppid() != process_id:
                os._exit(0)

    threading.Thread(target=watch, daemon=True).start()


def poll_cancels():
    """Reads the messages waiting now: cancels count at once, requests
    wait their turn."""
    while True:
        try:
            message = _incoming.get(block=False)
        except queue.Empty:
            return
        if message is not None and message.get("method") == "$/cancel_request":
            cancelled.add((message.get("params") or {}).get("id"))
        else:
            _set_aside.append(message)


def now():
    stamp = datetime.datetime.now(datetime.timezone.utc)
    return stamp.strftime("%Y-%m-%dT%H:%M:%SZ")


# CHANGE: the store. A path is a tuple of names; the root is ().
nodes = {(): {"kind": "folder", "modified": now()}}
writes = {}
_next_write = [0]


def path_of(place):
    """memfs:/a/b, memfs:///a/b or /a/b as ("a", "b")."""
    text = place or ""
    if text.startswith(SCHEME + ":"):
        text = text[len(SCHEME) + 1:]
    parts = tuple(p for p in text.split("/") if p)
    for part in parts:
        if part in (".", ".."):
            raise Problem(INVALID_NAME, "a name cannot be . or ..")
    return parts


def place_of(path):
    return "%s:/%s" % (SCHEME, "/".join(path))


def check_name(name):
    if not name or "/" in name or "\0" in name or len(name) > 255:
        raise Problem(INVALID_NAME, "the name %r cannot be used here" % name)


def entry(path):
    node = nodes[path]
    found = {"name": path[-1] if path else "", "kind": node["kind"],
             "place": place_of(path), "modified": node["modified"]}
    if node["kind"] == "file":
        found["size"] = len(node["data"])
    return found


def existing(path):
    if path not in nodes:
        raise Problem(NOT_FOUND, "%s is not there" % place_of(path))
    return nodes[path]


def folder(path):
    node = existing(path)
    if node["kind"] != "folder":
        raise Problem(NOT_A_DIRECTORY, "%s is not a folder" % place_of(path))
    return node


def children(path):
    depth = len(path) + 1
    return sorted(p for p in nodes if len(p) == depth and p[:-1] == path)


# The provider methods, one function each, taking the params.

def connect(params, request_id):
    return {"capabilities": {"writable": True, "case_sensitive": True},
            "display_name": "Memory files (toy)", "root": place_of(())}


def list_(params, request_id):
    path = path_of(params.get("place"))
    folder(path)
    found = [entry(p) for p in children(path)]
    token = params.get("partial_token") or params.get("progress_token")
    if settings.get("batch_lists") and token and len(found) > BATCH:
        for start in range(0, len(found), BATCH):
            poll_cancels()
            if request_id in cancelled:
                raise Problem((-32800, "request_cancelled"), "cancelled")
            notify("$/progress", {"token": token, "value": {
                "kind": "partial", "entries": found[start:start + BATCH]}})
        return {"complete": True, "next": None}
    return {"entries": found}


def stat(params, request_id):
    path = path_of(params.get("place"))
    existing(path)
    return entry(path)


def read(params, request_id):
    path = path_of(params.get("place"))
    node = existing(path)
    if node["kind"] != "file":
        raise Problem(IS_A_DIRECTORY, "%s is a folder" % place_of(path))
    if params.get("transfer", "inline") != "inline":
        raise Problem(UNSUPPORTED, "only inline transfers")
    offset = int(params.get("offset") or 0)
    length = int(params.get("length") or len(node["data"]))
    data = bytes(node["data"][offset:offset + length])
    return {"data": base64.b64encode(data).decode("ascii"),
            "eof": offset + len(data) >= len(node["data"])}


def write_begin(params, request_id):
    path = path_of(params.get("place"))
    if not path:
        raise Problem(IS_A_DIRECTORY, "the root is a folder")
    check_name(path[-1])
    folder(path[:-1])
    if path in nodes and nodes[path]["kind"] == "folder":
        raise Problem(IS_A_DIRECTORY, "%s is a folder" % place_of(path))
    if path in nodes and params.get("conflict", "fail") in ("fail", "ask", "skip"):
        raise Problem(ALREADY_EXISTS, "%s is there already" % place_of(path))
    _next_write[0] += 1
    write_id = "w%d" % _next_write[0]
    writes[write_id] = {"path": path, "data": bytearray()}
    return {"write_id": write_id}


def write_chunk(params, request_id):
    session = writes.get(params.get("write_id"))
    if session is None:
        raise Problem(NOT_FOUND, "no such write")
    data = base64.b64decode(params.get("data") or "")
    offset = int(params.get("offset", len(session["data"])))
    buffer = session["data"]
    if offset > len(buffer):
        buffer.extend(b"\0" * (offset - len(buffer)))
    buffer[offset:offset + len(data)] = data
    return {}


def write_end(params, request_id):
    session = writes.pop(params.get("write_id"), None)
    if session is None:
        raise Problem(NOT_FOUND, "no such write")
    if not params.get("commit"):
        return None
    path = session["path"]
    folder(path[:-1])
    nodes[path] = {"kind": "file", "data": session["data"], "modified": now()}
    return entry(path)


def create_dir(params, request_id):
    path = path_of(params.get("place"))
    for name in path:
        check_name(name)
    if path in nodes:
        raise Problem(ALREADY_EXISTS, "%s is there already" % place_of(path))
    if params.get("parents"):
        for depth in range(1, len(path)):
            nodes.setdefault(path[:depth], {"kind": "folder", "modified": now()})
    folder(path[:-1])
    nodes[path] = {"kind": "folder", "modified": now()}
    return entry(path)


def rename(params, request_id):
    old = path_of(params.get("from"))
    new = path_of(params.get("to"))
    existing(old)
    if not old or not new:
        raise Problem(INVALID_NAME, "the root cannot be renamed")
    check_name(new[-1])
    folder(new[:-1])
    if new in nodes:
        raise Problem(ALREADY_EXISTS, "%s is there already" % place_of(new))
    if new[:len(old)] == old:
        raise Problem(INVALID_NAME, "a folder cannot go inside itself")
    for path in sorted(p for p in nodes if p[:len(old)] == old):
        nodes[new + path[len(old):]] = nodes.pop(path)
    return entry(new)


def remove(params, request_id):
    path = path_of(params.get("place"))
    node = existing(path)
    if not path:
        raise Problem(INVALID_NAME, "the root cannot be removed")
    if node["kind"] == "folder" and children(path):
        if not params.get("recursive"):
            raise Problem(NOT_EMPTY, "%s is not empty" % place_of(path))
        for inner in [p for p in nodes if p[:len(path)] == path]:
            del nodes[inner]
        return {}
    del nodes[path]
    return {}


METHODS = {  # CHANGE: the provider methods your plugin answers.
    "provider/connect": connect,
    "provider/list": list_,
    "provider/stat": stat,
    "provider/read": read,
    "provider/write_begin": write_begin,
    "provider/write_chunk": write_chunk,
    "provider/write_end": write_end,
    "provider/create_dir": create_dir,
    "provider/rename": rename,
    "provider/remove": remove,
}


def handle(message):
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}
    if method is None:
        return
    if not state_of["initialized"] and method not in ("initialize", "exit"):
        # Before initialize, a request is refused and a notification
        # dropped.
        if request_id is not None:
            fail(request_id, -32002, "initialize comes first", "not_initialized")
        return
    if method == "initialize":
        state_of["initialized"] = True
        settings.update(params.get("settings") or {})
        _watch_parent(params.get("process_id"))
        names = sorted(m.split("/", 1)[1] for m in METHODS)
        reply(request_id, {
            "protocol": PROTOCOL,
            "plugin": {"name": params.get("plugin_id") or "plugin",
                       "version": VERSION},
            "plugin_capabilities": {"providers": [
                {"scheme": SCHEME, "methods": names}]},
        })
    elif method == "settings/changed":
        settings.update(params.get("settings") or {})
    elif method == "$/cancel_request":
        cancelled.add(params.get("id"))
    elif method == "provider/disconnect":
        pass
    elif method in METHODS:
        try:
            reply(request_id, METHODS[method](params, request_id))
        except Problem as problem:
            fail(request_id, problem.code, str(problem), problem.kind)
        except (TypeError, ValueError) as error:
            fail(request_id, -32602, "the parameters are wrong: %s" % error,
                 "invalid_params")
    elif method in ("provider/trash", "provider/set_attributes"):
        fail(request_id, UNSUPPORTED[0], "memory files do not do that", UNSUPPORTED[1])
    elif method == "shutdown":
        reply(request_id, None)
    elif method == "exit":
        raise SystemExit(0)
    elif request_id is not None:
        fail(request_id, -32601, "no method %s" % method, "method_not_found")


def main():
    threading.Thread(target=_read_input, daemon=True).start()
    while True:
        message = _set_aside.pop(0) if _set_aside else _incoming.get()
        if message is None:
            return
        handle(message)


if __name__ == "__main__":
    main()
