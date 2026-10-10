# ADR-0008: Safety rules

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0).

## Context

On Saturday, September 26, 2026, the owner lost most of a user profile to a delete that read a hidden character as a drive letter. A file manager's whole job is operations like that one, so the rules the owner keeps for agents become features of the product (plan section 5.12).

## Decision

- **List before touching.** Every delete, move and overwrite shows what it will touch first: `Plan::listing` writes every step, with hidden, control, private-use and direction characters spelled out as `[U+XXXX]` (`diroxide_core::names::spell_out`), and the count in words.
- **Confirmations start on No.** `Dialog::Confirm` holds the question and the listing; questions end "y or n"; Escape answers No.
- **The trash is the default.** Delete and F8 move to the trash; deleting for good is its own command (`file.delete-permanently`, Shift+Delete) with its own listing.
- **Paths are literal and full.** `Place` refuses relative paths, drive-relative forms such as `C:` and `C:docs`, a leading slash without a drive on Windows, and device paths; the app resolves typed input to a full place and shows it before acting. Arguments to `diro` that are not valid Unicode are refused, never changed.
- **No shell, anywhere.** Built-in operations use providers and `std::fs`; tools run with argument lists (`diroxide_tools`), with no console window; plugins and script commands get argument lists, and placeholders expand into separate arguments.
- **Dangerous names are flagged.** `diroxide_vfs::names::check_name` finds empty names, hidden characters, reserved characters, trailing dots and spaces, leading spaces, Windows device names, shell characters and names that differ only by case from a sibling; rows say "name needs care".
- **Boundaries hold.** `Place::is_within` keeps recursive work inside its root; archive member names that climb out of the archive are refused (`normalize_member_name`), archives nest at most four deep, and the archive limits cap sizes and counts.
- **Undo.** Renames and moves are recorded and replayed backwards; a trashed entry is restored from the trash.
- **`unsafe` stays in one module.** Only the local provider's `sys` module may allow it, and every block there says why it is sound.
- **Tests never touch real data.** Phase 0's tests only read the repository's own files; tests that write use a folder under the target directory, and tests never touch the real trash or the real config folder.

## Alternatives

- **Confirmations that start on Yes.** Faster, and exactly how an unchecked delete happens.
- **Permanent delete as a modifier of trash.** One command fewer, but a single key would then decide between recoverable and not.

## Fallback

None: these rules are not weakened. If one blocks a real need, the owner decides, and the change gets its own record.

## Consequences

- Phase 0 has the listing, the spelled-out names, the refused places, the name checks and the member rules; nothing in Phase 0 deletes, moves or overwrites a file. W1b and W1d build the confirmation flow and the trash on these pieces.

## See also

- [The clean-room rule](../dev/clean-room.md)
- [ADR-0003: The provider trait](0003-the-provider-trait.md)
- [ADR-0004: The job engine](0004-the-job-engine.md)
