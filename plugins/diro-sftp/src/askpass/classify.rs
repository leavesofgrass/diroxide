//! What ssh is asking (R24 section 5.3): a new host key, its own password
//! prompt, a key's passphrase, a confirmation, a notice, or a question
//! the server wrote.
//!
//! The classifier works on the prompt's text exactly as ssh gave it; the
//! words the owner hears are the plugin's own, built from what is parsed
//! here, and server text is cleaned and only ever placed last. A prompt
//! that does not match one of ssh's own forms exactly is the server's.

use super::fingerprint::Fingerprint;
use super::listener::Prompt;
use crate::lines;

/// The longest server text kept, in characters (R24 section 5.4).
pub const MAX_SERVER_TEXT: usize = 300;

/// What a prompt is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// ssh has never seen this host's key and asks whether to trust it.
    NewHostKey {
        /// The host as ssh named it ("build.example", "[10.0.0.7]:2222").
        host: String,
        /// The key's type ("ED25519").
        key_type: String,
        /// The fingerprint ssh printed.
        fingerprint: Fingerprint,
    },
    /// ssh's own password prompt, `user@host's password: `.
    HostPassword {
        /// The user it names.
        user: String,
        /// The host it names.
        host: String,
    },
    /// A key file's passphrase, `Enter passphrase for key '...': `.
    KeyPassphrase {
        /// The key file's name, without its folder.
        key: String,
    },
    /// A yes or no with no text answer (`SSH_ASKPASS_PROMPT=confirm`).
    Confirm {
        /// ssh's words, cleaned.
        text: String,
    },
    /// A notice that wants no answer, such as "touch your security key"
    /// (`SSH_ASKPASS_PROMPT=none`).
    Notice {
        /// ssh's words, cleaned.
        text: String,
    },
    /// Anything else: keyboard-interactive questions, codes, PINs. Always
    /// asked, never filled in.
    Server {
        /// The server's words, cleaned and cut.
        text: String,
    },
}

/// Server or ssh text made safe to show: control, format and direction
/// characters and terminal escapes removed, cut to [`MAX_SERVER_TEXT`].
pub fn clean(text: &str) -> String {
    lines::cleaned(text, MAX_SERVER_TEXT)
}

/// A host or key-type word from ssh's own prompt: plain characters only.
fn plain_word(text: &str) -> Option<String> {
    let ok = !text.is_empty()
        && text.chars().count() <= 260
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']' | '@')
        });
    ok.then(|| text.to_owned())
}

fn new_host_key(text: &str) -> Option<PromptKind> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    if !lines
        .last()
        .is_some_and(|l| l.starts_with("Are you sure you want to continue connecting"))
    {
        return None;
    }
    let host_line = lines
        .iter()
        .find_map(|l| l.strip_prefix("The authenticity of host '"))?;
    let named = host_line.split('\'').next()?;
    let host = plain_word(named.split(" (").next()?)?;
    let (key_type, fingerprint) = lines.iter().find_map(|l| {
        let (key_type, rest) = l.split_once(" key fingerprint is")?;
        let rest = rest.trim_start_matches(':').trim();
        Some((plain_word(key_type)?, Fingerprint::parse(rest)?))
    })?;
    Some(PromptKind::NewHostKey {
        host,
        key_type,
        fingerprint,
    })
}

fn host_password(text: &str) -> Option<PromptKind> {
    let account = text.strip_suffix("'s password: ")?;
    let (user, host) = account.rsplit_once('@')?;
    Some(PromptKind::HostPassword {
        user: plain_word(user)?,
        host: plain_word(host)?,
    })
}

fn key_passphrase(text: &str) -> Option<PromptKind> {
    let path = text
        .strip_prefix("Enter passphrase for key '")?
        .strip_suffix("': ")?;
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let key = clean(name);
    (!key.is_empty()).then_some(PromptKind::KeyPassphrase { key })
}

/// Tells a prompt's kind.
pub fn classify(prompt: &Prompt) -> PromptKind {
    let text = prompt.text();
    match prompt.kind.as_deref() {
        Some("confirm") => {
            return PromptKind::Confirm { text: clean(&text) };
        }
        Some("none") => {
            return PromptKind::Notice { text: clean(&text) };
        }
        _ => {}
    }
    new_host_key(&text)
        .or_else(|| host_password(&text))
        .or_else(|| key_passphrase(&text))
        .unwrap_or_else(|| PromptKind::Server { text: clean(&text) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str, kind: Option<&str>) -> Prompt {
        Prompt {
            raw: text.as_bytes().to_vec(),
            kind: kind.map(str::to_owned),
        }
    }

    /// The questions spike SF1 captured from both Windows builds.
    #[test]
    fn the_captured_host_key_questions_classify() {
        for text in [
            include_str!("../../../../fixtures/sftp/prompts/new-host-key-windows-9.5p2.txt"),
            include_str!("../../../../fixtures/sftp/prompts/new-host-key-msys-10.3p1.txt"),
        ] {
            let PromptKind::NewHostKey {
                host,
                key_type,
                fingerprint,
            } = classify(&prompt(text, None))
            else {
                panic!("{text}");
            };
            assert_eq!(host, "[127.0.0.1]:2222");
            assert_eq!(key_type, "ED25519");
            assert_eq!(
                fingerprint.original,
                "SHA256:HcIVc9zwwjD8lnpxADs5mQMGnCOBDUkP9dC/28u+FDA"
            );
        }
    }

    #[test]
    fn ssh_s_own_prompts_are_told_apart() {
        assert_eq!(
            classify(&prompt("ada@build.example's password: ", None)),
            PromptKind::HostPassword {
                user: "ada".into(),
                host: "build.example".into()
            }
        );
        assert_eq!(
            classify(&prompt(
                "Enter passphrase for key 'C:\\Users\\ada/.ssh/id_ed25519': ",
                None
            )),
            PromptKind::KeyPassphrase {
                key: "id_ed25519".into()
            }
        );
        assert_eq!(
            classify(&prompt("Allow use of key id_ed25519?", Some("confirm"))),
            PromptKind::Confirm {
                text: "Allow use of key id_ed25519?".into()
            }
        );
        assert_eq!(
            classify(&prompt(
                "Confirm user presence for key ED25519-SK",
                Some("none")
            )),
            PromptKind::Notice {
                text: "Confirm user presence for key ED25519-SK".into()
            }
        );
    }

    /// A server cannot pass its question off as ssh's: anything that is
    /// not exactly one of ssh's forms is the server's, cleaned and cut.
    #[test]
    fn server_text_is_the_servers_and_is_cleaned() {
        for (text, cleaned) in [
            ("Verification code: ", "Verification code:"),
            (
                "(ada@build.example) Password: ",
                "(ada@build.example) Password:",
            ),
            (
                "ada@build.example's password: \u{1B}[2J",
                "ada@build.example's password:",
            ),
            ("evil host's password: ", "evil host's password:"),
            (
                "Are you sure you want to continue connecting (yes/no)? ",
                "Are you sure you want to continue connecting (yes/no)?",
            ),
            ("Enter PIN\u{202E}rof yek: \u{7}", "Enter PINrof yek:"),
        ] {
            assert_eq!(
                classify(&prompt(text, None)),
                PromptKind::Server {
                    text: cleaned.into()
                },
                "{text:?}"
            );
        }
        let long = "x".repeat(1000);
        let PromptKind::Server { text } = classify(&prompt(&long, None)) else {
            panic!();
        };
        assert_eq!(text.chars().count(), MAX_SERVER_TEXT);
    }
}
