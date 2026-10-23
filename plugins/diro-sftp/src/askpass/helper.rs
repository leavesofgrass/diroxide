//! The helper's side: what the program does when ssh starts it.
//!
//! It reads the prompt from its one argument and `SSH_ASKPASS_PROMPT`
//! from the environment, reaches the endpoint the plugin named for this
//! connection, checks the plugin's proof, sends the prompt with its own,
//! and writes the answer for ssh: a secret or a "yes" and a line break on
//! standard output, or nothing and exit code 0 for a confirmation. On a
//! refusal, a cancel or any failure it writes nothing and exits non-zero,
//! which ssh takes as "no answer". It writes nothing to standard error,
//! which ssh may show, and keeps no copy of what it was told.

use std::io::Write;
use std::time::Duration;

use super::listener::connect;
use super::proof::{LEN, Token, random};
use super::wire::{
    ANSWER, ASK, CHALLENGE, HELLO, STATUS_CONFIRM, STATUS_TEXT, read_frame, write_frame,
};
use super::{ENDPOINT_VARIABLE, TOKEN_VARIABLE};

/// The exit code for "no answer": ssh stops asking.
pub const NO_ANSWER: i32 = 1;

/// How long the helper waits for the owner's answer: the plugin's own
/// waiting limit is five minutes; this is a little more.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(330);

/// What the helper was given.
#[derive(Debug)]
pub struct Request<'a> {
    /// The endpoint (`DIRO_ASKPASS_ENDPOINT`).
    pub endpoint: &'a str,
    /// The token in hexadecimal (`DIRO_ASKPASS_TOKEN`).
    pub token: &'a str,
    /// `SSH_ASKPASS_PROMPT`, when set.
    pub kind: Option<&'a str>,
    /// The prompt's bytes, ssh's one argument.
    pub prompt: &'a [u8],
}

/// What ssh gets.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Text and a line break on standard output, exit 0.
    Text(Vec<u8>),
    /// Nothing written, exit 0.
    Confirm,
    /// Nothing written, exit [`NO_ANSWER`].
    NoAnswer,
}

/// Asks the plugin; any failure is no answer.
pub fn ask(request: &Request<'_>) -> Outcome {
    try_ask(request).unwrap_or(Outcome::NoAnswer)
}

fn try_ask(request: &Request<'_>) -> Option<Outcome> {
    let token = Token::from_hex(request.token)?;
    let mut stream = connect(request.endpoint).ok()?;
    let helper_nonce = random()?;
    write_frame(&mut stream, HELLO, &[&helper_nonce]).ok()?;
    let (kind, parts) = read_frame(&mut stream).ok()?;
    let (plugin_nonce, proof) = match (kind, parts.as_slice()) {
        (CHALLENGE, [nonce, proof]) if nonce.len() == LEN => (nonce.clone(), proof.clone()),
        _ => return None,
    };
    // An endpoint that cannot prove it holds the token gets no prompt.
    if !token.check_plugin(&helper_nonce, &plugin_nonce, &proof) {
        return None;
    }
    let kind = request.kind.unwrap_or("").as_bytes();
    let mine = token.helper_proof(&helper_nonce, &plugin_nonce, kind, request.prompt);
    write_frame(&mut stream, ASK, &[kind, request.prompt, &mine]).ok()?;
    // The owner may take a while to answer.
    stream.set_read_timeout(Some(ANSWER_TIMEOUT)).ok()?;
    let (kind, parts) = read_frame(&mut stream).ok()?;
    match (kind, parts.as_slice()) {
        (ANSWER, [status, text]) if status.as_slice() == [STATUS_TEXT] => {
            Some(Outcome::Text(text.clone()))
        }
        (ANSWER, [status, _]) if status.as_slice() == [STATUS_CONFIRM] => Some(Outcome::Confirm),
        _ => Some(Outcome::NoAnswer),
    }
}

/// Runs the helper with the prompt ssh gave as its one argument and
/// returns the exit code.
pub fn run(prompt: Option<&[u8]>) -> i32 {
    let endpoint = std::env::var(ENDPOINT_VARIABLE).unwrap_or_default();
    let token = std::env::var(TOKEN_VARIABLE).unwrap_or_default();
    let kind = std::env::var("SSH_ASKPASS_PROMPT").ok();
    let outcome = ask(&Request {
        endpoint: &endpoint,
        token: &token,
        kind: kind.as_deref(),
        prompt: prompt.unwrap_or_default(),
    });
    match outcome {
        Outcome::Text(mut text) => {
            text.push(b'\n');
            let mut out = std::io::stdout().lock();
            let written = out.write_all(&text).and_then(|()| out.flush());
            text.fill(0);
            if written.is_ok() { 0 } else { NO_ANSWER }
        }
        Outcome::Confirm => 0,
        Outcome::NoAnswer => NO_ANSWER,
    }
}
