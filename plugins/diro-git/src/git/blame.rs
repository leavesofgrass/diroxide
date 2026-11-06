//! Who changed a line (plan section 5.16; R29 2.4): `git blame --porcelain
//! -L N,N -- PATH`, read into "Line 42: Ada Example, Friday, October 9,
//! 2026: Fix the parser, a1b2c3d". The date is the commit's own, in its
//! own time zone, with the weekday computed from it, never typed.

use diroxide_core::{Timestamp, UtcOffset};

/// What blame says about one line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blame {
    /// The commit's full hash; all zeros for a line not committed yet.
    pub hash: String,
    /// The author's name.
    pub author: String,
    /// The author's time, seconds since 1970.
    pub time: i64,
    /// The author's time zone, seconds east of UTC.
    pub offset: i32,
    /// The commit's subject.
    pub subject: String,
}

impl Blame {
    /// Whether the line is not committed yet.
    pub fn uncommitted(&self) -> bool {
        !self.hash.is_empty() && self.hash.bytes().all(|b| b == b'0')
    }

    /// The line's words, the line and the author first.
    pub fn words(&self, line: u32) -> String {
        if self.uncommitted() {
            return format!("Line {line}: not committed yet");
        }
        let date = Timestamp::from_unix(self.time, 0)
            .to_civil(UtcOffset::from_seconds(self.offset))
            .long_date();
        let short: String = self.hash.chars().take(7).collect();
        format!(
            "Line {line}: {}, {date}: {}, {short}",
            self.author, self.subject
        )
    }
}

/// Reads git's `+HHMM` or `-HHMM` into seconds east of UTC.
fn offset(text: &str) -> i32 {
    let (sign, digits) = match text.as_bytes().first() {
        Some(b'-') => (-1, &text[1..]),
        Some(b'+') => (1, &text[1..]),
        _ => (1, text),
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    let hours: i32 = digits[..2].parse().unwrap_or(0);
    let minutes: i32 = digits[2..].parse().unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Reads `git blame --porcelain` for one line; `None` when it holds no
/// commit line.
pub fn parse(text: &str) -> Option<Blame> {
    let mut lines = text.lines();
    let first = lines.next()?;
    let hash = first.split(' ').next()?;
    if hash.len() < 7 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut blame = Blame {
        hash: hash.to_owned(),
        ..Blame::default()
    };
    for line in lines {
        if line.starts_with('\t') {
            break;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "author" => blame.author = clean(value),
            // Between 1970 and the end of the year 9999: a time outside
            // that is not a commit's.
            "author-time" => blame.time = value.parse().unwrap_or(0).clamp(0, 253_402_300_799),
            "author-tz" => blame.offset = offset(value),
            "summary" => blame.subject = clean(value),
            _ => {}
        }
    }
    Some(blame)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORCELAIN: &str = "f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57 1 1 1\n\
author Ada Example\n\
author-mail <ada@example.invalid>\n\
author-time 1760032800\n\
author-tz -0700\n\
committer Ada Example\n\
summary Fix the parser\n\
filename hello.txt\n\
\tthe line\n";

    #[test]
    fn a_line_reads_author_and_date_first_with_the_weekday_computed() {
        let blame = parse(PORCELAIN).unwrap();
        assert_eq!(blame.author, "Ada Example");
        assert_eq!(blame.offset, -7 * 3600);
        // 1760032800 is 2025-10-09 18:00 UTC: 11:00 AM in -0700, a
        // Thursday.
        assert_eq!(
            blame.words(42),
            "Line 42: Ada Example, Thursday, October 9, 2025: Fix the parser, f2e5e54"
        );
        let words = blame.words(42);
        assert!(diroxide_core::format::within_cells(&words, "Ada Example"));
    }

    #[test]
    fn an_uncommitted_line_and_odd_input_read_plainly() {
        let text = PORCELAIN.replace(
            "f2e5e546b7deda3d5dc87bf4d3ef37e9add9bd57",
            "0000000000000000000000000000000000000000",
        );
        assert_eq!(parse(&text).unwrap().words(3), "Line 3: not committed yet");
        assert!(parse("").is_none());
        assert!(parse("not a hash\n").is_none());
        assert_eq!(offset("+0530"), 5 * 3600 + 30 * 60);
        assert_eq!(offset("junk"), 0);
    }
}
