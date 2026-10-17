//! The parser: words into a tree, with the strict and-or rule (mixing
//! and with or at one level without brackets is an error), and the
//! expected-next list kept at the end of the text for completion.
//!
//! A space between terms is and. A comma joins the values of one term,
//! which means any of them (`*.txt, *.md`, `ext pdf, docx`), or none of
//! them after `is not`. As in the first rung, `not` before a list's
//! first value excludes that value only (`*.pdf, not draft*`).

use crate::pattern::{
    Glob, GlobScope, PatternError, compile_regex, folder_rule, is_glob, regex_reason,
};
use crate::sort::NameCase;

use super::FilterOptions;
use super::dates::{
    CalendarDate, DateProblem, DateValue, DateWord, Duration, TimeUnit, month_from_name,
    parse_clock_time, parse_duration, parse_iso,
};
use super::error::{Expected, FilterError, Keyword, Problem};
use super::token::{Kind, Sign, Token, lex, split_head};
use super::tree::{
    Amount, ContentOp, ContentTest, DateOp, DateTest, Decimal, Expr, Field, FieldShape, Flag,
    KindTest, NumberOp, NumberTest, SizeUnit, Spot, Test, TextOp, TextTest, kind_from_word,
};

/// The deepest brackets may nest.
pub(crate) const MAX_DEPTH: usize = 32;

/// The words that may follow a field to compare it.
const COMPARISON_WORDS: [&str; 19] = [
    "is", "contains", "starts", "ends", "matches", "like", "regex", "over", "under", "at",
    "between", "from", "on", "in", "before", "after", "since", "within", "older",
];

/// The edit distance between two words, by characters.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

/// The nearest of some words to a typed word: within one edit for three
/// letters, two for longer words.
pub(crate) fn nearest<'w>(word: &str, words: impl IntoIterator<Item = &'w str>) -> Option<&'w str> {
    let lower = word.to_lowercase();
    let length = lower.chars().count();
    let limit = match length {
        0..=2 => return None,
        3 => 1,
        _ => 2,
    };
    words
        .into_iter()
        .map(|w| (edit_distance(&lower, w), w))
        .filter(|(d, _)| *d > 0 && *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, w)| w)
}

/// The field nearest a mistyped word.
pub(crate) fn nearest_field(word: &str) -> Option<Field> {
    let words = Field::ALL.iter().map(|f| f.word()).chain(["ext"]);
    nearest(word, words).and_then(Field::from_word)
}

/// The comparison a value gets when none is written: a glob matches, a
/// plain word is contained in a name or path and equals anything else.
pub(crate) fn default_op(field: Field, value: &str) -> TextOp {
    let core = if field == Field::Name {
        folder_rule(value).0
    } else {
        value
    };
    match (field, is_glob(core)) {
        (_, true) => TextOp::Matches,
        (Field::Name | Field::Path, false) => TextOp::Contains,
        _ => TextOp::Is,
    }
}

/// The pattern after `re:` or `regex:`, ignoring case.
pub(crate) fn strip_regex_prefix(text: &str) -> Option<&str> {
    for prefix in ["regex:", "re:"] {
        if text
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            return Some(&text[prefix.len()..]);
        }
    }
    None
}

/// The filter problem a pattern module error is.
pub(crate) fn pattern_problem(error: &PatternError, source: &str) -> Problem {
    match error {
        PatternError::UnclosedBracket(_) => Problem::GlobBracket,
        PatternError::TooLong => Problem::PatternTooLong,
        PatternError::TooComplex => Problem::PatternTooComplex,
        PatternError::LookAround => Problem::LookAround,
        PatternError::Backreference => Problem::Backreference,
        _ => Problem::BadRegex(
            regex_reason(source).unwrap_or_else(|| "it could not be read".to_owned()),
        ),
    }
}

/// Compiles a text test's pattern once, so its problem is said with its
/// word.
fn check_text(field: Field, op: TextOp, value: &str, case: NameCase) -> Result<(), Problem> {
    match op {
        TextOp::Matches => {
            let (core, scope) = match field {
                Field::Name => (folder_rule(value).0, GlobScope::Name),
                Field::Path => (value, GlobScope::Path),
                _ => (value, GlobScope::Name),
            };
            Glob::with_scope(core, case, scope)
                .map(drop)
                .map_err(|e| pattern_problem(&e, core))
        }
        TextOp::Regex => compile_regex(value, case)
            .map(drop)
            .map_err(|e| pattern_problem(&e, value)),
        _ => Ok(()),
    }
}

fn wrap(negate: bool, expr: Expr) -> Expr {
    if negate {
        Expr::Not(Box::new(expr))
    } else {
        expr
    }
}

/// A list's values: any of the positive ones (all of them after `is
/// not`), and none of those after `not`.
fn combine(positives: Vec<Expr>, negatives: Vec<Expr>, all: bool) -> Expr {
    let positive = (!positives.is_empty()).then(|| {
        if all {
            Expr::and(positives)
        } else {
            Expr::or(positives)
        }
    });
    let negative = (!negatives.is_empty()).then(|| Expr::Not(Box::new(Expr::or(negatives))));
    match (positive, negative) {
        (Some(p), Some(n)) => Expr::and(vec![p, n]),
        (Some(one), None) | (None, Some(one)) => one,
        (None, None) => Expr::And(Vec::new()),
    }
}

fn set_case(expr: &mut Expr, case: NameCase) {
    if let Expr::Test { test, .. } = expr {
        match test {
            Test::Text(text) => text.case = case,
            Test::Content(content) => content.case = case,
            _ => {}
        }
    }
}

fn is_digits(token: &Token) -> bool {
    token.kind == Kind::Word
        && !token.text.is_empty()
        && token.text.bytes().all(|b| b.is_ascii_digit())
}

fn spot(token: &Token) -> Spot {
    Spot::new(token.word, token.span.clone())
}

/// What a date comparison wants after it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Date,
    Duration,
    Either,
}

/// A date or a duration, as read.
enum When {
    Date(DateValue),
    Duration(Duration),
}

/// A number comparison, before its amounts are read.
enum NumberCmp {
    Plain,
    Is,
    IsNot,
    Over,
    Under,
    AtLeast,
    AtMost,
    Between,
}

/// A date comparison, before its date is read.
enum DateCmp {
    Plain,
    On,
    Before,
    Since,
    After,
    OnOrBefore,
    Between,
    Within,
    Older,
    Sign(Sign),
}

pub(crate) struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    options: &'a FilterOptions,
    expected: Vec<Expected>,
    end: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(text: &str, options: &'a FilterOptions) -> Result<Self, FilterError> {
        Ok(Self {
            tokens: lex(text)?,
            pos: 0,
            options,
            expected: Vec::new(),
            end: text.len(),
        })
    }

    /// Reads the whole text.
    pub(crate) fn run(&mut self) -> Result<Expr, FilterError> {
        if self.tokens.is_empty() {
            self.want(Expected::Term);
            return Err(self.fail_end(Problem::Empty));
        }
        let expr = self.expr(0)?;
        if self.peek().is_some() {
            return Err(self.fail_here(Problem::ExtraBracket, Vec::new()));
        }
        Ok(expr)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.pos + ahead)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn next_is(&self, word: &str) -> bool {
        self.peek().is_some_and(|t| t.is_word(word))
    }

    fn next_kind(&self, kind: Kind) -> bool {
        self.peek().is_some_and(|t| t.kind == kind)
    }

    /// Notes what could come here, when here is the end of the text.
    fn want(&mut self, expected: Expected) {
        if self.pos >= self.tokens.len() && !self.expected.contains(&expected) {
            self.expected.push(expected);
        }
    }

    fn end_word(&self) -> usize {
        self.tokens.last().map_or(1, |t| t.word + 1)
    }

    fn fail_end(&self, problem: Problem) -> FilterError {
        FilterError::new(
            problem,
            self.end_word(),
            self.end..self.end,
            self.expected.clone(),
        )
    }

    fn fail_at(&self, token: &Token, problem: Problem, expected: Vec<Expected>) -> FilterError {
        FilterError::new(problem, token.word, token.span.clone(), expected)
    }

    fn fail_here(&self, problem: Problem, expected: Vec<Expected>) -> FilterError {
        match self.peek() {
            Some(token) => self.fail_at(token, problem, expected),
            None => self.fail_end(problem),
        }
    }

    fn expr(&mut self, depth: usize) -> Result<Expr, FilterError> {
        let mut parts = vec![self.unary(depth)?];
        let mut joiner: Option<Keyword> = None;
        loop {
            let Some(token) = self.peek() else {
                self.want(Expected::Joiner);
                if depth > 0 {
                    self.want(Expected::CloseBracket);
                }
                self.want(Expected::Term);
                break;
            };
            let (this, explicit) = if token.is_word("and") {
                (Keyword::And, true)
            } else if token.is_word("or") {
                (Keyword::Or, true)
            } else {
                match token.kind {
                    Kind::Close => break,
                    Kind::Comma if self.at_hidden_note() => {
                        self.pos += 3;
                        break;
                    }
                    Kind::Comma => return Err(self.fail_here(Problem::CommaNotHere, Vec::new())),
                    Kind::Sign(sign) => {
                        return Err(
                            self.fail_here(Problem::SignNeedsField(sign.text()), Vec::new())
                        );
                    }
                    _ => (Keyword::And, false),
                }
            };
            if joiner.is_some_and(|previous| previous != this) {
                return Err(self.fail_here(Problem::MixedAndOr, Vec::new()));
            }
            joiner = Some(this);
            if explicit {
                self.bump();
                match self.peek() {
                    None => {
                        self.want(Expected::Term);
                        return Err(self.fail_end(Problem::NothingAfter(this)));
                    }
                    Some(t) if t.kind == Kind::Close || t.is_word("and") || t.is_word("or") => {
                        return Err(
                            self.fail_here(Problem::NothingAfter(this), vec![Expected::Term])
                        );
                    }
                    Some(_) => {}
                }
            }
            parts.push(self.unary(depth)?);
        }
        Ok(match joiner {
            Some(Keyword::Or) => Expr::or(parts),
            _ => Expr::and(parts),
        })
    }

    fn unary(&mut self, depth: usize) -> Result<Expr, FilterError> {
        let Some(token) = self.peek().cloned() else {
            self.want(Expected::Term);
            return Err(self.fail_end(Problem::Empty));
        };
        match token.kind {
            Kind::Open => {
                self.bump();
                if depth + 1 > MAX_DEPTH {
                    return Err(self.fail_at(&token, Problem::TooDeep, Vec::new()));
                }
                match self.peek() {
                    None => {
                        self.want(Expected::Term);
                        let expected = self.expected.clone();
                        return Err(self.fail_at(&token, Problem::UnclosedBracket, expected));
                    }
                    Some(t) if t.kind == Kind::Close => {
                        return Err(self.fail_here(Problem::EmptyBrackets, vec![Expected::Term]));
                    }
                    Some(_) => {}
                }
                let inner = self.expr(depth + 1)?;
                if self.next_kind(Kind::Close) {
                    self.bump();
                    Ok(inner)
                } else {
                    let expected = self.expected.clone();
                    Err(self.fail_at(&token, Problem::UnclosedBracket, expected))
                }
            }
            Kind::Bang => self.not(depth),
            Kind::Word if token.is_word("not") => self.not(depth),
            Kind::Word if token.is_word("and") => Err(self.fail_at(
                &token,
                Problem::NothingBefore(Keyword::And),
                vec![Expected::Term],
            )),
            Kind::Word if token.is_word("or") => Err(self.fail_at(
                &token,
                Problem::NothingBefore(Keyword::Or),
                vec![Expected::Term],
            )),
            Kind::Close => Err(self.fail_at(&token, Problem::ExtraBracket, Vec::new())),
            Kind::Comma => Err(self.fail_at(&token, Problem::CommaNotHere, Vec::new())),
            Kind::Sign(sign) => {
                Err(self.fail_at(&token, Problem::SignNeedsField(sign.text()), Vec::new()))
            }
            Kind::Word | Kind::Raw | Kind::Quoted => self.test(false),
        }
    }

    /// `not` or `!`: binds to the next test, bracket or `not`; before a
    /// list, to its first value.
    fn not(&mut self, depth: usize) -> Result<Expr, FilterError> {
        self.bump();
        match self.peek() {
            None => {
                self.want(Expected::Term);
                Err(self.fail_end(Problem::NothingAfter(Keyword::Not)))
            }
            Some(t) if t.kind == Kind::Close || t.is_word("and") || t.is_word("or") => {
                Err(self.fail_here(Problem::NothingAfter(Keyword::Not), vec![Expected::Term]))
            }
            Some(t) if t.is_value() && !t.is_word("not") => self.test(true),
            Some(_) => Ok(Expr::Not(Box::new(self.unary(depth)?))),
        }
    }

    fn test(&mut self, negate: bool) -> Result<Expr, FilterError> {
        let mut case = None;
        while let Some(token) = self.peek() {
            if token.is_word("case") {
                case = Some(NameCase::Respect);
            } else if token.is_word("anycase") {
                case = Some(NameCase::Ignore);
            } else {
                break;
            }
            self.bump();
        }
        let Some(token) = self.peek().cloned() else {
            self.want(Expected::Term);
            return Err(self.fail_end(Problem::CaseNeedsText));
        };
        if !token.is_value() {
            return Err(self.fail_at(&token, Problem::CaseNeedsText, Vec::new()));
        }
        if token.kind == Kind::Quoted {
            return self.bare(case, negate);
        }
        if let Some((head, rest)) = split_head(&token.text) {
            let lower = head.to_ascii_lowercase();
            if lower == "re" || lower == "regex" {
                return self.bare(case, negate);
            }
            let (head_len, rest) = (head.len(), rest.to_owned());
            if let Some(field) = Field::from_word(head) {
                self.split_current(head_len, &rest);
                return self.field(field, case, negate);
            }
            if lower == "state" {
                self.split_current(head_len, &rest);
                return self.state(case, negate);
            }
            let problem = Problem::UnknownField {
                nearest: nearest_field(head),
                word: head.to_owned(),
            };
            return Err(self.fail_at(&token, problem, Vec::new()));
        }
        if token.kind == Kind::Word {
            if let Some(field) = Field::from_word(&token.text) {
                self.bump();
                return self.field(field, case, negate);
            }
            if let Some(flag) = Flag::from_word(&token.text) {
                if case.is_some() {
                    return Err(self.fail_at(&token, Problem::CaseNeedsText, Vec::new()));
                }
                self.bump();
                let test = Expr::Test {
                    test: Test::Flag(flag),
                    at: spot(&token),
                };
                return Ok(wrap(negate, test));
            }
            if token.is_word("state") {
                self.bump();
                return self.state(case, negate);
            }
            if let Some(next) = self.peek_at(1) {
                let sign = matches!(next.kind, Kind::Sign(_));
                let comparison = next.kind == Kind::Word
                    && COMPARISON_WORDS
                        .iter()
                        .any(|w| next.text.eq_ignore_ascii_case(w));
                let near = nearest_field(&token.text);
                if sign || (comparison && near.is_some()) {
                    let problem = Problem::UnknownField {
                        word: token.text.clone(),
                        nearest: near,
                    };
                    return Err(self.fail_at(&token, problem, Vec::new()));
                }
            }
        }
        self.bare(case, negate)
    }

    /// Splits `FIELD:rest` into the field word and its value, which keeps
    /// the word's number.
    fn split_current(&mut self, head_len: usize, rest: &str) {
        let token = self.tokens[self.pos].clone();
        let cut = token.span.start + head_len + 1;
        self.tokens[self.pos] = Token {
            kind: Kind::Word,
            text: token.text[..head_len].to_owned(),
            span: token.span.start..cut,
            word: token.word,
        };
        if !rest.is_empty() {
            let kind = if token.kind == Kind::Raw {
                Kind::Raw
            } else {
                Kind::Word
            };
            let value = Token {
                kind,
                text: rest.to_owned(),
                span: cut..token.span.end,
                word: token.word,
            };
            self.tokens.insert(self.pos + 1, value);
        }
        self.pos += 1;
    }

    /// A word or pattern with no field: a name test, or a list of them.
    fn bare(&mut self, case: Option<NameCase>, negate: bool) -> Result<Expr, FilterError> {
        self.list_of(Field::Name, false, negate, |p| {
            p.text_value(Field::Name, None, case)
        })
    }

    fn field(
        &mut self,
        field: Field,
        case: Option<NameCase>,
        negate: bool,
    ) -> Result<Expr, FilterError> {
        let first = self.tokens[self.pos - 1].clone();
        if case.is_some() && !field.has_case() {
            return Err(self.fail_at(&first, Problem::CaseNeedsText, Vec::new()));
        }
        let single = |parser: &Self, test: Test| {
            let last = &parser.tokens[parser.pos - 1];
            Expr::Test {
                test,
                at: Spot::new(first.word, first.span.start..last.span.end),
            }
        };
        // A test of one value is placed from its field word; the values of
        // a list each keep their own word.
        let whole = |parser: &Self, expr: Expr| match expr {
            Expr::Test { test, .. } => single(parser, test),
            list => list,
        };
        let expr = match field.shape() {
            FieldShape::Text => {
                let op = self.text_op(field);
                let all = op == Some(TextOp::IsNot);
                let list = self.list_of(field, all, false, |p| p.text_value(field, op, case))?;
                whole(self, list)
            }
            FieldShape::Kind => {
                let list = self.kind_term()?;
                whole(self, list)
            }
            FieldShape::Size | FieldShape::Count => {
                let test = self.number_term(field)?;
                single(self, test)
            }
            FieldShape::Date => {
                let (test, not) = self.date_term(field)?;
                wrap(not, single(self, test))
            }
            FieldShape::Content => {
                let test = self.content_term(case)?;
                single(self, test)
            }
        };
        Ok(wrap(negate, expr))
    }

    /// One value, then more after commas; `, match case` or `, any case`
    /// ends the list and sets the last value's case.
    fn list_of(
        &mut self,
        field: Field,
        all: bool,
        negate_first: bool,
        mut item: impl FnMut(&mut Self) -> Result<Expr, FilterError>,
    ) -> Result<Expr, FilterError> {
        let mut positives = Vec::new();
        let mut negatives = Vec::new();
        let mut negate = negate_first;
        loop {
            let one = item(self)?;
            let last_negative = negate;
            if negate {
                negatives.push(one);
            } else {
                positives.push(one);
            }
            negate = false;
            if !self.next_kind(Kind::Comma) || self.at_hidden_note() {
                break;
            }
            if let Some(case) = self.case_annotation(field)? {
                let last = if last_negative {
                    negatives.last_mut()
                } else {
                    positives.last_mut()
                };
                if let Some(last) = last {
                    set_case(last, case);
                }
                break;
            }
            self.bump();
            if self.next_is("not") {
                self.bump();
                negate = true;
            }
        }
        Ok(combine(positives, negatives, all))
    }

    /// Whether the text ends with the read-back's note ", including
    /// hidden", which says what a positive `hidden` test already means and
    /// is left out of the tree.
    fn at_hidden_note(&self) -> bool {
        self.next_kind(Kind::Comma)
            && self.peek_at(1).is_some_and(|t| t.is_word("including"))
            && self.peek_at(2).is_some_and(|t| t.is_word("hidden"))
            && self.peek_at(3).is_none()
    }

    /// At a comma: `, match case` or `, any case`.
    fn case_annotation(&mut self, field: Field) -> Result<Option<NameCase>, FilterError> {
        let Some(first) = self.peek_at(1) else {
            return Ok(None);
        };
        let case = if first.is_word("match") {
            NameCase::Respect
        } else if first.is_word("any") {
            NameCase::Ignore
        } else {
            return Ok(None);
        };
        if !self.peek_at(2).is_some_and(|t| t.is_word("case")) {
            return Ok(None);
        }
        if !field.has_case() {
            let first = first.clone();
            return Err(self.fail_at(&first, Problem::CaseNeedsText, Vec::new()));
        }
        self.pos += 3;
        Ok(Some(case))
    }

    fn value_token(&mut self, field: Field) -> Result<Token, FilterError> {
        match self.peek() {
            Some(token) if token.is_value() => {}
            Some(token) => {
                return Err(self.fail_at(
                    token,
                    Problem::NeedsValue(field),
                    vec![Expected::Value(field)],
                ));
            }
            None => {
                self.want(Expected::Value(field));
                return Err(self.fail_end(Problem::NeedsValue(field)));
            }
        }
        let token = self.tokens[self.pos].clone();
        self.pos += 1;
        Ok(token)
    }

    fn text_op(&mut self, field: Field) -> Option<TextOp> {
        let Some(token) = self.peek().cloned() else {
            self.want(Expected::Comparison(field));
            self.want(Expected::Value(field));
            return None;
        };
        let op = match token.kind {
            Kind::Sign(Sign::Eq) => TextOp::Is,
            Kind::Sign(Sign::Ne) => TextOp::IsNot,
            Kind::Word => match token.text.to_ascii_lowercase().as_str() {
                "is" => {
                    self.bump();
                    if self.next_is("not") {
                        self.bump();
                        return Some(TextOp::IsNot);
                    }
                    return Some(TextOp::Is);
                }
                "starts" | "ends" => {
                    self.bump();
                    if self.next_is("with") {
                        self.bump();
                    }
                    return Some(if token.is_word("starts") {
                        TextOp::StartsWith
                    } else {
                        TextOp::EndsWith
                    });
                }
                "contains" => TextOp::Contains,
                "matches" => TextOp::Matches,
                "like" => TextOp::Like,
                "regex" => TextOp::Regex,
                _ => return None,
            },
            _ => return None,
        };
        self.bump();
        Some(op)
    }

    fn resolve_case(&self, regex: bool, value: &str) -> NameCase {
        if regex {
            self.options.case.resolve_regex(value)
        } else {
            self.options.case.resolve(value)
        }
    }

    fn text_value(
        &mut self,
        field: Field,
        op: Option<TextOp>,
        case: Option<NameCase>,
    ) -> Result<Expr, FilterError> {
        let token = self.value_token(field)?;
        let mut at = spot(&token);
        let mut value = token.text.clone();
        let mut op = op;
        if op.is_none()
            && token.kind != Kind::Quoted
            && let Some(pattern) = strip_regex_prefix(&value)
        {
            op = Some(TextOp::Regex);
            value = pattern.to_owned();
            if value.is_empty() {
                let next = self.value_token(field)?;
                at = Spot::new(token.word, token.span.start..next.span.end);
                value = next.text;
            }
        }
        if field == Field::Extension {
            value = value.trim_start_matches('.').to_owned();
        }
        if value.is_empty() {
            return Err(self.fail_at(
                &token,
                Problem::NeedsValue(field),
                vec![Expected::Value(field)],
            ));
        }
        let op = op.unwrap_or_else(|| default_op(field, &value));
        let case = if field.has_case() {
            case.unwrap_or_else(|| self.resolve_case(op == TextOp::Regex, &value))
        } else {
            NameCase::Ignore
        };
        if let Err(problem) = check_text(field, op, &value, case) {
            return Err(FilterError::new(problem, at.word(), at.span(), Vec::new()));
        }
        let value = self.canonical(field, value);
        Ok(Expr::Test {
            test: Test::Text(TextTest {
                field,
                op,
                value,
                case,
            }),
            at,
        })
    }

    /// A value as the tree keeps it.
    fn canonical(&self, _field: Field, value: String) -> String {
        value
    }

    fn kind_term(&mut self) -> Result<Expr, FilterError> {
        let is = match self.peek() {
            Some(t) if t.kind == Kind::Sign(Sign::Eq) => {
                self.bump();
                true
            }
            Some(t) if t.kind == Kind::Sign(Sign::Ne) => {
                self.bump();
                false
            }
            Some(t) if t.is_word("is") => {
                self.bump();
                if self.next_is("not") {
                    self.bump();
                    false
                } else {
                    true
                }
            }
            Some(_) => true,
            None => {
                self.want(Expected::Comparison(Field::Kind));
                true
            }
        };
        self.list_of(Field::Kind, !is, false, |p| {
            let token = p.value_token(Field::Kind)?;
            match kind_from_word(&token.text) {
                Some(kind) => Ok(Expr::Test {
                    test: Test::Kind(KindTest { is, kind }),
                    at: spot(&token),
                }),
                None => Err(p.fail_at(
                    &token,
                    Problem::UnknownKind(token.text.clone()),
                    vec![Expected::Value(Field::Kind)],
                )),
            }
        })
    }

    fn number_cmp(&mut self, field: Field) -> Result<NumberCmp, FilterError> {
        let Some(token) = self.peek().cloned() else {
            self.want(Expected::Comparison(field));
            self.want(Expected::Value(field));
            return Ok(NumberCmp::Plain);
        };
        let cmp = match token.kind {
            Kind::Sign(Sign::Eq) => NumberCmp::Is,
            Kind::Sign(Sign::Ne) => NumberCmp::IsNot,
            Kind::Sign(Sign::Gt) => NumberCmp::Over,
            Kind::Sign(Sign::Lt) => NumberCmp::Under,
            Kind::Sign(Sign::Ge) => NumberCmp::AtLeast,
            Kind::Sign(Sign::Le) => NumberCmp::AtMost,
            Kind::Word => match token.text.to_ascii_lowercase().as_str() {
                "is" => {
                    self.bump();
                    if self.next_is("not") {
                        self.bump();
                        return Ok(NumberCmp::IsNot);
                    }
                    return Ok(NumberCmp::Is);
                }
                "at" => {
                    self.bump();
                    if self.next_is("least") {
                        self.bump();
                        return Ok(NumberCmp::AtLeast);
                    }
                    if self.next_is("most") {
                        self.bump();
                        return Ok(NumberCmp::AtMost);
                    }
                    self.want(Expected::Comparison(field));
                    return Err(self.fail_here(
                        Problem::NeedsValue(field),
                        vec![Expected::Comparison(field)],
                    ));
                }
                "over" => NumberCmp::Over,
                "under" => NumberCmp::Under,
                "between" | "from" => NumberCmp::Between,
                _ => return Ok(NumberCmp::Plain),
            },
            _ => return Ok(NumberCmp::Plain),
        };
        self.bump();
        Ok(cmp)
    }

    fn range_joiner(&mut self) -> Result<(), FilterError> {
        match self.peek() {
            Some(t) if t.is_word("and") || t.is_word("to") => {
                self.bump();
                Ok(())
            }
            Some(_) => Err(self.fail_here(Problem::RangeNeedsAnd, vec![Expected::RangeJoiner])),
            None => {
                self.want(Expected::RangeJoiner);
                Err(self.fail_end(Problem::RangeNeedsAnd))
            }
        }
    }

    /// A number in text, and the unit written right after it.
    fn number_text(
        &self,
        text: &str,
        field: Field,
        token: &Token,
    ) -> Result<(Decimal, Option<SizeUnit>), FilterError> {
        let split = text
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(text.len());
        let (digits, unit) = text.split_at(split);
        let fail = |problem: Problem, expected: Vec<Expected>| {
            FilterError::new(problem, token.word, token.span.clone(), expected)
        };
        if digits.is_empty() || digits == "." {
            return Err(fail(
                Problem::NeedsValue(field),
                vec![Expected::Value(field)],
            ));
        }
        let Some(number) = Decimal::parse(digits) else {
            let many = digits.bytes().filter(u8::is_ascii_digit).count() > 18;
            let problem = if many {
                Problem::NumberTooLarge
            } else {
                Problem::NeedsValue(field)
            };
            return Err(fail(problem, Vec::new()));
        };
        if unit.is_empty() {
            return Ok((number, None));
        }
        if field == Field::Depth {
            return Err(fail(Problem::DepthHasNoUnit, Vec::new()));
        }
        match SizeUnit::from_word(unit) {
            Some(unit) => Ok((number, Some(unit))),
            None => Err(fail(unit_problem(unit), vec![Expected::SizeUnit])),
        }
    }

    /// A unit as the next word, when there is one.
    fn unit_after(&mut self, field: Field) -> Result<Option<SizeUnit>, FilterError> {
        let Some(token) = self.peek().cloned() else {
            if field == Field::Size {
                self.want(Expected::SizeUnit);
            }
            return Ok(None);
        };
        if token.kind != Kind::Word {
            return Ok(None);
        }
        if let Some(unit) = SizeUnit::from_word(&token.text) {
            if field == Field::Depth {
                return Err(self.fail_at(&token, Problem::DepthHasNoUnit, Vec::new()));
            }
            self.bump();
            return Ok(Some(unit));
        }
        if field == Field::Size && looks_like_unit(&token.text) {
            return Err(self.fail_at(&token, unit_problem(&token.text), vec![Expected::SizeUnit]));
        }
        Ok(None)
    }

    /// A number and its unit, if one was written.
    fn number(&mut self, field: Field) -> Result<(Decimal, Option<SizeUnit>, Token), FilterError> {
        let token = self.value_token(field)?;
        let (number, unit) = self.number_text(&token.text, field, &token)?;
        let unit = match unit {
            Some(unit) => Some(unit),
            None => self.unit_after(field)?,
        };
        Ok((number, unit, token))
    }

    fn amount(
        &self,
        field: Field,
        number: Decimal,
        unit: Option<SizeUnit>,
        token: &Token,
    ) -> Result<Amount, FilterError> {
        let amount = Amount {
            number,
            unit: (field == Field::Size).then(|| unit.unwrap_or(SizeUnit::Bytes)),
        };
        match amount.value() {
            Some(_) => Ok(amount),
            None => Err(self.fail_at(token, Problem::NumberTooLarge, Vec::new())),
        }
    }

    /// Two ends of a range; the first takes the second's unit when it has
    /// none ("between 1 and 5 MB").
    fn range(
        &self,
        field: Field,
        first: (Decimal, Option<SizeUnit>, &Token),
        second: (Decimal, Option<SizeUnit>, &Token),
    ) -> Result<NumberOp, FilterError> {
        let low = self.amount(field, first.0, first.1.or(second.1), first.2)?;
        let high = self.amount(field, second.0, second.1, second.2)?;
        Ok(NumberOp::Between(low, high))
    }

    fn number_term(&mut self, field: Field) -> Result<Test, FilterError> {
        let cmp = self.number_cmp(field)?;
        let op = match cmp {
            NumberCmp::Between => {
                let (a, ua, ta) = self.number(field)?;
                self.range_joiner()?;
                let (b, ub, tb) = self.number(field)?;
                self.range(field, (a, ua, &ta), (b, ub, &tb))?
            }
            NumberCmp::Plain => {
                let range_word = self
                    .peek()
                    .filter(|t| t.kind == Kind::Word && t.text.contains(".."))
                    .cloned();
                if let Some(token) = range_word {
                    self.bump();
                    let (x, y) = token
                        .text
                        .split_once("..")
                        .unwrap_or((token.text.as_str(), ""));
                    let (a, ua) = self.number_text(x, field, &token)?;
                    let (b, ub) = if y.is_empty() {
                        let (b, ub, _) = self.number(field)?;
                        (b, ub)
                    } else {
                        let (b, ub) = self.number_text(y, field, &token)?;
                        let ub = match ub {
                            Some(unit) => Some(unit),
                            None => self.unit_after(field)?,
                        };
                        (b, ub)
                    };
                    self.range(field, (a, ua, &token), (b, ub, &token))?
                } else {
                    let (a, ua, ta) = self.number(field)?;
                    if self.next_is("..") {
                        self.bump();
                        let (b, ub, tb) = self.number(field)?;
                        self.range(field, (a, ua, &ta), (b, ub, &tb))?
                    } else {
                        NumberOp::Is(self.amount(field, a, ua, &ta)?)
                    }
                }
            }
            other => {
                let (a, ua, ta) = self.number(field)?;
                let amount = self.amount(field, a, ua, &ta)?;
                match other {
                    NumberCmp::Is => NumberOp::Is(amount),
                    NumberCmp::IsNot => NumberOp::IsNot(amount),
                    NumberCmp::Over => NumberOp::Over(amount),
                    NumberCmp::Under => NumberOp::Under(amount),
                    NumberCmp::AtLeast => NumberOp::AtLeast(amount),
                    _ => NumberOp::AtMost(amount),
                }
            }
        };
        Ok(Test::Number(NumberTest { field, op }))
    }

    fn date_cmp(&mut self, field: Field) -> DateCmp {
        let Some(token) = self.peek().cloned() else {
            self.want(Expected::Comparison(field));
            self.want(Expected::Value(field));
            return DateCmp::Plain;
        };
        let cmp = match token.kind {
            Kind::Sign(sign) => DateCmp::Sign(sign),
            Kind::Word => match token.text.to_ascii_lowercase().as_str() {
                "on" => {
                    self.bump();
                    if self.next_is("or") && self.peek_at(1).is_some_and(|t| t.is_word("before")) {
                        self.pos += 2;
                        return DateCmp::OnOrBefore;
                    }
                    return DateCmp::On;
                }
                "older" => {
                    self.bump();
                    if self.next_is("than") {
                        self.bump();
                    }
                    return DateCmp::Older;
                }
                "in" | "is" => DateCmp::On,
                "before" => DateCmp::Before,
                "after" => DateCmp::After,
                "since" => DateCmp::Since,
                "between" | "from" => DateCmp::Between,
                "within" => DateCmp::Within,
                _ => return DateCmp::Plain,
            },
            _ => return DateCmp::Plain,
        };
        self.bump();
        cmp
    }

    fn date_term(&mut self, field: Field) -> Result<(Test, bool), FilterError> {
        let mut not = false;
        let date = |p: &mut Self| -> Result<DateValue, FilterError> {
            match p.when(field, Want::Date)? {
                When::Date(value) => Ok(value),
                When::Duration(_) => Err(p.fail_here(Problem::WithinOrOlder, Vec::new())),
            }
        };
        let duration = |p: &mut Self| -> Result<Duration, FilterError> {
            match p.when(field, Want::Duration)? {
                When::Duration(value) => Ok(value),
                When::Date(_) => Err(p.fail_here(Problem::NeedsDuration, Vec::new())),
            }
        };
        let op = match self.date_cmp(field) {
            DateCmp::On => DateOp::On(date(self)?),
            DateCmp::Before => DateOp::Before(date(self)?),
            DateCmp::Since => DateOp::Since(date(self)?),
            DateCmp::After => DateOp::After(date(self)?),
            DateCmp::OnOrBefore => DateOp::OnOrBefore(date(self)?),
            DateCmp::Within => DateOp::Within(duration(self)?),
            DateCmp::Older => DateOp::OlderThan(duration(self)?),
            DateCmp::Between => {
                let low = date(self)?;
                self.range_joiner()?;
                DateOp::Between(low, date(self)?)
            }
            DateCmp::Plain => {
                let range_word = self
                    .peek()
                    .filter(|t| t.kind == Kind::Word && t.text.contains(".."))
                    .cloned();
                if let Some(token) = range_word {
                    self.bump();
                    let (x, y) = token
                        .text
                        .split_once("..")
                        .unwrap_or((token.text.as_str(), ""));
                    let low = self.iso_at(x, &token, field)?;
                    let high = self.iso_at(y, &token, field)?;
                    DateOp::Between(low, high)
                } else {
                    match self.when(field, Want::Either)? {
                        When::Date(value) => DateOp::On(value),
                        When::Duration(_) => {
                            let last = self.tokens[self.pos - 1].clone();
                            return Err(self.fail_at(&last, Problem::WithinOrOlder, Vec::new()));
                        }
                    }
                }
            }
            DateCmp::Sign(sign) => match self.when(field, Want::Either)? {
                When::Date(value) => match sign {
                    Sign::Eq => DateOp::On(value),
                    Sign::Ne => {
                        not = true;
                        DateOp::On(value)
                    }
                    Sign::Gt => DateOp::After(value),
                    Sign::Lt => DateOp::Before(value),
                    Sign::Ge => DateOp::Since(value),
                    Sign::Le => DateOp::OnOrBefore(value),
                },
                When::Duration(value) => match sign {
                    Sign::Gt | Sign::Ge => DateOp::OlderThan(value),
                    Sign::Lt | Sign::Le => DateOp::Within(value),
                    Sign::Eq | Sign::Ne => {
                        let last = self.tokens[self.pos - 1].clone();
                        return Err(self.fail_at(&last, Problem::WithinOrOlder, Vec::new()));
                    }
                },
            },
        };
        Ok((Test::Date(DateTest { field, op }), not))
    }

    /// One end of a range written in one word: `2026-01-01..2026-06-30`.
    fn iso_at(&self, text: &str, token: &Token, field: Field) -> Result<DateValue, FilterError> {
        let fail = |problem| self.fail_at(token, problem, vec![Expected::Value(field)]);
        match parse_iso(text) {
            Ok(Some(date)) => Ok(DateValue::Date(date)),
            Ok(None) => Err(fail(Problem::DateNotUnderstood)),
            Err(problem) => Err(fail(date_problem(problem))),
        }
    }

    /// A date, a date word or a duration.
    fn when(&mut self, field: Field, want: Want) -> Result<When, FilterError> {
        let token = self.value_token(field)?;
        let fail = |p: &Self, problem| p.fail_at(&token, problem, vec![Expected::Value(field)]);
        let text = token.text.as_str();
        let found = if token.kind == Kind::Quoted {
            None
        } else if let Some(word) = DateWord::from_one(text) {
            self.skip_annotation();
            Some(When::Date(DateValue::Word(word)))
        } else if DateWord::starts_two(text) {
            match self.peek().cloned() {
                Some(next) => match DateWord::from_two(text, &next.text) {
                    Some(word) if next.kind == Kind::Word => {
                        self.bump();
                        self.skip_annotation();
                        Some(When::Date(DateValue::Word(word)))
                    }
                    _ => {
                        return Err(self.fail_at(
                            &next,
                            Problem::DateNotUnderstood,
                            vec![Expected::Period],
                        ));
                    }
                },
                None => {
                    self.want(Expected::Period);
                    return Err(self.fail_end(Problem::DateNotUnderstood));
                }
            }
        } else if let Some(month) = month_from_name(text) {
            Some(When::Date(DateValue::Date(self.spelled(month, field)?)))
        } else {
            match parse_iso(text) {
                Ok(Some(date)) => Some(When::Date(DateValue::Date(date))),
                Err(problem) => return Err(fail(self, date_problem(problem))),
                Ok(None) => None,
            }
        };
        let found = match found {
            Some(found) => found,
            None => match parse_duration(text) {
                Some(duration) => When::Duration(duration),
                None if is_digits(&token) => {
                    let Ok(count) = text.parse::<u32>() else {
                        return Err(fail(self, Problem::NumberTooLarge));
                    };
                    match self.peek().cloned() {
                        Some(next) => match TimeUnit::from_word(&next.text) {
                            Some(unit) if next.kind == Kind::Word => {
                                self.bump();
                                When::Duration(Duration { count, unit })
                            }
                            _ if want == Want::Duration => {
                                return Err(self.fail_at(
                                    &next,
                                    Problem::NeedsTimeUnit,
                                    vec![Expected::TimeUnit],
                                ));
                            }
                            _ => return Err(fail(self, Problem::DateNotUnderstood)),
                        },
                        None => {
                            self.want(Expected::TimeUnit);
                            let problem = if want == Want::Date {
                                Problem::DateNotUnderstood
                            } else {
                                Problem::NeedsTimeUnit
                            };
                            return Err(self.fail_end(problem));
                        }
                    }
                }
                None => return Err(fail(self, Problem::DateNotUnderstood)),
            },
        };
        match (want, &found) {
            (Want::Date, When::Duration(_)) => Err(fail(self, Problem::WithinOrOlder)),
            (Want::Duration, When::Date(_)) => Err(fail(self, Problem::NeedsDuration)),
            _ => Ok(found),
        }
    }

    /// A spelled date after its month: "October 11, 2026", "October 2026",
    /// "October 11, 2026, 2:30 PM".
    fn spelled(&mut self, month: u8, field: Field) -> Result<CalendarDate, FilterError> {
        let fail =
            |p: &Self, t: &Token, problem| p.fail_at(t, problem, vec![Expected::Value(field)]);
        let Some(next) = self.peek().cloned() else {
            self.want(Expected::Value(field));
            return Err(self.fail_end(Problem::DateNotUnderstood));
        };
        if !is_digits(&next) {
            return Err(fail(self, &next, Problem::DateNotUnderstood));
        }
        self.bump();
        if next.text.len() == 4 {
            let year = next.text.parse().unwrap_or(0);
            return CalendarDate::new(year, Some(month), None, None)
                .map_err(|p| fail(self, &next, date_problem(p)));
        }
        let Some(day) = next
            .text
            .parse::<u8>()
            .ok()
            .filter(|d| (1..=31).contains(d))
        else {
            return Err(fail(self, &next, Problem::DateNotUnderstood));
        };
        if self.next_kind(Kind::Comma) {
            self.bump();
        }
        let year_token = match self.peek().cloned() {
            Some(t) if is_digits(&t) && t.text.len() == 4 => {
                self.bump();
                t
            }
            Some(t) => return Err(fail(self, &t, Problem::DateNotUnderstood)),
            None => {
                self.want(Expected::Value(field));
                return Err(self.fail_end(Problem::DateNotUnderstood));
            }
        };
        let year = year_token.text.parse().unwrap_or(0);
        let mut time = None;
        let clock = self
            .peek_at(1)
            .filter(|t| t.kind == Kind::Word)
            .and_then(|t| parse_clock_time(&t.text));
        if let Some((hour, minute, second)) = clock.filter(|_| self.next_kind(Kind::Comma)) {
            self.pos += 2;
            let mut hour = hour;
            if let Some(half) = self
                .peek()
                .cloned()
                .filter(|t| t.is_word("am") || t.is_word("pm"))
            {
                if hour == 0 || hour > 12 {
                    return Err(fail(self, &half, Problem::DateNotUnderstood));
                }
                self.bump();
                hour = match (hour, half.is_word("pm")) {
                    (12, false) => 0,
                    (12, true) => 12,
                    (h, true) => h + 12,
                    (h, false) => h,
                };
            }
            time = Some((hour, minute, second));
        }
        CalendarDate::new(year, Some(month), Some(day), time)
            .map_err(|p| fail(self, &year_token, date_problem(p)))
    }

    /// The real days the read-back adds after a date word (", October 4
    /// to 10"): read and left out of the tree.
    fn skip_annotation(&mut self) {
        let starts = self.next_kind(Kind::Comma)
            && self.peek_at(1).is_some_and(|t| {
                t.kind == Kind::Word && (month_from_name(&t.text).is_some() || is_digits(t))
            });
        if !starts {
            return;
        }
        self.bump();
        let spelled = |t: &Token| {
            t.kind == Kind::Word && (month_from_name(&t.text).is_some() || is_digits(t))
        };
        while let Some(token) = self.peek() {
            // "to" belongs to the days only when a day or month follows,
            // so "from last week, October 4 to 10 to today" keeps its
            // second "to".
            let to = token.is_word("to") && self.peek_at(1).is_some_and(spelled);
            let part = token.kind == Kind::Word
                && (month_from_name(&token.text).is_some()
                    || is_digits(token)
                    || to
                    || token.is_word("am")
                    || token.is_word("pm")
                    || parse_clock_time(&token.text).is_some());
            let comma = token.kind == Kind::Comma && self.peek_at(1).is_some_and(is_digits);
            if !(part || comma) {
                break;
            }
            self.bump();
        }
    }

    fn content_term(&mut self, case: Option<NameCase>) -> Result<Test, FilterError> {
        let field = Field::Content;
        let mut op = match self.peek() {
            Some(t) if t.is_word("contains") => {
                self.bump();
                Some(ContentOp::Contains)
            }
            Some(t) if t.is_word("regex") => {
                self.bump();
                Some(ContentOp::Regex)
            }
            Some(_) => None,
            None => {
                self.want(Expected::Comparison(field));
                None
            }
        };
        let token = self.value_token(field)?;
        let mut text = token.text.clone();
        if op.is_none()
            && token.kind != Kind::Quoted
            && let Some(pattern) = strip_regex_prefix(&text)
        {
            op = Some(ContentOp::Regex);
            text = pattern.to_owned();
            if text.is_empty() {
                text = self.value_token(field)?.text;
            }
        }
        if text.is_empty() {
            return Err(self.fail_at(
                &token,
                Problem::NeedsValue(field),
                vec![Expected::Value(field)],
            ));
        }
        let op = op.unwrap_or(ContentOp::Contains);
        let regex = op == ContentOp::Regex;
        let mut case = case.unwrap_or_else(|| self.resolve_case(regex, &text));
        let limit = if self.next_is("first") {
            self.bump();
            let (number, unit, at) = self.number(Field::Size)?;
            Some(self.amount(Field::Size, number, unit, &at)?)
        } else {
            None
        };
        if self.next_kind(Kind::Comma)
            && let Some(rule) = self.case_annotation(field)?
        {
            case = rule;
        }
        if regex && let Err(error) = compile_regex(&text, case) {
            let problem = pattern_problem(&error, &text);
            return Err(self.fail_at(&token, problem, Vec::new()));
        }
        Ok(Test::Content(ContentTest {
            op,
            text,
            case,
            limit,
        }))
    }

    fn state(&mut self, case: Option<NameCase>, negate: bool) -> Result<Expr, FilterError> {
        let first = self.tokens[self.pos - 1].clone();
        if case.is_some() {
            return Err(self.fail_at(&first, Problem::CaseNeedsText, Vec::new()));
        }
        if self
            .peek()
            .is_some_and(|t| t.kind == Kind::Sign(Sign::Eq) || t.is_word("is"))
        {
            self.bump();
        }
        match self.peek().cloned() {
            Some(t) if t.is_word("selected") => {
                self.bump();
                let test = Expr::Test {
                    test: Test::Flag(Flag::Selected),
                    at: Spot::new(first.word, first.span.start..t.span.end),
                };
                Ok(wrap(negate, test))
            }
            Some(t) => Err(self.fail_at(&t, Problem::StateIsSelected, vec![Expected::Selected])),
            None => {
                self.want(Expected::Selected);
                Err(self.fail_end(Problem::StateIsSelected))
            }
        }
    }
}

fn date_problem(problem: DateProblem) -> Problem {
    match problem {
        DateProblem::NotUnderstood => Problem::DateNotUnderstood,
        DateProblem::YearFirst => Problem::YearFirst,
        DateProblem::NoSuchDay => Problem::NoSuchDay,
    }
}

/// Why a word after a size is not a unit: a bare k, m, g or t, or
/// anything else.
fn unit_problem(word: &str) -> Problem {
    let lower = word.to_ascii_lowercase();
    match lower.as_str() {
        "k" | "m" | "g" | "t" => Problem::BareUnit(lower.chars().next().unwrap_or('k')),
        _ => Problem::UnknownUnit(word.to_owned()),
    }
}

/// Whether a word after a size number was meant as its unit: a bare k, m,
/// g or t, or a short word one edit from a unit.
fn looks_like_unit(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    if lower.len() == 1 {
        return "kmgt".contains(lower.as_str());
    }
    lower.len() <= 5
        && lower.chars().all(|c| c.is_ascii_alphabetic())
        && SizeUnit::ALL
            .iter()
            .filter(|unit| **unit != SizeUnit::Bytes)
            .map(|unit| unit.symbol().to_ascii_lowercase())
            .chain(["bytes".to_owned()])
            .any(|unit| edit_distance(&lower, &unit) == 1)
}

/// Reads a filter into its tree.
pub(crate) fn parse(text: &str, options: &FilterOptions) -> Result<Expr, FilterError> {
    Parser::new(text, options)?.run()
}
