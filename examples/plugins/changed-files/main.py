#!/usr/bin/env python3
"""Changed files: an example diroxide lister plugin, standard library only.

"Show changed files" asks diroxide for the active panel's folder, runs
git status through diroxide (never a shell), and opens a lister whose
rows are not files: each says its state first ("Modified, src/lib.rs").
Enter on a row runs "diff", which shows the row's changes as a text
document; the stage action adds the marked rows to the next commit.
The parts to change for a lister of your own are marked CHANGE.
"""

import base64
import json
import os
import queue
import sys
import threading
import time

PROTOCOL = 1
VERSION = "0.1.0"  # CHANGE: keep it equal to plugin.toml's version.

sys.stdin.reconfigure(encoding="utf-8", errors="replace")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")

_incoming = queue.Queue()
_set_aside = []
_next_id = 0
_lock = threading.Lock()

settings = {}
cancelled = set()
state_of = {"initialized": False}
state = {"repository": None}


class Refused(Exception):
    """diroxide answered one of this plugin's requests with an error."""

    def __init__(self, error):
        super().__init__(error.get("message", "refused"))
        self.code = error.get("code")


def send(message):
    """Writes one message as one line, and flushes it at once."""
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


def announce(text, importance="info"):
    """Has diroxide say a sentence; it adds this plugin's name last."""
    notify("ui/announce", {"text": text, "importance": importance})


def log(message, level="info"):
    notify("log", {"level": level, "message": message})


def call(method, params):
    """Asks diroxide something and waits for the answer; messages that
    come meanwhile are set aside and handled after."""
    global _next_id
    _next_id += 1
    my_id = "plugin-%d" % _next_id
    send({"jsonrpc": "2.0", "id": my_id, "method": method, "params": params})
    while True:
        message = _incoming.get()
        if message is None:
            raise SystemExit(0)
        if message.get("method") == "$/cancel_request":
            # A cancel counts at once, even while this call waits.
            cancelled.add((message.get("params") or {}).get("id"))
            continue
        if message.get("id") == my_id and "method" not in message:
            if "error" in message:
                raise Refused(message["error"])
            return message.get("result") or {}
        _set_aside.append(message)


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


def next_message():
    if _set_aside:
        return _set_aside.pop(0)
    return _incoming.get()


def git(args, folder):
    """Runs git through diroxide, with an argument list; the output comes
    back as bytes."""
    result = call("tools/run", {"tool": "git", "args": args, "cwd": folder,
                                "encoding": "base64"})
    output = base64.b64decode(result.get("stdout") or "")
    return result.get("exit_code"), output


# CHANGE: the words for git's two-letter states, the state first.
def state_word(code):
    if code == "??":
        return "Untracked"
    if code == "!!":
        return "Ignored"
    if "U" in code or code in ("AA", "DD"):
        return "Conflict"
    index, tree = code[0], code[1]
    if "R" in code:
        return "Renamed"
    if index == "A":
        return "Added"
    if tree == " ":
        return "Staged"
    if "D" in code:
        return "Deleted"
    return "Modified"


def parse_status(output):
    """Reads `git status --porcelain=v1 -z`: records ended by NUL; a
    rename or copy has a second record, its old path."""
    records = output.split(b"\0")
    rows = []
    index = 0
    while index < len(records):
        record = records[index]
        index += 1
        if len(record) < 4:
            continue
        code = record[:2].decode("ascii", "replace")
        path = record[3:].decode("utf-8", "replace")
        if code[0] in "RC":
            index += 1
        word = state_word(code)
        rows.append({
            "name": "%s, %s" % (word, path),
            "kind": "item",
            "extra": {"id": path, "state": word, "path": path},
        })
    return rows


def open_lister():
    """Lists the changes of the repository and shows them; the title
    says the count first."""
    folder = state["repository"]
    code, output = git(["status", "--porcelain=v1", "-z",
                        "--untracked-files=all"], folder)
    if code == 128:
        announce("This folder is not in a git repository.", "warning")
        return None
    if code != 0:
        announce("git status did not work here, code %s." % code, "warning")
        return None
    rows = parse_status(output)
    untracked = sum(1 for r in rows if r["extra"]["state"] == "Untracked")
    title = "Changed files: %d" % len(rows)
    if untracked:
        title += ", %d untracked" % untracked
    call("lister/open", {"lister": "changes", "title": title, "rows": rows,
                         "total": len(rows), "panel": "active"})
    return rows


def show(request_id, params):
    """CHANGE: the command that opens the lister."""
    panel = call("panel/get", {"panel": "active", "fields": ["place"]})
    state["repository"] = panel.get("place")
    rows = open_lister()
    reply(request_id, {"done": len(rows or []), "skipped": 0, "failed": []})


def selected_paths(params):
    context = params.get("context") or {}
    return [e.get("extra", {}).get("path") for e in context.get("selection") or []
            if e.get("extra", {}).get("path")]


def diff(request_id, params):
    """Enter on a row: its changes, as a diff document."""
    paths = selected_paths(params)
    if not paths or not state["repository"]:
        fail(request_id, -32602, "no row to show", "invalid_params")
        return
    path = paths[0]
    code, output = git(["diff", "--no-color", "--", path], state["repository"])
    text = output.decode("utf-8", "replace")
    if code != 0:
        fail(request_id, -32099, "git diff ended with code %s" % code)
        return
    if not text.strip():
        announce("No changes to show for %s." % path)
    else:
        call("ui/show_text", {"title": "Changes in %s" % path, "text": text,
                              "kind": "diff"})
    reply(request_id, {"done": 1, "skipped": 0, "failed": []})


def stage(request_id, params):
    """The stage action: git add for the marked rows, then the list again."""
    paths = selected_paths(params)
    if not paths or not state["repository"]:
        fail(request_id, -32602, "no rows to stage", "invalid_params")
        return
    code, _ = git(["add", "--"] + paths, state["repository"])
    if code != 0:
        fail(request_id, -32099, "git add ended with code %s" % code)
        return
    announce("Staged %d files" % len(paths))
    open_lister()
    reply(request_id, {"done": len(paths), "skipped": 0, "failed": []})


COMMANDS = {"show": show, "diff": diff, "stage": stage}


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
        reply(request_id, {
            "protocol": PROTOCOL,
            "plugin": {"name": params.get("plugin_id") or "plugin",
                       "version": VERSION},
            "plugin_capabilities": {"commands": sorted(COMMANDS),
                                    "listers": ["changes"]},
        })
    elif method == "settings/changed":
        settings.update(params.get("settings") or {})
    elif method == "$/cancel_request":
        cancelled.add(params.get("id"))
    elif method == "lister/closed":
        pass
    elif method == "command/run":
        command = (params.get("command") or "").rsplit(".", 1)[-1]
        if command not in COMMANDS:
            fail(request_id, -32601, "no command %s" % command, "method_not_found")
            return
        try:
            COMMANDS[command](request_id, params)
        except Refused as refused:
            if request_id in cancelled:
                fail(request_id, -32800, "cancelled", "request_cancelled")
            else:
                fail(request_id, -32099, "diroxide refused: %s" % refused)
    elif method == "shutdown":
        reply(request_id, None)
    elif method == "exit":
        raise SystemExit(0)
    elif request_id is not None:
        fail(request_id, -32601, "no method %s" % method, "method_not_found")


def main():
    threading.Thread(target=_read_input, daemon=True).start()
    while True:
        message = next_message()
        if message is None:
            return
        handle(message)


if __name__ == "__main__":
    main()
