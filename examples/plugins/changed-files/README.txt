Changed files, an example diroxide lister plugin in Python.

Show changed files lists a git repository's changes, the state first.
Enter on a row shows its diff; Alt+S stages the marked rows.
Check it:          diro plugin check changed-files
Enable it:         diro plugin enable changed-files
Read its log:      diro plugin log changed-files
Run its tests:     python3 -I -m unittest   (py -3 -I -m unittest on Windows)
It needs git, which it runs through diroxide, never through a shell.
It needs Python 3 and nothing else; it uses no network.
