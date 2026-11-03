#!/usr/bin/env python3
"""Dump what Orca would see of diroxide's window through AT-SPI.

Finds diroxide's window on the accessibility bus by its process id,
prints its tree (roles, names, the states a screen reader says), checks
the panels' list boxes (each a list with its options, the focused row
selected as the active descendant does), records the announcement and
focus events for a few seconds, and exits non-zero when a check fails.

Run it with tools/a11y/atspi-check.sh (Xvfb, a private D-Bus session and
the AT-SPI bus), or by hand on a desktop with accessibility on:

    python3 tools/a11y/atspi-dump.py --pid PID --seconds 6

Adapted from textweaver's crates/textweaver-xilem/tools/atspi-dump.py (the
owner's own project).
"""

import argparse
import sys
import time

import pyatspi  # python3-pyatspi

EVENTS = []
PID = None
START = 0.0
SAID = ("focusable", "focused", "selected", "selectable", "multiselectable",
        "editable", "enabled", "showing", "modal")


def ours(acc):
    try:
        return acc.get_process_id() == PID
    except Exception:
        return False


def on_event(event):
    if not ours(event.source):
        return
    detail = ""
    if event.type.startswith("object:announcement"):
        detail = repr(event.any_data)
    else:
        try:
            detail = f"{event.source.getRoleName()} {event.source.name!r}"
        except Exception:
            detail = "(gone)"
    EVENTS.append(f"{time.monotonic() - START:7.2f} s  {event.type:36} {detail}")


def states(acc):
    return sorted(pyatspi.stateToString(s) for s in acc.getState().getStates())


def place(acc):
    """'3 of 40' from the node's posinset and setsize attributes."""
    try:
        attrs = dict(a.split(":", 1) for a in acc.getAttributes() if ":" in a)
    except Exception:
        return ""
    if "posinset" in attrs and "setsize" in attrs:
        return f" {attrs['posinset']} of {attrs['setsize']}"
    return ""


def walk(acc, depth, out, lists):
    if depth > 16 or len(out) > 400:
        return
    role = acc.getRoleName()
    name = acc.name or ""
    st = ",".join(s for s in states(acc) if s in SAID)
    out.append(f"{'  ' * depth}- {role} {name!r}{place(acc)} [{st}]")
    if role in ("list", "list box"):
        lists.append(acc)
    for i in range(acc.childCount):
        try:
            walk(acc.getChildAtIndex(i), depth + 1, out, lists)
        except Exception as e:  # a node went away mid-walk
            out.append(f"{'  ' * (depth + 1)}- (gone: {e})")


def find_app():
    """The application with our process id, and its first window."""
    desktop = pyatspi.Registry.getDesktop(0)
    for i in range(desktop.childCount):
        app = desktop.getChildAtIndex(i)
        if app is None:
            continue
        try:
            if app.get_process_id() != PID:
                continue
        except Exception:
            continue
        for j in range(app.childCount):
            w = app.getChildAtIndex(j)
            if w is not None:
                return app, w
        return app, None
    return None, None


def bus_listing():
    desktop = pyatspi.Registry.getDesktop(0)
    out = []
    for i in range(desktop.childCount):
        app = desktop.getChildAtIndex(i)
        try:
            out.append(f"{app.name!r} pid {app.get_process_id()} windows {app.childCount}")
        except Exception as e:
            out.append(f"(unreadable: {e})")
    return out


def main():
    global PID, START
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--seconds", type=float, default=6.0)
    args = ap.parse_args()
    PID = args.pid
    START = time.monotonic()
    failures = []

    for kind in ("object:announcement", "object:state-changed:focused",
                 "object:active-descendant-changed"):
        pyatspi.Registry.registerEventListener(on_event, kind)

    app, window = None, None
    for _ in range(300):
        app, window = find_app()
        if window is not None:
            break
        time.sleep(0.2)
    if window is None:
        print("Fail: no diroxide window on the AT-SPI bus (is accessibility on?).")
        print("On the bus: " + "; ".join(bus_listing()))
        return 1
    print("# diroxide AT-SPI report\n")
    print(f"- Application: {app.name!r} (toolkit {app.get_toolkit_name()!r})")
    print(f"- Window: {window.getRoleName()} {window.name!r}\n")
    if not (window.name or "").endswith(" - diroxide"):
        failures.append(f"the window is named {window.name!r}, not after its place")

    from gi.repository import GLib

    def stop():
        pyatspi.Registry.stop()
        return False

    GLib.timeout_add(int(args.seconds * 1000), stop)
    pyatspi.Registry.start()

    out, lists = [], []
    walk(window, 0, out, lists)
    print("## Tree\n\n```")
    print("\n".join(out))
    print("```\n")
    panels = [lst for lst in lists if (lst.name or "").startswith(("Left panel", "Right panel"))]
    print("## The panels\n")
    if len(panels) != 2:
        failures.append(f"{len(panels)} panel lists, not 2")
    for lst in panels:
        n = lst.childCount
        print(f"- {lst.name!r}: {n} options in the tree, states {states(lst)}")
        if n == 0:
            failures.append(f"{lst.name!r} has no options")
        elif not place(lst.getChildAtIndex(0)):
            failures.append(f"{lst.name!r}'s first option has no posinset and setsize")
    print("\n## Events\n\n```")
    print("\n".join(EVENTS) or "(none)")
    print("```\n")
    print("## Result\n")
    if failures:
        print("Fail:")
        for f in failures:
            print(f"- {f}")
        return 1
    print("Pass: every check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
