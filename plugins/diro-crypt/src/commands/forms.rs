//! The two encryption forms (`ui/form`) and the reading of their answers,
//! kept apart from the host so the fuzz target and the tests read
//! answers as the host could send them.
//!
//! The passphrase form has two hidden fields, compared here and never
//! echoed, logged or said; the recipient form a text field for a pasted
//! key, a path field for a recipients file, and the armor checkbox. A
//! field the host left out reads as empty, a field of the wrong kind as
//! empty too.

use age::secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use crate::lines;

/// The passphrase form, with a note read first after a try that did not
/// fit.
pub fn passphrase_form(files: usize, note: Option<&str>) -> Value {
    let summary = match note {
        Some(note) => format!("{note} {}", lines::plan_with_passphrase(files)),
        None => format!(
            "{} {}",
            lines::plan_with_passphrase(files),
            lines::PASSPHRASE_WARNING
        ),
    };
    json!({
        "title": lines::PASSPHRASE_TITLE,
        "summary": summary,
        "fields": [
            {"id": "passphrase", "label": lines::PASSPHRASE_LABEL, "kind": "secret"},
            {"id": "repeat", "label": lines::REPEAT_LABEL, "kind": "secret"},
            {"id": "armor", "label": lines::ARMOR_LABEL, "kind": "checkbox", "default": false},
        ],
        "focus": "passphrase",
        "default_button": lines::ENCRYPT_BUTTON,
    })
}

/// The recipient form, with a note read first after a try that did not
/// fit.
pub fn recipient_form(files: usize, note: Option<&str>) -> Value {
    let lead = format!("Encrypt {}", plural_files(files));
    let summary = match note {
        Some(note) => format!("{note} {lead}."),
        None => format!("{lead}. {}", lines::RECIPIENT_SUMMARY),
    };
    json!({
        "title": lines::RECIPIENT_TITLE,
        "summary": summary,
        "fields": [
            {"id": "key", "label": lines::KEY_LABEL, "kind": "text"},
            {"id": "file", "label": lines::RECIPIENTS_FILE_LABEL, "kind": "path"},
            {"id": "armor", "label": lines::ARMOR_LABEL, "kind": "checkbox", "default": false},
        ],
        "focus": "key",
        "default_button": lines::ENCRYPT_BUTTON,
    })
}

fn plural_files(files: usize) -> String {
    diroxide_core::format::count(files as u64, "file", "files")
}

/// Takes a secret field's text out of a form's values into a wiping
/// string, leaving nothing of it in the map.
pub fn take_secret(values: &mut Map<String, Value>, id: &str) -> Option<SecretString> {
    match values.remove(id) {
        Some(Value::String(text)) => Some(SecretString::from(text)),
        _ => None,
    }
}

/// A checkbox's value; off when missing.
pub fn checked(values: &Map<String, Value>, id: &str) -> bool {
    values.get(id).and_then(Value::as_bool).unwrap_or(false)
}

/// A text or path field's value, trimmed; empty when missing.
pub fn text(values: &Map<String, Value>, id: &str) -> String {
    values
        .get(id)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// What the passphrase form's answer comes to.
pub enum Passphrase {
    /// The two fit: the passphrase and the armor choice.
    Fits(SecretString, bool),
    /// They did not, with the note for the next try.
    Again(&'static str),
}

/// Reads the passphrase form's answer: the two fields compared, an empty
/// one refused. The values are consumed, so no copy of a secret stays.
pub fn read_passphrase(mut values: Map<String, Value>) -> Passphrase {
    let armor = checked(&values, "armor");
    let first = take_secret(&mut values, "passphrase");
    let second = take_secret(&mut values, "repeat");
    match (first, second) {
        (Some(first), _) if first.expose_secret().is_empty() => {
            Passphrase::Again(lines::PASSPHRASE_EMPTY)
        }
        (None, _) => Passphrase::Again(lines::PASSPHRASE_EMPTY),
        (Some(first), Some(second)) if first.expose_secret() == second.expose_secret() => {
            Passphrase::Fits(first, armor)
        }
        _ => Passphrase::Again(lines::PASSPHRASES_DIFFER),
    }
}

/// What the recipient form's answer asks for: a pasted key's text, a
/// recipients file's path, and the armor choice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipientAnswer {
    /// The pasted text.
    pub key: String,
    /// The recipients file's path, empty when none.
    pub file: String,
    /// ASCII armor.
    pub armor: bool,
}

/// Reads the recipient form's answer.
pub fn read_recipient(values: &Map<String, Value>) -> RecipientAnswer {
    RecipientAnswer {
        key: text(values, "key"),
        file: text(values, "file"),
        armor: checked(values, "armor"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    #[test]
    fn two_passphrases_are_compared_and_never_said() {
        let fits = read_passphrase(values(
            json!({"passphrase": "thrown-away-a", "repeat": "thrown-away-a", "armor": true}),
        ));
        match fits {
            Passphrase::Fits(secret, armor) => {
                assert_eq!(secret.expose_secret(), "thrown-away-a");
                assert!(armor);
                assert!(!format!("{secret:?}").contains("thrown-away"));
            }
            Passphrase::Again(why) => panic!("{why}"),
        }
        let differ = read_passphrase(values(
            json!({"passphrase": "thrown-away-a", "repeat": "thrown-away-b"}),
        ));
        assert!(matches!(
            differ,
            Passphrase::Again(lines::PASSPHRASES_DIFFER)
        ));
        assert!(!lines::PASSPHRASES_DIFFER.contains("thrown-away"));
        let empty = read_passphrase(values(json!({"passphrase": "", "repeat": ""})));
        assert!(matches!(empty, Passphrase::Again(lines::PASSPHRASE_EMPTY)));
        let wrong_kind = read_passphrase(values(json!({"passphrase": 7, "repeat": 7})));
        assert!(matches!(
            wrong_kind,
            Passphrase::Again(lines::PASSPHRASE_EMPTY)
        ));
    }

    #[test]
    fn the_forms_read_well_and_start_where_the_owner_types() {
        let form = passphrase_form(3, None);
        assert_eq!(form["title"], "Encrypt with a passphrase");
        assert!(
            form["summary"]
                .as_str()
                .unwrap()
                .starts_with("Encrypt 3 files with a passphrase")
        );
        assert_eq!(form["fields"][0]["kind"], "secret");
        assert_eq!(form["fields"][1]["kind"], "secret");
        assert_eq!(form["focus"], "passphrase");
        let again = passphrase_form(1, Some(lines::PASSPHRASES_DIFFER));
        assert!(
            again["summary"]
                .as_str()
                .unwrap()
                .starts_with("The two passphrases differ.")
        );
        let form = recipient_form(1, None);
        assert_eq!(form["fields"][1]["kind"], "path");
        assert!(
            form["summary"]
                .as_str()
                .unwrap()
                .starts_with("Encrypt 1 file.")
        );
        let answer = read_recipient(&values(json!({"key": "  age1x \n", "armor": true})));
        assert_eq!(answer.key, "age1x");
        assert_eq!(answer.file, "");
        assert!(answer.armor);
    }
}
