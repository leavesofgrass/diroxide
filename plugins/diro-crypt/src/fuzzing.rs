//! The bodies of the fuzz targets, here so the plugin's own tests replay
//! them, and the fuzz crate under `plugins/diro-crypt/fuzz` stays a thin
//! wrapper (`docs/dev/fuzzing.md`'s shape). Each reads what a hostile
//! file or a host could hand the plugin and must never panic, loop, or
//! allocate by a number it read.

use serde_json::{Map, Value};

use crate::commands::forms;
use crate::{header, lines, names, recipients};

/// A recipients file or a pasted key, and the words of what it refuses.
pub fn recipients(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    match recipients::parse(&text) {
        Ok(keys) => {
            let _ = lines::plan_for_recipients(1, keys.len());
        }
        Err(refused) => {
            let _ = refused.0.len();
        }
    }
}

/// A form's answer as the host could send it: the passphrase form's and
/// the recipient form's readings.
pub fn form_answer(data: &[u8]) {
    let Ok(Value::Object(values)) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let recipient = forms::read_recipient(&values);
    let _ = recipients::parse(&recipient.key);
    let _ = forms::read_passphrase(values);
    let _ = forms::read_passphrase(Map::new());
}

/// An age file's header, armored or not, and the names made from a
/// file's name.
pub fn header(data: &[u8]) {
    let peek = header::peek(data);
    let _ = peek.too_much_work();
    let _ = header::peek_reader(data);
    let name = String::from_utf8_lossy(&data[..data.len().min(300)]);
    let _ = names::decrypted_name(&name);
    let _ = lines::decrypted(&name, &names::encrypted_name(&name));
}
