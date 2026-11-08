# ADR-0015: Encryption and signatures: age in a plugin, gpg as a program, minisign in process

Status: Accepted, Sunday, November 8, 2026 (alpha 4, Wave 7: lane E's draft, written beside commit 71 for the orchestrator's acceptance at the close; spikes ES1 in the contract step, ES2 and ES3 in lane E).

## Context

Alpha 4 (the plan's alpha.7) brings file encryption, decryption, detached signatures and the signature chain around a checksum list (R29 part 3, R9 `research/encryption.md`, R30 commits 71 to 73). Answer 47 says a young or unaudited Rust crate never guards a password, a key or a connection, and that each such choice is an ADR. Answer 115 put age in the main binary if spike ES1 passed, and in a `diro-crypt` plugin otherwise.

ES1 failed (contract step, Friday, November 6, 2026): age 0.12.1 with only `armor` kept the tree free of C and passed the native gate, but `cargo deny` refused two advisories on its path, and the release binary grew by 568,832 bytes and a clean build by 51 seconds. The advisories, as the cached advisory database names them:

- **RUSTSEC-2026-0387, crypto-bigint 0.5.5** (a vulnerability, "timing variability issues in operations intended to be constant-time"; patched in 0.7.6). It arrives through p256 and elliptic-curve 0.13 under hpke, which age uses only for its hardware "tag" recipients (`age1tag1`, `age1tagpq1`).
- **RUSTSEC-2026-0374, basic-toml 0.1.10** (unmaintained, no patched version). It arrives at build time, under the translation macros of age's message catalog.

ADR-0009 already holds that a secret lives in a wiping type, is asked through the masked prompt, and is never written down; ADR-0011 holds that only the SFTP plugin may use the system keyring, and that key passphrases are never stored.

## Decision

- **age lives in the `diro-crypt` plugin, never in the main workspace.** A workspace of its own (`plugins/diro-crypt`), shipped as an optional component, with age 0.12.1 and only its `armor` feature: `ssh` stays off (the `rsa` crate's open timing advisory, RUSTSEC-2023-0071), and so do `plugin`, `async` and `cli-common`. Pure Rust: `cargo tree -i cc --target all` over the plugin's graph finds `cc` only as a build dependency of `psm` under carta's `stacker`, which reaches every plugin through `diroxide-plugins` and `diroxide-ops` and which the main workspace's native gate already allows; age adds none. `unsafe_code` is denied.
- **Only passphrases and age's own X25519 keys are used.** SSH keys, plugin keys and the hardware tag keys are refused in words ("Hardware keys are not used: line 1"), so the P-256 arithmetic under RUSTSEC-2026-0387 is compiled into the plugin and never reached; basic-toml runs only while the plugin is built. Both advisories are named in the plugin's manifest and `Cargo.toml`. **What fixes them:** an age release on the RustCrypto 0.14 elliptic curves (crypto-bigint 0.7.6 or later) and a translation stack without basic-toml; the plugin moves to it when it is published, and answer 115's first branch (age in the main binary) can be weighed again then.
- **Three commands, every secret asked and none kept.** "Encrypt with a passphrase" asks it twice in one form's hidden fields (`ui/form`, kind `secret`) and compares the two in the plugin; "Encrypt for a recipient" takes a pasted `age1` key, a recipients file by its path, or both; "Decrypt" looks at the file's header first and asks "Passphrase for report.pdf.age" in a hidden question or an identity file by its path. A passphrase is taken at once into age's wiping string and dropped with the command; an identity file's keys (and, for one under a passphrase, its decrypted text, in a wiping buffer) live in memory for that command only. Nothing is written to a setting, a file, the log, standard error or the keyring. A passphrase recipient is always a file's only recipient (age's rule), because it has its own command.
- **The scrypt work factor: 18 to encrypt, at most 20 to decrypt** (spike ES2, on the owner's Ryzen 5 5600G, release build, 1 MiB in memory; three runs, two on Friday, November 6, 2026 near 11:00 PM and one on Saturday, November 7, 2026 at 11:04 AM, with 37 to 65 percent of the processor busy with other builders; a fourth run at 97 percent load, just after the machine woke, roughly doubled the times from 19 up and is left out):

  | Work factor | Encrypt | Decrypt |
  |---|---|---|
  | 16 | 0.12 to 0.14 s | 0.12 to 0.14 s |
  | 17 | 0.27 to 0.29 s | 0.27 to 0.28 s |
  | 18 | 0.50 to 0.55 s | 0.49 to 0.54 s |
  | 19 | 0.99 to 1.16 s | 1.05 to 1.18 s |
  | 20 | 2.11 to 2.23 s | 1.97 to 2.26 s |
  | 21 | 4.25 to 4.78 s | 4.08 to 4.93 s |
  | 22 | 8.31 to 9.46 s | 8.26 to 9.58 s |

  The reference program (the Go `age`) encrypts at 18 by default (from memory of its source; not re-read in this lane). age the crate picks a factor for "about one second" on the computer it runs on, and picked 20 here (about two seconds), so a file made by `rage` on a fast computer may ask for more. The plugin encrypts at 18, so its files open in half a second here and on slower computers, and accepts at most 20: a file at the reference default opens in half a second, a file at 20 in about two, and 21 or more is refused before anything is asked, from the header itself, in 2.4 ms: "Could not decrypt report.pdf.age", then "It asks for more work than diroxide allows. It may be hostile." age's own check refuses it again inside the decryption, at the same cap.
- **Writing.** Each output is written beside its file through the local provider's committing writer (an unfinished `.diro-part-` name in the target folder, renamed last with the rename that never replaces); a taken name is numbered as diroxide's copies are ("report.pdf (2).age"); a cancel or a failure drops the writer, which removes the unfinished file; the original is never touched. age's stream authenticates each 64 KiB chunk, so a damaged or truncated file fails ("The file is damaged or was changed.") and its partial output is removed. Each command answers what it made (`made: [{from, to}]`), so the app records each new file in its undo list and offers the trash for an original, starting on No and saying the trash does not erase it.
- **gpg runs as the program the tools registry finds, never as a library and never hosted** (answer 116), with R29 3.3's rules exactly: the common prefix `--batch --no-tty --status-fd 1 --no-greeting --disable-dirmngr --no-auto-key-locate`; a passphrase only on standard input with `--pinentry-mode loopback --passphrase-fd 0`, never in an argument; the input always a file after `--`; the output always a fresh name with `--output` and never `--yes`; only `[GNUPG:]` lines read; `--trust-model always` never added; a recipient gpg will not use is said ("Key not certified: sign it in your key tool, or choose another"). Decryption and signing go in two steps: `--pinentry-mode error` first, so a key the owner's own agent holds never asks, then the masked prompt ("Passphrase for key ending 4E7B (hidden):", 40 cells) and loopback; a passphrase that worked is held for the rest of that job only, one per key, and three wrong ones fail the file. Verification maps the status words (GOODSIG, BADSIG, ERRSIG, EXPSIG, EXPKEYSIG, REVKEYSIG, VALIDSIG, NO_PUBKEY and the TRUST words) to one verdict and one sentence; the signer's own text is cleaned of hidden characters and said only after "key ending 4E7B".
- **minisign is verified in process,** by `minisign-verify` 0.3.0 (MIT, pure Rust, no dependencies, 2,500 lines, verify only) in the main binary, through `diroxide-ops`, with a public key from a file or a pasted string. It guards no secret: it checks a public signature with a public key, which answer 47 does not reach.
- **The chain's sentences never overclaim.** "List is signed: 12 of 12 files match" only after a good signature; with no signature, "List not signed: 12 of 12 match", then "This shows the files are not damaged. It does not show who made them."; with one that is bad or cannot be checked, its verdict first, then "List not verified: 12 of 12 match". A good signature is followed by "Good means the key fits, not who holds it." Nothing is ever called safe, genuine or authored.

## Why age satisfies answer 47 conditionally

age's primitives come from RustCrypto (ChaCha20-Poly1305, HKDF, HMAC, scrypt, SHA-2) and dalek (X25519); the format is the reference one, shared with the Go program and `rage`; it is widely used. It is pre-1.0 by its own words and no audit is named on its pages (R9). So it is used where the owner opts in (a plugin he installs), only for the two recipient types it was built around, with the work cap that keeps a hostile file from costing hours, and with the age program as the road for anyone who prefers it. That is a conditional pass, recorded as such.

## Alternatives

- **age in the main binary, behind a default-on feature** (answer 115's first branch). Refused by ES1: two advisories that `cargo deny` refuses, 569 KB and 51 seconds. Weighed again when age moves past them.
- **Only the age or rage program through the tools registry.** Works with no crate at all, but a passphrase then reaches the program only through its own prompt, not diroxide's masked one (R9). Kept as the fallback.
- **OpenPGP in process (rPGP, Sequoia).** rPGP leaves expiry and revocation to the application, so a "good signature" from a revoked key would be a wrong answer; Sequoia is LGPL with a C backend by default. gpg stays a program the owner's other tools already share.
- **A plugin that writes through `fs/stage` and `fs/commit`.** The host has their shape only; until it serves them, the plugin writes through the same committing writer the engine uses and answers what it made.

## Fallback

If the plugin is not installed, "Encrypt" and "Decrypt" say so in the app's words and offer it (the component), and a user with the age program can run it through the tools registry; if gpg is missing, "gpg not found. Install GnuPG first." with the package command; minisign has no fallback because it needs none. If age's API moves under the plugin, it stays pinned at 0.12.1 until a lane moves it.

## Consequences

- The plugin writes the owner's files itself, outside the host's job engine: its undo entries come from its answer, not from the engine, until the host serves `fs/stage` and `fs/commit` (a contract request).
- Spike ES3 (Friday, November 6, 2026) ran every gpg flow in batch and loopback with throwaway homes and keys made in batch mode, through the tools registry's own runner: Git for Windows' gpg 2.4.9 (found "in Git for Windows"; Gpg4win is not installed here, and its folders are looked in first) and Debian's gnupg 2.4.7 in the tools image. No window: 165 gpg, gpg-agent, gpgconf and scdaemon processes were watched every 30 ms and none owned a window; no pinentry ever started; no dirmngr started. The status words were the same on both. Four findings shape the driver:
  - gpg 2.4 prints no `NEED_PASSPHRASE` when decrypting with a key: the first step's failure is `ENC_TO KEYID`, `NO_SECKEY` and `ERROR pkdecrypt_failed` with code 85 ("no pinentry"), so the key id for "Passphrase for key ending 4E7B" comes from `ENC_TO`, and a code of 11 is a wrong passphrase.
  - The one runner stops a program's whole tree when it ends, gpg-agent included on Windows (a job object), so a passphrase is never cached between two of diroxide's runs, and each later run that needs the agent waited about 6.4 seconds for it to start again (0.1 to 0.3 seconds when the agent was started outside the runner). On Linux the agent leaves the tree and survives. A way to start `gpgconf --launch gpg-agent` outside the run's job is a contract request.
  - Git for Windows' gpg is an MSYS program: a Windows path in `--homedir` is taken as relative (it read `/d/diroxide/D:\...`), while file arguments with Windows paths work. diroxide passes no `--homedir`, so the owner's own home is used as it is.
  - `--output` to a name that exists fails with "File exists" and code 33587235 when `--yes` is left out, as the driver wants.
- The tools image (answer 118) holds gnupg, gpgv, age and minisign for the container's checks; nothing is installed on the owner's computer, and macOS was not tried.

## See also

- ADR-0009, secrets and the masked prompt; ADR-0011, the keyring and stored secrets (unchanged: nothing here is stored); ADR-0006, the plugin protocol.
- `research/encryption.md` (R9), `research/disk-git-and-file-commands.md` part 3 (R29), `research/alpha4-tools-plan.md` (R30) commits 71 to 73 and spikes ES1 to ES3.
- `plugins/diro-crypt`, and `crates/diroxide-ops/src/crypt` for gpg, minisign and the chain.
