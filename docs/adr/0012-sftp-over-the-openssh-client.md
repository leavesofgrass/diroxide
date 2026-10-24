# ADR-0012: SFTP over the OpenSSH client, and the askpass helper

Status: Accepted, Friday, October 23, 2026 (alpha 2, Wave 4, lanes S-A, S-B and S-C, from research/sftp-openssh.md, R24, and the owner's answers 78, 82, 84, 85, 86, 87, 88, 94, 95, 100 and 101 in plan section 13).

## Context

The plan's first design for SFTP was a provider on russh with a vetted cryptography provider inside a plugin binary, since the main binary is pure Rust and holds no cryptography (ADR-0001, answer 47). The owner's own ssh setup, keys, agent, known hosts and configuration already exist on every machine he uses, and R24 found that the OpenSSH client can carry the whole SSH side: `ssh -s sftp` opens the SFTP subsystem over the user's own connection, and since OpenSSH 8.4 `SSH_ASKPASS_REQUIRE=force` makes ssh ask every question, a password or a new host key, through a helper program even with no console window. The Windows port honors it; spike SF1 proved it on the in-box OpenSSH 9.5p2 and on Git's 10.3.

## Decision

- **The SFTP provider is the plugin `diro-sftp`, in its own workspace, pure Rust, and it drives the machine's OpenSSH client.** No key, host key or cipher is ever diroxide's. The owner's agent, key files, known hosts and ssh configuration work as they are. A self-contained plugin on russh may come later as an optional component for machines without OpenSSH.
- **The plugin speaks SFTP version 3 itself,** with its own codec and request multiplexer (names as bytes, 64 requests of 32 KiB in flight, a fuzz target), and uses OpenSSH's extensions where the server advertises them: `posix-rename` for atomic replace, `statvfs` for free space, `limits`, `fsync` before a write session commits, `copy-data` for server-side copy, `expand-path` for a home.
- **ssh is found, not configured:** the highest version on the PATH or in the known places wins, the Windows flavor on a tie, Say Status names it, and the `ssh_program` setting overrides. Below 8.4 the plugin works with keys and the agent only and says so, with the line that updates ssh; no console window ever flashes. The plugin reads only the `Host` names (and `Include` files) of the user's ssh configuration, and asks `ssh -G` for everything else. `ControlMaster` is off on Windows, where ssh has no connection sharing, and left alone on Unix.
- **The askpass helper is the plugin's own executable in a second mode,** chosen by the environment, since `SSH_ASKPASS` cannot carry arguments. It proves itself to the plugin with a per-connection token and a two-way challenge, over a user-only socket on Unix and, on Windows, an owner-only named pipe (an owner-only security descriptor, the first-instance flag, remote clients refused) with the proof kept as a second layer. The plugin asks the owner through the app's own prompts (`ui/ask`): a new host key as hex in groups of four, confirmed on No; a changed host key refused, with the `ssh-keygen -R` line shown and never run; a password named for its account and host.
- **Secrets:** a password lives nowhere by default; per site the owner may keep it for the session, in memory, or in the system keyring as an opt-in (ADR-0011). A stored password is given to ssh at most once per connection and only for ssh's own password prompt for that exact account, never for a jump host's question. The plugin never writes known hosts, never passes a password on a command line, never logs a secret, never accepts a changed key.
- **Sessions and operations:** up to two ssh sessions per host, the second for transfers, reusing the typed secret from memory; delete over SFTP is permanent, behind the engine's full listing and a question that starts on No; a remote move reads the destination back and hashes it before the source goes; a lost link sends `provider/lost` once, and one automatic reconnect in batch mode happens only for a session that needed no typed secret.

## Alternatives

- russh with a vetted provider inside the plugin (the first design): self-contained, but then diroxide holds keys and host keys of its own and a second trust store beside the owner's; kept as the later component.
- libssh2 inside the plugin: a C build in the plugin, and the same second trust store.
- An existing SFTP crate for the packet layer: `openssh-sftp-client` types remote names as a path (the wrong type for a POSIX path on Windows) and its OpenSSH feature is Unix only; `russh-sftp` types names as `String`. Names are bytes, so the codec is ours.
- A loopback endpoint with the proof alone on Windows (S-A's first build): any local process could connect; the owner chose the named pipe (answer 101).

## Fallback

If a machine's ssh cannot be driven this way, the plugin says so in words and the later russh component takes over; nothing in the main binary changes.

## Consequences

- The riskiest code in the series, the network and host-key code, is not written at all; OpenSSH's own is used.
- SFTP works only where an OpenSSH client is installed, which is every current Windows, macOS and Linux.
- The plugin's tests run against a fake ssh and a fake server natively, and against OpenSSH's own server in the development container and on the Linux CI job (answer 99).

## See also

- The remote places guide, `docs/remote-places.md`, which lands with lane D2.
- [ADR-0011: The keyring and stored secrets](0011-the-keyring-and-stored-secrets.md).
- [ADR-0006: The plugin protocol](0006-the-plugin-protocol.md) and [ADR-0008: Safety rules](0008-safety-rules.md).
