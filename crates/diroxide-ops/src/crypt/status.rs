//! gpg's status lines (`--status-fd 1`): the only part of its output
//! diroxide reads. Each line starts `[GNUPG:] `, then a keyword in
//! capitals and its arguments split by spaces (GnuPG's `DETAILS` file).
//! Everything else on standard output and all of standard error is the
//! program's own text, never parsed and never said unfiltered.
//!
//! The reader is bounded: at most [`MAX_LINES`] lines of at most
//! [`MAX_LINE`] bytes each are kept, so a hostile key's long user id
//! cannot make the job hold megabytes.

/// The prefix every status line carries.
pub const PREFIX: &str = "[GNUPG:] ";

/// The most status lines kept from one run.
pub const MAX_LINES: usize = 4096;

/// The longest status line kept, in bytes; a longer one is cut.
pub const MAX_LINE: usize = 4096;

/// One status line: its keyword and arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The keyword: `GOODSIG`, `ENC_TO`.
    pub word: String,
    /// The arguments, in order.
    pub args: Vec<String>,
}

impl Status {
    /// The argument at `index`, or "".
    pub fn arg(&self, index: usize) -> &str {
        self.args.get(index).map_or("", String::as_str)
    }

    /// The arguments from `index` on, joined by one space: a user id.
    pub fn rest(&self, index: usize) -> String {
        self.args
            .get(index..)
            .map(|a| a.join(" "))
            .unwrap_or_default()
    }

    /// The GnuPG error code in an `ERROR` or `FAILURE` line's second
    /// argument: its low 16 bits (`67108949` is 85, "no pinentry";
    /// `67108875` is 11, "bad passphrase"). `None` when it is not a
    /// number.
    pub fn code(&self) -> Option<u32> {
        let raw = self.arg(1);
        let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u64>().ok().map(|n| (n & 0xFFFF) as u32)
    }
}

/// GnuPG's "no pinentry": a passphrase was needed and none could be
/// asked (the first step of a decryption or a signature).
pub const NO_PINENTRY: u32 = 85;

/// GnuPG's "bad passphrase".
pub const BAD_PASSPHRASE: u32 = 11;

/// GnuPG's "no secret key".
pub const NO_SECRET_KEY: u32 = 17;

/// The status lines of a run's standard output, in order.
pub fn parse(stdout: &[u8]) -> Vec<Status> {
    let mut out = Vec::new();
    for line in stdout.split(|b| *b == b'\n') {
        if out.len() >= MAX_LINES {
            break;
        }
        let line = &line[..line.len().min(MAX_LINE)];
        let text = String::from_utf8_lossy(line);
        let text = text.trim_end_matches('\r');
        let Some(rest) = text.strip_prefix(PREFIX) else {
            continue;
        };
        let mut parts = rest.split(' ');
        let word = parts.next().unwrap_or_default();
        if word.is_empty() || !word.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') {
            continue;
        }
        out.push(Status {
            word: word.to_owned(),
            args: parts.map(str::to_owned).collect(),
        });
    }
    out
}

/// The first line with this keyword.
pub fn find<'a>(lines: &'a [Status], word: &str) -> Option<&'a Status> {
    lines.iter().find(|s| s.word == word)
}

/// Whether a line with this keyword is there.
pub fn has(lines: &[Status], word: &str) -> bool {
    find(lines, word).is_some()
}

/// Whether an `ERROR` or `FAILURE` line carries this GnuPG error code.
pub fn has_code(lines: &[Status], code: u32) -> bool {
    lines
        .iter()
        .any(|s| (s.word == "ERROR" || s.word == "FAILURE") && s.code() == Some(code))
}

/// A key id's last four hexadecimal digits, upper case, for "key ending
/// 4E7B"; a fingerprint or a long key id works the same. `None` for
/// anything that is not hexadecimal.
pub fn key_ending(id: &str) -> Option<String> {
    let id = id.trim().trim_start_matches("0x");
    if id.len() < 4 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(id[id.len() - 4..].to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_status_lines_are_read() {
        let out = b"[GNUPG:] NEWSIG\n\
plain words from gpg\n\
[GNUPG:] GOODSIG 168B6F650E9C8469 Ada Example <ada@example.invalid>\r\n\
[GNUPG:] lowercase is not a keyword\n\
[GNUPG:] FAILURE sign 67108949\n";
        let lines = parse(out);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].word, "GOODSIG");
        assert_eq!(lines[1].arg(0), "168B6F650E9C8469");
        assert_eq!(lines[1].rest(1), "Ada Example <ada@example.invalid>");
        assert_eq!(lines[2].code(), Some(NO_PINENTRY));
        assert!(has_code(&lines, NO_PINENTRY));
        assert!(has(&lines, "NEWSIG"));
        assert_eq!(key_ending("168B6F650E9C8469").as_deref(), Some("8469"));
        assert_eq!(key_ending("0xabcd").as_deref(), Some("ABCD"));
        assert_eq!(key_ending("ada@example.invalid"), None);
        let symkey = parse(b"[GNUPG:] ERROR symkey_decrypt.maybe_error 11_BAD_PASSPHRASE\n");
        assert_eq!(symkey[0].code(), Some(BAD_PASSPHRASE));
    }

    #[test]
    fn a_flood_is_bounded() {
        let mut out = Vec::new();
        for _ in 0..(MAX_LINES + 10) {
            out.extend_from_slice(b"[GNUPG:] KEY_CONSIDERED ABCD 0\n");
        }
        assert_eq!(parse(&out).len(), MAX_LINES);
        let long = format!("[GNUPG:] GOODSIG ABCD {}\n", "x".repeat(MAX_LINE * 2));
        assert!(parse(long.as_bytes())[0].rest(1).len() < MAX_LINE);
    }
}
