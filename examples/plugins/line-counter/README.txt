Line counter, an example diroxide plugin in Python.

It counts the lines of the selected files and says the total.
Check it:          diro plugin check line-counter
Try it once:       diro plugin run line-counter -- FILE
Enable it:         diro plugin enable line-counter
Read its log:      diro plugin log line-counter
Run its tests:     python3 -I -m unittest   (py -3 -I -m unittest on Windows)
Change main.py where it says CHANGE; keep plugin.toml in step.
It needs Python 3 and nothing else; it uses no network.
