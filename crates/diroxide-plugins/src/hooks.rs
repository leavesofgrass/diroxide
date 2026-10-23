//! Hook shapes (R3, section 9; R22, commit 15): what `hook/before` and
//! `hook/after` carry, which events wake a plugin, and what its silence
//! means. Nothing is wired to the job engine in this alpha; the shapes and
//! the decision are fixed here so the engine's hooks (stage 2) find them
//! ready.
//!
//! - **`hook/before`** (request) comes after the engine's plan and before
//!   the confirmation, so a veto spares the owner a question: the event,
//!   the job's id, its items (`from`, `to`, `kind`), a count and a
//!   summary in words, and the destination. The answer is `continue`, or
//!   `cancel` with a reason. A hook cannot change the plan.
//! - **`hook/after`** (notification) says a job ran (its status and each
//!   item's result), a folder was entered, or the selection changed; it is
//!   never waited for.
//! - **Filters** before any message: the event, `schemes`, a `glob` on the
//!   place with `/` separators, and `kind` (`file` or `folder`), so a
//!   plugin is not woken for everything.
//! - **Silence:** an error or a timeout from `hook/before` is what the
//!   manifest's `on_timeout` says, with a warning: continue (the default)
//!   or cancel. It is never a silent veto. Every line ends with the
//!   plugin's name.

use diroxide_core::pattern::Glob;
use diroxide_core::{Message, NameCase, Place};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::PluginError;
use crate::lines;
use crate::manifest::{HookSpec, OnTimeout};
use crate::services::{MAX_ANNOUNCE_CHARS, clean_text};

/// One item a job is about to touch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookItem {
    /// Where it is.
    pub from: String,
    /// Where it goes, for a copy, a move or a rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// `file` or `folder`.
    pub kind: String,
    /// What became of it, in `hook/after`: `done`, `skipped`, `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// `hook/before`'s parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookBefore {
    /// The event, such as `before_delete`.
    pub event: String,
    /// The job's id.
    pub job: u64,
    /// The items, at most the first thousand.
    pub items: Vec<HookItem>,
    /// How many items in all.
    pub count: u64,
    /// The plan in words: "Delete 3 files to the trash".
    pub summary: String,
    /// The destination folder, for a copy or a move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

/// `hook/before`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HookAnswer {
    /// Go on.
    Continue,
    /// Stop the job, with the reason in words.
    Cancel {
        /// Why, meaning first.
        #[serde(default)]
        reason: String,
    },
}

/// `hook/after`'s parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookAfter {
    /// The event: `after_copy`, `enter_folder`, `select` and the rest.
    pub event: String,
    /// The job's id, for a job's event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<u64>,
    /// `done`, `failed` or `cancelled`, for a job's event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The items with their results, for a job's event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<HookItem>,
    /// The folder, for `enter_folder` and `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// How many entries are selected, for `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_count: Option<u64>,
    /// The entry under the cursor, for `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// What the engine does after a `before_*` hook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookDecision {
    /// Go on, saying nothing.
    Continue,
    /// Go on, and say why: the plugin did not answer.
    ContinueWithWarning(Message),
    /// Stop the job, saying why.
    Cancel(Message),
}

/// Whether a hook wakes for an event at a place of a kind.
pub fn wakes(spec: &HookSpec, event: &str, place: &Place, folder: bool) -> bool {
    if spec.event != event {
        return false;
    }
    if !spec.schemes.is_empty() {
        let scheme = place.remote().map_or("file", |r| r.scheme());
        if !spec.schemes.iter().any(|s| s == scheme) {
            return false;
        }
    }
    if let Some(kind) = &spec.kind
        && (kind == "folder") != folder
    {
        return false;
    }
    match &spec.glob {
        None => true,
        Some(glob) => {
            let text = place.to_string().replace('\\', "/");
            Glob::new(glob, NameCase::Ignore).is_ok_and(|g| g.is_match(&text))
        }
    }
}

/// The decision for a `before_*` hook's outcome; `verb` is the job's
/// word ("Copy"), `plugin` its name, last in every line.
pub fn decide(
    spec: &HookSpec,
    outcome: &Result<Value, PluginError>,
    verb: &str,
    plugin: &str,
) -> HookDecision {
    let answer = outcome
        .as_ref()
        .ok()
        .and_then(|v| serde_json::from_value::<HookAnswer>(v.clone()).ok());
    match answer {
        Some(HookAnswer::Continue) => HookDecision::Continue,
        Some(HookAnswer::Cancel { reason }) => {
            let reason = clean_text(&reason, MAX_ANNOUNCE_CHARS);
            HookDecision::Cancel(Message::warning(lines::hook_cancelled(
                verb, &reason, plugin,
            )))
        }
        None => match spec.on_timeout {
            OnTimeout::Continue => HookDecision::ContinueWithWarning(Message::warning(
                lines::hook_silent(verb, false, plugin),
            )),
            OnTimeout::Cancel => {
                HookDecision::Cancel(Message::warning(lines::hook_silent(verb, true, plugin)))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::manifest::Manifest;
    use crate::testing::{FakePlugin, Mode, run_dialogue};

    fn hooks(extra: &str) -> Vec<HookSpec> {
        let text = format!(
            "[plugin]\nid = \"guard\"\nname = \"Guard\"\nversion = \"0.1.0\"\nprotocol = 1\n[run]\ncommand = [\"x\"]\n{extra}"
        );
        let report = Manifest::check(&text);
        let problems = report.text();
        report
            .manifest
            .unwrap_or_else(|| panic!("{problems}"))
            .hooks
    }

    fn p(text: &str) -> Place {
        Place::parse(text).unwrap()
    }

    fn local(path: &str) -> Place {
        p(&if cfg!(windows) {
            format!("D:{}", path.replace('/', "\\"))
        } else {
            path.to_owned()
        })
    }

    #[test]
    fn hooks_parse_with_their_filters() {
        let parsed = hooks(
            "[[hook]]\nevent = \"before_delete\"\nglob = \"**/*.docx\"\nkind = \"file\"\non_timeout = \"cancel\"\ntimeout = \"5s\"\n[[hook]]\nevent = \"after_copy\"\nschemes = [\"sftp\"]\n",
        );
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].is_before());
        assert_eq!(parsed[0].on_timeout, OnTimeout::Cancel);
        assert!(!parsed[1].is_before());
    }

    #[test]
    fn filters_decide_who_is_woken() {
        let parsed = hooks(
            "[[hook]]\nevent = \"before_delete\"\nglob = \"**/*.docx\"\nkind = \"file\"\n[[hook]]\nevent = \"after_copy\"\nschemes = [\"sftp\"]\n",
        );
        let docx = local("/work/report.docx");
        assert!(wakes(&parsed[0], "before_delete", &docx, false));
        assert!(!wakes(&parsed[0], "before_delete", &docx, true));
        assert!(!wakes(&parsed[0], "before_copy", &docx, false));
        assert!(!wakes(
            &parsed[0],
            "before_delete",
            &local("/work/a.txt"),
            false
        ));
        assert!(wakes(
            &parsed[1],
            "after_copy",
            &p("sftp://example.org/x"),
            false
        ));
        assert!(!wakes(&parsed[1], "after_copy", &docx, false));
    }

    #[test]
    fn silence_is_what_on_timeout_says_never_a_quiet_veto() {
        let lenient = &hooks("[[hook]]\nevent = \"before_copy\"\n")[0];
        let strict = &hooks("[[hook]]\nevent = \"before_copy\"\non_timeout = \"cancel\"\n")[0];
        let silent = Err(PluginError::Timeout("Guard".into()));
        assert_eq!(
            decide(lenient, &silent, "Copy", "Guard"),
            HookDecision::ContinueWithWarning(Message::warning("Copy goes on: no answer (Guard)"))
        );
        assert_eq!(
            decide(strict, &silent, "Copy", "Guard"),
            HookDecision::Cancel(Message::warning("Copy stopped: no answer (Guard)"))
        );
        assert_eq!(
            decide(lenient, &Ok(json!({"action": "continue"})), "Copy", "Guard"),
            HookDecision::Continue
        );
        assert_eq!(
            decide(
                lenient,
                &Ok(json!({"action": "cancel", "reason": "keep.txt is protected\u{7}"})),
                "Copy",
                "Guard"
            ),
            HookDecision::Cancel(Message::warning(
                "Copy stopped: keep.txt is protected (Guard)"
            ))
        );
        // An answer that cannot be read is silence, not a veto.
        assert_eq!(
            decide(lenient, &Ok(json!({"action": "maybe"})), "Copy", "Guard"),
            HookDecision::ContinueWithWarning(Message::warning("Copy goes on: no answer (Guard)"))
        );
    }

    #[test]
    fn the_shapes_round_trip() {
        let before = HookBefore {
            event: "before_delete".into(),
            job: 7,
            items: vec![HookItem {
                from: "/w/keep.txt".into(),
                to: None,
                kind: "file".into(),
                result: None,
            }],
            count: 1,
            summary: "Delete 1 file to the trash".into(),
            destination: None,
        };
        let value = serde_json::to_value(&before).unwrap();
        assert_eq!(serde_json::from_value::<HookBefore>(value).unwrap(), before);
        assert_eq!(
            serde_json::to_value(HookAnswer::Cancel {
                reason: "no".into()
            })
            .unwrap(),
            json!({"action": "cancel", "reason": "no"})
        );
        let after: HookAfter = serde_json::from_value(json!({
            "event": "select", "place": "/w", "selection_count": 2, "cursor": "/w/a.txt"
        }))
        .unwrap();
        assert_eq!(after.selection_count, Some(2));
    }

    /// Through the host: the hook's answer, attributed, decides the job.
    #[test]
    fn the_host_asks_a_hook_and_decides() {
        use std::sync::Arc;

        use crate::host::{Host, Launcher, QUIT_BUDGET};
        use crate::testing::{FakeLauncher, plugin_info};

        let mut host = Host::new(&diroxide_config::PluginSettings::default())
            .with_launcher(FakeLauncher::new(Mode::Echo) as Arc<dyn Launcher>);
        host.add(plugin_info("\n[[hook]]\nevent = \"before_delete\"\n"));
        let hook = host.plugin("line-counter").unwrap().manifest.hooks[0].clone();
        let before = |from: &str| HookBefore {
            event: "before_delete".into(),
            job: 1,
            items: vec![HookItem {
                from: from.into(),
                to: None,
                kind: "file".into(),
                result: None,
            }],
            count: 1,
            summary: "Delete 1 file to the trash".into(),
            destination: None,
        };
        assert_eq!(
            host.hook_before("line-counter", &hook, &before("/w/a.txt"), "Delete"),
            HookDecision::Continue
        );
        assert_eq!(
            host.hook_before("line-counter", &hook, &before("/w/keep.txt"), "Delete"),
            HookDecision::Cancel(Message::warning(
                "Delete stopped: keep.txt is protected (Line counter)"
            ))
        );
        let after = HookAfter {
            event: "after_delete".into(),
            job: Some(1),
            status: Some("done".into()),
            items: Vec::new(),
            place: None,
            selection_count: None,
            cursor: None,
        };
        host.hook_after("line-counter", &after).unwrap();
        host.shutdown_all(QUIT_BUDGET);
    }

    /// The fixture dialogue: a veto, a go-ahead, and an after event that
    /// is never answered.
    #[test]
    fn the_hook_dialogue_plays() {
        let text = include_str!("../../../fixtures/plugins/dialogues/hooks.txt");
        run_dialogue(text, FakePlugin::new(Mode::Echo)).unwrap();
    }
}
