# Changelog

Notable changes to diroxide, newest first. Agents write under a scratch heading with their id; the orchestrator regroups the entries by area before each alpha. Versions follow the alpha series in the plan; nothing is released until the owner says so.

## 0.1.0-alpha.1

### P0b, the Wave 1 contract (scratch)

- The contract the Wave 1 builders build on: entries, names, places, the list model, keys, sorting, patterns, secrets, detail rows, the job vocabulary and the message catalog in core; the provider contract with the trash gate and its post-check; the engine's conflicts, errors, progress words, reconciliation, undo of created entries and the fake provider.
- Themes: textweaver's theme crate, ported as `diroxide-theme`, with the file manager's roles and the contrast verdict in words.
- The plugin boundary: the SFTP provider moves to `plugins/diro-sftp`, the in-process Lua tier is gone, and `cargo xtask native --check` keeps C out of the main workspace but for the downloader's TLS provider behind the `fetch` feature.
- Dependencies for the alphas to come, the container's archivers, curl and wget, and ADR-0009 on secrets and the masked prompt.
