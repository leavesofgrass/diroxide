"""Writes the fuzz targets' seeds for the git plugin (R30 commit 106).

Each seed is what git, or a hostile repository, could hand the plugin:
porcelain v2 status records, log records with unit separators, a config
listing with scopes, blame, branch and stash lines and progress lines.
Every name and identity is a placeholder ("Ada Example", example.invalid).
The files have plain names; run from anywhere:

    py -3 plugins/diro-git/fixtures/make-seeds.py

The plugin's tests replay every seed, cut and bent (tests/it/fuzz.rs).
"""

import os

HERE = os.path.dirname(os.path.abspath(__file__))
OID = ("100644 100644 100644 5626abf0f72e58d7a153368ba57db4c673c0e171 "
       "5626abf0f72e58d7a153368ba57db4c673c0e171")
NUL = b"\x00"
US = b"\x1f"


def records(*items):
    return b"".join(item.encode("utf-8") + NUL for item in items)


SEEDS = {
    "status_v2": {
        "branch": records(
            "# branch.oid f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "# stash 1",
        ),
        "initial": records("# branch.oid (initial)", "# branch.head main",
                           "? new.txt"),
        "changes": records(
            "# branch.oid f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57",
            "# branch.head (detached)",
            "1 .M N... " + OID + " a.txt",
            "1 MM N... " + OID + " sub/c.txt",
            "1 AD N... " + OID + " d.txt",
            "2 R. N... " + OID + " R100 b2.txt",
            "b.txt",
            "2 CM N... " + OID + " C75 copy name.txt",
            "orig name.txt",
            "? u/",
            "! target/",
        ),
        "conflicts": records(
            "u UU N... 100644 100644 100644 100644 a1 b2 c3 both.txt",
            "u AA N... 100644 100644 100644 100644 a1 b2 c3 added.txt",
            "u DU N... 100644 100644 100644 100644 a1 b2 c3 gone.txt",
        ),
        "submodules": records(
            "1 .M SC.. " + OID + " libs/one",
            "1 .M S.MU " + OID + " libs/two",
        ),
        "odd-names": records(
            "1 .M N... " + OID + " two  spaces.txt",
            "1 .M N... " + OID + " \"quoted\".txt",
            "1 .M N... " + OID + " café/日本.txt",
            "1 .M N... " + OID + " -dash.txt",
            "1 .M N... " + OID + " line\nbreak.txt",
        ) + b"1 .M N... " + OID.encode() + b" bad-\xff\xfe.txt" + NUL,
    },
    "log_record": {
        "log": (b"f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57" + US + b"f2e5e54" + US
                + b"2026-10-09" + US + b"Ada Example" + US + b"Fix the parser"
                + NUL + b"\n0123456789abcdef0123456789abcdef01234567" + US
                + b"0123456" + US + b"2026-10-08" + US + b"Ada Example" + US
                + b"First" + NUL),
        "name-status": b"M\x00src/lib.rs\x00R087\x00a.txt\x00b.txt\x00A\x00new.txt\x00D\x00gone.txt\x00",
    },
    "config_scan": {
        "clean": records("global", "user.name\nAda Example",
                         "local", "core.repositoryformatversion\n0",
                         "local", "remote.origin.url\nhttps://example.invalid/r.git",
                         "command", "core.fsmonitor\nfalse"),
        "hostile": records("local", "core.fsmonitor\n./watch.sh",
                           "local", "filter.evil.clean\nsh evil.sh",
                           "worktree", "core.sshcommand\nevil",
                           "local", "alias.st\n!sh -c evil",
                           "local", "include.path\nextra.cfg",
                           "local", "core.pager"),
    },
    "other_readers": {
        "blame": ("f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57 1 1 1\n"
                  "author Ada Example\nauthor-mail <ada@example.invalid>\n"
                  "author-time 1760032800\nauthor-tz -0700\n"
                  "summary Fix the parser\nfilename hello.txt\n\tthe line\n").encode(),
        "branches": ("*\x1fmain\x1forigin/main\x1fahead 2, behind 1\x1ff2e5e54"
                     "\x1f2026-11-06\x1fFirst\n \x1ftopic\x1f\x1f\x1f0123456"
                     "\x1f2026-11-05\x1fTry\n").encode(),
        "stashes": (b"stash@{0}" + US + b"238ace184a78226b47540f8639c73f7918fd663f"
                    + US + b"On main: diroxide discard" + US + b"2026-11-06" + NUL),
        "progress": (b"Cloning into 'repo'...\nremote: Enumerating objects: 5, done.\n"
                     b"Receiving objects:  45% (450/1000)\rReceiving objects: 100%"
                     b" (1000/1000), done.\r\n"),
    },
}


def main():
    for target, seeds in SEEDS.items():
        folder = os.path.join(HERE, "seeds", target)
        os.makedirs(folder, exist_ok=True)
        for name, data in seeds.items():
            with open(os.path.join(folder, name), "wb") as out:
                out.write(data)


if __name__ == "__main__":
    main()
