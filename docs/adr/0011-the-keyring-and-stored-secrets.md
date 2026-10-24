# ADR-0011: The keyring and stored secrets

Status: Accepted, Friday, October 23, 2026 (alpha 2, Wave 4: lane S-C's draft, accepted by the orchestrator at the close of stage 2; the owner's answer 83).

## Context

ADR-0009 said nothing is remembered on disk and left a system keyring "until after the final alpha, and only if the owner asks". The owner asked: answer 83 (Friday, October 23, 2026) gives each SFTP site three secret modes, never (the default), this session only (in memory) and the system keyring (Windows Credential Manager, the macOS keychain, the Secret Service on Linux), and asks for an ADR under answer 47 that names the keyring crates and the reason. Answer 47 says a young or unaudited Rust crate never guards a password. R24 (section 3.4 and question Q5) found the keyring crates to be 1.x releases less than a year old, and argued that the operating system's store does the guarding while the crate only calls it.

The SFTP plugin also meets a second risk R24 and S-A named: ssh asks for its own password through the askpass helper with a prompt such as `alice@build.example's password: `, and a server's keyboard-interactive question can be worded the same way. A stored password given to the wrong prompt goes to the wrong server.

## Decision

- **Three modes per site, never by default.** `sites.toml` holds the mode word (`never`, `session`, `keyring`) and never the secret. A file in which any table holds a key that names a password, a passphrase or a token, or a `secret` that is not a mode word, is refused whole and said ("Sites file refused: it holds a password."), and nothing is written over it.
- **This session** keeps a password the owner typed in the plugin's memory, as the core's `Secret` (wiped on drop, `Debug` hidden). It is forgotten when the plugin closes, on Forget password, when the server refuses it, and when the owner's own Disconnect closes the last connection. A lease that ends (the panel moved away) does not forget it.
- **The system keyring is opt-in, asked only after a successful login.** For a site in keyring mode, a password the owner typed for ssh's own prompt, and that ssh then accepted, brings the question "Remember this password? y or n", which starts on No. A yes saves it under service `diroxide-sftp`, account `site:ID:user@host`, and says "Saved in the system keyring, not a file." A machine with no store says "No key store here; nothing saved." and keeps it for this session only. Forget password asks first, starting on No, and removes it from both places; deleting a site removes it too. Key passphrases are never stored: the site's `add_key_to_agent` hands an unlocked key to the agent instead.
- **A stored secret is released only for the exact prompt of its own connection.** It is offered to the connection before ssh starts and given to the askpass helper at most once, only for ssh's own password prompt whose user and host equal the connection's `ssh -G` answer. When ssh asks again in the same session, the password it was given was refused: it is forgotten, "Saved password refused; asking you." is said, and the owner is asked. **Through a jump host or a proxy command nothing is filled in or remembered at all**, because another server on the road could word its question as ssh's own prompt for the target. Below OpenSSH 8.4 the helper is never used (answer 85), and from 8.4 ssh prefixes every keyboard-interactive question with `(user@host)` (captured on OpenSSH 10.0p2 in the development container: `(builder@127.0.0.1) Password:`), so the exact match cannot be met by a server's question on a direct connection.
- **The crates.** `keyring-core` 1.0.0 (MIT OR Apache-2.0) defines the store interface and calls nothing itself. One store crate per system, chosen by target: `windows-native-keyring-store` 1.1.0 (MIT OR Apache-2.0, on `windows-sys`, Windows Credential Manager), `apple-native-keyring-store` 1.0.2 with its `keychain` feature (MIT OR Apache-2.0, on `security-framework`, the macOS keychain), and `zbus-secret-service-keyring-store` 1.0.1 with `crypto-rust` (MIT OR Apache-2.0, on `zbus` and `secret-service` with RustCrypto's AES, CBC, HKDF and SHA-2 for the session's encrypted transfer, the Secret Service on Linux). All are pure Rust: `cargo tree -i cc --target all` over the plugin's whole graph finds no `cc`, `cmake` or `cxx`. They live only in the SFTP plugin, which is its own workspace, never in the main binary.
- **Why these satisfy answer 47.** The secret is guarded by the operating system's store, which encrypts it at rest and ties it to the owner's account; the crates are thin bindings that hand bytes to it and read them back. No crate here implements cryptography that protects the stored password. The one place where Rust crypto runs is the Secret Service transfer on Linux, a session encryption between the plugin and the store on the same machine, from the RustCrypto project.
- **Tests never touch the owner's keyring.** The plugin is built with a keyring in memory for every test; the one round trip through the real store (spike SF9) is ignored by default with its reason in words and runs only when the owner says so.

## Alternatives

- **The `keyring` 4.2.0 umbrella crate.** It bundles the same stores plus others (a file-backed store, Linux kernel keyutils, a CLI) behind features. Depending on `keyring-core` and the three stores directly keeps the graph small and every store named here.
- **A password in `sites.toml`, encoded or encrypted with a master password.** FileZilla's history shows where that goes: a file a backup or a sync copies. Refused; the file holds no secret, and the check refuses a file that does.
- **libsecret or the macOS Security framework through C bindings.** Mature, and a C build in a plugin that is pure Rust today; not needed while the pure-Rust stores call the same system services.
- **Filling a stored password for any prompt that names the right account.** Simple, and the road by which a jump host learns the target's password. Refused for every connection through a proxy.

## Fallback

If a store crate fails on a system (no Secret Service on a headless Linux, a locked keychain), the plugin says so in words and keeps the password for this session only; the site works with the owner typing it. If a store crate proves unfit, the keyring mode is turned off for that system and the two other modes remain; nothing else in the plugin depends on it. Windows Credential Manager entries can be read by other programs of the same user, and are limited in size; the guide says so.

## Consequences

- ADR-0009's "No keyring in the alpha series" is superseded for the SFTP plugin by this record once accepted; every other secret (archives, encrypted files) stays as ADR-0009 says.
- `Memory` in the plugin's connect module gains the offered password and the "asked again means refused" rule, which also stops a wrong typed password from being replayed for ssh's second and third prompt.
- The plugin's manifest declares the Forget password command, and the Sites list binds it to Alt+F.
- The owner's listening check of stage 2 includes the remember question and Forget password (R25 section 6, stage 2, step 10).

## See also

- [ADR-0008: Safety rules](0008-safety-rules.md)
- [ADR-0009: Secrets and the masked prompt](0009-secrets-and-the-masked-prompt.md)
- The plan, answers 47, 83, 85 and 94
