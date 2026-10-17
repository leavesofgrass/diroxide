//! The limiter chain around every member's decoder (research/archives.md
//! 5.3 and group C), from the inside out: the codec; a size limiter that
//! stops with an error, never a silent cut, when a member delivers more or
//! less than it claims; a ratio meter; the job's byte budget; a CRC check
//! at the end; and a cancel check every 64 KiB. Nothing here allocates
//! from a declared size: a claim is a number to check against, never a
//! capacity.
//!
//! A limit reached is either a question ("Expands to 40 GB from 40 KB.
//! Read big.bin? y or n"), which the caller asks once and then reads
//! again with an [`Allowances`] that says yes, or a refusal with its
//! reason in words.

use std::fmt;
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use diroxide_core::Message;
use diroxide_core::format::{grouped, human_size};
use diroxide_vfs::{CancelToken, VfsError};

use crate::limits::{Limits, limit_words};

/// How often the cancel flag is checked, in bytes of output.
pub const CANCEL_EVERY: u64 = 64 << 10;

/// What a question is about, so the answer can be remembered for the rest
/// of a job.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuestionKind {
    /// One member grows past the ratio limit.
    Ratio,
    /// One member is larger than the member size limit.
    MemberSize,
    /// The job unpacks more than the total it asks about.
    Total,
    /// A dictionary or window larger than the one it asks about.
    Dictionary,
    /// A sequential archive must be read up to the member first.
    Cost,
}

/// A question a limit asks, ending "y or n", answered No by default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// What it is about.
    pub kind: QuestionKind,
    /// The words, without the "? y or n" that [`Question::message`] adds:
    /// "Expands to 67 MB from 65 KB. Read zeros.bin".
    pub text: String,
}

impl Question {
    /// The question as a message: "Expands to 67 MB from 65 KB. Read
    /// zeros.bin? y or n".
    pub fn message(&self) -> Message {
        Message::question(&self.text)
    }

    /// The limit it stands for, as the error a caller that cannot ask
    /// gives instead.
    pub fn as_error(&self) -> VfsError {
        let what = match self.kind {
            QuestionKind::Ratio => "expansion",
            QuestionKind::MemberSize => "one member's size",
            QuestionKind::Total => "bytes unpacked",
            QuestionKind::Dictionary => "dictionary size",
            QuestionKind::Cost => "reading ahead",
        };
        VfsError::LimitReached {
            what: what.to_owned(),
            limit: format!("asked first: {}", self.message().text),
        }
    }
}

/// The answers given so far in a job: each lets its kind of question pass
/// for the rest of the job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Allowances {
    /// Yes to an expansion question.
    pub ratio: bool,
    /// Yes to a member size question.
    pub member_size: bool,
    /// Yes to a total question.
    pub total: bool,
    /// Yes to a dictionary question.
    pub dictionary: bool,
    /// Yes to a reading-ahead question.
    pub cost: bool,
}

impl Allowances {
    /// Every question answered yes: what a caller passes after the user
    /// agreed to them all, and what the spikes use to see past them.
    pub const fn all() -> Self {
        Self {
            ratio: true,
            member_size: true,
            total: true,
            dictionary: true,
            cost: true,
        }
    }

    /// The same answers with one more yes.
    pub fn with(mut self, kind: QuestionKind) -> Self {
        match kind {
            QuestionKind::Ratio => self.ratio = true,
            QuestionKind::MemberSize => self.member_size = true,
            QuestionKind::Total => self.total = true,
            QuestionKind::Dictionary => self.dictionary = true,
            QuestionKind::Cost => self.cost = true,
        }
        self
    }

    /// Whether a question of this kind passes.
    pub const fn allows(self, kind: QuestionKind) -> bool {
        match kind {
            QuestionKind::Ratio => self.ratio,
            QuestionKind::MemberSize => self.member_size,
            QuestionKind::Total => self.total,
            QuestionKind::Dictionary => self.dictionary,
            QuestionKind::Cost => self.cost,
        }
    }
}

/// A limit reached: a question to ask first, or a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LimitHit {
    /// Ask, then try again with the answer in [`Allowances`].
    Ask(Question),
    /// Refused, with the reason.
    Refuse(VfsError),
}

impl LimitHit {
    /// The error a caller that cannot ask gives.
    pub fn into_error(self) -> VfsError {
        match self {
            LimitHit::Ask(question) => question.as_error(),
            LimitHit::Refuse(error) => error,
        }
    }

    /// The question, if this is one.
    pub fn question(&self) -> Option<&Question> {
        match self {
            LimitHit::Ask(question) => Some(question),
            LimitHit::Refuse(_) => None,
        }
    }
}

impl fmt::Display for LimitHit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LimitHit::Ask(question) => f.write_str(&question.message().text),
            LimitHit::Refuse(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LimitHit {}

/// A refusal for a hard limit: "it passes the limit for WHAT: LIMIT".
pub fn refuse(what: &str, limit: String) -> LimitHit {
    LimitHit::Refuse(VfsError::LimitReached {
        what: what.to_owned(),
        limit,
    })
}

/// A damaged member: "it is damaged: WHAT".
pub fn damaged(what: &str) -> VfsError {
    VfsError::Corrupt {
        what: what.to_owned(),
    }
}

/// Wraps an error in an I/O error that carries it, so a reader's caller
/// gets it back whole through [`from_io`].
pub fn to_io(error: VfsError) -> io::Error {
    io::Error::other(Carried::Vfs(error))
}

/// Wraps a limit reached in an I/O error, for [`from_io`] and
/// [`limit_of`].
pub fn limit_to_io(hit: LimitHit) -> io::Error {
    io::Error::other(Carried::Limit(hit))
}

#[derive(Debug)]
enum Carried {
    Vfs(VfsError),
    Limit(LimitHit),
}

impl fmt::Display for Carried {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Carried::Vfs(error) => error.fmt(f),
            Carried::Limit(hit) => hit.fmt(f),
        }
    }
}

impl std::error::Error for Carried {}

/// The limit an I/O error from the chain carries, if any: a question to
/// ask, or a refusal.
pub fn limit_of(error: &io::Error) -> Option<&LimitHit> {
    match error.get_ref()?.downcast_ref::<Carried>()? {
        Carried::Limit(hit) => Some(hit),
        Carried::Vfs(_) => None,
    }
}

/// The provider error an I/O error from the chain or a codec stands for:
/// the carried error whole, a limit as its error, and anything else
/// through the system's words. A codec's "unexpected end" is damage.
pub fn from_io(error: io::Error) -> VfsError {
    if let Some(carried) = error.get_ref().and_then(|e| e.downcast_ref::<Carried>()) {
        return match carried {
            Carried::Vfs(error) => error.clone(),
            Carried::Limit(hit) => hit.clone().into_error(),
        };
    }
    if let Some(inner) = error.get_ref().and_then(|e| e.downcast_ref::<VfsError>()) {
        return inner.clone();
    }
    match error.kind() {
        io::ErrorKind::UnexpectedEof => damaged("its data ends early"),
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => {
            damaged(&lowercase_first(&error.to_string()))
        }
        _ => VfsError::from(error),
    }
}

/// The text with its first letter in lowercase, for a cause in a sentence.
pub(crate) fn lowercase_first(text: &str) -> String {
    let text = text.trim().trim_end_matches('.');
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "the data could not be read".to_owned(),
    }
}

/// The byte budget of one job, shared by every member it reads: past the
/// soft total it asks once, past the hard total it refuses. Cloning shares
/// the count.
#[derive(Clone, Debug)]
pub struct Budget {
    used: Arc<AtomicU64>,
    ask_at: u64,
    max: u64,
}

impl Budget {
    /// A fresh budget with the job's limits.
    pub fn new(limits: &Limits) -> Self {
        Self {
            used: Arc::new(AtomicU64::new(0)),
            ask_at: limits.ask_total_bytes,
            max: limits.max_total_bytes,
        }
    }

    /// The bytes charged so far.
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// Charges `bytes` against the budget, with checked sums: an overflow
    /// is a refusal like the hard total.
    pub fn charge(&self, bytes: u64, allow: Allowances) -> Result<(), LimitHit> {
        let before = self.used.fetch_add(bytes, Ordering::Relaxed);
        let Some(now) = before.checked_add(bytes) else {
            return Err(refuse("bytes unpacked", limit_words(self.max)));
        };
        if now > self.max {
            return Err(refuse("bytes unpacked", limit_words(self.max)));
        }
        if now > self.ask_at && before <= self.ask_at && !allow.total {
            return Err(LimitHit::Ask(Question {
                kind: QuestionKind::Total,
                text: format!("Expands to more than {}. Go on", human_size(self.ask_at)),
            }));
        }
        Ok(())
    }
}

/// Checked sums of declared sizes: the total, or `None` on an overflow.
pub fn checked_total(sizes: impl IntoIterator<Item = u64>) -> Option<u64> {
    sizes.into_iter().try_fold(0u64, u64::checked_add)
}

/// The check before a member is read at all: a member larger than the
/// member size limit asks first ("big.bin is 4.3 GB. Read it? y or n").
pub fn check_member(
    name: &str,
    declared: u64,
    limits: &Limits,
    allow: Allowances,
) -> Result<(), LimitHit> {
    if declared > limits.max_member_size && !allow.member_size {
        return Err(LimitHit::Ask(Question {
            kind: QuestionKind::MemberSize,
            text: format!("{name} is {}. Read it", human_size(declared)),
        }));
    }
    Ok(())
}

/// The check of a dictionary or window before its decoder is made: over
/// the hard limit refuses, over the soft one asks.
pub fn check_dictionary(
    name: &str,
    bytes: u64,
    limits: &Limits,
    allow: Allowances,
) -> Result<(), LimitHit> {
    if bytes > limits.max_dictionary {
        return Err(refuse(
            "dictionary size",
            format!(
                "{}, and {name} asks for {}",
                limit_words(limits.max_dictionary),
                limit_words(bytes)
            ),
        ));
    }
    if bytes > limits.max_memory_bytes {
        return Err(refuse(
            "memory for one operation",
            limit_words(limits.max_memory_bytes),
        ));
    }
    if bytes > limits.ask_dictionary_bytes && !allow.dictionary {
        return Err(LimitHit::Ask(Question {
            kind: QuestionKind::Dictionary,
            text: format!("Needs {} of memory for {name}. Go on", human_size(bytes)),
        }));
    }
    Ok(())
}

/// The check before a member of a sequential archive is read alone: more
/// than `threshold` bytes to read first asks ("Read 3.2 GB first to open
/// note.txt? y or n").
pub fn check_cost(
    name: &str,
    ahead: u64,
    threshold: u64,
    allow: Allowances,
) -> Result<(), LimitHit> {
    if ahead > threshold && !allow.cost {
        return Err(LimitHit::Ask(Question {
            kind: QuestionKind::Cost,
            text: format!("Read {} first to open {name}", human_size(ahead)),
        }));
    }
    Ok(())
}

/// What one member read promises, for the chain to hold it to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    /// The member's name, for the words.
    pub name: String,
    /// The size it claims; `None` when the format does not say (gzip).
    pub size: Option<u64>,
    /// The bytes it takes compressed, for the ratio; `None` when unknown.
    pub packed: Option<u64>,
    /// The CRC-32 it claims; `None` when there is none to check.
    pub crc32: Option<u32>,
}

/// The limiter chain around a member's decoder. See the module
/// documentation.
pub struct Guarded<R> {
    inner: R,
    claims: Claims,
    delivered: u64,
    crc: Option<crc32fast::Hasher>,
    cancel: CancelToken,
    since_check: u64,
    budget: Budget,
    limits: Limits,
    allow: Allowances,
    finished: bool,
}

impl<R> fmt::Debug for Guarded<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guarded")
            .field("claims", &self.claims)
            .field("delivered", &self.delivered)
            .finish_non_exhaustive()
    }
}

impl<R: Read> Guarded<R> {
    /// Wraps a decoder with the member's claims, the job's budget and
    /// answers, and a cancel token.
    pub fn new(
        inner: R,
        claims: Claims,
        limits: Limits,
        budget: Budget,
        allow: Allowances,
        cancel: CancelToken,
    ) -> Self {
        let crc = claims.crc32.map(|_| crc32fast::Hasher::new());
        Self {
            inner,
            claims,
            delivered: 0,
            crc,
            cancel,
            since_check: CANCEL_EVERY,
            budget,
            limits,
            allow,
            finished: false,
        }
    }

    /// The bytes delivered so far.
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// The decoder inside.
    pub fn into_inner(self) -> R {
        self.inner
    }

    fn end(&mut self) -> io::Result<usize> {
        if let Some(size) = self.claims.size
            && self.delivered < size
        {
            return Err(to_io(damaged(&format!(
                "{} is shorter than its stated size ({} of {} bytes)",
                self.claims.name,
                grouped(self.delivered),
                grouped(size)
            ))));
        }
        if let (Some(expected), Some(hasher)) = (self.claims.crc32, self.crc.take()) {
            let found = hasher.finalize();
            if found != expected {
                return Err(to_io(damaged(&format!(
                    "the checksum of {} failed",
                    self.claims.name
                ))));
            }
        }
        self.finished = true;
        Ok(0)
    }
}

impl<R: Read> Read for Guarded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.finished || buf.is_empty() {
            return Ok(0);
        }
        if self.since_check >= CANCEL_EVERY {
            self.since_check = 0;
            self.cancel.check().map_err(to_io)?;
        }
        // Never ask the codec for more than one byte past the claim, so a
        // member that runs long is caught at its first extra byte.
        let room = match self.claims.size {
            Some(size) => size.saturating_sub(self.delivered).saturating_add(1),
            None => u64::MAX,
        };
        let take = buf.len().min(usize::try_from(room).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..take])?;
        if n == 0 {
            return self.end();
        }
        let n64 = n as u64;
        self.delivered = self.delivered.saturating_add(n64);
        if let Some(size) = self.claims.size
            && self.delivered > size
        {
            return Err(to_io(damaged(&format!(
                "{} is longer than its stated size of {} bytes",
                self.claims.name,
                grouped(size)
            ))));
        }
        if !self.allow.ratio
            && self.delivered > self.limits.ratio_floor_bytes
            && let Some(packed) = self.claims.packed
            && self.delivered / packed.max(1) > self.limits.max_ratio
        {
            return Err(limit_to_io(LimitHit::Ask(Question {
                kind: QuestionKind::Ratio,
                text: format!(
                    "Expands to {} from {}. Read {}",
                    human_size(
                        self.claims
                            .size
                            .unwrap_or(self.delivered)
                            .max(self.delivered)
                    ),
                    human_size(packed),
                    self.claims.name
                ),
            })));
        }
        self.budget.charge(n64, self.allow).map_err(limit_to_io)?;
        if let Some(hasher) = self.crc.as_mut() {
            hasher.update(&buf[..n]);
        }
        self.since_check = self.since_check.saturating_add(n64);
        Ok(n)
    }
}

/// A reader that counts the bytes read through it into a shared counter:
/// the compressed bytes a sequential listing has consumed, for its
/// progress line and its ratio.
#[derive(Debug)]
pub struct Counting<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R> Counting<R> {
    /// Counts into `count`.
    pub fn new(inner: R, count: Arc<AtomicU64>) -> Self {
        Self { inner, count }
    }
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn guard(data: Vec<u8>, claims: Claims, limits: Limits) -> Guarded<Cursor<Vec<u8>>> {
        Guarded::new(
            Cursor::new(data),
            claims,
            limits,
            Budget::new(&limits),
            Allowances::default(),
            CancelToken::new(),
        )
    }

    fn claims(name: &str, size: Option<u64>, packed: Option<u64>, data: &[u8]) -> Claims {
        Claims {
            name: name.to_owned(),
            size,
            packed,
            crc32: Some(crc32fast::hash(data)),
        }
    }

    fn read_all(mut reader: impl Read) -> Result<Vec<u8>, VfsError> {
        let mut out = Vec::new();
        reader.read_to_end(&mut out).map_err(from_io)?;
        Ok(out)
    }

    #[test]
    fn an_honest_member_reads_whole() {
        let data = b"hello, world".to_vec();
        let c = claims("a.txt", Some(12), Some(12), &data);
        assert_eq!(
            read_all(guard(data.clone(), c, Limits::default())).unwrap(),
            data
        );
    }

    #[test]
    fn a_member_that_lies_about_its_size_stops_with_an_error() {
        // It claims 4 GB and holds 1 KB: damaged at the end, and nothing
        // was ever allocated from the claim.
        let data = vec![7u8; 1024];
        let c = claims("big.bin", Some(4_000_000_000), Some(1024), &data);
        let error = read_all(guard(data, c, Limits::default())).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("big.bin is shorter than its stated size"),
            "{error}"
        );
        // It claims 10 bytes and holds more.
        let data = vec![b'z'; 100];
        let c = claims("small.bin", Some(10), Some(5), &data);
        let error = read_all(guard(data, c, Limits::default())).unwrap_err();
        assert_eq!(
            error.to_string(),
            "it is damaged: small.bin is longer than its stated size of 10 bytes"
        );
    }

    #[test]
    fn a_bad_checksum_is_said_after_the_data() {
        let data = b"hello".to_vec();
        let mut c = claims("a.txt", Some(5), Some(5), &data);
        c.crc32 = Some(1);
        let error = read_all(guard(data, c, Limits::default())).unwrap_err();
        assert_eq!(
            error.to_string(),
            "it is damaged: the checksum of a.txt failed"
        );
    }

    #[test]
    fn the_ratio_meter_asks_past_its_floor() {
        let limits = Limits::strict();
        let data = vec![0u8; 3 << 20];
        let c = claims("zeros.bin", Some(3 << 20), Some(1000), &data);
        let mut reader = guard(data.clone(), c.clone(), limits);
        let mut out = Vec::new();
        let error = reader.read_to_end(&mut out).unwrap_err();
        let question = limit_of(&error).and_then(LimitHit::question).unwrap();
        assert_eq!(question.kind, QuestionKind::Ratio);
        assert_eq!(
            question.message().text,
            "Expands to 3.1 MB from 1.0 KB. Read zeros.bin? y or n"
        );
        // Answered yes, it reads to the end.
        let reader = Guarded::new(
            Cursor::new(data.clone()),
            c,
            limits,
            Budget::new(&limits),
            Allowances::default().with(QuestionKind::Ratio),
            CancelToken::new(),
        );
        assert_eq!(read_all(reader).unwrap().len(), data.len());
    }

    #[test]
    fn the_budget_asks_once_then_refuses() {
        let limits = Limits {
            ask_total_bytes: 100,
            max_total_bytes: 200,
            ..Limits::default()
        };
        let budget = Budget::new(&limits);
        assert!(budget.charge(100, Allowances::default()).is_ok());
        let asked = budget.charge(1, Allowances::default()).unwrap_err();
        assert_eq!(asked.question().map(|q| q.kind), Some(QuestionKind::Total));
        assert!(
            budget.charge(50, Allowances::default()).is_ok(),
            "asked once"
        );
        let refused = budget.charge(100, Allowances::all()).unwrap_err();
        assert_eq!(
            refused.into_error().to_string(),
            "it passes the limit for bytes unpacked: 200 bytes"
        );
        let overflow = Budget::new(&Limits::default());
        assert!(overflow.charge(u64::MAX, Allowances::all()).is_err());
        assert!(overflow.charge(u64::MAX, Allowances::all()).is_err());
        assert_eq!(checked_total([u64::MAX, 1]), None);
        assert_eq!(checked_total([1, 2, 3]), Some(6));
    }

    #[test]
    fn cancel_is_seen_within_64_kib() {
        let cancel = CancelToken::new();
        let limits = Limits::default();
        let mut reader = Guarded::new(
            Cursor::new(vec![1u8; 1 << 20]),
            Claims::default(),
            limits,
            Budget::new(&limits),
            Allowances::default(),
            cancel.clone(),
        );
        let mut buf = vec![0u8; 16 << 10];
        let mut read = 0usize;
        cancel.cancel();
        let error = loop {
            match reader.read(&mut buf) {
                Ok(n) => read += n,
                Err(e) => break e,
            }
        };
        assert!(read as u64 <= CANCEL_EVERY);
        assert_eq!(from_io(error), VfsError::Cancelled);
    }

    #[test]
    fn the_checks_before_a_read_ask_and_refuse_in_words() {
        let limits = Limits::strict();
        let asked =
            check_member("big.bin", 4_294_967_280, &limits, Allowances::default()).unwrap_err();
        assert_eq!(
            asked.question().unwrap().message().text,
            "big.bin is 4.3 GB. Read it? y or n"
        );
        assert!(check_member("big.bin", 4_294_967_280, &limits, Allowances::all()).is_ok());
        let refused = check_dictionary("dict.7z", u64::from(u32::MAX), &limits, Allowances::all())
            .unwrap_err();
        assert_eq!(
            refused.into_error().to_string(),
            "it passes the limit for dictionary size: 64 MiB, and dict.7z asks for 3.9 GiB"
        );
        let asked = check_dictionary("a.7z", 32 << 20, &limits, Allowances::default()).unwrap_err();
        assert_eq!(
            asked.question().unwrap().message().text,
            "Needs 33.6 MB of memory for a.7z. Go on? y or n"
        );
        let cost =
            check_cost("note.txt", 3_200_000_000, 256 << 20, Allowances::default()).unwrap_err();
        assert_eq!(
            cost.question().unwrap().message().text,
            "Read 3.2 GB first to open note.txt? y or n"
        );
    }

    #[test]
    fn codec_errors_become_damage_in_words() {
        let eof = io::Error::from(io::ErrorKind::UnexpectedEof);
        assert_eq!(
            from_io(eof).to_string(),
            "it is damaged: its data ends early"
        );
        let carried = to_io(VfsError::WrongPassword);
        assert_eq!(from_io(carried), VfsError::WrongPassword);
    }
}
