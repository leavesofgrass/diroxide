# ADR-0006: The plugin protocol

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0). Updated the same day with the wire conventions, error codes and shared shapes of R3's review (`research/plugin-protocol.md`, sections 2 and 7, in the planning repository); the full message list of its section 3 is W3a's.

Status update, Friday, October 16, 2026 (the Wave 1 contract, P0b): the in-process Lua tier is retired (the owner, Sunday, October 11, 2026; plan section 5.8, item 3). `lua.rs` and the `lua` feature are gone from `diroxide-plugins`, and Lua scripts run through a Lua runner plugin, a process like any other: the plugin boundary is the language boundary (ADR-0001). The wire entry now carries `extra` both ways and knows the `item` kind.

## Context

The owner wants to automate file management in bash and Python first, then Lua, and to reach other protocols such as SSH. Worker's scripting is a stack language and a flag language inside its configuration, plus a small Lua hook. diroxide makes plugins a protocol, not a language.

## Decision

- **Process plugins over newline-delimited JSON-RPC on standard input and output.** A plugin is a folder with `plugin.toml` and a program in any language. The host talks JSON-RPC 2.0 to it, one message per line, UTF-8 without a byte order mark, both ways. The host ends lines with `\n` and accepts `\r\n`, because Python on Windows writes it. The longest line is 8 MiB by default, 64 MiB at most. Batches are refused with -32600. Standard error is free text for the plugin's log.
- **Stray output is tolerated.** A line that does not start with `{` is logged as stray output and not answered, so a forgotten `print` costs one line instead of the whole stream; a line that starts with `{` and does not parse is answered with -32700 and a null id.
- **Envelopes.** Requests have an integer or string id; notifications have none; a null id appears only in the parse-error answer. `params` is always an object, absent meaning `{}`. A response has `result` or `error`, never both. Unknown fields are ignored; an unknown request is answered -32601 and an unknown notification ignored. `initialize` carries the protocol's major version, 1 for the alpha series; within it only additions are made, gated by capabilities that both sides declare.
- **`snake_case` everywhere.** Field names in JSON and TOML, and methods as `area/verb_noun`: `provider/create_dir`, `provider/write_begin`, `ui/announce`, `$/cancel_request`, `$/progress`. Names starting `rpc.` are never used. The constants are in `diroxide_plugins::protocol::methods`.
- **Error codes.** JSON-RPC's five; -32002 `not_initialized` and -32800 `request_cancelled` borrowed from LSP; and diroxide's range: -32010 to -32022 for provider outcomes (`not_found`, `already_exists`, `permission_denied`, `not_a_directory`, `is_a_directory`, `not_empty`, `invalid_name`, `unsupported`, `unavailable`, `auth_required`, `no_space`, `too_large`, `conflict`), -32030 to -32034 for the host's own refusals (`denied_by_host`, `limit_exceeded`, `reentrant`, `plugin_unavailable`, `timed_out`), and -32099 `failed`. `error.message` is the cause only, meaning first; `error.data` carries `kind` (the name) and optionally `place`, `hint`, `detail` and `retry_after_ms`. The host builds the user's "Could not VERB OBJECT: CAUSE. NEXT STEP." line from them. All of this is `diroxide_plugins::protocol::ErrorKind`.
- **Shared shapes** (`diroxide_plugins::shapes`). A place is its text. An entry has `name` and `kind` (`folder`, `file`, `link`, `archive`, `special`) and optionally `place`, `size`, `modified` (RFC 3339 in UTC), `mode`, `attributes` (words such as `read_only`), `owner`, `link_target`, `mime`, `revision` and `extra`; a name that is not valid Unicode travels with a `_raw` sibling in base64. The command context names the protocol, the command, its arguments, the locale, both panels with their providers, the cursor and the selection (falling back to the cursor, cut at 10,000 with a flag). Importance travels as `status`, `info`, `warning` or `error`, mapped onto the core's importance and kinds. The host computes display names; a plugin never replaces characters.
- **The manifest** names the plugin (plain characters only), its version, its program as an argument list, its capabilities, its commands (with ids checked as command ids), its providers, previewers, converters and hooks. Capabilities are said in words on first enable. Nothing is enabled by default.
- **The host** finds plugins, starts them on demand, routes calls, and serves their requests through `HostServices`, which the app implements over its own dialogs, panels and job engine, so a plugin's file operations get the same listing, trash, progress and undo as built-in ones. A crash or a timeout is reported and the app goes on; `--no-plugins` starts clean.
- **Plugin providers.** A `PluginProvider` in this crate will implement the provider trait of ADR-0003 over the `provider/*` methods (connect, list in batches, ranged reads, write sessions that commit or leave nothing, copy hints, watching), keeping the async runtime inside the plugin host (R3, section 7; W3a).
- **The transport is a trait,** `PluginTransport`: lines in, lines out. Today it is a process's standard input and output (`StdioTransport`), with a reader thread so a receive can time out.
- **Script commands.** A one-file script with `diro:` header lines becomes a command. It runs with `DIRO_ACTIVE`, `DIRO_OTHER`, `DIRO_CURSOR` and `DIRO_SELECTION_FILE` in its environment and the command context as JSON on standard input; output lines starting `diro:` are directives. Placeholders expand into an argument list, never a shell string.
- **Lua** is an in-process tier behind the `lua` feature, with the same API names. Phase 0's feature compiles an empty tier; Wave 3 adds `mlua` with a vendored Lua 5.4.
- **`diro serve --stdio`** speaks the same protocol, so scripts and tests can drive the app core.

## Alternatives

- **Content-Length framing, as LSP and DAP do.** Robust for binary-safe streams, but a stray `print` desynchronizes it for good, and it is harder to write by hand in bash; JSON encoders escape line breaks, so one message per line is enough.
- **camelCase names, as LSP and MCP use.** Familiar to some authors, but no library of theirs is used here, serde needs no renames for `snake_case`, and the project's TOML is already `snake_case`.
- **Embedded Python (PyO3).** Ties the binary to one Python version and makes a plugin crash a diroxide crash.
- **Lua only.** Fast, but not the languages the owner automates in.
- **WebAssembly first.** Sandboxed, but the owner's scripts are bash and Python today.

## Fallback

WebAssembly, through the transport trait. If process plugins prove too slow to start, too hard to distribute, or too open (they run with the user's rights), a component on wasmtime, or an Extism plugin, becomes a second `PluginTransport` carrying the same messages and shapes; the plugin API, the manifest and the host do not change, and the sandbox then enforces the manifest's capabilities instead of trusting them.

## Consequences

- Phase 0 has the envelopes and their parser, the error kinds and codes, the method names, the shared shapes with their conversions to the core's types, the manifest, the script header and directive parsers, and the shapes of the host and the transport; calling a plugin says "not built yet".
- W3a owns the host, the protocol, the full message list, the plugin provider and the mapping of error kinds onto the core's errors; W3b owns the script commands, Lua, the examples and `diro plugin`.

## See also

- [The plan, section 5.8](../plan.md)
- [ADR-0003: The provider trait](0003-the-provider-trait.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)
