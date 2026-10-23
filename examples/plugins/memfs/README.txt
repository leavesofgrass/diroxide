Memory files (toy), an example diroxide provider plugin in Python.

It keeps files in memory at the place memfs:/ until diroxide quits.
Check it:          diro plugin check memfs
Enable it:         diro plugin enable memfs
Then open memfs:/ in a panel and copy files in and out with F5.
Read its log:      diro plugin log memfs
Run its tests:     python3 -I -m unittest   (py -3 -I -m unittest on Windows)
Quitting diroxide loses everything in it; that is why it is a toy.
It needs Python 3 and nothing else; it uses no network.
