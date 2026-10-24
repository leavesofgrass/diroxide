#!/usr/bin/env python3
"""Folder guard: an example diroxide plugin, Python standard library only.

It stops a delete inside the folders you protect and logs every delete.
diroxide asks it before a delete (hook/before, a request it answers with
continue or cancel) and tells it after one (hook/after, a notification
it never answers). It speaks JSON-RPC 2.0 over its standard input and
output, one JSON object a line, UTF-8, both ways.

The parts to change for a guard of your own are marked CHANGE.
"""

import json
import os
import sys

PROTOCOL = 1
VERSION = "0.1.0"  # CHANGE: keep it equal to plugin.toml's version.

# UTF-8 both ways and "\n" line ends on every system.
sys.stdin.reconfigure(encoding="utf-8", errors="replace")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")

settings = {"protected": ""}  # CHANGE: your settings' defaults.
state_of = {"initialized": False}


def send(message):
    """Writes one message as one line, and flushes it at once."""
    line = json.dumps(message, ensure_ascii=False, separators=(",", ":"))
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def reply(request_id, result=None):
    """Answers a request."""
    send({"jsonrpc": "2.0", "id": request_id, "result": result})


def fail(request_id, code, message, kind="failed"):
    """Answers a request with an error, the cause in words."""
    send({"jsonrpc": "2.0", "id": request_id,
          "error": {"code": code, "message": message, "data": {"kind": kind}}})


def log(message, level="info"):
    """Writes a line to this plugin's log (diro plugin log folder-guard)."""
    send({"jsonrpc": "2.0", "method": "log",
          "params": {"level": level, "message": message}})


def folders():
    """The protected folders, from the setting: full paths separated by
    semicolons, compared without case on Windows."""
    text = settings.get("protected") or ""
    found = []
    for part in text.split(";"):
        part = part.strip().rstrip("\\/")
        if part:
            found.append(os.path.normcase(os.path.normpath(part)))
    return found


def inside(place, folder):
    """Whether a place is the folder or anything below it."""
    path = os.path.normcase(os.path.normpath(place))
    return path == folder or path.startswith(folder + os.sep)


def before_delete(request_id, params):
    """CHANGE: the guard. Stops the delete when any item is inside a
    protected folder; the reason is said with this plugin's name last."""
    protected = folders()
    for item in params.get("items") or []:
        place = item.get("from") or ""
        if "://" in place:
            continue  # a remote place: not this guard's to judge
        if any(inside(place, folder) for folder in protected):
            log("Stopped a delete in a protected folder: %s" % place)
            reply(request_id, {"action": "cancel",
                               "reason": "folder is protected"})
            return
    reply(request_id, {"action": "continue"})


def after_delete(params):
    """Logs each item a delete touched, with what became of it."""
    status = params.get("status") or "done"
    for item in params.get("items") or []:
        log("Delete %s: %s, %s" % (status, item.get("from") or "",
                                    item.get("result") or "done"))


def handle(message):
    """Answers one message by its method."""
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}
    if method is None:
        return
    if not state_of["initialized"] and method not in ("initialize", "exit"):
        if request_id is not None:
            fail(request_id, -32002, "initialize comes first", "not_initialized")
        return
    if method == "initialize":
        state_of["initialized"] = True
        settings.update(params.get("settings") or {})
        reply(request_id, {
            "protocol": PROTOCOL,
            "plugin": {"name": params.get("plugin_id") or "folder-guard",
                       "version": VERSION},
            "plugin_capabilities": {"commands": [], "hooks": ["before_delete", "after_delete"]},
        })
    elif method == "settings/changed":
        settings.update(params.get("settings") or {})
    elif method == "hook/before":
        if params.get("event") == "before_delete":
            before_delete(request_id, params)
        else:
            reply(request_id, {"action": "continue"})
    elif method == "hook/after":
        if params.get("event") == "after_delete":
            after_delete(params)
    elif method == "shutdown":
        reply(request_id, None)
    elif method == "exit":
        raise SystemExit(0)
    elif request_id is not None:
        fail(request_id, -32601, "no method %s" % method, "method_not_found")
    # An unknown notification is ignored.


def main():
    for line in sys.stdin:
        line = line.strip()
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
            handle(message)


if __name__ == "__main__":
    main()
