"""A correct plugin with the standard library alone: the conformance
suite's Python subject. One JSON object per line, UTF-8, a flush after
every message."""

import json
import sys

stdin = open(sys.stdin.fileno(), "r", encoding="utf-8", newline="\n")
stdout = open(sys.stdout.fileno(), "w", encoding="utf-8", newline="\n")


def send(message):
    message["jsonrpc"] = "2.0"
    stdout.write(json.dumps(message, ensure_ascii=False, separators=(",", ":")) + "\n")
    stdout.flush()


def error(id, code, text):
    send({"id": id, "error": {"code": code, "message": text}})


started = False
stopping = False
for line in stdin:
    line = line.rstrip("\r\n")
    if not line.strip():
        continue
    if line.lstrip().startswith("["):
        error(None, -32600, "batches are not supported")
        continue
    try:
        message = json.loads(line)
    except ValueError:
        error(None, -32700, "the line is not JSON")
        continue
    method = message.get("method")
    id = message.get("id")
    if method is None:
        continue
    if method == "exit":
        sys.exit(0)
    if id is None:
        continue
    if stopping:
        error(id, -32600, "shutting down, only exit is accepted")
    elif method == "initialize":
        started = True
        send({"id": id, "result": {
            "protocol": 1,
            "plugin": {"name": "Python echo", "version": "0.1.0"},
            "plugin_capabilities": {"commands": ["echo"]},
        }})
    elif not started:
        error(id, -32002, "initialize has not been answered yet")
    elif method == "shutdown":
        stopping = True
        send({"id": id, "result": None})
    elif method == "command/run":
        context = message.get("params", {}).get("context") or {}
        count = len(context.get("selection") or [])
        send({"id": id, "result": {"done": count, "skipped": 0, "failed": []}})
    else:
        error(id, -32601, "no method " + str(method))
sys.exit(0)
