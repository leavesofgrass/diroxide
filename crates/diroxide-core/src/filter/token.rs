//! The filter's words. Spaces separate words; double quotes hold spaces
//! and operators, with a doubled quote for a quote itself; backslash is
//! never an escape, so a Windows path in quotes is itself.
//!
//! A `(` opens a group only at the start of a word, and a `)` closes one
//! only at the end of a word while a group is open, so `*(1)*` is one
//! pattern. A word that starts with `re:` or `regex:` (or `FIELD:re:`),
//! and the word after `regex`, is a regular expression up to the next
//! space, with its commas, pipes and brackets kept. Outside quotes and
//! regular expressions a pipe or an ampersand is refused with the word to
//! use, and `=`, `!=`, `<`, `>`, `<=` and `>=` are signs of their own.

use std::ops::Range;

use super::error::{FilterError, Problem};

/// The longest filter text, in characters.
pub(crate) const MAX_TEXT_CHARS: usize = 10_000;

/// A comparison sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sign {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
}

impl Sign {
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Sign::Eq => "=",
            Sign::Ne => "!=",
            Sign::Gt => ">",
            Sign::Lt => "<",
            Sign::Ge => ">=",
            Sign::Le => "<=",
        }
    }
}

/// What a word is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A plain word or pattern.
    Word,
    /// A regular expression, kept whole to the next space.
    Raw,
    /// Text in double quotes, without them.
    Quoted,
    /// `(`.
    Open,
    /// `)`.
    Close,
    /// `,` or `;`.
    Comma,
    /// `!` before a term.
    Bang,
    /// A comparison sign.
    Sign(Sign),
}

/// One word of the filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: Kind,
    /// The word's text (a quoted word without its quotes).
    pub(crate) text: String,
    /// Its bytes in the filter.
    pub(crate) span: Range<usize>,
    /// Its number, counted from 1.
    pub(crate) word: usize,
}

impl Token {
    /// Whether this is the plain word `word`, ignoring case.
    pub(crate) fn is_word(&self, word: &str) -> bool {
        self.kind == Kind::Word && self.text.eq_ignore_ascii_case(word)
    }

    /// Whether this can be a value: a word, a pattern or quoted text.
    pub(crate) fn is_value(&self) -> bool {
        matches!(self.kind, Kind::Word | Kind::Raw | Kind::Quoted)
    }
}

/// The length of a regular expression's prefix (`re:`, `regex:`,
/// `name:re:`), when the word has one.
pub(crate) fn regex_prefix(word: &str) -> Option<usize> {
    fn re_at(text: &str) -> Option<usize> {
        let lower = text.get(..6).unwrap_or(text).to_ascii_lowercase();
        if lower.starts_with("regex:") {
            Some(6)
        } else if lower.starts_with("re:") {
            Some(3)
        } else {
            None
        }
    }
    if let Some(n) = re_at(word) {
        return Some(n);
    }
    let (head, _) = split_head(word)?;
    re_at(&word[head.len() + 1..]).map(|n| head.len() + 1 + n)
}

/// A word's field part before a colon, when it is two or more letters and
/// hyphens: `ext:pdf` gives `ext` and `pdf`. A drive letter (`D:\work`)
/// is not a field.
pub(crate) fn split_head(word: &str) -> Option<(&str, &str)> {
    let colon = word.find(':')?;
    let head = &word[..colon];
    let letters = head.len() >= 2
        && head.starts_with(|c: char| c.is_ascii_alphabetic())
        && head.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
    letters.then(|| (head, &word[colon + 1..]))
}

/// The opening and closing brackets in a word, as groups count them: a
/// regular expression's escaped characters and classes do not count.
fn brackets(word: &str, regex: bool) -> (usize, usize) {
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut chars = word.chars();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if regex => {
                chars.next();
            }
            '[' if regex => in_class = true,
            ']' if regex => in_class = false,
            '(' if !in_class => opens += 1,
            ')' if !in_class => closes += 1,
            _ => {}
        }
    }
    (opens, closes)
}

/// How many closing brackets at the end of a word close groups: those
/// with no opening bracket in the word, up to the groups open and the
/// brackets there. A regular expression's escaped characters and classes
/// do not count.
fn closing(word: &str, open_groups: usize, regex: bool) -> usize {
    let (opens, closes) = brackets(word, regex);
    let trailing = word.chars().rev().take_while(|&c| c == ')').count();
    closes.saturating_sub(opens).min(open_groups).min(trailing)
}

/// Whether a bare word reads as itself both alone and before the bracket
/// that closes its group: its brackets balance, or it has more closing
/// ones but none at its end. Otherwise a group's closing bracket joins
/// the word (`0(` and `)` read as `0()`) or the word's own closes the
/// group, so the read-back quotes it (A1f).
pub(crate) fn stays_whole(word: &str, regex: bool) -> bool {
    let (opens, closes) = brackets(word, regex);
    opens == closes || (closes > opens && !word.ends_with(')'))
}

struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
    raw_next: bool,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn push(&mut self, kind: Kind, text: &str, start: usize) {
        let word = self.tokens.len() + 1;
        self.tokens.push(Token {
            kind,
            text: text.to_owned(),
            span: start..self.pos,
            word,
        });
    }

    fn fail(&self, problem: Problem, span: Range<usize>) -> FilterError {
        FilterError::new(problem, self.tokens.len() + 1, span, Vec::new())
    }

    fn run(mut self) -> Result<Vec<Token>, FilterError> {
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
                continue;
            }
            let start = self.pos;
            if self.raw_next && c != '"' {
                self.raw_next = false;
                self.raw_word(start, 0);
                continue;
            }
            self.raw_next = false;
            match c {
                '"' => self.quoted(start)?,
                '(' => {
                    self.pos += 1;
                    self.depth += 1;
                    self.push(Kind::Open, "(", start);
                }
                ')' => {
                    self.pos += 1;
                    self.depth = self.depth.saturating_sub(1);
                    self.push(Kind::Close, ")", start);
                }
                ',' | ';' => {
                    self.pos += 1;
                    self.push(Kind::Comma, ",", start);
                }
                '|' => return Err(self.fail(Problem::Pipe, start..start + 1)),
                '&' => return Err(self.fail(Problem::Ampersand, start..start + 1)),
                '!' if self.rest()[1..].starts_with('=') => self.sign(start, Sign::Ne, 2),
                '!' => {
                    self.pos += 1;
                    self.push(Kind::Bang, "!", start);
                }
                '<' | '>' | '=' => {
                    let doubled = self.rest()[1..].starts_with('=');
                    let (sign, len) = match (c, doubled) {
                        ('<', true) => (Sign::Le, 2),
                        ('<', false) => (Sign::Lt, 1),
                        ('>', true) => (Sign::Ge, 2),
                        ('>', false) => (Sign::Gt, 1),
                        (_, true) => (Sign::Eq, 2),
                        (_, false) => (Sign::Eq, 1),
                    };
                    self.sign(start, sign, len);
                }
                _ => self.word(start),
            }
        }
        Ok(self.tokens)
    }

    fn sign(&mut self, start: usize, sign: Sign, len: usize) {
        self.pos += len;
        self.push(Kind::Sign(sign), sign.text(), start);
    }

    fn quoted(&mut self, start: usize) -> Result<(), FilterError> {
        self.pos += 1;
        let mut value = String::new();
        loop {
            let Some(c) = self.rest().chars().next() else {
                return Err(self.fail(Problem::UnclosedQuote, start..self.text.len()));
            };
            self.pos += c.len_utf8();
            if c == '"' {
                if self.rest().starts_with('"') {
                    self.pos += 1;
                    value.push('"');
                } else {
                    break;
                }
            } else {
                value.push(c);
            }
        }
        self.push(Kind::Quoted, &value, start);
        Ok(())
    }

    /// The end of the run of characters that are not spaces.
    fn run_end(&self, start: usize) -> usize {
        self.text[start..]
            .find(char::is_whitespace)
            .map_or(self.text.len(), |i| start + i)
    }

    /// A regular expression to the next space; `prefix` bytes of it are
    /// `re:` or `FIELD:re:` and do not count for brackets. A comma at its
    /// end is a comma of its own, so `re:a, re:b` is a list and the
    /// read-back's ", match case" stays outside the pattern.
    fn raw_word(&mut self, start: usize, prefix: usize) {
        let mut end = self.run_end(start);
        let comma = end - start > prefix && self.text[..end].ends_with(',');
        if comma {
            end -= 1;
        }
        let word = &self.text[start..end];
        let closes = closing(&word[prefix..], self.depth, true);
        self.pos = end - closes;
        self.push(Kind::Raw, &self.text[start..end - closes], start);
        self.closes(closes);
        if comma {
            let at = self.pos;
            self.pos += 1;
            self.push(Kind::Comma, ",", at);
        }
    }

    fn closes(&mut self, count: usize) {
        for _ in 0..count {
            let at = self.pos;
            self.pos += 1;
            self.depth = self.depth.saturating_sub(1);
            self.push(Kind::Close, ")", at);
        }
    }

    fn word(&mut self, start: usize) {
        let end = self.run_end(start);
        if let Some(prefix) = regex_prefix(&self.text[start..end]) {
            let after = start + prefix;
            if after == end || self.text[after..].starts_with('"') {
                // The pattern is the next word: quoted, or after a space.
                self.pos = after;
                self.push(Kind::Word, &self.text[start..after], start);
                self.raw_next = after == end;
            } else {
                self.raw_word(start, prefix);
            }
            return;
        }
        let rest = &self.text[start..end];
        let mut stop = rest.len();
        for (i, c) in rest.char_indices() {
            let sign = matches!(c, '<' | '>' | '=') || (c == '!' && rest[i + 1..].starts_with('='));
            if sign || matches!(c, '"' | ',' | ';' | '|' | '&') {
                stop = i;
                break;
            }
        }
        let word = &rest[..stop];
        let closes = closing(word, self.depth, false);
        let body = &word[..word.len() - closes];
        self.pos = start + body.len();
        if !body.is_empty() {
            self.push(Kind::Word, body, start);
            self.raw_next = body.eq_ignore_ascii_case("regex");
        }
        self.closes(closes);
    }
}

/// Splits a filter into its words.
pub(crate) fn lex(text: &str) -> Result<Vec<Token>, FilterError> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(FilterError::new(Problem::TooLong, 1, 0..0, Vec::new()));
    }
    Lexer {
        text,
        pos: 0,
        depth: 0,
        raw_next: false,
        tokens: Vec::new(),
    }
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(Kind, String)> {
        lex(text)
            .unwrap()
            .into_iter()
            .map(|t| (t.kind, t.text))
            .collect()
    }

    fn w(text: &str) -> (Kind, String) {
        (Kind::Word, text.to_owned())
    }

    #[test]
    fn words_quotes_and_brackets() {
        assert_eq!(
            kinds("(ext jpg or ext png) and not name thumb*"),
            vec![
                (Kind::Open, "(".into()),
                w("ext"),
                w("jpg"),
                w("or"),
                w("ext"),
                w("png"),
                (Kind::Close, ")".into()),
                w("and"),
                w("not"),
                w("name"),
                w("thumb*"),
            ]
        );
        assert_eq!(kinds("*(1)*"), vec![w("*(1)*")]);
        assert_eq!(
            kinds("(a or *(1))"),
            vec![
                (Kind::Open, "(".into()),
                w("a"),
                w("or"),
                w("*(1)"),
                (Kind::Close, ")".into())
            ]
        );
        assert_eq!(
            kinds("\"say \"\"hi\"\"\" D:\\work"),
            vec![(Kind::Quoted, "say \"hi\"".into()), w("D:\\work")]
        );
        assert_eq!(
            kinds("content:\"total due\""),
            vec![w("content:"), (Kind::Quoted, "total due".into())]
        );
    }

    #[test]
    fn signs_split_words_and_commas_list() {
        assert_eq!(
            kinds("size>=1mb ext:txt,md !hidden size!=0"),
            vec![
                w("size"),
                (Kind::Sign(Sign::Ge), ">=".into()),
                w("1mb"),
                w("ext:txt"),
                (Kind::Comma, ",".into()),
                w("md"),
                (Kind::Bang, "!".into()),
                w("hidden"),
                w("size"),
                (Kind::Sign(Sign::Ne), "!=".into()),
                w("0"),
            ]
        );
        assert_eq!(kinds("hello!"), vec![w("hello!")]);
        assert_eq!(
            kinds("a;b"),
            vec![w("a"), (Kind::Comma, ",".into()), w("b")]
        );
    }

    #[test]
    fn regular_expressions_stay_whole() {
        assert_eq!(
            kinds("re:\\.(jpe?g|png)$ x"),
            vec![(Kind::Raw, "re:\\.(jpe?g|png)$".into()), w("x")]
        );
        assert_eq!(
            kinds("re:\"^IMG_\\d{4}\""),
            vec![w("re:"), (Kind::Quoted, "^IMG_\\d{4}".into())]
        );
        assert_eq!(
            kinds("content:re:\"due \\d+\""),
            vec![w("content:re:"), (Kind::Quoted, "due \\d+".into())]
        );
        assert_eq!(
            kinds("name regex a|b,c"),
            vec![w("name"), w("regex"), (Kind::Raw, "a|b,c".into())]
        );
        assert_eq!(
            kinds("(name regex (a|b))"),
            vec![
                (Kind::Open, "(".into()),
                w("name"),
                w("regex"),
                (Kind::Raw, "(a|b)".into()),
                (Kind::Close, ")".into())
            ]
        );
        assert_eq!(
            kinds("(re:a\\))"),
            vec![
                (Kind::Open, "(".into()),
                (Kind::Raw, "re:a\\)".into()),
                (Kind::Close, ")".into())
            ]
        );
        assert_eq!(
            kinds("name:re:a{1,3}"),
            vec![(Kind::Raw, "name:re:a{1,3}".into())]
        );
        assert_eq!(kinds("re: x|y"), vec![w("re:"), (Kind::Raw, "x|y".into())]);
    }

    #[test]
    fn pipes_ampersands_and_open_quotes_are_refused_at_their_word() {
        let error = lex("*.txt | *.md").unwrap_err();
        assert_eq!(error.problem(), &Problem::Pipe);
        assert_eq!(error.word(), 2);
        assert_eq!(error.span(), 6..7);
        assert_eq!(lex("R&D").unwrap_err().problem(), &Problem::Ampersand);
        assert_eq!(
            lex("a \"open").unwrap_err().problem(),
            &Problem::UnclosedQuote
        );
        assert!(lex("\"a | b & c\"").is_ok());
        assert_eq!(
            lex(&"a".repeat(MAX_TEXT_CHARS + 1)).unwrap_err().problem(),
            &Problem::TooLong
        );
    }

    #[test]
    fn word_positions_count_every_word() {
        let tokens = lex("size over  10 MB").unwrap();
        let words: Vec<usize> = tokens.iter().map(|t| t.word).collect();
        assert_eq!(words, [1, 2, 3, 4]);
        assert_eq!(tokens[2].span, 11..13);
        assert_eq!(split_head("ext:pdf"), Some(("ext", "pdf")));
        assert_eq!(split_head("D:\\x"), None);
        assert_eq!(split_head("re:x"), Some(("re", "x")));
        assert_eq!(split_head("14:30"), None);
        assert_eq!(regex_prefix("name:REGEX:x"), Some(11));
    }
}
