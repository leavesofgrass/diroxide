# ADR-0002: The core model

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

Status update, Friday, October 16, 2026 (the Wave 1 contract, P0b, from R8 to R12, R19 to R23 and the owner's answers 29 to 44). Core gains these, and closes again:

- **Entries.** `Entry` gains `created`, `volume` and `extra` (a string map for plugin columns and item rows); `changed` means the status-change time, never the creation time. `EntryKind` gains `Item`, a row that is not a file; an unknown kind reads as `Special`. `Attributes` say themselves in words: `octal`, `words(kind)` and `short_words(kind)`; `describe` writes `s`, `S`, `t` and `T`; the Windows words gain sparse, compressed, offline, encrypted and online only.
- **Names and places.** `split_extension` is the one stem and extension rule, with compound extensions; `split_part` reads `.001` parts. `ARCHIVE_EXTENSIONS` is a slice; zip-based documents open as archives without being listed as archives; `!` after a numbered part starts a member path, so a split archive opens as one place; `join` inside an archive refuses an archive marker; `strip_extended_prefix` keeps the extended-length prefix out of anything shown or spoken.
- **The list model.** Marks travel by row id; `set_rows` returns `RowsReplaced`, which says whether the focus moved to another row and how many marks were lost; `freeze` gives the `FrozenSelection` a confirmation acts on, whose note says when the focused row is left out. This replaces "replacing the rows keeps the focus on the same id" above, which kept the old index when the id was gone.
- **Keys.** A binding is one key with its modifiers; there are no sequences (`CoreError::KeySequence`), and `KeyChord` is gone. `KeyColumn` names the keymap's terminal and window columns (ADR-0005).
- **New modules.** `sort`: every panel key, a direction per key, folders first, mixed or last, and one total-order comparer with the name in natural order last. `pattern`: words, globs, commas, `not`, a trailing slash for folders, and `re:`. `secret`: ADR-0009. `details`: `DetailRow` and `DetailGroup`. `catalog`: the safety sentences, among them "Deleted but not found in the trash."
- **Jobs.** `JobKind` gains `Split`, `Join`, `Download` and `PluginCommand`; an unknown kind reads as `Unknown`, never planned. `ConflictPolicy` gains `Newer`; `ConflictKind`, `ConflictAction`, `ConflictAnswer`, `JobState`, `ErrorAction` and `JobOrigin` arrive; `JobRequest` gains `renames` and `origin`, both defaulted so older requests still read.
- Core now depends on `regex` and `zeroize`, both pure Rust.

Status update, Friday, October 16, 2026 (the alpha.2 contract step, P2, from `research/alpha2-plan.md` part five, commits 2 to 9, and the items held from Wave 1). Core gains these, and closes again:

- **Jobs.** `JobKind` gains `TestArchive` ("test", "Testing"), and `JobKind::ALL` lists every kind; a word test walks it with a match that has no fallback arm, so a kind cannot arrive without its verb and progress word. `JobRequest` gains `parts: Option<PartSize>` (`Bytes` per part or `EqualParts`), defaulted and left out of the JSON when absent, with `split_into`, `part_bytes` and `parts_for`.
- **Places.** The `.001` rules P0b wrote are held by a property test (every archive extension, any case, three to five digits, both path styles) and by look-alikes that stay plain (`photo.jpg.001`, `a.7z.01`).
- **The command text form** moves from the app into core (W1c's request): `Display` for `Command`, `Command::parse_text`, `Command::PLACE_ARGUMENTS` and `CommandTextError`, whose sentences are the ones the app said; `diroxide_app::commands::{command_text, parse_command_text}` delegate.
- **Effects.** `Effect::AppRequests` (W1d-1's request): a runtime effect saying the app queued requests whose types the core cannot name; the app ends its effects with it, `Runtime::carry_out` takes the requests there, and a frontend never sees it. `is_for_frontend` is false for it.
- **The catalog** gains `trash_copies_across` ("Trashing copies 3.2 GB to another drive. Go on? y or n"), and `format` gains `grouped` for exact byte counts ("4,700,372,992").

Status update, Saturday, October 17, 2026 (the pre-wiring contract step, P2b, from the requests pairs 1 to 3 held at their integrations). Core gains these, and closes again:

- **Places.** `.gz` joins `ARCHIVE_EXTENSIONS`, so a one-member gzip's member place (`notes.txt.gz!notes.txt`) parses (W2a's request 2). A name opens as an archive only with a stem before its extension that is not empty and not only dots, so `.zip` and `..zip` are hidden files' names, not archives; the `!` that starts a member path is found left to right, each after the name since the last slash, backslash or split. A name holding an archive marker (`holds_archive_marker`, now public) is refused by `join` in every kind of place: inside an archive with `ArchiveMarker`, as before, and in a local or remote folder with the new `PlaceError::ReadsAsArchive` ("Rename it with another program first."), so every place's text reads back as itself; a property test holds it for joined names (W2b's fuzzing finding: the local name `.zip!.xz` read as a member of an archive called `.zip`). The archive crate's `has_archive_marker` is core's rule, and the walkers leave such a local file out by name, as they do any name a place cannot hold.
- **Attribute and link jobs in the request** (R11 9.5; W2c's request 3). A new module, `access`, holds what those jobs carry: `AttributeEdit` (the mode as `ModeChange`, Windows attributes as `WindowsChange`, the three times, the offset they are said in, and its `Recursion`), `LinkEdit` (a `LinkKind`, a `LinkForm`, and `retarget` for changing where one link points), and the four Windows attribute bits. They moved from the job engine and the provider trait, which re-export them under their old paths. `JobRequest` gains `attributes` (boxed, so the effect that starts a job stays small) and `link`, both defaulted and left out of the JSON when absent, with `changing`, `linking`, `recursion` and `link_edit`; so a permissions, times or link job travels whole in a script or the journal. The recursion travels inside the edit, where the engine already kept it, rather than as a second field. Each type has a text form (`+44-2` and `644/755` for a mode, `+rh-a` for Windows attributes, UTC times such as `2026-10-17T14:05:00Z`, offsets such as `-07:00`), and `AttributeEdit::add_to` and `from_command`, with `LinkEdit`'s pair, put them in a command's arguments and read them back, so the command text form round-trips them; a value of the wrong kind is the new `CommandTextError::BadValue`, which says what to write ("Not a value for recursion: all. Write this_item, files_and_folders, files_only or folders_only."). A property test holds the round trip.

## Context

Every crate speaks about places, entries, lists, commands, effects, messages, keys and time. If each defined its own, the frontends would read differently and the owner's screen readers would hear different words for the same thing. `diroxide-core` holds these types once, does no I/O, and is finished in Phase 0, so later waves build on it without changing it.

## Decision

- **Place.** A full location of one of three kinds: local (`D:\photos`, `/home/owner`), inside an archive (`D:\docs\course.zip!week1`), or remote (`sftp://user@host:22/path`). It is opaque, with accessors, so no crate can build an invalid one. Parsing is textual in one path style, the platform's by default: `D:\photos` is full on Windows and a file name on Linux. Relative paths, `C:` and `C:docs` (drive-relative), `\home` without a drive on Windows, and device paths are refused with errors that say what to type instead; `Place::resolve` reads typed relative input against a base. `!` starts a member path only after an archive extension, so names such as `wow!` stay names; the last such `!` splits, so nested archives work. `..` is resolved lexically, and a member path that climbs out of its archive is refused. A place is text in TOML and JSON and round-trips. `title()` says the name first ("week1, in course.zip"). `is_within` follows parents, into archives, ignoring case on Windows, so recursive work can stay inside its root.
- **Entry.** The real name is kept for operations; `display_name()` replaces control, format, direction and private-use characters with a question mark. Private-use characters are added to the plan's list because the character behind September 26 was one. `needs_care` marks names a row must flag. Names that are not valid Unicode are listed with a lossy copy and `name_not_unicode`, and operations on them are refused with a message. `Entry` is non-exhaustive, built with `Entry::new` and its fields.
- **The list model.** One `ListModel` for panels, results, menus, the palette and every chooser: a title, rows, a focus, F1 help and an empty text. A row is said label first, "selected" next when it is, then its details, then its position: "report.pdf, PDF, 2.1 MB, modified Tuesday, 3 of 40". Letters jump to the next row starting with them and wrap; moves stop at the ends; replacing the rows keeps the focus on the same id. Say Status reads the title, the count, the selection and the focus, in that order.
- **Commands and effects.** A command is a dotted lowercase id (`file.copy`) with named arguments. Dispatching returns `Effect`s, a non-exhaustive enum split into screen-and-speech effects for frontends and I/O effects for the runtime (ADR-0007). The job vocabulary (`JobKind`, `ConflictPolicy`, `JobRequest`) is in core so the app, the engine, plugins and `diro` share it.
- **Messages.** Text with an importance (Low: shown, said on Say Status; Normal: polite; High: assertive) and a kind. Errors read "Could not VERB OBJECT: CAUSE. NEXT STEP."; every error type implements `Explain`, whose display is the cause and whose `next_step` is the advice. Questions end "y or n".
- **Keys.** `Key`, `KeyChord` (one or two keys) and `Layer` are in core, so the configuration can read `keymap.toml` without the app. Written letters ignore case (`Ctrl+X` is Control and x) and Shift is written when meant; frontends build keys with `Key::new`, which turns an uppercase letter into the letter with Shift. Keys display as they are said: "Ctrl+Shift+T", "Page Down".
- **Time.** The `Clock` trait gives the time and the local offset; the app supplies the offset, and tests use `FixedClock`. Times are said relative to today on a 12-hour US clock: "today, 2:05 PM", "yesterday", "Tuesday", "October 3", "October 3, 2025". The calendar arithmetic is a few lines in core, not a crate.
- **Words.** `format` has sizes in decimal units ("2.1 MB", "1 byte"), counts ("1 item", "no items"), and the 40-cell Braille test `within_cells`.

## Alternatives

- **`PathBuf` or `OsString` inside `Place`.** Exact for every name, but platform-dependent text operations and no stable text form for configuration and JSON. Recorded as the fallback.
- **The `url` crate for places.** Good for remote places, wrong for Windows paths and archive members.
- **Public enum variants for `Place`.** Easier matching, but any crate could build a relative or drive-relative place.
- **Keys in the configuration crate.** Then frontends would depend on configuration to name a key.
- **A date crate in core.** jiff or time would do it, but the core needs only civil dates and an offset; the time zone lookup is the app's.

## Fallback

If names that are not valid Unicode prove common on Linux, `Place` and `Entry` move to byte-exact storage behind the same accessors, by a contract change request.

## Consequences

- Core has 50 unit tests and its own integration test; every later crate builds on its types.
- A change to core is a contract change request, resolved by the orchestrator at integration.

## See also

- [ADR-0007: Frontends over one core](0007-frontends-over-one-core.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)
