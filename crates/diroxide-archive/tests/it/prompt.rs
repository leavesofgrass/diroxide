//! A scripted masked prompt for the password tests: it answers from a
//! list and records every request, so a test can say what was asked, in
//! which try, and that the canary never reached anything it should not.

#![allow(dead_code)]

use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

use diroxide_core::{Secret, SecretRequest};
use diroxide_vfs::{CancelToken, SecretSource};

/// The canary of ADR-0009: a fixed test string that must never reach a
/// log line, a `Debug` output, a serialized job or an announcement.
pub const CANARY: &str = "canary-7f3a-not-a-password";

/// What the prompt does on one ask.
#[derive(Clone, Debug)]
pub enum Answer {
    /// Types this password.
    Type(&'static str),
    /// Escape: no password.
    Escape,
    /// Escape, and the job is cancelled while the prompt is up.
    Cancel(CancelToken),
}

/// The scripted prompt.
#[derive(Default)]
pub struct Prompt {
    answers: Mutex<Vec<Answer>>,
    asked: Mutex<Vec<SecretRequest>>,
}

impl Prompt {
    /// A prompt that gives these answers in order, then nothing.
    pub fn new(answers: Vec<Answer>) -> Arc<Self> {
        let mut answers = answers;
        answers.reverse();
        Arc::new(Self {
            answers: Mutex::new(answers),
            asked: Mutex::new(Vec::new()),
        })
    }

    /// A prompt that always types `password`.
    pub fn always(password: &'static str) -> Arc<Self> {
        Self::new(vec![Answer::Type(password); 64])
    }

    /// The requests asked so far.
    pub fn asked(&self) -> Vec<SecretRequest> {
        self.asked.lock().unwrap().clone()
    }
}

impl SecretSource for Prompt {
    fn ask(&self, request: &SecretRequest, _cancel: &CancelToken) -> Option<Secret> {
        self.asked.lock().unwrap().push(request.clone());
        match self.answers.lock().unwrap().pop() {
            Some(Answer::Type(text)) => Some(Secret::new(text.to_owned())),
            Some(Answer::Cancel(token)) => {
                token.cancel();
                None
            }
            Some(Answer::Escape) | None => None,
        }
    }
}

/// A zip of AES-256 members under `password`, by the zip crate.
pub fn aes_zip(members: &[(&str, &[u8])], password: &str) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .with_aes_encryption(zip::AesMode::Aes256, password);
    for (name, data) in members {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
