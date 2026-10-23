#!/usr/bin/env python3
"""Line counter: an example diroxide plugin, Python standard library only.

diroxide starts this program and speaks JSON-RPC 2.0 with it over its
standard input and output, one JSON object a line, UTF-8, both ways.
Read it top to bottom: the helpers, then the command, then the loop.
The parts to change for a plugin of your own are marked CHANGE.
"""

import json
import os
import queue
import sys
import threading
import time

PROTOCOL = 1
VERSION = "0.1.0"  # CHANGE: keep it equal to plugin.toml's version.

# The streams: UTF-8 both ways and "\n" line ends on every system, so the
# messages are the same bytes on Windows.
sys.stdin.reconfigure(encoding="utf-8", errors="replace")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")

_incoming = queue.Queue()
_set_aside = []
_next_id = 0
_lock = threading.Lock()

settings = {"skip_blank": False}  # CHANGE: your settings' defaults.
cancelled = set()


def send(message):
    """Writes one message as one line, and flushes it at once."""
    line = json.dumps(message, ensure_ascii=False, separators=(",", ":"))
    with _lock:
        sys.stdout.write(line + "\n")
        sys.stdout.flush()


def reply(request_id, result=None):
    """Answers a request."""
    send({"jsonrpc": "2.0", "id": request_id, "result": result})


def fail(request_id, code, message, kind="failed"):
    """Answers a request with an error: the cause in words, meaning first."""
    send({"jsonrpc": "2.0", "id": request_id,
          "error": {"code": code, "message": message, "data": {"kind": kind}}})


def notify(method, params):
    """Sends a notification, which has no answer."""
    send({"jsonrpc": "2.0", "method": method, "params": params})


def announce(text, importance="info"):
    """Has diroxide say a sentence; it adds this plugin's name last."""
    notify("ui/announce", {"text": text, "importance": importance})


def log(message, level="info"):
    """Writes a line to this plugin's log (diro plugin log NAME)."""
    notify("log", {"level": level, "message": message})


def progress(token, **value):
    """Reports progress: kind begin, report or end, with words."""
    notify("$/progress", {"token": token, "value": value})


def call(method, params):
    """Asks diroxide something and waits for the answer. Messages that
    come meanwhile are set aside and handled after."""
    global _next_id
    _next_id += 1
    my_id = "plugin-%d" % _next_id
    send({"jsonrpc": "2.0", "id": my_id, "method": method, "params": params})
    while True:
        message = _incoming.get()
        if message is None:
            raise SystemExit(0)
        if message.get("id") == my_id and "method" not in message:
            if "error" in message:
                raise RuntimeError(message["error"].get("message", "refused"))
            return message.get("result")
        _set_aside.append(message)


def _read_input():
    """Reads messages on a thread of its own, so a command can look for a
    cancel between files without waiting."""
    for line in sys.stdin:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            _incoming.put(json.loads(line))
        except ValueError:
            log("a line that is not JSON came in", "warn")
    _incoming.put(None)


def _watch_parent(process_id):
    """Ends this plugin when diroxide is gone (Linux and macOS; on Windows
    diroxide's job object ends it)."""
    if os.name != "posix" or not process_id or os.getppid() != process_id:
        return

    def watch():
        while True:
            time.sleep(2)
            if os.getppid() != process_id:
                os._exit(0)

    threading.Thread(target=watch, daemon=True).start()


def next_message(block=True):
    """The next message: one set aside first. None when the input ended;
    False when none is waiting and block is False."""
    if _set_aside:
        return _set_aside.pop(0)
    try:
        return _incoming.get(block=block)
    except queue.Empty:
        return False


def was_cancelled(request_id):
    """Reads what is waiting now; True when diroxide cancelled this
    request. Requests that come meanwhile wait their turn."""
    waiting = []
    while True:
        message = next_message(block=False)
        if message is False:
            break
        if message is None or "id" in message and "method" in message:
            waiting.append(message)
            continue
        handle(message)
    _set_aside[:0] = waiting
    return request_id in cancelled


def count_lines(request_id, params):
    """CHANGE: the command. It counts the lines of each selected file,
    reports progress for a long list, and stops when cancelled."""
    context = params.get("context") or {}
    entries = context.get("selection") or []
    token = params.get("progress_token") or "count-%s" % request_id
    total = len(entries)
    long_list = total > 20
    if long_list:
        progress(token, kind="begin", title="Counting lines",
                 current=0, total=total, unit="files")
    done = 0
    skipped = 0
    failed = []
    lines_in_all = 0
    for index, entry in enumerate(entries):
        if was_cancelled(request_id):
            if long_list:
                progress(token, kind="end", message="Cancelled")
            fail(request_id, -32800, "cancelled", "request_cancelled")
            return
        place = entry.get("place") or entry.get("name") or ""
        if entry.get("name_raw"):
            failed.append({"place": place, "reason": "name is not Unicode"})
            continue
        if entry.get("kind") not in (None, "file", "archive", "link"):
            skipped += 1
            continue
        try:
            count = 0
            with open(place, "rb") as handle:
                for line in handle:
                    if settings.get("skip_blank") and not line.strip():
                        continue
                    count += 1
            lines_in_all += count
            done += 1
        except OSError as error:
            failed.append({"place": place,
                           "reason": error.strerror or "could not be read"})
        if long_list and (index + 1) % 10 == 0:
            progress(token, kind="report",
                     message="Counting %d of %d files" % (index + 1, total),
                     current=index + 1, total=total)
    if long_list:
        progress(token, kind="end")
    announce("Lines: %d, files: %d" % (lines_in_all, done))
    if failed:
        announce("Could not read %d files" % len(failed), "warning")
    reply(request_id, {"done": done, "skipped": skipped, "failed": failed})


COMMANDS = {"count_lines": count_lines}  # CHANGE: your commands, by id.


def handle(message):
    """Answers one message by its method."""
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}
    if method is None:
        return
    if method == "initialize":
        settings.update(params.get("settings") or {})
        _watch_parent(params.get("process_id"))
        reply(request_id, {
            "protocol": PROTOCOL,
            "plugin": {"name": params.get("plugin_id") or "plugin",
                       "version": VERSION},
            "plugin_capabilities": {"commands": sorted(COMMANDS)},
        })
    elif method == "settings/changed":
        settings.update(params.get("settings") or {})
    elif method == "$/cancel_request":
        cancelled.add(params.get("id"))
    elif method == "command/run":
        command = (params.get("command") or "").rsplit(".", 1)[-1]
        if command in COMMANDS:
            COMMANDS[command](request_id, params)
        else:
            fail(request_id, -32601, "no command %s" % command, "method_not_found")
    elif method == "shutdown":
        reply(request_id, None)
    elif method == "exit":
        raise SystemExit(0)
    elif request_id is not None:
        fail(request_id, -32601, "no method %s" % method, "method_not_found")
    # An unknown notification is ignored.


def main():
    threading.Thread(target=_read_input, daemon=True).start()
    while True:
        message = next_message()
        if message is None:
            return
        handle(message)


if __name__ == "__main__":
    main()
