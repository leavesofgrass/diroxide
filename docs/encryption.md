# Encryption and signatures

This guide covers the Encryption submenu of the File menu: three commands that use age to encrypt and decrypt files, three that use gpg to encrypt, decrypt and sign them, and Verify signature, which checks a signature against its file. It also covers the sentences Verify checksum adds when the checksum list is signed.

The short version: Encryption makes a new file beside each file you choose and leaves the original where it is. A passphrase is typed in a hidden field and is never kept. A good signature means the key fits the file; it never means you know who holds the key. diroxide never fetches a key, and it never calls a file safe or genuine.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written, and no real age or gpg program had been run through the program's own commands; if your experience differs, that is a finding.

## Where the commands are

Encryption is a submenu of the File menu. Alt+F opens the File menu and Y opens the submenu. Its seven items, each with its own access key:

- Encrypt with a passphrase (P), age.
- Encrypt for a recipient (R), age.
- Decrypt (D), age.
- Encrypt with gpg (G).
- Decrypt with gpg (E).
- Sign with gpg (S).
- Verify signature (V).

The commands are also in the palette, and none has a key of its own. You can give one a key in `keymap.toml` ([Customizing keys](customizing-keys.md)). Each acts on the marked files, or on the focused file when nothing is marked. The three age commands run as a job in the jobs line, as any plugin command does. Folders are left out. When only folders are chosen, a gpg command says "Encrypt with gpg works on files; no file is chosen", and Verify signature on a folder says "Verify signature works on files: Backup is a folder".

Three programs do the work, and each is found a different way:

- age runs inside the Encryption plugin, an optional component. It needs no other program. [Optional components](components.md) says how to get it.
- gpg is the GnuPG program on your computer. diroxide finds it the way it finds every helper ([Tools and diro doctor](tools.md)). Without it: "gpg not found. Install GnuPG first."
- minisign signatures are checked inside diroxide. They need no program.

Without the Encryption plugin, the three age commands offer it, and the question starts on No. The first line says "Encrypting with age needs a plugin." When the plugin is here but not turned on, a command says "Encryption is not enabled. Enable it in Manage plugins." Turning it on asks the usual question ([Plugins](plugins.md)); the plugin declares that it uses no network and no program, and that it writes each result beside its file.

## Encrypt with a passphrase

This is age with a passphrase. A form opens, titled "Encrypt with a passphrase" with the plugin's name last. Its rows, in order:

- The first row is the default button, "Encrypt", with its summary after it. The summary is the plan, "Encrypt 3 files with a passphrase", and then a warning: "Typing is hidden. A lost passphrase cannot be got back, so note it where you keep such things."
- "Passphrase" and "Repeat the passphrase", both hidden fields.
- "ASCII armor", off unless you turn it on. Turned on, it writes the result as text instead of bytes.

Typing a passphrase twice is how a typing mistake is caught. If the two differ, the form comes back with "The two passphrases differ. Nothing was changed. Try again." An empty one says "A passphrase cannot be empty." After three tries that do not fit, the command stops with "Cancelled; nothing more was done". Escape on the form stops it too, and nothing is done.

The plugin says its plan, then each file: "Encrypt 3 files with a passphrase", then "Encrypted report.pdf to report.pdf.age". The result is `NAME.age` beside the file. A file of a megabyte or more says its progress at each tenth, such as "Encrypting 40 percent of backup.tar". Cancel stops the command and removes the unfinished file.

When it is done, the plugin says "The original is still here. The trash does not erase it." and diroxide asks about the original, as [the next section](#after-an-encryption-the-originals-and-the-trash) says.

## Encrypt for a recipient

This is age with a public key. The form is titled "Encrypt for a recipient". Its summary says "Encrypt 1 file. Paste an age public key, or give a file of them, one a line." The rows are:

- "Public key, starting age1": paste the recipient's key. Their key is public, so pasting it is safe.
- "Or a recipients file": a file's path, one key a line. A path completes as a path does.
- "ASCII armor".

Give a key, a file, or both. The form comes back with the reason first, up to three times, when something is wrong:

- "No recipient: paste a key or give a file".
- "Not an age public key: line 2". The line is counted in the pasted text or the file.
- "SSH keys are not used: line 3" and "Plugin keys are not used: line 3". diroxide uses only age's own keys, and a passphrase.
- "Hardware keys are not used: line 1" for the hardware tag keys, which start `age1tag1` and `age1tagpq1`.
- "The recipients file is over 1 MB" and "Recipients file unreadable: CAUSE".

The result is `NAME.age` beside each file, and the same lines and the same question about the originals follow as for a passphrase.

## Decrypt

Decrypt looks at each file's header before it asks anything, so you are never asked for what the file does not need. It says its plan, "Decrypt 3 files", as the job begins, and then, for each file:

- A file made with a passphrase asks "Passphrase for report.pdf.age (hidden):". A wrong one says "Wrong passphrase for report.pdf.age. Try again, 2 of 3." After three, "Gave up on report.pdf.age: 3 wrong tries".
- A file made for a key asks "Identity file for report.pdf.age". Type the path of your identity file, the one that holds your `AGE-SECRET-KEY-1` line. It may itself be locked with a passphrase, which is then asked the same hidden way. An identity file over 64 KB is refused.
- Within one command, a passphrase or an identity file that opened one file is tried first on the next, so ten files for one key ask once. Nothing outlives the command.

The result is the name without `.age`: `report.pdf` from `report.pdf.age`. A file whose name does not end in `.age` gets `.decrypted` after its name. A name that is taken is numbered the way copies are, such as `report.pdf (2)`; Decrypt never writes over a file.

Each result says "Decrypted report.pdf.age to report.pdf." and then, once, "This does not prove who made it." An age file carries no signature, so decrypting shows that the passphrase or key fit, and nothing about who sent it.

When a file cannot be decrypted, the line says "Could not decrypt report.pdf.age", and its cause follows as a sentence of its own:

- "It is not an age file."
- "Wrong passphrase, or the file was changed."
- "None of the keys in that file can open it." A file made for a key that is not in your identity file says this too.
- "The file is damaged or was changed." age checks every 64 KB piece as it goes, so a damaged or cut file fails, and its partial result is removed.
- "It is from a newer age; update the plugin."

A folder is passed over: "Skipped Photos: it is a folder", then "Pack the folder first, then encrypt the archive." A file that is not on this computer says "Skipped NAME: not on this computer", because the plugin works on local files. With nothing marked, "Nothing to do: mark the files first".

With several files, in an encryption or a decryption, a count comes before the closing sentence: "Encrypted 3 of 3 files", or "Decrypted 2 of 3 files; 1 failed".

## After an encryption: the originals and the trash

diroxide never removes an original by itself. After an age or gpg encryption it asks, and the question starts on No: "Move the original to the trash? No is the default. y or n". The file's name is listed under it, and then a warning: "The trash is not secure: the original can be restored from it." For several files the question counts them: "Move the 3 originals to the trash".

- No says "The originals stay where they are."
- Yes moves them to the trash as one job. That is not erasing: anyone who opens the trash can restore them, and the data stays on the disk. [Operations and undo](operations.md) says what deleting does and does not do.

Ctrl+Z undoes an encryption or a decryption by putting the new files in the trash. The originals were never touched, so there is nothing else to put back. Moving the original to the trash is its own job, and its own Ctrl+Z.

## Encrypt with gpg

This is OpenPGP, through your own gpg. It asks for the keys first. The field is "Encrypt for gpg keys (empty: passphrase)". Type one or more keys, as gpg knows them in your keyring: a key id, a fingerprint or an address, separated by commas, semicolons or spaces. Leave it empty to use a passphrase instead.

A plan comes next, and its question starts on No, with the files listed:

- "Encrypt 1 file for 1 recipient? No is the default. y or n", and under it "notes.txt to notes.txt.gpg".
- With no keys, "Encrypt 1 file with a passphrase".

"Planning with gpg" says the plan is being made. Planning runs nothing. No says "Nothing changed." Yes starts a job in the jobs line, "Encrypt started: 1 file for 1 recipient", and Cancel stops it.

For a passphrase, gpg asks for a new one twice, in the hidden prompt: "New passphrase for notes.txt (hidden):", then "Same passphrase again for notes.txt (hidden):". If the two differ, "Not the same. New passphrase for notes.txt (hidden):" asks again. The passphrase goes to gpg through its standard input and never appears in a command line, a message or a log.

The result is `NAME.gpg` beside the file: "Encrypted report.pdf to report.pdf.gpg". gpg's ASCII armor is not offered here. Several files, or a failure, say the job's summary first and the first "Could not encrypt NAME: CAUSE" after it. The same question about the originals follows an encryption.

### What gpg is told

diroxide runs gpg with a fixed set of rules, so the program cannot surprise you:

- gpg never asks on a terminal and never looks a key up on a server. Looking a key up would send an address away.
- diroxide never changes how much gpg trusts a key. A recipient key you have not certified is not used, and the first file fails with the cause: "Could not encrypt notes.txt: Key not certified: sign it in your key tool, or choose another". A key gpg does not know says "Key not found: ada@example.invalid". In both cases the rest of the files are skipped for the same cause, and nothing is made.
- gpg never writes over a file. It writes a new name.
- Only gpg's own status words are read, and the owner of a key is named only after the key's ending, because a key's name is text its maker chose.

## Decrypt with gpg

Decrypt with gpg has no field. It plans, asks "Decrypt 1 file? No is the default. y or n", and then runs. A key that gpg's own agent already holds asks nothing. Otherwise the hidden prompt says "Passphrase for key ending 4E7B (hidden):", with the last four digits of the key. A wrong passphrase asks "Try again?", starting on No, the way a wrong archive password does, and Yes shows the hidden prompt again. After three wrong ones the file fails: "Could not decrypt report.pdf.gpg: wrong passphrase 3 times".

A file for a key you do not have says "No secret key for it: key ending 4E7B". A file that is not OpenPGP data says "Could not decrypt report.pdf: it is not an OpenPGP file". A good result says "Decrypted report.pdf.gpg to report.pdf." and, once, "This does not prove who made it."

## Sign with gpg

Sign with gpg makes a detached signature, a separate file that holds only the signature and leaves the file as it is. Its field is "Sign with gpg key (empty: your default)". Type a key id, a fingerprint or an address, or leave it empty for your default key.

The plan reads "Sign 3 files with key ending 4E7B", or "Sign 3 files with your default key", and its question starts on No. The result is `NAME.sig` beside each file: "Signed report.pdf in report.pdf.sig". The key's passphrase is asked in the same hidden prompt, "Passphrase for key ending 4E7B (hidden):", or "Passphrase for your signing key (hidden):" for the default key.

## Verify signature

Verify signature checks one file's signature. Put the focus on the file or on its signature, and choose Verify signature. It looks beside the file for `NAME.asc`, `NAME.sig` and `NAME.minisig`, in that order, and then for `SHA256SUMS.asc`, the signature of a checksum list that may name the file.

- Focus on a file with a signature beside it, and the signature is checked against the file.
- Focus on a signature, and it is checked against the file it signs, which has the signature's name without its last extension. A minisign signature with no file beside it says "notes.txt.minisig signs no file here". An OpenPGP signature with no file beside it is checked as a file that was signed inline.
- With no signature: "No signature beside report.pdf".
- A signature for the checksum list says so after the verdict: "That signature is on SHA256SUMS; Verify checksum uses it."

An OpenPGP signature, `.asc` or `.sig`, is checked by gpg. "Checking the signature" says the check has started. A minisign signature, `.minisig`, is checked inside diroxide, and it needs the signer's public key, which is asked every time and kept nowhere. The field is "minisign public key: a file or pasted". Paste the one-line key that starts `RW`, or type the path of the `minisign.pub` file, which Tab completes; a path that is not whole is read from the signature's own folder. An empty answer says "No key given; nothing was checked."

### What you hear

The verdict comes first. When there is more to say, a list opens with the verdict as its title and the rest as its lines, and when there is not, the verdict is the status line alone.

A good signature says "Signature good: key ending 4E7B". The lines after it:

- "Key ending 4E7B is named Ada Example <ada@example.invalid>", the name the key's maker gave it, cleaned of hidden characters and cut at 120 characters. For minisign the line is the signature's trusted comment.
- "You have not certified this key", when gpg does not trust the key.
- "Good means the key fits, not who holds it." This is said after every good signature, and it is the line to take seriously. A signature proves that the holder of one key signed the file, and only you can know whose key that is.

The other verdicts, each with its meaning first:

- "Signature bad: the file was changed". The signature does not fit the file, or it belongs to another file.
- "Cannot check: public key is missing", then "The key ends 4E7B." and "diroxide does not fetch keys." Import the key yourself, then check again.
- "Good signature, but the key expired" and "Good signature, but it has expired".
- "Key revoked. Do not trust this file."
- "Cannot check: signed by another key", "Cannot check: public key unreadable" and "Cannot check: no public key given" for minisign.
- "Cannot check: signature unreadable" and "Cannot check: the file is unreadable".
- "gpg not found. Install GnuPG first." when an OpenPGP signature needs gpg and it is not there.
- "Signature check cancelled".

## The chain in Verify checksum

When a checksum list has a signature beside it, Verify checksum checks that signature too and says one more sentence about it, so a match tells you what it can. [Checksums and verifying](checksums.md#verify-checksum) has the whole list of rows. The three sentences are:

- "List is signed: 12 of 12 files match". The signature is good, so the list is the one its key signed, and the files match it.
- "List not signed: 12 of 12 match", then "This shows the files are not damaged. It does not show who made them." No signature sits beside the list.
- A signature that is bad, or cannot be checked, says its verdict first, then "List not verified: 12 of 12 match".

Verify checksum looks for `NAME.asc`, `NAME.sig` and `NAME.minisig` beside the list. It checks an OpenPGP signature with gpg. It cannot check a minisign signature, because it has no key to check it with: it says "Cannot check: no public key given" and then "List not verified". To check a list signed with minisign, focus the list and use Verify signature, which asks for the key.

## What is never stored

- A passphrase is typed into a hidden field, kept in memory while a command needs it, and written nowhere: not a setting, a file, a log, a message or the system's keyring. The age plugin wipes it when the command ends. For gpg, diroxide wipes its own copy, but the bytes it hands gpg on its standard input travel in a request that does not wipe its copy yet. That is a known limit, and the request is never printed, logged or said.
- An age identity file is asked for by its path each time, read for that one command, and not remembered.
- A minisign public key is asked for each time.
- gpg gets a passphrase on its standard input only, never in a command line or an environment variable. One passphrase that worked is held for the rest of that job, once for each key, so ten files for one key ask once.
- No key is ever fetched from a server.
- Nothing about any of this leaves your computer.

## A hostile file

An age file can ask for so much work to open that one tries, for hours, to get at its passphrase. diroxide encrypts at a modest cost and opens files that ask for at most about two seconds' work on a fast computer. A file that asks for more is refused from its header, before anything is asked and before the work starts: "Could not decrypt report.pdf.age", then "It asks for more work than diroxide allows. It may be hostile."

A name that tries to look like another is said with its hidden characters spelled out, such as `[U+000A]` for a line break, so a file called "report.pdf" with a hidden character in it reads as what it is.

## Why age is a plugin

age is in an optional plugin and not in the program itself, because two security advisories sit in the libraries it uses: RUSTSEC-2026-0387, in crypto-bigint, about timing, and RUSTSEC-2026-0374, in basic-toml, which is unmaintained. The first is reached only through the hardware keys, which diroxide refuses to use. The second runs only while the plugin is built. The plugin uses only passphrases and age's own keys. [ADR-0015](adr/0015-encryption.md) has the reasons and what would move age into the program.

## On Windows

gpg keeps a small helper, its agent, that remembers a passphrase for a while. On Windows diroxide stops everything a program started when the program ends, and that includes the agent, so no passphrase is remembered from one command to the next, and a command that needs the agent waits for it to start again. It was measured at about six seconds on one computer. In the same test the runs took a fraction of a second when the agent had been started outside diroxide, but that was not tried through the program's own commands, so it is not a promise. Within one job a passphrase is asked once for each key.

## Known limits

- The age commands need the Encryption plugin, and the plugin writes the files itself. The program records the new files for Undo from the plugin's answer, and that record lives for the session only: it does not survive a restart.
- ASCII armor is offered for age only. gpg's armor is not offered, so every gpg result is `.gpg` or `.sig`.
- A passphrase typed for gpg passes through the request that starts gpg, as bytes that are not wiped when the request is dropped. The age plugin wipes its copy.
- SSH keys, plugin keys and hardware keys are refused for age. The age program is not used as a road for them.
- The commands are in the program. No dedicated `diro` command encrypts, decrypts, signs or checks a signature from a shell yet. `diro run` starts a command by its id, but it has nobody to type a passphrase, so a command that asks for one is left open and ends with exit code 1.
- A gpg run that needs its agent is slow on Windows, as above, and a passphrase is not remembered between commands there.
- A minisign public key is asked every time, and Verify checksum does not check a list's minisign signature.
- Nothing about how these dialogs and verdicts read on a Braille display has been heard yet.

## See also

- [Checksums and verifying](checksums.md)
- [Optional components](components.md)
- [Tools and diro doctor](tools.md)
- [Plugins](plugins.md)
- [Operations and undo](operations.md)
- [Security](security.md)
- [Privacy](privacy.md)
- [Known limits](known-limits.md)
- [Customizing keys](customizing-keys.md)
- [The documentation index](README.md)
