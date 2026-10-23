//! The askpass listener, its proof and the helper (R25 commit 51): the
//! plugin's own program run as the helper against an endpoint whose
//! answers a test script gives.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use diro_sftp::askpass::proof::{Token, random};
use diro_sftp::askpass::wire::{ANSWER, ASK, CHALLENGE, HELLO, read_frame, write_frame};
use diro_sftp::askpass::{ENDPOINT_VARIABLE, Listener, Prompt, Reply, TOKEN_VARIABLE};
use diroxide_core::Secret;

use crate::plugin_exe;

/// The one secret of these tests; a log scan fails if it shows anywhere.
const SECRET: &str = "fixture-secret-correct-horse";

fn listener(seen: &Arc<Mutex<Vec<Prompt>>>) -> Listener {
    let seen = Arc::clone(seen);
    let root = std::env::temp_dir();
    Listener::open(
        Some(&root),
        Arc::new(move |prompt: &Prompt| {
            seen.lock().unwrap().push(prompt.clone());
            let text = prompt.text();
            if prompt.kind.as_deref() == Some("confirm") {
                Reply::Confirm
            } else if text.contains("password") {
                Reply::Answer(Secret::new(SECRET.to_owned()))
            } else if text.contains("continue connecting") {
                Reply::Answer(Secret::new("yes".to_owned()))
            } else {
                Reply::Refuse
            }
        }),
    )
    .unwrap()
}

fn helper(endpoint: &str, token: &str, kind: Option<&str>, prompt: &str) -> Output {
    let mut command = Command::new(plugin_exe());
    command
        .arg(prompt)
        .env(ENDPOINT_VARIABLE, endpoint)
        .env(TOKEN_VARIABLE, token)
        .env_remove("SSH_ASKPASS_PROMPT")
        .stdin(Stdio::null());
    if let Some(kind) = kind {
        command.env("SSH_ASKPASS_PROMPT", kind);
    }
    command.output().unwrap()
}

fn token_of(listener: &Listener) -> String {
    listener.askpass_env(plugin_exe()).token
}

/// Nothing the helper writes to standard error, and nothing in what it
/// writes there, may hold a secret: it writes nothing at all.
fn assert_quiet(out: &Output) {
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains(SECRET));
}

#[test]
fn the_helper_proves_itself_and_writes_the_answer_for_ssh() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = listener(&seen);
    let token = token_of(&listener);
    let prompt = "ada@build.example's password: ";
    let out = helper(listener.endpoint(), &token, None, prompt);
    assert_quiet(&out);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, format!("{SECRET}\n").as_bytes());
    let host_key = "Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
    let out = helper(listener.endpoint(), &token, None, host_key);
    assert_quiet(&out);
    assert_eq!(out.stdout, b"yes\n");
    let out = helper(
        listener.endpoint(),
        &token,
        Some("confirm"),
        "Allow use of key?",
    );
    assert_quiet(&out);
    assert_eq!((out.status.code(), out.stdout.len()), (Some(0), 0));
    let out = helper(listener.endpoint(), &token, None, "Verification code: ");
    assert_quiet(&out);
    assert_eq!((out.status.code(), out.stdout.len()), (Some(1), 0));
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].text(), prompt);
    assert_eq!(seen[0].kind, None);
    assert_eq!(seen[2].kind.as_deref(), Some("confirm"));
    assert_eq!(listener.refused(), 0);
}

#[test]
fn a_wrong_token_is_refused_and_three_close_the_endpoint() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = listener(&seen);
    let right = token_of(&listener);
    let wrong = Token::new().unwrap().to_hex();
    for n in 1..=3 {
        let out = helper(
            listener.endpoint(),
            &wrong,
            None,
            "ada@build.example's password: ",
        );
        assert_quiet(&out);
        assert_eq!((out.status.code(), out.stdout.len()), (Some(1), 0));
        for _ in 0..200 {
            if listener.refused() == n {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(listener.refused(), n);
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "a refused helper was answered"
    );
    assert!(!listener.is_open());
    // Closed: even the right token gets no answer now.
    let out = helper(
        listener.endpoint(),
        &right,
        None,
        "ada@build.example's password: ",
    );
    assert_eq!((out.status.code(), out.stdout.len()), (Some(1), 0));
    // A token that is not 64 hexadecimal digits never reaches it at all.
    let out = helper(listener.endpoint(), "not-a-token", None, "x");
    assert_eq!(out.status.code(), Some(1));
}

/// A recorded question is worth nothing on the next connection: the
/// plugin's nonce is new each time, so the old proof no longer holds.
#[test]
fn a_replayed_question_is_refused() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = listener(&seen);
    let token = Token::from_hex(&token_of(&listener)).unwrap();
    let address = listener.endpoint().strip_prefix("tcp:").map(str::to_owned);
    let Some(address) = address else {
        // The Unix socket is the owner's alone; the replay is the same
        // exchange and the TCP case shows it.
        return;
    };
    let prompt = b"ada@build.example's password: ";
    let helper_nonce = random().unwrap();
    let mut first = std::net::TcpStream::connect(&address).unwrap();
    write_frame(&mut first, HELLO, &[&helper_nonce]).unwrap();
    let (_, parts) = read_frame(&mut first).unwrap();
    let plugin_nonce = parts[0].clone();
    let proof = token.helper_proof(&helper_nonce, &plugin_nonce, b"", prompt);
    let mut recorded = Vec::new();
    write_frame(&mut recorded, ASK, &[b"", prompt, &proof]).unwrap();
    first.write_all(&recorded).unwrap();
    let (kind, parts) = read_frame(&mut first).unwrap();
    assert_eq!(kind, ANSWER);
    assert_eq!(parts[1], SECRET.as_bytes());
    // The replay: the same hello, the recorded question.
    let mut second = std::net::TcpStream::connect(&address).unwrap();
    write_frame(&mut second, HELLO, &[&helper_nonce]).unwrap();
    let (kind, _) = read_frame(&mut second).unwrap();
    assert_eq!(kind, CHALLENGE);
    second.write_all(&recorded).unwrap();
    let mut rest = Vec::new();
    let _ = second.read_to_end(&mut rest);
    assert!(rest.is_empty(), "the replay got an answer");
    assert_eq!(seen.lock().unwrap().len(), 1);
    for _ in 0..200 {
        if listener.refused() == 1 {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(listener.refused(), 1);
}

/// An endpoint that cannot prove it holds the token gets no prompt: the
/// helper stops after the hello.
#[test]
fn an_impostor_endpoint_gets_no_prompt() {
    let impostor = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("tcp:{}", impostor.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = impostor.accept().unwrap();
        let (kind, _) = read_frame(&mut stream).unwrap();
        assert_eq!(kind, HELLO);
        write_frame(&mut stream, CHALLENGE, &[&random().unwrap(), &[7u8; 32]]).unwrap();
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
        rest
    });
    let token = Token::new().unwrap().to_hex();
    let out = helper(&endpoint, &token, None, "ada@build.example's password: ");
    assert_quiet(&out);
    assert_eq!((out.status.code(), out.stdout.len()), (Some(1), 0));
    let rest = server.join().unwrap();
    assert!(rest.is_empty(), "the helper sent its prompt to an impostor");
}
