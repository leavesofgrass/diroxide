# ADR-0006: The plugin protocol

Status: Accepted, Saturday, October 10, 2026 (Phase 0, P0). To be reviewed against R3's report (`research/plugin-protocol.md`) before Wave 3.

## Context

The owner wants to automate file management in bash and Python first, then Lua, and to reach other protocols such as SSH. Worker's scripting is a stack language and a flag language inside its configuration, plus a small Lua hook. diroxide makes plugins a protocol, not a language.

## Decision

- **Process plugins.** A plugin is a folder with `plugin.toml` and a program in any language. The host talks JSON-RPC 2.0 to it over standard input and output, one JSON object per line, UTF-8, both ways. `diroxide_plugins::protocol` has the request, notification and response types, the standard error codes, the method names, and a parser that tells the three apart and refuses anything else with words.
- **The manifest** names the plugin (plain characters only), its version, its program as an argument list, its capabilities, its commands (with ids checked as command ids), its providers, previewers, converters and hooks. Capabilities are said in words on first enable ("This plugin can read, write and delete files; run external tools."). Nothing is enabled by default.
- **The host** finds plugins, starts them on demand, routes calls, and serves their requests through `HostServices`, which the app implements over its own dialogs, panels and job engine, so a plugin's file operations get the same listing, trash, progress and undo as built-in ones. A crash or a timeout is reported and the app goes on; `--no-plugins` starts clean.
- **The transport** is a trait, `PluginTransport`: lines in, lines out. Today it is a process's standard input and output (`StdioTransport`), with a reader thread so a receive can time out. A WebAssembly component can be a second transport later without changing the API.
- **Script commands.** A one-file script with `diro:` header lines in its top comments becomes a command. It runs with `DIRO_ACTIVE`, `DIRO_OTHER`, `DIRO_CURSOR` and `DIRO_SELECTION_FILE` in its environment and the context as JSON on standard input; output lines starting `diro:` are directives (announce, refresh, cd, select, progress N of M). Placeholders (`{file}`, `{files}`, `{dir}`, `{other}`, `{dest}`) expand into an argument list, never a shell string.
- **Lua** is an in-process tier behind the `lua` feature, with the same API names. Phase 0's feature compiles an empty tier; Wave 3 adds `mlua` with a vendored Lua 5.4.
- **`diro serve --stdio`** speaks the same protocol, so scripts and tests can drive the app core.

## Alternatives

- **Embedded Python (PyO3).** Ties the binary to one Python version and makes a plugin crash a diroxide crash.
- **Lua only.** Fast, but not the languages the owner automates in.
- **Content-Length framing, as LSP does.** Robust, but harder to write by hand in bash; one object per line is enough because JSON encoders escape newlines inside strings.
- **WebAssembly first.** Sandboxed, but the owner's scripts are bash and Python; WebAssembly stays a later transport.

## Fallback

If line framing proves fragile with some plugin language, the transport gains Content-Length framing as an option declared in the manifest.

## Consequences

- Phase 0 has the protocol types, the manifest, the script header and directive parsers, and the shapes of the host and transport; calling a plugin says "not built yet".
- W3a owns the host and protocol, W3b the script commands, Lua, the examples and `diro plugin`.

## See also

- [The plan, section 5.8](../plan.md)
- [ADR-0008: Safety rules](0008-safety-rules.md)
