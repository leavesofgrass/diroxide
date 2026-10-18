# ADR-0009: Secrets and the masked prompt

Status: Accepted, Friday, October 16, 2026 (the Wave 1 contract, P0b, from research/encryption.md, R9, sections "Where the contract has to change" and "Changes to the contracts", items 1 to 4).

## Context

From the plan's alpha.2 on (which ships as part of alpha 1), diroxide asks for passwords and passphrases: an encrypted zip or 7z, an SSH key or host (the SFTP plugin), an encrypted file. The owner reads with a screen reader and a Braille display, so a masked field must say what it is for and still let the owner check what was typed. A password that reaches a log, a debug print, a saved job or a crash report is a leak. Phase 0 kept a `Secret` and a `PasswordPrompt` inside the SFTP crate, where the archive provider and the encryption jobs could not use them; the provider trait had no way to ask; `VfsError` had no word for "a password is needed"; and `Dialog::Prompt` kept its text in a plain `String` that derives `Debug`.

## Decision

- **One secret type, in core.** `diroxide_core::Secret` wraps the text in a buffer that is wiped when dropped (`zeroize`); a clone is wiped too. Its `Debug` says only "Secret(hidden)". It has no `Serialize`, so no `JobRequest`, `Command`, message, journal or saved file can hold one. Its one way out is `expose`, for the library that needs the text. `push` and `pop` serve the masked prompt; a buffer that grows wipes its old copy.
- **A request says what is asked, and holds no secret.** `SecretRequest` names the kind (`SecretKind`: archive password, file passphrase, key passphrase, host password, a server's own question), the subject, the user, the try and the tries allowed (three), whether the answer may echo, and whether it is a new secret typed twice. Its title and prompt are the words: "Password for course.zip", and in the terminal "Password for course.zip (hidden): ", a long name shortened in the middle so "Password for" and the name's ends fit 40 cells.
- **The provider asks through its context.** `OpContext` carries a `SecretSource`; its `ask` blocks the worker thread while the app shows the prompt, the way a conflict question does (ADR-0004), and the jobs line says "Waiting for the password for course.zip." at once, as a change of state. The default is `NoSecrets`, which answers nothing. `VfsError` gains `PasswordRequired` ("a password is needed") and `WrongPassword` ("the password is wrong, or the data is damaged"; some formats cannot tell the two apart), each with a next step.
- **The dialog is its own shape.** `Dialog::Secret` holds the request, the typed `Secret` and the Show toggle; `Answer::Secret` carries the secret on Enter, straight to the job that asked, never into a command. In the terminal the field shows one asterisk per character by default (proof a key registered, visible on the Braille display, revealing only the length), with silence as a setting. The Show toggle is off by default; turning it on says "Showing the password. Anyone who can see or hear your screen can read it." and turning it off says "Hidden." A new secret is typed twice; a difference says "The two passwords differ. Nothing was changed. Try again."; an empty one says "A password cannot be empty." A wrong one says "Wrong password for course.zip. Try again, 2 of 3.", and after the third, "Gave up on course.zip after 3 wrong passwords. Skipped." Three is a limit on one operation, not a lockout. The words are in the core catalog.
- **Nothing is remembered on disk.** No password is stored, logged or put in a place's text; an archive's password is kept only while the archive is open or a job uses it. No keyring in the alpha series.
- **The canary test.** Every crate that touches a secret has a test with a canary string ("canary-7f3a-not-a-password") that fails if the canary reaches a `Debug` output, a serialized request or a message: core, vfs, the archive requests, the app's dialog, the SFTP plugin.
- **The honest delete.** There is no secure delete: diroxide cannot know whether a file sits on flash, a journaled or copy-on-write file system, a share, a synced folder or a snapshot. Every permanent-delete confirmation carries "Deleted data may stay on the disk. diroxide cannot erase it for good."

## Alternatives

- **`secrecy::SecretString`.** A good wrapper, and declared in the workspace for the SSH plugin; core uses `zeroize` directly because the masked prompt needs to add and remove characters, which `SecretString` does not offer without copying the text out.
- **Passwords as command arguments.** Simple, and the way a password reaches a log, a journal and a crash report. Refused.
- **A system keyring.** Convenient, and a store diroxide would have to explain and secure; left until after the final alpha, and only if the owner asks.
- **Secure delete by overwriting.** Unreliable on modern storage, and a promise diroxide cannot keep; refused, as Apple refused it.

## Fallback

If a library needs the secret as owned text, `expose` gives a borrowed string that the caller copies into the library's own wiping type where it has one. If the asterisks prove noisy with a screen reader in the owner's test, the silent setting becomes the default.

## Consequences

- The SFTP plugin's own `Secret` and `PasswordPrompt` are gone; it asks through the context with a host-password request.
- `PackRequest` and `UnpackRequest` hold an optional `Secret`, so they are never serialized; options that travel in jobs stay free of secrets.
- W1d draws the masked prompt and its Show toggle; W1c adds the setting for asterisks or silence; W2a asks for archive passwords; W3c for SSH.

## See also

- [ADR-0004: The job engine](0004-the-job-engine.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)
