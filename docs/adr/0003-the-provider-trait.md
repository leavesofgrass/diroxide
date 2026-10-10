# ADR-0003: The provider trait

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

## Context

diroxide shows local folders, archives, SFTP hosts and whatever plugins serve, through the same panels and the same job engine. Each kind of place needs one interface that the engine and the app can use without knowing which kind it is.

## Decision

- `diroxide_vfs::Provider` is a synchronous, object-safe trait: `scheme`, `capabilities`, `list`, `stat`, `read`, `write`, `create_dir`, `rename`, `remove`, `trash`, `set_attributes`, `copy_hint`, `copy_within` and `watch`. Every method blocks and is called off the input thread.
- Every operation takes an `OpContext`: a `CancelToken` checked between steps, and a `ProgressSink` told how far it has got.
- An operation acts on one entry. The job engine does the walking, the listing before a delete, and the conflict questions, so every provider gets them the same way.
- `write` takes a `WriteMode`; the default, `CreateNew`, fails when the name is taken.
- `trash` is a provider method with a default of "unsupported"; the local provider implements it with the `trash` crate (W1a), and `Capabilities::trash` says whether it works. The plan's "delete goes to the trash through the trash crate" holds, through the provider rather than around it.
- `copy_hint` says whether a fast path exists (reflink, server-side copy); `copy_within` takes it. `watch` is optional and returns a handle that stops watching when dropped.
- `Capabilities` lists what a place allows (writable, trash, links, attributes, case sensitivity, the longest name, reserved device names, watching), so commands that cannot work are listed as unavailable with the reason in words.
- The `Registry` resolves local places to the local provider, places inside archives to the archive provider, and remote places by scheme. The archive provider reaches archive files through the registry, held weakly so the registry can hold the provider.
- Errors are `VfsError`, which implements `Explain`; I/O errors map to words ("it does not exist", "permission was denied").
- Platform details of the local provider live in its `sys` module, the one module where `unsafe` may be allowed.

## Alternatives

- **An async trait.** It fits SFTP, but async traits are not object-safe without boxing every future, and the app core is synchronous. The SFTP provider runs its own runtime inside and blocks.
- **Streaming listings.** `list` returns the whole folder; a provider that is slow reports progress through the sink. A batch callback can be added beside it if 100,000-entry folders on slow shares need it.
- **Recursive operations in providers.** Simpler for one provider, but then each would need its own listing, conflicts and undo.

## Fallback

If a provider needs streaming or async work, it gets an extension trait beside `Provider`, so the trait the engine uses does not change.

## Consequences

- Phase 0's local provider is `std::fs` only; W1a adds attributes, long paths, links, owners, the drive list, the trash and watching.
- The archive and SFTP providers exist with every method saying "not built yet", so the registry and the app can be wired before them.

## See also

- [ADR-0004: The job engine](0004-the-job-engine.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)
