# ADR-0003: The provider trait

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Friday, October 16, 2026 (the Wave 1 contract, P0b, from R7, R8 C1 to C8, R9, R10, R11 9.1 to 9.4, R12, R22, R23 and spikes three, four and six):

- **Writing.** `write` returns a committing `FileWriter`: the bytes go to an unfinished `.diro-part-` name beside the target, `commit` makes them its content, and a drop leaves the target as it was.
- **New defaulted methods.** `open_seekable` (a `ReadSeek`); `create_link` (a `LinkRequest`: symbolic, junction or hard); `restore`; `space` (`SpaceInfo`); `volume` (`VolumeInfo` with a `VolumeKind`: fixed, removable, network or unknown, asked of the path's own volume root); `trash_gate`; `as_streaming`, which finds `StreamingList`, this record's fallback, declared now.
- **Changed methods.** `set_attributes` returns the inverse change; `AttributeChange` gains `created`, `expect` and owners as a `Principal`. `trash` returns a `TrashCheck`, behind the gate of ADR-0008. `rename` never replaces, lets a case-only or form-only change of the same entry through, and fails across volumes with `CrossesDevices`. `copy_within` is the clone tried before the destination is touched.
- **Capabilities** is `non_exhaustive` and gains `restore`, an `access` block (`Can`: no, with an administrator, yes), `normalization_insensitive`, `random_read` and `clone_copy`, a capability distinct from `writable`.
- **Context and errors.** `OpContext` carries a `SecretSource` (ADR-0009). `VfsError` gains R8's classes, R7's and R9's data and password errors, and `TrashRefused`, with next steps that name no key; `class()` gives the retry rules and a scope (file, rule, environment). `WatchEvent::Gone` says the shown folder went away.
- **Names.** The superscript device names, dot names, the length check by a place's `NameRules`, refused or only flagged, `numbered_name` with compound extensions, `unused_name`, and the shared unfinished-name functions.

Status update, Friday, October 16, 2026 (the alpha.2 contract step, P2, with W1b's two held requests):

- **The writer's unfinished place.** `FileWriter` gains a defaulted `unfinished_place`, `None` unless the writer says; the local writer answers with its `.diro-part-` file beside the target, so the journal can tie the name to its target.
- **Links in words.** `Capabilities` gains `can_link` (links held and the place writable) and `links_in_words` ("Links can be made here", on Windows with "symbolic links may need Developer Mode", or "Cannot link here: this place holds no links"). The local provider no longer claims links on FAT and exFAT under Windows. The engine refuses a link job where the destination cannot link, before anything is touched.
- **The trash that copies.** `TrashFacts` gains `copies_across_drives`: on Linux a drive whose own trash cannot be made sends its items to the home trash by copying them. The gate answers the new `TrashGate::Copies`, which `allows()`, with `asks_first(bytes)` above `TRASH_COPY_ASK_BYTES` (64 MB) and `copy_question`. The rule is the pure `copies_across_drives`; the local provider fills the fact on the freedesktop systems and says false elsewhere. ADR-0008's gate is otherwise unchanged; the question is wired into the trash plan by the engine's next brief.

Status update, Saturday, October 17, 2026 (the pre-wiring contract step, P2b, from the requests pairs 1 to 3 held at their integrations):

- **Read only here.** `VfsError::ReadOnlyHere { what, instead }` (built with `read_only_here`) is the refusal of a place this program only reads, with its own class, `ErrorClass::ReadOnlyHere` (no retry, a rule's scope, apart from a read-only drive), and its own next step, the `instead` sentence. `VfsError::sentence` says it whole: "Archives are read only here. Use Pack to make a new one." Every change inside an archive gives it, and the archive crate's `read_only_refusal` reads it (W2a's request 1).
- **Renaming over a link in one step.** `Provider::rename_replacing`, defaulted to unsupported, renames a file or link over the file or link a name holds, so the name always answers with the old entry or the new one; a folder is never replaced (`AlreadyExists`), never moved over a name, and on any failure both names are as they were. `Capabilities::replace_rename` says where it works. The local provider implements it through the `sys` module: `rename(2)` on Unix, `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` alone on Windows (no copy across volumes; the system refuses a folder link, and the engine falls back). Plain `rename` still never replaces (W2c's request 2).
- **A writer that seeks.** `FileWriter::seek_to(SeekFrom)`, defaulted to unsupported, moves where the next bytes go in the unfinished file and returns the position, so a format that writes its header last is written straight into the committing writer; nothing reaches the target before the commit either way. The local writer seeks in its `.diro-part-` file. The archive crate's 7z writer goes straight into a writer that seeks, with no body held in memory or in a body file and no copy; a writer that cannot seek still gets the old way. The fake provider gains `seekable_writes`, off by default, and counts seeks (W2b's request 1).

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
