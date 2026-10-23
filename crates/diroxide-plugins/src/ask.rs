//! Host services, part two: what a plugin asks (`ui/ask`) and shows
//! (`ui/show_text`), with completion and history in its prompts (R3,
//! section 3.6; R22, section 2.3, N3, N4 and N10; plan section 5.19).
//!
//! - **One dialog per question,** the app's own: modal, named, closed by
//!   Escape, with focus returned. Escape is an answer (`cancelled`), not
//!   an error. Five kinds: `confirm`, `choose`, `text`, `path` and
//!   `secret`.
//! - **Cleaned and attributed.** The title and every line of the message
//!   are cleaned like announcements, and the title ends with the plugin's
//!   name, so a question never sounds like diroxide's own: "Trust this
//!   host? (SFTP places)". A message that is a list of lines is shown and
//!   spoken line by line, for the 40-cell Braille display.
//! - **History per prompt,** keyed by the plugin and the prompt's id, and
//!   never for a secret.
//! - **Completion:** a source the host knows (paths, places, commands,
//!   labels, types, history), a fixed list the plugin hands over, or the
//!   plugin itself answering `completion/query` as the owner types; the
//!   next keystroke cancels the query before it. A secret never completes
//!   and nothing the owner types into one reaches the plugin before the
//!   answer.
//! - **The owner's thinking time** does not count against the plugin's
//!   timeout: the supervisor pauses a request's timer while the host
//!   serves one of the plugin's requests.
//!
//! `ui/form` and `ui/show_diff` are shapes only in this alpha ([`Form`],
//! [`DiffDocuments`]): fixed so they are decided once, served when the
//! git plugin arrives.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use diroxide_core::Secret;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::lines;
use crate::live::LivePlugin;
use crate::protocol::{ErrorKind, Id, Params, RpcError, methods};
use crate::services::{MAX_ANNOUNCE_CHARS, clean_keeping_spaces, clean_text};
use crate::shapes::WireText;

/// The most candidates one completion answer keeps.
pub const MAX_CANDIDATES: usize = 200;

/// How many earlier answers a prompt's history keeps.
pub const HISTORY_PER_PROMPT: usize = 50;

/// After this long without candidates the prompt says "searching" in
/// words (the app shows it; the query goes on).
pub const SEARCHING_AFTER: Duration = Duration::from_millis(300);

/// How long a completion query may take before the prompt gives up on it.
pub const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest line of a text document, in characters.
pub const MAX_DOCUMENT_LINE_CHARS: usize = 10_000;

/// The most lines a text document keeps.
pub const MAX_DOCUMENT_LINES: usize = 100_000;

/// One choice of a `choose` question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The id the answer carries.
    pub id: String,
    /// What the owner reads, meaning first.
    pub label: String,
    /// More words, read after the label.
    pub description: Option<String>,
}

/// What a `path` question wants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathMode {
    /// A file.
    File,
    /// A folder.
    Folder,
    /// Either.
    #[default]
    Any,
}

/// A text pattern the answer must match, with the words that explain it.
#[derive(Clone, Debug)]
pub struct TextPattern {
    regex: Regex,
    /// The pattern as the plugin wrote it.
    pub source: String,
    /// What the owner hears when the text does not match, meaning first.
    pub message: String,
}

impl PartialEq for TextPattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.message == other.message
    }
}

impl TextPattern {
    /// Whether a text matches the whole pattern.
    pub fn matches(&self, text: &str) -> bool {
        self.regex.find(text).is_some_and(|m| m.as_str() == text)
    }
}

/// The kind of a question, with its own fields.
#[derive(Clone, Debug, PartialEq)]
pub enum QuestionKind {
    /// Yes or no. A destructive question (the default) starts on No and
    /// ends "y or n".
    Confirm {
        /// Whether the dialog starts on No.
        destructive: bool,
        /// The yes button's words.
        yes_label: Option<String>,
        /// The no button's words.
        no_label: Option<String>,
    },
    /// One or several of a list.
    Choose {
        /// The choices, in order.
        choices: Vec<Choice>,
        /// The choice the dialog starts on.
        default_id: Option<String>,
        /// Whether several may be chosen.
        multiple: bool,
    },
    /// A line of text.
    Text {
        /// What the field starts with.
        initial: String,
        /// The pattern the answer must match.
        pattern: Option<TextPattern>,
    },
    /// A place.
    Path {
        /// What it wants.
        mode: PathMode,
        /// Whether the place must exist.
        must_exist: bool,
        /// What the field starts with.
        initial: String,
    },
    /// A password or passphrase: never echoed, never logged, never kept
    /// in history, never completed.
    Secret,
}

impl QuestionKind {
    /// The kind's word on the wire.
    pub const fn word(&self) -> &'static str {
        match self {
            QuestionKind::Confirm { .. } => "confirm",
            QuestionKind::Choose { .. } => "choose",
            QuestionKind::Text { .. } => "text",
            QuestionKind::Path { .. } => "path",
            QuestionKind::Secret => "secret",
        }
    }

    /// Whether the owner types into it, so history and completion apply.
    pub const fn is_typed(&self) -> bool {
        matches!(self, QuestionKind::Text { .. } | QuestionKind::Path { .. })
    }
}

/// A completion source the host serves itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostSource {
    /// Names under a place, through its provider.
    Path,
    /// Places: favorites, history, the panels'.
    Place,
    /// diroxide's commands.
    Command,
    /// Labels.
    Label,
    /// File types.
    Type,
    /// This prompt's history.
    History,
}

impl HostSource {
    /// The source's word on the wire.
    pub const fn word(self) -> &'static str {
        match self {
            HostSource::Path => "path",
            HostSource::Place => "place",
            HostSource::Command => "command",
            HostSource::Label => "label",
            HostSource::Type => "type",
            HostSource::History => "history",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "path" => HostSource::Path,
            "place" => HostSource::Place,
            "command" => HostSource::Command,
            "label" => HostSource::Label,
            "type" => HostSource::Type,
            "history" => HostSource::History,
            _ => return None,
        })
    }
}

/// One completion candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// The text it inserts.
    pub text: String,
    /// What it is, in a word: "folder", "branch".
    #[serde(default)]
    pub kind: String,
}

impl Candidate {
    /// A candidate, cleaned like an announcement; `None` when nothing is
    /// left.
    pub fn cleaned(text: &str, kind: &str) -> Option<Self> {
        let text = clean_text(text, MAX_ANNOUNCE_CHARS);
        if text.is_empty() {
            return None;
        }
        Some(Self {
            text,
            kind: clean_text(kind, lines::MAX_LINE_CELLS),
        })
    }
}

/// Cleans a list of candidates from a plugin: each cleaned, the empty
/// ones dropped, at most [`MAX_CANDIDATES`].
pub fn clean_candidates(value: &Value) -> Vec<Candidate> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let text = item
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())?;
                    let kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
                    Candidate::cleaned(text, kind)
                })
                .take(MAX_CANDIDATES)
                .collect()
        })
        .unwrap_or_default()
}

/// Where a prompt's candidates come from.
#[derive(Clone, Debug)]
pub enum Completion {
    /// A source the host serves.
    Host(HostSource),
    /// A fixed list the plugin handed over.
    List(Vec<Candidate>),
    /// The plugin, through `completion/query`.
    Plugin(PluginCompleter),
}

impl PartialEq for Completion {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Completion::Host(a), Completion::Host(b)) => a == b,
            (Completion::List(a), Completion::List(b)) => a == b,
            (Completion::Plugin(a), Completion::Plugin(b)) => a.prompt_id == b.prompt_id,
            _ => false,
        }
    }
}

impl Completion {
    /// Candidates for the text typed so far, from a fixed list: those
    /// that start with it, ignoring case.
    pub fn from_list(candidates: &[Candidate], typed: &str) -> Vec<Candidate> {
        let typed = typed.to_lowercase();
        candidates
            .iter()
            .filter(|c| c.text.to_lowercase().starts_with(&typed))
            .take(MAX_CANDIDATES)
            .cloned()
            .collect()
    }
}

struct CompleterInner {
    live: Weak<LivePlugin>,
    plugin: String,
    prompt_id: String,
    outstanding: Mutex<Option<Id>>,
}

/// Asks the plugin for candidates as the owner types (`completion/query`).
/// The app calls [`PluginCompleter::query`] on a worker thread at each
/// keystroke; a new query cancels the one before it with
/// `$/cancel_request`, and the earlier caller gets the cancellation.
#[derive(Clone)]
pub struct PluginCompleter {
    inner: Arc<CompleterInner>,
    prompt_id: String,
}

impl fmt::Debug for PluginCompleter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginCompleter")
            .field("plugin", &self.inner.plugin)
            .field("prompt_id", &self.prompt_id)
            .finish()
    }
}

impl PluginCompleter {
    pub(crate) fn new(live: Weak<LivePlugin>, plugin: &str, prompt_id: &str) -> Self {
        Self {
            inner: Arc::new(CompleterInner {
                live,
                plugin: plugin.to_owned(),
                prompt_id: prompt_id.to_owned(),
                outstanding: Mutex::new(None),
            }),
            prompt_id: prompt_id.to_owned(),
        }
    }

    /// The prompt's id.
    pub fn prompt_id(&self) -> &str {
        &self.prompt_id
    }

    /// Whether a query is waiting for its answer.
    pub fn is_waiting(&self) -> bool {
        self.inner
            .outstanding
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Candidates for the text typed so far, with the cursor's position in
    /// characters. Cancels the query before it, which is still answered.
    pub fn query(&self, typed: &str, cursor: usize) -> Result<Vec<Candidate>, RpcError> {
        let live = self.inner.live.upgrade().ok_or_else(|| {
            RpcError::new(
                ErrorKind::PluginUnavailable,
                format!("{} stopped", self.inner.plugin),
            )
        })?;
        let connection = live.connection();
        let earlier = self
            .inner
            .outstanding
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(earlier) = earlier {
            let _ = connection.cancel(&earlier);
        }
        let mut params = Params::new();
        params.insert("prompt_id".into(), json!(self.inner.prompt_id));
        params.insert("text".into(), json!(typed));
        params.insert("cursor".into(), json!(cursor));
        params.insert("limit".into(), json!(MAX_CANDIDATES));
        let call = connection
            .request_then(methods::COMPLETION_QUERY, params, |id| {
                *self
                    .inner
                    .outstanding
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(id.clone());
            })
            .map_err(|e| RpcError::new(ErrorKind::PluginUnavailable, e.to_string()))?;
        let outcome = call.wait(COMPLETION_TIMEOUT).unwrap_or_else(|| {
            let _ = connection.cancel(&call.id);
            Err(RpcError::new(ErrorKind::TimedOut, "no candidates in time"))
        });
        {
            let mut outstanding = self
                .inner
                .outstanding
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if outstanding.as_ref() == Some(&call.id) {
                *outstanding = None;
            }
        }
        let value = outcome?;
        Ok(clean_candidates(
            value.get("candidates").unwrap_or(&Value::Null),
        ))
    }
}

/// A question a plugin asks, as the app's dialog shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    /// The plugin's id.
    pub plugin_id: String,
    /// The plugin's name, which the title ends with.
    pub plugin: String,
    /// The dialog's title, cleaned, the plugin's name last: "Trust this
    /// host? (SFTP places)".
    pub title: String,
    /// The message, line by line, cleaned.
    pub lines: Vec<String>,
    /// The kind, with its fields.
    pub kind: QuestionKind,
    /// The prompt's id, when the plugin gave one: history is kept per
    /// prompt.
    pub prompt_id: Option<String>,
    /// Earlier answers to this prompt, newest first; never for a secret.
    pub history: Vec<String>,
    /// Where candidates come from as the owner types; never for a secret.
    pub completion: Option<Completion>,
}

impl Question {
    /// Whether a typed answer is acceptable; the error is the sentence the
    /// dialog says ("Use letters and digits only").
    pub fn check_text(&self, text: &str) -> Result<(), String> {
        match &self.kind {
            QuestionKind::Text {
                pattern: Some(pattern),
                ..
            } if !pattern.matches(text) => Err(pattern.message.clone()),
            _ => Ok(()),
        }
    }
}

/// The owner's answer to a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// A `confirm` answer.
    Confirmed(bool),
    /// The ids chosen, in the list's order.
    Chosen(Vec<String>),
    /// The text typed.
    Text(String),
    /// The secret typed; it goes to the plugin and nowhere else.
    Secret(Secret),
    /// The place chosen, as text.
    Place(String),
    /// Escape.
    Cancelled,
}

impl Answer {
    /// The answer on the wire, checked against the question: a kind the
    /// question did not ask for is a host error, and chosen ids must be the
    /// question's.
    pub fn to_value(&self, question: &Question) -> Result<Value, RpcError> {
        let wrong = || {
            RpcError::new(
                ErrorKind::InternalError,
                format!(
                    "the dialog answered a {} question wrongly",
                    question.kind.word()
                ),
            )
        };
        Ok(match (self, &question.kind) {
            (Answer::Cancelled, _) => json!({"cancelled": true}),
            (Answer::Confirmed(yes), QuestionKind::Confirm { .. }) => json!({"answer": yes}),
            (
                Answer::Chosen(ids),
                QuestionKind::Choose {
                    choices, multiple, ..
                },
            ) => {
                let known: HashSet<&str> = choices.iter().map(|c| c.id.as_str()).collect();
                if ids.iter().any(|id| !known.contains(id.as_str())) || (!multiple && ids.len() > 1)
                {
                    return Err(wrong());
                }
                json!({"ids": ids})
            }
            (Answer::Text(text), QuestionKind::Text { .. }) => json!({"text": text}),
            (Answer::Secret(secret), QuestionKind::Secret) => json!({"text": secret.expose()}),
            (Answer::Place(place), QuestionKind::Path { .. }) => json!({"place": place}),
            _ => return Err(wrong()),
        })
    }

    /// The text to keep in the prompt's history: a typed text or place,
    /// never a secret, never a cancel.
    pub fn for_history(&self) -> Option<&str> {
        match self {
            Answer::Text(text) | Answer::Place(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }
}

/// Earlier answers per prompt, newest first, at most
/// [`HISTORY_PER_PROMPT`] each; a secret is never kept.
#[derive(Clone, Debug, Default)]
pub struct PromptHistory {
    prompts: HashMap<String, VecDeque<String>>,
}

impl PromptHistory {
    fn key(plugin_id: &str, prompt_id: &str) -> String {
        format!("{plugin_id}/{prompt_id}")
    }

    /// A prompt's history, newest first.
    pub fn recall(&self, plugin_id: &str, prompt_id: &str) -> Vec<String> {
        self.prompts
            .get(&Self::key(plugin_id, prompt_id))
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Keeps an answer at the front of its prompt's history.
    pub fn remember(&mut self, plugin_id: &str, prompt_id: &str, text: &str) {
        let items = self
            .prompts
            .entry(Self::key(plugin_id, prompt_id))
            .or_default();
        items.retain(|item| item != text);
        items.push_front(text.to_owned());
        items.truncate(HISTORY_PER_PROMPT);
    }

    /// How many prompts have a history.
    pub fn prompts(&self) -> usize {
        self.prompts.len()
    }
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn text_field(params: &Params, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn cleaned_field(params: &Params, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(|t| clean_text(t, MAX_ANNOUNCE_CHARS))
        .filter(|t| !t.is_empty())
}

/// The message lines of a question, each cleaned; empty lines dropped.
fn message_lines(params: &Params) -> Result<Vec<String>, RpcError> {
    let Some(message) = params.get("message") else {
        return Ok(Vec::new());
    };
    let text: WireText = serde_json::from_value(message.clone())
        .map_err(|_| invalid("message must be a text or a list of lines"))?;
    Ok(text
        .lines()
        .into_iter()
        .map(|line| clean_text(line, MAX_ANNOUNCE_CHARS))
        .filter(|line| !line.is_empty())
        .collect())
}

fn read_choices(params: &Params) -> Result<Vec<Choice>, RpcError> {
    let items = params
        .get("choices")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| invalid("choose needs a list of choices"))?;
    let mut seen = HashSet::new();
    let mut choices = Vec::new();
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("every choice needs an id"))?;
        if !seen.insert(id.to_owned()) {
            return Err(invalid(format!("the choice id {id} appears twice")));
        }
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .map(|l| clean_text(l, MAX_ANNOUNCE_CHARS))
            .filter(|l| !l.is_empty())
            .ok_or_else(|| invalid(format!("the choice {id} needs a label")))?;
        let description = item
            .get("description")
            .and_then(Value::as_str)
            .map(|d| clean_text(d, MAX_ANNOUNCE_CHARS))
            .filter(|d| !d.is_empty());
        choices.push(Choice {
            id: id.to_owned(),
            label,
            description,
        });
    }
    Ok(choices)
}

fn read_kind(params: &Params) -> Result<QuestionKind, RpcError> {
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("confirm");
    let flag =
        |key: &str, default: bool| params.get(key).and_then(Value::as_bool).unwrap_or(default);
    Ok(match kind {
        "confirm" => QuestionKind::Confirm {
            destructive: flag("destructive", true),
            yes_label: cleaned_field(params, "yes_label"),
            no_label: cleaned_field(params, "no_label"),
        },
        "choose" => {
            let choices = read_choices(params)?;
            let default_id = text_field(params, "default_id");
            if let Some(id) = &default_id
                && !choices.iter().any(|c| &c.id == id)
            {
                return Err(invalid(format!("default_id {id} is not a choice")));
            }
            QuestionKind::Choose {
                choices,
                default_id,
                multiple: flag("multiple", false),
            }
        }
        "text" => {
            let pattern = match text_field(params, "pattern") {
                Some(source) => {
                    let regex = Regex::new(&source)
                        .map_err(|_| invalid("pattern is not a valid regular expression"))?;
                    let message = cleaned_field(params, "pattern_message")
                        .unwrap_or_else(lines::text_does_not_fit);
                    Some(TextPattern {
                        regex,
                        source,
                        message,
                    })
                }
                None => None,
            };
            QuestionKind::Text {
                initial: text_field(params, "initial").unwrap_or_default(),
                pattern,
            }
        }
        "path" => QuestionKind::Path {
            mode: match params.get("mode").and_then(Value::as_str) {
                Some("file") => PathMode::File,
                Some("folder") => PathMode::Folder,
                Some("any") | None => PathMode::Any,
                Some(other) => {
                    return Err(invalid(format!("mode {other} is not file, folder or any")));
                }
            },
            must_exist: flag("must_exist", false),
            initial: text_field(params, "initial").unwrap_or_default(),
        },
        "secret" => QuestionKind::Secret,
        other => {
            return Err(invalid(format!(
                "kind {other} is not confirm, choose, text, path or secret"
            )));
        }
    })
}

/// What the completion object asks for, before the host decides whether
/// it can serve it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CompletionWanted {
    Host(HostSource),
    List(Vec<Candidate>),
    Plugin,
}

/// Reads `ui/ask`'s parameters into a question, without history or a live
/// completer (the caller adds those). The title ends with the plugin's
/// name.
pub(crate) fn read_question(
    params: &Params,
    plugin_id: &str,
    plugin: &str,
) -> Result<(Question, Option<CompletionWanted>), RpcError> {
    let kind = read_kind(params)?;
    let title = lines::dialog_title(&cleaned_field(params, "title").unwrap_or_default(), plugin);
    let lines = message_lines(params)?;
    let complete = params.get("complete").and_then(Value::as_object);
    let prompt_id = text_field(params, "prompt_id")
        .or_else(|| complete.and_then(|c| text_field(c, "prompt_id")))
        .filter(|id| !id.is_empty());
    let wanted = match complete {
        Some(complete) if kind.is_typed() => {
            let source = complete
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("complete needs a source"))?;
            Some(match source {
                "list" => CompletionWanted::List(clean_candidates(
                    complete.get("candidates").unwrap_or(&Value::Null),
                )),
                "plugin" => CompletionWanted::Plugin,
                word => CompletionWanted::Host(
                    HostSource::from_word(word)
                        .ok_or_else(|| invalid(format!("complete source {word} is not known")))?,
                ),
            })
        }
        // A secret, a confirm or a choose never completes.
        _ => None,
    };
    if wanted == Some(CompletionWanted::Plugin) && prompt_id.is_none() {
        return Err(invalid("a plugin completion source needs a prompt_id"));
    }
    Ok((
        Question {
            plugin_id: plugin_id.to_owned(),
            plugin: plugin.to_owned(),
            title,
            lines,
            kind,
            prompt_id,
            history: Vec::new(),
            completion: None,
        },
        wanted,
    ))
}

/// How a text document is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    /// Plain text.
    #[default]
    Plain,
    /// A unified diff, read as one.
    Diff,
    /// Markdown, read with its structure.
    Markdown,
}

/// A text document a plugin shows in the viewer (`ui/show_text`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDocument {
    /// The plugin's name.
    pub plugin: String,
    /// The viewer's title, cleaned, the plugin's name last.
    pub title: String,
    /// The lines, each cleaned of control characters and escape
    /// sequences, with their spaces kept.
    pub lines: Vec<String>,
    /// How it is shown.
    pub kind: TextKind,
    /// Whether the plugin waits until the owner closes it.
    pub wait: bool,
}

/// Cleans one line of a document: control, format and direction
/// characters and terminal escape sequences removed, tabs made spaces,
/// other spaces kept, cut to [`MAX_DOCUMENT_LINE_CHARS`].
pub fn clean_document_line(line: &str) -> String {
    clean_keeping_spaces(line, MAX_DOCUMENT_LINE_CHARS)
}

/// Reads `ui/show_text`'s parameters into a document.
pub(crate) fn read_document(params: &Params, plugin: &str) -> Result<TextDocument, RpcError> {
    let text: WireText = params
        .get("text")
        .or_else(|| params.get("lines"))
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|_| invalid("text must be a text or a list of lines"))?
        .ok_or_else(|| invalid("show_text needs a text"))?;
    let kind = match params.get("kind") {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|_| invalid("kind must be plain, diff or markdown"))?,
        None => TextKind::Plain,
    };
    let title = lines::document_title(&cleaned_field(params, "title").unwrap_or_default(), plugin);
    Ok(TextDocument {
        plugin: plugin.to_owned(),
        title,
        lines: text
            .lines()
            .into_iter()
            .take(MAX_DOCUMENT_LINES)
            .map(clean_document_line)
            .collect(),
        kind,
        wait: params.get("wait").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// A field of a form (`ui/form`, shape only in this alpha).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// The id the answer carries.
    pub id: String,
    /// The label, its meaning in the first 40 cells.
    pub label: String,
    /// `text`, `multiline`, `choice`, `checkbox`, `path` or `secret`.
    pub kind: String,
    /// The value it starts with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// The choices, for `choice`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// A pattern the value must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// What is said when it does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_message: Option<String>,
}

/// A dialog of several labeled fields (`ui/form`, shape only in this
/// alpha: the git plugin's commit and new-repository dialogs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Form {
    /// The title.
    pub title: String,
    /// A paragraph read before the fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The fields, in reading order.
    pub fields: Vec<FormField>,
    /// The field that has the focus first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// The default button's words; No or Cancel when anything in the form
    /// is destructive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_button: Option<String>,
}

/// Two texts for the diff view (`ui/show_diff`, shape only in this
/// alpha).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffDocuments {
    /// The view's title.
    pub title: String,
    /// The old document: its title and text.
    pub old: DiffSide,
    /// The new document.
    pub new: DiffSide,
}

/// One side of a diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSide {
    /// Its title: "HEAD", "working tree".
    pub title: String,
    /// Its text.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(value: Value) -> Params {
        value.as_object().cloned().unwrap()
    }

    fn ask(value: Value) -> Result<(Question, Option<CompletionWanted>), RpcError> {
        read_question(&params(value), "sftp", "SFTP places")
    }

    #[test]
    fn each_kind_reads_with_its_fields() {
        let (q, _) = ask(json!({"kind": "confirm", "title": "Trust this host?"})).unwrap();
        assert_eq!(q.title, "Trust this host? (SFTP places)");
        assert_eq!(
            q.kind,
            QuestionKind::Confirm {
                destructive: true,
                yes_label: None,
                no_label: None
            }
        );
        let (q, _) = ask(json!({"kind": "choose", "choices": [
            {"id": "a", "label": "Main"}, {"id": "b", "label": "Dev", "description": "work"}
        ], "default_id": "b", "multiple": true}))
        .unwrap();
        let QuestionKind::Choose {
            choices, multiple, ..
        } = &q.kind
        else {
            panic!()
        };
        assert_eq!(choices.len(), 2);
        assert!(*multiple);
        let (q, _) = ask(json!({"kind": "text", "initial": "x", "pattern": "[a-z]+",
            "pattern_message": "Use small letters only"}))
        .unwrap();
        assert_eq!(q.check_text("abc"), Ok(()));
        assert_eq!(
            q.check_text("ab1"),
            Err("Use small letters only".to_owned())
        );
        let (q, _) = ask(json!({"kind": "path", "mode": "folder", "must_exist": true})).unwrap();
        assert_eq!(
            q.kind,
            QuestionKind::Path {
                mode: PathMode::Folder,
                must_exist: true,
                initial: String::new()
            }
        );
        let (q, _) = ask(json!({"kind": "secret", "title": "Password"})).unwrap();
        assert_eq!(q.kind, QuestionKind::Secret);
        assert_eq!(q.title, "Password (SFTP places)");
    }

    #[test]
    fn bad_questions_are_refused_with_the_field() {
        for (value, words) in [
            (json!({"kind": "maybe"}), "kind maybe"),
            (json!({"kind": "choose"}), "choose needs"),
            (
                json!({"kind": "choose", "choices": [{"id": "a", "label": "A"}, {"id": "a", "label": "B"}]}),
                "appears twice",
            ),
            (json!({"kind": "text", "pattern": "("}), "pattern"),
            (json!({"kind": "path", "mode": "disk"}), "mode disk"),
            (json!({"message": 5}), "message"),
            (
                json!({"kind": "text", "complete": {"source": "plugin"}}),
                "prompt_id",
            ),
        ] {
            let error = ask(value).unwrap_err();
            assert_eq!(error.code, -32602);
            assert!(error.message.contains(words), "{}", error.message);
        }
    }

    #[test]
    fn a_message_of_lines_stays_lines_and_is_cleaned() {
        let (q, _) = ask(json!({
            "title": "\u{1B}[31mHost key changed\u{202E}",
            "message": ["Host key changed: example.org", "", "Old: ab12 cd34\u{7}"]
        }))
        .unwrap();
        assert_eq!(q.title, "Host key changed (SFTP places)");
        assert_eq!(q.lines, ["Host key changed: example.org", "Old: ab12 cd34"]);
    }

    /// R25's commit 46: the SFTP plugin's questions as the dialog shows
    /// them, golden row for row: the title with the plugin's name last,
    /// then each line of the message cleaned, the fingerprint in hex
    /// groups of four one part a line (answer 84), the server's own words
    /// last and cleaned.
    #[test]
    fn the_sftp_dialogs_keep_their_golden_rows() {
        for (json, rows) in [
            (
                include_str!("../../../fixtures/sftp/dialogs/new-host-key.json"),
                include_str!("../../../fixtures/sftp/dialogs/new-host-key.rows"),
            ),
            (
                include_str!("../../../fixtures/sftp/dialogs/password.json"),
                include_str!("../../../fixtures/sftp/dialogs/password.rows"),
            ),
            (
                include_str!("../../../fixtures/sftp/dialogs/server-question.json"),
                include_str!("../../../fixtures/sftp/dialogs/server-question.rows"),
            ),
        ] {
            let (q, _) = ask(serde_json::from_str(json).unwrap()).unwrap();
            let shown: Vec<&str> = std::iter::once(q.title.as_str())
                .chain(q.lines.iter().map(String::as_str))
                .collect();
            let golden: Vec<&str> = rows.lines().collect();
            assert_eq!(shown, golden);
            // Each row's meaning is whole within one Braille line; the one
            // longer row is ssh's own text, after the line that says so.
            for row in &shown {
                assert!(
                    row.chars().count() <= 40
                        || row.starts_with("SHA256:")
                        || row.ends_with("(SFTP places)"),
                    "{row}"
                );
            }
        }
        let (q, _) = ask(serde_json::from_str(include_str!(
            "../../../fixtures/sftp/dialogs/new-host-key.json"
        ))
        .unwrap())
        .unwrap();
        assert!(matches!(
            q.kind,
            QuestionKind::Confirm {
                destructive: true,
                ..
            }
        ));
    }

    #[test]
    fn completion_sources_are_read_and_never_for_a_secret() {
        let (_, wanted) = ask(json!({"kind": "text", "prompt_id": "branch",
            "complete": {"source": "list", "candidates": [{"text": "main", "kind": "branch"}, "dev", {"text": "\u{200B}"}]}}))
        .unwrap();
        assert_eq!(
            wanted,
            Some(CompletionWanted::List(vec![
                Candidate {
                    text: "main".into(),
                    kind: "branch".into()
                },
                Candidate {
                    text: "dev".into(),
                    kind: String::new()
                },
            ]))
        );
        let (_, wanted) = ask(json!({"kind": "path", "complete": {"source": "path"}})).unwrap();
        assert_eq!(wanted, Some(CompletionWanted::Host(HostSource::Path)));
        let (q, wanted) = ask(json!({"kind": "secret", "prompt_id": "pw",
            "complete": {"source": "plugin"}}))
        .unwrap();
        assert_eq!(wanted, None);
        assert_eq!(q.prompt_id.as_deref(), Some("pw"));
        assert!(ask(json!({"kind": "text", "complete": {"source": "web"}})).is_err());
    }

    #[test]
    fn a_fixed_list_completes_by_its_start() {
        let list = vec![
            Candidate::cleaned("main", "branch").unwrap(),
            Candidate::cleaned("Maint", "branch").unwrap(),
            Candidate::cleaned("dev", "branch").unwrap(),
        ];
        let found = Completion::from_list(&list, "ma");
        assert_eq!(found.len(), 2);
        let many: Vec<Value> = (0..500).map(|i| json!(format!("c{i}"))).collect();
        assert_eq!(clean_candidates(&Value::Array(many)).len(), MAX_CANDIDATES);
    }

    #[test]
    fn answers_are_checked_against_the_question() {
        let (choose, _) =
            ask(json!({"kind": "choose", "choices": [{"id": "a", "label": "A"}]})).unwrap();
        assert_eq!(
            Answer::Chosen(vec!["a".into()]).to_value(&choose).unwrap(),
            json!({"ids": ["a"]})
        );
        assert!(Answer::Chosen(vec!["z".into()]).to_value(&choose).is_err());
        assert!(Answer::Text("x".into()).to_value(&choose).is_err());
        assert_eq!(
            Answer::Cancelled.to_value(&choose).unwrap(),
            json!({"cancelled": true})
        );
        let (secret, _) = ask(json!({"kind": "secret"})).unwrap();
        let answer = Answer::Secret(Secret::new("canary-7".into()));
        assert_eq!(
            answer.to_value(&secret).unwrap(),
            json!({"text": "canary-7"})
        );
        assert!(answer.for_history().is_none());
        assert!(!format!("{answer:?}").contains("canary"));
        let (confirm, _) = ask(json!({})).unwrap();
        assert_eq!(
            Answer::Confirmed(false).to_value(&confirm).unwrap(),
            json!({"answer": false})
        );
    }

    #[test]
    fn history_is_per_prompt_newest_first_and_capped() {
        let mut history = PromptHistory::default();
        history.remember("git", "branch", "main");
        history.remember("git", "branch", "dev");
        history.remember("git", "branch", "main");
        history.remember("git", "remote", "origin");
        assert_eq!(history.recall("git", "branch"), ["main", "dev"]);
        assert_eq!(history.recall("git", "remote"), ["origin"]);
        assert!(history.recall("other", "branch").is_empty());
        for i in 0..80 {
            history.remember("git", "branch", &format!("b{i}"));
        }
        assert_eq!(history.recall("git", "branch").len(), HISTORY_PER_PROMPT);
        assert_eq!(history.recall("git", "branch")[0], "b79");
    }

    #[test]
    fn a_document_keeps_its_spaces_and_loses_its_escapes() {
        let doc = read_document(
            &params(json!({"title": "Diff", "kind": "diff",
                "text": ["@@ -1 +1 @@", "-  old\u{1B}[31m line", "+\tnew"], "wait": true})),
            "Git",
        )
        .unwrap();
        assert_eq!(doc.title, "Diff (Git)");
        assert_eq!(doc.kind, TextKind::Diff);
        assert!(doc.wait);
        assert_eq!(doc.lines, ["@@ -1 +1 @@", "-  old line", "+    new"]);
        assert!(read_document(&params(json!({"kind": "html", "text": "x"})), "Git").is_err());
        assert!(read_document(&params(json!({})), "Git").is_err());
    }

    #[test]
    fn the_form_and_diff_shapes_read_their_fixtures() {
        let form: Form = serde_json::from_value(json!({
            "title": "Commit",
            "summary": "3 files staged.",
            "fields": [
                {"id": "message", "label": "Message", "kind": "multiline"},
                {"id": "amend", "label": "Amend the last commit", "kind": "checkbox", "default": false}
            ],
            "focus": "message",
            "default_button": "Commit"
        }))
        .unwrap();
        assert_eq!(form.fields.len(), 2);
        let diff: DiffDocuments = serde_json::from_value(json!({
            "title": "a.txt", "old": {"title": "HEAD", "text": "a"}, "new": {"title": "working tree", "text": "b"}
        }))
        .unwrap();
        assert_eq!(diff.new.title, "working tree");
    }
}
