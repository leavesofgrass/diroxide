//! The plugin's commands (R24, "The plugin's shape"; R25 commit 73): the
//! Sites list, Connect, Disconnect, Cancel connection, New site, Edit
//! site, Delete site and Forget password.
//!
//! - **A row or the panel says which site.** A command run from the
//!   Sites list acts on its row (`site:ID` or `config:NAME`); run from a
//!   panel showing an `sftp` place, on that place's connection.
//! - **Connect** goes through the provider's own connect, so the login
//!   flow, its questions and its refusals are the same as opening the
//!   place; then the active panel opens the site's start folder and, when
//!   the site names one, the other panel its local folder.
//! - **Delete asks first,** a destructive question that starts on No.
//!   Nothing else here destroys anything.
//! - **Every answer is announced** in a sentence with its meaning first;
//!   a refusal is an error with the next step as its hint.

use std::sync::Arc;
use std::time::Duration;

use diroxide_plugins::protocol::{ErrorKind, Params, RpcError, methods};
use serde_json::{Value, json};

use super::lister::{CONFIG_ROW, SITE_ROW};
use super::model::{SecretMode, Site, new_id};
use super::{Sites, StoreError, words};
use crate::askpass::dialogs::ANSWER_LIMIT;
use crate::plugin::State;
use crate::provider::{At, Call, Provider};
use crate::rpc::{Cancel, Peer};
use crate::ssh::target::Target;

/// The lister the Sites command opens.
pub const LISTER: &str = "sites";

/// The commands this build answers, plugin-local ids.
pub const COMMANDS: &[&str] = &[
    "sites",
    "connect",
    "disconnect",
    "cancel_connection",
    "new_site",
    "edit_site",
    "delete_site",
    "forget_password",
];

/// How long a host call other than a question may take.
pub const HOST_CALL: Duration = Duration::from_secs(30);

/// One command run, with what answering it needs.
#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    /// The host.
    pub peer: &'a Arc<Peer>,
    /// `command/run`'s parameters: `command` and `context`.
    pub params: &'a Params,
    /// The request's cancel flag.
    pub cancel: &'a Cancel,
    /// The plugin's settings and folders.
    pub state: &'a State,
    /// The sites.
    pub sites: &'a Arc<Sites>,
    /// The provider, for connections.
    pub provider: &'a Arc<Provider>,
}

fn params(value: Value) -> Params {
    match value {
        Value::Object(map) => map,
        _ => Params::new(),
    }
}

/// A command's answer: how many things it did.
pub fn done(count: usize) -> Value {
    json!({"done": count, "skipped": 0, "failed": []})
}

/// What a row or the active panel names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// A saved site.
    Site(Box<Site>),
    /// A host of the ssh configuration.
    Config(String),
    /// An `sftp` place in the active panel, not a saved site.
    Place(Target),
}

impl Chosen {
    /// The name said for it.
    pub fn name(&self) -> String {
        match self {
            Chosen::Site(site) => site.name.clone(),
            Chosen::Config(name) => name.clone(),
            Chosen::Place(target) => target.host().to_owned(),
        }
    }

    /// Where it connects.
    pub fn target(&self) -> Result<Target, RpcError> {
        let target = match self {
            Chosen::Site(site) => site.target(),
            Chosen::Config(name) => Target::new(name, None, None),
            Chosen::Place(target) => Ok(target.clone()),
        };
        target.map_err(|e| RpcError::new(ErrorKind::InvalidParams, e.to_string()))
    }
}

impl Run<'_> {
    /// The plugin-local command id: `connect` of `sftp.connect`.
    pub fn command(&self) -> &str {
        let full = self
            .params
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        full.rsplit_once('.').map_or(full, |(_, local)| local)
    }

    fn context(&self) -> Option<&Value> {
        self.params.get("context")
    }

    /// An argument the caller passed (`context.args`, or the top level).
    pub fn arg(&self, key: &str) -> Option<&str> {
        self.context()
            .and_then(|c| c.get("args"))
            .and_then(|a| a.get(key))
            .or_else(|| self.params.get("args").and_then(|a| a.get(key)))
            .and_then(Value::as_str)
    }

    /// The row id the selection or the cursor names.
    pub fn row_id(&self) -> Option<String> {
        let context = self.context()?;
        let from = |entry: &Value| {
            entry
                .get("extra")
                .and_then(|e| e.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        context
            .get("selection")
            .and_then(Value::as_array)
            .and_then(|s| s.first())
            .and_then(from)
            .or_else(|| context.get("cursor").and_then(from))
    }

    /// The active panel's place, when it is an `sftp` place.
    pub fn active_place(&self) -> Option<At> {
        let place = self.context()?.get("active")?.get("place")?.as_str()?;
        At::parse(place).ok()
    }

    /// What the command acts on: the row, else the active panel's place.
    pub fn chosen(&self) -> Result<Chosen, RpcError> {
        if let Some(id) = self.row_id() {
            if let Some(site_id) = id.strip_prefix(SITE_ROW) {
                let file = self.sites.file().map_err(|e| refusal(&e))?;
                return file
                    .site(site_id)
                    .cloned()
                    .map(|s| Chosen::Site(Box::new(s)))
                    .ok_or_else(no_site);
            }
            if let Some(name) = id.strip_prefix(CONFIG_ROW) {
                return Ok(Chosen::Config(name.to_owned()));
            }
        }
        let at = self.active_place().ok_or_else(no_site)?;
        Ok(match self.sites.for_target(&at.target) {
            Some(site) => Chosen::Site(Box::new(site)),
            None => Chosen::Place(at.target),
        })
    }

    /// Says a line through the host.
    pub fn announce(&self, text: &str, importance: &str) {
        self.peer.notify(
            methods::UI_ANNOUNCE,
            params(json!({"text": text, "importance": importance})),
        );
    }

    /// Asks the owner; `None` when the question was left with Escape or
    /// the host could not ask.
    pub fn ask(&self, question: Value) -> Option<Value> {
        let answer = self
            .peer
            .call(
                methods::UI_ASK,
                params(question),
                Some(self.cancel),
                ANSWER_LIMIT,
            )
            .ok()?;
        (answer.get("cancelled").and_then(Value::as_bool) != Some(true)).then_some(answer)
    }

    /// A yes or no that starts on No; true only for a yes.
    pub fn confirm(&self, title: &str, message: &[&str], yes: &str, no: &str) -> bool {
        self.ask(json!({
            "kind": "confirm",
            "destructive": true,
            "title": title,
            "message": message,
            "yes_label": yes,
            "no_label": no,
        }))
        .and_then(|a| a.get("answer").and_then(Value::as_bool))
        .unwrap_or(false)
    }

    /// A text question with a starting value; `None` on Escape.
    pub fn text(&self, title: &str, initial: &str) -> Option<String> {
        self.ask(json!({"kind": "text", "title": title, "initial": initial}))
            .and_then(|a| a.get("text").and_then(Value::as_str).map(str::to_owned))
    }

    /// Calls the host for something other than a question.
    fn host(&self, method: &str, value: Value) -> Result<Value, RpcError> {
        self.peer
            .call(method, params(value), Some(self.cancel), HOST_CALL)
    }
}

fn no_site() -> RpcError {
    RpcError::with_hint(
        ErrorKind::InvalidParams,
        words::NO_SITE_HERE,
        words::NO_SITE_HINT,
    )
}

/// A refused sites file as the host's error, its lines kept.
pub fn refusal(error: &StoreError) -> RpcError {
    let lines = error.lines();
    RpcError::with_hint(
        ErrorKind::Failed,
        lines.first().cloned().unwrap_or_default(),
        lines[1.min(lines.len())..].join("\n"),
    )
}

/// The state word for a site: connected, lost, or saved.
pub fn state_of(provider: &Provider, site: &Site) -> &'static str {
    match provider.link_state(|target| site.matches(target)) {
        Some(true) => words::CONNECTED,
        Some(false) => words::LOST,
        None => words::SAVED,
    }
}

/// Runs one command.
pub fn run(run: &Run<'_>) -> Result<Value, RpcError> {
    run.cancel.check()?;
    match run.command() {
        "sites" => sites(run),
        "connect" => connect(run),
        "disconnect" => disconnect(run, false),
        "cancel_connection" => disconnect(run, true),
        "new_site" => new_site(run),
        "edit_site" => edit_site(run),
        "delete_site" => delete_site(run),
        "forget_password" => forget_password(run),
        other => Err(RpcError::new(
            ErrorKind::MethodNotFound,
            format!("no command {other}"),
        )),
    }
}

/// The Sites list: the rows with their states, in the active panel.
fn sites(run: &Run<'_>) -> Result<Value, RpcError> {
    let (rows, notes) = run
        .sites
        .rows(|site| state_of(run.provider, site))
        .map_err(|e| refusal(&e))?;
    let lister_rows: Vec<Value> = rows.iter().map(super::Row::lister_row).collect();
    run.host(
        methods::LISTER_OPEN,
        json!({
            "lister": LISTER,
            "title": super::lister::title(&rows),
            "rows": lister_rows,
            "total": rows.len(),
            "panel": "active",
        }),
    )?;
    for note in notes {
        run.announce(&note, "info");
    }
    Ok(done(rows.len()))
}

/// Connect: the provider's own connect, then the site's start folders.
fn connect(run: &Run<'_>) -> Result<Value, RpcError> {
    let chosen = run.chosen()?;
    let target = chosen.target()?;
    let place = match &chosen {
        Chosen::Site(site) => site.place(),
        _ => format!("{}/", crate::provider::authority(&target)),
    };
    run.announce(&words::connecting(target.host()), "status");
    let call_params = params(json!({"place": place, "interactive": true}));
    let call = Call {
        peer: run.peer,
        params: &call_params,
        cancel: run.cancel,
        state: run.state,
    };
    let answer = run.provider.connect(&call)?;
    run.announce(&words::connected(target.host()), "status");
    let start = match &chosen {
        Chosen::Site(site) if site.start_remote.is_some() => site.place(),
        _ => answer
            .get("root")
            .and_then(Value::as_str)
            .map_or(place, str::to_owned),
    };
    // Opening the folders is a convenience: a host that does not let the
    // plugin move its panels leaves the owner connected all the same.
    let _ = run.host(
        methods::PANEL_SET,
        json!({"panel": "active", "place": start}),
    );
    if let Chosen::Site(site) = &chosen
        && let Some(local) = &site.start_local
    {
        let _ = run.host(
            methods::PANEL_SET,
            json!({"panel": "other", "place": local}),
        );
    }
    Ok(done(1))
}

/// Disconnect, quietly; or Cancel connection, which kills ssh at once
/// and is said as a loss.
fn disconnect(run: &Run<'_>, cancel: bool) -> Result<Value, RpcError> {
    let chosen = run.chosen()?;
    let name = chosen.name();
    let targets: Vec<Target> = match &chosen {
        Chosen::Site(site) => run
            .provider
            .connected()
            .into_iter()
            .filter(|t| site.matches(t))
            .collect(),
        other => vec![other.target()?],
    };
    let mut closed = 0;
    for target in &targets {
        let was = if cancel {
            run.provider.cancel_connection(target)
        } else {
            run.provider.disconnect(target)
        };
        closed += usize::from(was);
    }
    if closed == 0 {
        return Err(RpcError::new(
            ErrorKind::Failed,
            words::not_connected(&name),
        ));
    }
    if !cancel {
        run.announce(&words::disconnected(&name), "status");
    }
    // The owner closed the last connection: this session's passwords go
    // (R24 section 3.4).
    if run.provider.connected().is_empty() {
        run.sites.clear_session();
    }
    Ok(done(closed))
}

/// Reads an address as a new site's host, user, port and start folder:
/// `sftp://alice@build.example:2222/srv`, `alice@build.example`,
/// `build.example`.
pub fn read_address(text: &str) -> Result<(Target, Option<String>), RpcError> {
    let text = text.trim();
    let place = if text.starts_with("sftp://") {
        text.to_owned()
    } else {
        format!("sftp://{text}/")
    };
    let at = At::parse(&place).map_err(|e| {
        RpcError::with_hint(
            ErrorKind::InvalidParams,
            words::field_refused("address"),
            e.message,
        )
    })?;
    let start = String::from_utf8(at.path.clone())
        .ok()
        .filter(|p| p != "/" && p.starts_with('/'));
    Ok((at.target, start))
}

/// The three secret modes as choices, never first.
fn mode_choices() -> Value {
    json!([
        {"id": "never", "label": words::mode_words("never")},
        {"id": "session", "label": words::mode_words("session")},
        {"id": "keyring", "label": words::mode_words("keyring")},
    ])
}

fn ask_mode(run: &Run<'_>, current: SecretMode) -> Option<SecretMode> {
    if let Some(word) = run.arg("secret") {
        return SecretMode::from_word(word);
    }
    let answer = run.ask(json!({
        "kind": "choose",
        "title": "Remember its password?",
        "choices": mode_choices(),
        "default_id": current.word(),
    }))?;
    answer
        .get("ids")
        .and_then(Value::as_array)
        .and_then(|ids| ids.first())
        .and_then(Value::as_str)
        .and_then(SecretMode::from_word)
}

fn save(
    run: &Run<'_>,
    change: impl FnOnce(&mut super::SitesFile) -> Result<(), StoreError>,
) -> Result<(), RpcError> {
    let mut file = run.sites.file().map_err(|e| refusal(&e))?;
    change(&mut file).map_err(|e| refusal(&e))?;
    file.save().map_err(|e| refusal(&e))
}

fn nothing_changed(run: &Run<'_>) -> Result<Value, RpcError> {
    run.announce(words::NOTHING_CHANGED, "info");
    Ok(done(0))
}

/// New site: from a configuration host's row, an alias site with its
/// name; otherwise the address, the name and the secret mode, asked.
fn new_site(run: &Run<'_>) -> Result<Value, RpcError> {
    if let Some(id) = run.row_id()
        && let Some(name) = id.strip_prefix(CONFIG_ROW)
    {
        let site = Site {
            id: new_id(),
            name: name.to_owned(),
            alias: Some(name.to_owned()),
            ..Site::default()
        };
        let said = words::site_saved(&site.name);
        save(run, |file| file.add(site))?;
        run.announce(&said, "info");
        return Ok(done(1));
    }
    let address = match run.arg("address") {
        Some(address) => address.to_owned(),
        None => match run.text("Address of the new site", "") {
            Some(address) => address,
            None => return nothing_changed(run),
        },
    };
    let (target, start_remote) = read_address(&address)?;
    let name = match run.arg("name") {
        Some(name) => name.to_owned(),
        None => match run.text("Name of the new site", target.host()) {
            Some(name) => name,
            None => return nothing_changed(run),
        },
    };
    let Some(secret) = ask_mode(run, SecretMode::Never) else {
        return nothing_changed(run);
    };
    let site = Site {
        id: new_id(),
        name,
        host: Some(target.host().to_owned()),
        user: target.user().map(str::to_owned),
        port: target.port(),
        start_remote,
        secret,
        ..Site::default()
    }
    .checked()
    .map_err(|e| {
        RpcError::with_hint(
            ErrorKind::InvalidParams,
            words::field_refused("site"),
            e.to_string(),
        )
    })?;
    let said = words::site_saved(&site.name);
    save(run, |file| file.add(site))?;
    run.announce(&said, "info");
    Ok(done(1))
}

/// Edit site: the name, the address, the group, the start folder and the
/// secret mode, each asked with its current value.
fn edit_site(run: &Run<'_>) -> Result<Value, RpcError> {
    let Chosen::Site(site) = run.chosen()? else {
        return Err(no_site());
    };
    let mut site = *site;
    let Some(name) = run.text("Name of the site", &site.name) else {
        return nothing_changed(run);
    };
    let current = if site.is_alias() {
        site.host_or_alias().to_owned()
    } else {
        site.account()
    };
    let Some(address) = run.text("Address of the site", &current) else {
        return nothing_changed(run);
    };
    if address.trim() != current {
        let (target, start) = read_address(&address)?;
        site.host = Some(target.host().to_owned());
        site.alias = None;
        site.user = target.user().map(str::to_owned);
        site.port = target.port();
        if start.is_some() {
            site.start_remote = start;
        }
    }
    let Some(group) = run.text("Group of the site", &site.group) else {
        return nothing_changed(run);
    };
    let Some(start) = run.text(
        "Start folder on the server",
        site.start_remote.as_deref().unwrap_or_default(),
    ) else {
        return nothing_changed(run);
    };
    let Some(secret) = ask_mode(run, site.secret) else {
        return nothing_changed(run);
    };
    site.name = name;
    site.group = group;
    site.start_remote = Some(start.trim().to_owned()).filter(|s| !s.is_empty());
    site.secret = secret;
    let said = words::site_saved(&site.name);
    save(run, |file| file.update(site))?;
    run.announce(&said, "info");
    Ok(done(1))
}

/// Delete site: asked first, starting on No.
fn delete_site(run: &Run<'_>) -> Result<Value, RpcError> {
    let Chosen::Site(site) = run.chosen()? else {
        return Err(no_site());
    };
    let question = words::delete_question(&site.name);
    if !run.confirm(&question, &[words::DELETE_KEEPS], "Delete", "Keep") {
        return nothing_changed(run);
    }
    save(run, |file| file.remove(&site.id).map(|_| ()))?;
    // Its saved password goes with it; a store that cannot be reached
    // leaves nothing the plugin can find again.
    let _ = run.sites.forget_secret(&site);
    run.announce(&words::site_deleted(&site.name), "info");
    Ok(done(1))
}

/// Forget password: asked first, starting on No; removes the site's
/// password from this session and the keyring.
fn forget_password(run: &Run<'_>) -> Result<Value, RpcError> {
    let Chosen::Site(site) = run.chosen()? else {
        return Err(no_site());
    };
    if !run.sites.has_secret(&site) {
        run.announce(&words::nothing_to_forget(&site.name), "info");
        return Ok(done(0));
    }
    let question = words::forget_question(&site.name);
    if !run.confirm(&question, &[], "Forget", "Keep") {
        return nothing_changed(run);
    }
    run.sites
        .forget_secret(&site)
        .map_err(|e| RpcError::new(ErrorKind::Failed, e.to_string()))?;
    run.announce(&words::forgotten(&site.name), "info");
    Ok(done(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_read_as_targets_and_start_folders() {
        let (target, start) = read_address("sftp://alice@build.example:2222/srv").unwrap();
        assert_eq!(target.to_string(), "alice@build.example:2222");
        assert_eq!(start.as_deref(), Some("/srv"));
        let (target, start) = read_address(" alice@build.example ").unwrap();
        assert_eq!(target.to_string(), "alice@build.example");
        assert_eq!(start, None);
        for bad in ["-oProxyCommand=x", "a b", "", "alice@"] {
            let refused = read_address(bad).unwrap_err();
            assert_eq!(
                refused.message, "Not saved: the address was not valid.",
                "{bad}"
            );
        }
    }
}
