Folder guard, an example diroxide plugin in Python.

It stops a delete inside the folders you protect, before the question is
asked, and logs every delete after it runs. diroxide says the reason with
the plugin's name last: "Delete stopped: folder is protected. By Folder
guard."

Check it:          diro plugin check folder-guard
Enable it:         diro plugin enable folder-guard
Protect a folder:  Plugins menu, Manage plugins, Folder guard, Settings,
                   then type the folder's full path; several are
                   separated by semicolons.
Read its log:      diro plugin log folder-guard
Run its tests:     python3 -I -m unittest   (py -3 -I -m unittest on Windows)

It needs Python 3 and nothing else; it uses no network and reads no
files. If it does not answer within two seconds, the delete goes on and
diroxide says so.
