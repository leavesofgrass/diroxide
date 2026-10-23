# ADR-0010: The optional components

Status: Accepted, Friday, October 23, 2026 (alpha 2, Wave 4, lane W4c, from research/alpha4-plan.md, R22, section 6, and the owner's answers 40, 44 and 46 in plan section 13).

## Context

Some of what diroxide offers cannot ship inside the one `diro` file: helper programs with their own licenses, plugin binaries built with crates the default build leaves out, and data packs such as the example plugins. Plan section 5.14 asks for one shared downloader that asks first, verifies every file against a pin, resumes, and installs from a file as well as from the network, with a private source the owner names only in his own settings. R22 found that the main binary could not link textweaver's HTTP client without a C build dependency, which the pure-Rust rule forbids; the owner then allowed one exception (answer 40) and approved curl and wget as fallbacks (answer 44).

## Decision

- **One crate, `diroxide-components`,** holds the pins, the list the binary embeds, the fetchers, the downloader, unpacking and install from a file, and the spoken lines. `diro components` (list, download, verify, remove, install) and the app's screens (Manage optional components, Get example plugins, the first-run list) are thin over it.
- **The transport is the built-in client first,** `ureq` with rustls behind the binary's default `fetch` feature, the one exception to the plugin boundary; then `curl`, then `wget`, found on the PATH through the tools registry and run with an argument array and `-q` first, so no configuration file of theirs is read. A token for the owner's private source reaches the built-in client as a header and curl on standard input, never a command line, and never wget.
- **The manifest the binary embeds is the authority.** `cargo xtask components` builds the packs this repository makes, writes them as a folder source under the target folder, and regenerates the embedded list from the built files; `regen --check` fails when the list is stale. No network refresh of the list in the alpha series (R22's question 2, its default). A source's own list may add components, never change a pin.
- **The actions:** nothing is fetched before the owner's yes to a line that names the size and the license; Escape stops and the next try resumes; a file whose size or SHA-256 differs from its pin is refused and removed; Remove lists each path first and removes only the component's own files; install from a zip or a folder always works, and a hostile archive writes nothing outside the component's folder.
- **The private source** is named only in the owner's own settings (`components_source`), never in code, tests, docs, CI or any public file. A "no" to a download is remembered for the session.

## Alternatives

- The system curl alone, as R22 first recommended: it leaves a machine without curl unable to fetch, and the owner chose the built-in client with curl and wget behind it (answer 40).
- A list fetched from the network and verified by a signature: it needs the owner to hold a signing key; deferred past the alpha series.
- Components inside the binary: they would carry licenses and sizes the one file must not.

## Fallback

If the built-in client's TLS stack keeps drawing advisories, the `fetch` feature is turned off by default and curl then wget carry every fetch; the pins, not the transport, guard integrity, so nothing else changes.

## Consequences

- The binary grows by the client and its roots; a build with `--no-default-features` leaves them out and still installs from a file.
- Every component's files are pinned, so a changed upstream file fails the build of the list rather than the owner's download.
- The release workflow must publish the example pack under its own tag, since the program derives the public address from the repository, that tag and the file name.

## See also

- [Optional components](../components.md), the guide.
- [ADR-0006: The plugin protocol](0006-the-plugin-protocol.md), since plugins are the first components.
- [ADR-0008: Safety rules](0008-safety-rules.md).
