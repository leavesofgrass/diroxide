//! Method names: `area/verb_noun`, in `snake_case`. Names starting `rpc.`
//! are reserved by JSON-RPC and never used.
//!
//! This module holds exactly the agreed list: R3's thirty-seven requests
//! and ten notifications (`research/plugin-protocol.md`, section 3) with
//! R22's additions (`research/alpha4-plan.md`, section 2.3), and none of
//! the names the reviews retired (`config/get`, `ui/progress`,
//! `hook/event`). [`ALL`] describes each one, and the protocol reference,
//! `docs/dev/plugin-protocol.md`, is generated from it
//! ([`super::reference::page`]).
//!
//! Every method is optional within protocol 1: a host method the plugin
//! did not declare in its manifest and confirm in `initialize` is never
//! called, and a service the host did not offer in `host_capabilities` is
//! never called by the plugin.

// Lifecycle.

/// Host to plugin, request: start; carries the protocol version, the
/// host's capabilities, the settings and the folders.
pub const INITIALIZE: &str = "initialize";
/// Host to plugin, notification: the host accepted the `initialize`
/// result.
pub const INITIALIZED: &str = "initialized";
/// Host to plugin, request: stop; answered with null, then only `exit`.
pub const SHUTDOWN: &str = "shutdown";
/// Host to plugin, notification: end the process now, with code 0.
pub const EXIT: &str = "exit";

// Cross-cutting.

/// Either way, notification: cancel an outstanding request, which is still
/// answered.
pub const CANCEL_REQUEST: &str = "$/cancel_request";
/// Plugin to host, notification: progress of a request, a waiting state,
/// and partial results.
pub const PROGRESS: &str = "$/progress";
/// Plugin to host, notification: a line for the plugin's log, never the
/// screen.
pub const LOG: &str = "log";

// Commands and settings.

/// Host to plugin, request: run one of the plugin's commands.
pub const COMMAND_RUN: &str = "command/run";
/// Host to plugin, notification: the owner changed the plugin's settings.
pub const SETTINGS_CHANGED: &str = "settings/changed";

// Providers.

/// Host to plugin, request: sign in to an authority (a host, a bucket).
pub const PROVIDER_CONNECT: &str = "provider/connect";
/// Host to plugin, notification: the last lease on an authority ended.
pub const PROVIDER_DISCONNECT: &str = "provider/disconnect";
/// Host to plugin, request: list a folder, possibly in batches.
pub const PROVIDER_LIST: &str = "provider/list";
/// Host to plugin, request: one entry's details.
pub const PROVIDER_STAT: &str = "provider/stat";
/// Host to plugin, request: read a range of a file.
pub const PROVIDER_READ: &str = "provider/read";
/// Host to plugin, request: start writing a file.
pub const PROVIDER_WRITE_BEGIN: &str = "provider/write_begin";
/// Host to plugin, request: one chunk of a write.
pub const PROVIDER_WRITE_CHUNK: &str = "provider/write_chunk";
/// Host to plugin, request: commit or abandon a write.
pub const PROVIDER_WRITE_END: &str = "provider/write_end";
/// Host to plugin, request: make a folder.
pub const PROVIDER_CREATE_DIR: &str = "provider/create_dir";
/// Host to plugin, request: rename or move within the provider.
pub const PROVIDER_RENAME: &str = "provider/rename";
/// Host to plugin, request: makes a link at a place, symbolic or hard,
/// to a target (S-B's request; the SFTP plugin serves it).
pub const PROVIDER_CREATE_LINK: &str = "provider/create_link";
/// Host to plugin, request: remove one entry.
pub const PROVIDER_REMOVE: &str = "provider/remove";
/// Host to plugin, request: move one entry to the provider's trash.
pub const PROVIDER_TRASH: &str = "provider/trash";
/// Host to plugin, request: change attributes and times.
pub const PROVIDER_SET_ATTRIBUTES: &str = "provider/set_attributes";
/// Host to plugin, request: whether a server-side copy is possible.
pub const PROVIDER_COPY_HINT: &str = "provider/copy_hint";
/// Host to plugin, request: copy on the server.
pub const PROVIDER_COPY: &str = "provider/copy";
/// Host to plugin, request: start watching a folder.
pub const PROVIDER_WATCH: &str = "provider/watch";
/// Host to plugin, request: stop watching.
pub const PROVIDER_UNWATCH: &str = "provider/unwatch";
/// Host to plugin, request: the storages an authority offers, with free
/// and total bytes; with a `place`, the one volume that holds it.
pub const PROVIDER_VOLUMES: &str = "provider/volumes";
/// Host to plugin, request: the places the plugin offers (its sites), as
/// rows with a state word first.
pub const PROVIDER_PLACES: &str = "provider/places";
/// Plugin to host, notification: something changed in a watched folder.
pub const PROVIDER_CHANGED: &str = "provider/changed";
/// Plugin to host, notification: a device was unplugged or a session
/// ended.
pub const PROVIDER_LOST: &str = "provider/lost";

// Previews and conversions.

/// Host to plugin, request: make a preview.
pub const PREVIEW_RENDER: &str = "preview/render";
/// Host to plugin, request: run a conversion.
pub const CONVERT_RUN: &str = "convert/run";

// Hooks.

/// Host to plugin, request: a job is about to run; continue or cancel.
pub const HOOK_BEFORE: &str = "hook/before";
/// Host to plugin, notification: a job ran, a folder was entered, or the
/// selection changed.
pub const HOOK_AFTER: &str = "hook/after";

// Listers, columns, place descriptions and completion.

/// Plugin to host, request: show a list of rows that are not files.
pub const LISTER_OPEN: &str = "lister/open";
/// Host to plugin, request: more rows of a lister, or a refresh.
pub const LISTER_ROWS: &str = "lister/rows";
/// Host to plugin, notification: the owner left a lister.
pub const LISTER_CLOSED: &str = "lister/closed";
/// Host to plugin, request: a batch of a column's values.
pub const COLUMN_FORMAT: &str = "column/format";
/// Plugin to host, notification: some of a column's values are stale.
pub const COLUMN_CHANGED: &str = "column/changed";
/// Host to plugin, request: a short description of a place for the title
/// line.
pub const PLACE_DESCRIBE: &str = "place/describe";
/// Host to plugin, request: candidates for the text typed in a plugin's
/// prompt.
pub const COMPLETION_QUERY: &str = "completion/query";

// Services the host offers.

/// Plugin to host, notification: say something, attributed to the plugin.
pub const UI_ANNOUNCE: &str = "ui/announce";
/// Plugin to host, request: ask yes or no, choose, type, a path or a
/// secret.
pub const UI_ASK: &str = "ui/ask";
/// Plugin to host, request: a dialog of several labeled fields.
pub const UI_FORM: &str = "ui/form";
/// Plugin to host, request: show a text document in the viewer.
pub const UI_SHOW_TEXT: &str = "ui/show_text";
/// Plugin to host, request: compare two texts in the diff view.
pub const UI_SHOW_DIFF: &str = "ui/show_diff";
/// Plugin to host, request: read a panel's place, selection and cursor.
pub const PANEL_GET: &str = "panel/get";
/// Plugin to host, request: change a panel.
pub const PANEL_SET: &str = "panel/set";
/// Host to a serve client, notification: a panel's place, cursor or
/// selection changed (W4d's request; `diro serve` sends it).
pub const PANEL_CHANGED: &str = "panel/changed";
/// Plugin to host, request: run a diroxide command by id.
pub const COMMAND_EXECUTE: &str = "command/execute";
/// Plugin to host, request: list a folder through the providers.
pub const FS_LIST: &str = "fs/list";
/// Plugin to host, request: one entry's details through the providers.
pub const FS_STAT: &str = "fs/stat";
/// Plugin to host, request: read a range of a file.
pub const FS_READ: &str = "fs/read";
/// Plugin to host, request: replace a whole file.
pub const FS_WRITE: &str = "fs/write";
/// Plugin to host, request: make folders, as a job.
pub const FS_MKDIR: &str = "fs/mkdir";
/// Plugin to host, request: rename entries, as a job.
pub const FS_RENAME: &str = "fs/rename";
/// Plugin to host, request: copy entries, as a job.
pub const FS_COPY: &str = "fs/copy";
/// Plugin to host, request: move entries, as a job.
pub const FS_MOVE: &str = "fs/move";
/// Plugin to host, request: delete entries, to the trash by default, as a
/// job.
pub const FS_DELETE: &str = "fs/delete";
/// Plugin to host, request: a staging path for an output the plugin
/// creates.
pub const FS_STAGE: &str = "fs/stage";
/// Plugin to host, request: move a staged output into place, as a job.
pub const FS_COMMIT: &str = "fs/commit";
/// Plugin to host, request: run an allowed tool with an argument list.
pub const TOOLS_RUN: &str = "tools/run";
/// Plugin to host, request: whether a tool is found, where, and its
/// version.
pub const TOOLS_FIND: &str = "tools/find";
/// Plugin to host, request: start an allowed program detached.
pub const TOOLS_LAUNCH: &str = "tools/launch";
/// Host to plugin, notification: a chunk of a streamed tool's output.
pub const TOOLS_OUTPUT: &str = "tools/output";

/// Who sends a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sender {
    /// The host (or, for `diro serve`, the connecting program's peer).
    Host,
    /// The plugin (or, for `diro serve`, the connecting program).
    Plugin,
    /// Either side.
    Either,
}

impl Sender {
    /// The direction in words: "host to plugin".
    pub const fn words(self) -> &'static str {
        match self {
            Sender::Host => "host to plugin",
            Sender::Plugin => "plugin to host",
            Sender::Either => "either way",
        }
    }
}

/// Whether a method is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Call {
    /// It has an id and is answered.
    Request,
    /// It has no id and no answer.
    Notification,
}

impl Call {
    /// The kind in words: "request".
    pub const fn words(self) -> &'static str {
        match self {
            Call::Request => "request",
            Call::Notification => "notification",
        }
    }
}

/// How far a method is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Availability {
    /// The host serves it, or calls it, in alpha 2.
    Served,
    /// Its shape is fixed, with fixtures, so it is decided once; the host
    /// serves it when the feature that needs it arrives.
    ShapeOnly,
}

impl Availability {
    /// The state in words.
    pub const fn words(self) -> &'static str {
        match self {
            Availability::Served => "served",
            Availability::ShapeOnly => "shape only, served later",
        }
    }
}

/// The group a method belongs to in the reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    /// Starting and stopping.
    Lifecycle,
    /// Cancel, progress and the log.
    CrossCutting,
    /// Commands and settings.
    Commands,
    /// A plugin-provided file system.
    Providers,
    /// Previews and conversions.
    Previews,
    /// Before and after events.
    Hooks,
    /// Listers, columns, place descriptions and completion.
    Lists,
    /// What the host offers the plugin.
    Services,
}

impl Area {
    /// Every area, in the reference's order.
    pub const ALL: [Area; 8] = [
        Area::Lifecycle,
        Area::CrossCutting,
        Area::Commands,
        Area::Providers,
        Area::Previews,
        Area::Hooks,
        Area::Lists,
        Area::Services,
    ];

    /// The area's heading in the reference.
    pub const fn title(self) -> &'static str {
        match self {
            Area::Lifecycle => "Lifecycle",
            Area::CrossCutting => "Cancel, progress and the log",
            Area::Commands => "Commands and settings",
            Area::Providers => "Providers",
            Area::Previews => "Previews and conversions",
            Area::Hooks => "Hooks",
            Area::Lists => "Listers, columns, descriptions and completion",
            Area::Services => "Services the host offers",
        }
    }
}

/// One method of the protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Method {
    /// The name on the wire.
    pub name: &'static str,
    /// Who sends it.
    pub sender: Sender,
    /// Whether it is answered.
    pub call: Call,
    /// Its group in the reference.
    pub area: Area,
    /// What allows it, in words: the manifest table that declares a host
    /// method, or the permission a service needs.
    pub gate: &'static str,
    /// What it does, one sentence, meaning first.
    pub summary: &'static str,
    /// How far it is built.
    pub availability: Availability,
}

const fn method(
    name: &'static str,
    sender: Sender,
    call: Call,
    area: Area,
    gate: &'static str,
    summary: &'static str,
    availability: Availability,
) -> Method {
    Method {
        name,
        sender,
        call,
        area,
        gate,
        summary,
        availability,
    }
}

use Area as A;
use Availability::{Served as NOW, ShapeOnly as LATER};
use Call::{Notification as N, Request as R};
use Sender::{Either as E, Host as H, Plugin as P};

/// Every method, in the reference's order.
pub const ALL: [Method; 66] = [
    method(
        INITIALIZE,
        H,
        R,
        A::Lifecycle,
        "always",
        "Starts the conversation: the protocol major, both names and versions, and both capability lists.",
        NOW,
    ),
    method(
        INITIALIZED,
        H,
        N,
        A::Lifecycle,
        "always",
        "Says the host accepted the initialize result.",
        NOW,
    ),
    method(
        SHUTDOWN,
        H,
        R,
        A::Lifecycle,
        "always",
        "Asks the plugin to stop; answered with null, after which only exit comes.",
        NOW,
    ),
    method(
        EXIT,
        H,
        N,
        A::Lifecycle,
        "always",
        "Ends the plugin's process with code 0.",
        NOW,
    ),
    method(
        CANCEL_REQUEST,
        E,
        N,
        A::CrossCutting,
        "always",
        "Cancels an outstanding request by id; the request is still answered.",
        NOW,
    ),
    method(
        PROGRESS,
        P,
        N,
        A::CrossCutting,
        "a progress or partial token from the request",
        "Reports begin, report, end, a waiting state, or a partial batch of entries.",
        NOW,
    ),
    method(
        LOG,
        P,
        N,
        A::CrossCutting,
        "always",
        "Writes a line to the plugin's own log file, never to the screen.",
        NOW,
    ),
    method(
        COMMAND_RUN,
        H,
        R,
        A::Commands,
        "a [[command]] table",
        "Runs one of the plugin's commands with the context.",
        NOW,
    ),
    method(
        SETTINGS_CHANGED,
        H,
        N,
        A::Commands,
        "a [[setting]] table",
        "Sends the whole settings object after the owner changes one.",
        NOW,
    ),
    method(
        PROVIDER_CONNECT,
        H,
        R,
        A::Providers,
        "a [[provider]] table",
        "Signs in to an authority and returns its capabilities.",
        NOW,
    ),
    method(
        PROVIDER_DISCONNECT,
        H,
        N,
        A::Providers,
        "a [[provider]] table",
        "Says the last lease on an authority ended.",
        NOW,
    ),
    method(
        PROVIDER_LIST,
        H,
        R,
        A::Providers,
        "a [[provider]] table",
        "Lists a folder, in one answer or in partial batches.",
        NOW,
    ),
    method(
        PROVIDER_STAT,
        H,
        R,
        A::Providers,
        "a [[provider]] table",
        "Gives one entry's details.",
        NOW,
    ),
    method(
        PROVIDER_READ,
        H,
        R,
        A::Providers,
        "a [[provider]] table",
        "Reads a range of a file, inline or through the spool.",
        NOW,
    ),
    method(
        PROVIDER_WRITE_BEGIN,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Starts a write session that commits or leaves nothing.",
        NOW,
    ),
    method(
        PROVIDER_WRITE_CHUNK,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Sends one chunk of a write session.",
        NOW,
    ),
    method(
        PROVIDER_WRITE_END,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Commits or abandons a write session.",
        NOW,
    ),
    method(
        PROVIDER_CREATE_DIR,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Makes a folder, with or without its parents.",
        NOW,
    ),
    method(
        PROVIDER_RENAME,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Renames or moves within the provider.",
        NOW,
    ),
    method(
        PROVIDER_CREATE_LINK,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Makes a link at a place, symbolic or hard, to a target.",
        NOW,
    ),
    method(
        PROVIDER_REMOVE,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Removes one entry; the job engine lists and confirms first.",
        NOW,
    ),
    method(
        PROVIDER_TRASH,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Moves one entry to the provider's own trash.",
        NOW,
    ),
    method(
        PROVIDER_SET_ATTRIBUTES,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Changes attributes and times.",
        NOW,
    ),
    method(
        PROVIDER_COPY_HINT,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Says whether a server-side copy is possible, with no side effects.",
        NOW,
    ),
    method(
        PROVIDER_COPY,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Copies on the server, with progress, cancellable.",
        NOW,
    ),
    method(
        PROVIDER_WATCH,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Starts watching a folder.",
        NOW,
    ),
    method(
        PROVIDER_UNWATCH,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Stops watching a folder.",
        NOW,
    ),
    method(
        PROVIDER_VOLUMES,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Lists an authority's storages, free and total bytes; with a place, its one volume.",
        NOW,
    ),
    method(
        PROVIDER_PLACES,
        H,
        R,
        A::Providers,
        "the provider's methods list",
        "Lists the places the plugin offers, such as its sites, each row state first.",
        NOW,
    ),
    method(
        PROVIDER_CHANGED,
        P,
        N,
        A::Providers,
        "a watch the host started",
        "Reports created, changed, removed and renamed entries in a watched folder.",
        NOW,
    ),
    method(
        PROVIDER_LOST,
        P,
        N,
        A::Providers,
        "a [[provider]] table",
        "Says a device was unplugged or a session ended, with the reason in words.",
        NOW,
    ),
    method(
        PREVIEW_RENDER,
        H,
        R,
        A::Previews,
        "a [[previewer]] table",
        "Makes a preview of a file.",
        NOW,
    ),
    method(
        CONVERT_RUN,
        H,
        R,
        A::Previews,
        "a [[converter]] table",
        "Runs a conversion into a staged output.",
        NOW,
    ),
    method(
        HOOK_BEFORE,
        H,
        R,
        A::Hooks,
        "a [[hook]] table with a before event",
        "Lets the plugin continue or cancel a job before its confirmation.",
        LATER,
    ),
    method(
        HOOK_AFTER,
        H,
        N,
        A::Hooks,
        "a [[hook]] table with an after event",
        "Says a job ran, a folder was entered, or the selection changed.",
        LATER,
    ),
    method(
        LISTER_OPEN,
        P,
        R,
        A::Lists,
        "a [[lister]] table",
        "Shows rows that are not files in a panel.",
        NOW,
    ),
    method(
        LISTER_ROWS,
        H,
        R,
        A::Lists,
        "a [[lister]] table",
        "Asks for more rows of a lister, or a refresh.",
        NOW,
    ),
    method(
        LISTER_CLOSED,
        H,
        N,
        A::Lists,
        "a [[lister]] table",
        "Says the owner left a lister.",
        NOW,
    ),
    method(
        COLUMN_FORMAT,
        H,
        R,
        A::Lists,
        "a [[column]] table",
        "Asks for a batch of a column's values, at most 500 places.",
        LATER,
    ),
    method(
        COLUMN_CHANGED,
        P,
        N,
        A::Lists,
        "a [[column]] table",
        "Says some of a column's values are stale.",
        LATER,
    ),
    method(
        PLACE_DESCRIBE,
        H,
        R,
        A::Lists,
        "a [[place_info]] table",
        "Asks for a short description of a place for the title line.",
        LATER,
    ),
    method(
        COMPLETION_QUERY,
        H,
        R,
        A::Lists,
        "a prompt whose completion source is the plugin",
        "Asks for candidates for the text typed so far.",
        LATER,
    ),
    method(
        UI_ANNOUNCE,
        P,
        N,
        A::Services,
        "always",
        "Says a sentence, cleaned, rate-limited and attributed to the plugin.",
        NOW,
    ),
    method(
        UI_ASK,
        P,
        R,
        A::Services,
        "always",
        "Asks one question in a dialog; its message is one text or a list of lines.",
        NOW,
    ),
    method(
        UI_FORM,
        P,
        R,
        A::Services,
        "always",
        "Asks several labeled fields in one accessible dialog.",
        LATER,
    ),
    method(
        UI_SHOW_TEXT,
        P,
        R,
        A::Services,
        "always",
        "Shows a text document in the viewer.",
        NOW,
    ),
    method(
        UI_SHOW_DIFF,
        P,
        R,
        A::Services,
        "always",
        "Compares two texts in the diff view.",
        LATER,
    ),
    method(
        PANEL_GET,
        P,
        R,
        A::Services,
        "permissions.panel = read",
        "Reads a panel's place, cursor, selection, sort and filter.",
        NOW,
    ),
    method(
        PANEL_SET,
        P,
        R,
        A::Services,
        "permissions.panel = write",
        "Changes a panel's place, cursor or marks, as ordinary commands.",
        NOW,
    ),
    method(
        PANEL_CHANGED,
        H,
        N,
        A::Services,
        "a serve client",
        "Tells a serve client that a panel's place, cursor or selection changed.",
        NOW,
    ),
    method(
        COMMAND_EXECUTE,
        P,
        R,
        A::Services,
        "permissions.commands",
        "Runs a diroxide command by id.",
        NOW,
    ),
    method(
        FS_LIST,
        P,
        R,
        A::Services,
        "permissions.fs = read",
        "Lists a folder through the providers.",
        NOW,
    ),
    method(
        FS_STAT,
        P,
        R,
        A::Services,
        "permissions.fs = read",
        "Gives one entry's details through the providers.",
        NOW,
    ),
    method(
        FS_READ,
        P,
        R,
        A::Services,
        "permissions.fs = read",
        "Reads a range of a file, inline or through the spool.",
        NOW,
    ),
    method(
        FS_WRITE,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Replaces a whole file, inline or from the spool.",
        NOW,
    ),
    method(
        FS_MKDIR,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Makes folders as a job.",
        NOW,
    ),
    method(
        FS_RENAME,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Renames entries as a job.",
        NOW,
    ),
    method(
        FS_COPY,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Copies entries as a job, with the engine's plan and undo.",
        NOW,
    ),
    method(
        FS_MOVE,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Moves entries as a job, with the engine's plan and undo.",
        NOW,
    ),
    method(
        FS_DELETE,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Deletes entries as a job, to the trash by default, after the listing.",
        NOW,
    ),
    method(
        FS_STAGE,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Hands out a staging path for an output the plugin creates.",
        LATER,
    ),
    method(
        FS_COMMIT,
        P,
        R,
        A::Services,
        "permissions.fs = write",
        "Moves a staged output into place as a job.",
        LATER,
    ),
    method(
        TOOLS_RUN,
        P,
        R,
        A::Services,
        "permissions.tools",
        "Runs an allowed tool with an argument list, never a shell line.",
        NOW,
    ),
    method(
        TOOLS_FIND,
        P,
        R,
        A::Services,
        "always",
        "Says whether a tool is found, where, and its version.",
        NOW,
    ),
    method(
        TOOLS_LAUNCH,
        P,
        R,
        A::Services,
        "permissions.launch",
        "Starts an allowed program detached and answers at once.",
        NOW,
    ),
    method(
        TOOLS_OUTPUT,
        H,
        N,
        A::Services,
        "a streamed tools/run",
        "Sends a chunk of a streamed tool's output.",
        NOW,
    ),
];

/// A method by name.
pub fn find(name: &str) -> Option<&'static Method> {
    ALL.iter().find(|m| m.name == name)
}

/// Whether a name is one of the protocol's methods.
pub fn is_known(name: &str) -> bool {
    find(name).is_some()
}

/// The methods a host offers a plugin as services, by name: every method
/// the plugin sends that the host serves in this version.
pub fn services() -> impl Iterator<Item = &'static Method> {
    ALL.iter()
        .filter(|m| m.sender == Sender::Plugin && m.availability == Availability::Served)
}

/// How many requests and notifications the list holds.
pub fn counts() -> (usize, usize) {
    let requests = ALL.iter().filter(|m| m.call == Call::Request).count();
    (requests, ALL.len() - requests)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn names_are_snake_case_unique_and_never_rpc() {
        let mut seen = HashSet::new();
        for m in ALL {
            assert!(seen.insert(m.name), "{} appears twice", m.name);
            assert!(!m.name.starts_with("rpc."), "{}", m.name);
            assert!(
                m.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || "/_$".contains(c)),
                "{} is not snake_case",
                m.name
            );
            assert!(
                !m.name.contains("//") && !m.name.ends_with('/'),
                "{}",
                m.name
            );
            assert!(m.summary.ends_with('.'), "{}", m.name);
            assert!(m.summary.len() <= 100, "{} has a long summary", m.name);
        }
    }

    #[test]
    fn the_retired_names_are_gone() {
        for retired in ["config/get", "ui/progress", "hook/event"] {
            assert!(!is_known(retired), "{retired}");
        }
    }

    #[test]
    fn the_count_is_r3s_list_with_r22s_additions() {
        // R3: 37 requests and 10 notifications. R22 adds lister/open,
        // lister/rows, column/format, ui/form, ui/show_text, ui/show_diff,
        // place/describe, tools/launch, fs/stage, fs/commit,
        // provider/volumes and completion/query (12 requests), and
        // lister/closed, column/changed, tools/output and provider/lost
        // (4 notifications). R25 adds provider/places (1 request). W4d adds
        // panel/changed, the serve event (1 notification). S-B adds
        // provider/create_link (1 request).
        assert_eq!(counts(), (37 + 12 + 1 + 1, 10 + 4 + 1));
    }

    #[test]
    fn services_are_what_plugins_send() {
        let names: Vec<&str> = services().map(|m| m.name).collect();
        assert!(names.contains(&UI_ANNOUNCE));
        assert!(names.contains(&FS_COPY));
        assert!(!names.contains(&COMMAND_RUN));
        assert!(!names.contains(&UI_FORM), "ui/form is shape only");
    }
}
