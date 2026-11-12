//! One file encrypted or decrypted with age, streamed: read in 64 KiB
//! pieces, written to a stage diroxide hands out (`fs/stage`: an
//! unfinished `.diro-part-` name in the target folder) and moved into
//! place by diroxide (`fs/commit`: a rename that never replaces, the made
//! file joining the command's Undo), the cancel looked at between pieces,
//! the share done reported as it goes. A cancel or a failure removes the
//! stage's file, so nothing half-made is ever seen under a real name. The
//! original is never touched.
//!
//! A passphrase is used at work factor [`WORK_FACTOR`] and accepted up to
//! [`WORK_CAP`]; an identity's keys stay in memory only for the command
//! that read them.

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use age::armor::{ArmoredReader, ArmoredWriter, Format};
use age::secrecy::SecretString;
use diroxide_vfs::VfsError;

use crate::header::{WORK_CAP, WORK_FACTOR};
use crate::lines;
use crate::names;

/// How much is read and written at a time.
pub const CHUNK: usize = 64 * 1024;

/// What encrypts a file.
pub enum Lock<'a> {
    /// A passphrase, as the file's only recipient.
    Passphrase(&'a SecretString),
    /// Public keys.
    Recipients(&'a [age::x25519::Recipient]),
}

/// What decrypts a file.
pub enum Unlock<'a> {
    /// A passphrase.
    Passphrase(&'a SecretString),
    /// The keys of an identity file.
    Identities(&'a [Box<dyn age::Identity + Send + Sync>]),
}

/// Why a file was not done, in words or as a case the command answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The passphrase was wrong, or the file was changed.
    WrongPassphrase,
    /// None of the identity file's keys opens it.
    NoMatchingKey,
    /// Its passphrase asks for more work than the cap.
    TooMuchWork,
    /// It is not an age file.
    NotAge,
    /// Any other cause, in words, lower case first.
    Words(String),
}

impl Failure {
    /// The cause after "Could not decrypt NAME", ending with a period.
    pub fn cause(&self) -> String {
        match self {
            Failure::WrongPassphrase => lines::WRONG_PASSPHRASE.to_owned(),
            Failure::NoMatchingKey => lines::NO_MATCHING_KEY.to_owned(),
            Failure::TooMuchWork => lines::TOO_MUCH_WORK.to_owned(),
            Failure::NotAge => lines::NOT_AGE.to_owned(),
            Failure::Words(words) => {
                if words.ends_with('.') {
                    words.clone()
                } else {
                    format!("{words}.")
                }
            }
        }
    }
}

/// How one file ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// Made, at this path.
    Made(PathBuf),
    /// Not made, for this reason.
    Failed(Failure),
    /// Cancelled; nothing was left behind.
    Cancelled,
}

/// What a run watches: the cancel, and where the share done goes.
pub struct Watch<'a> {
    /// Whether to stop now.
    pub cancelled: &'a dyn Fn() -> bool,
    /// The bytes of the source done, and its size.
    pub progress: &'a mut dyn FnMut(u64, u64),
}

/// A failure from a write or a read.
fn io_words(error: &io::Error) -> Failure {
    if error.kind() == io::ErrorKind::InvalidData {
        return Failure::Words(lines::DAMAGED.to_owned());
    }
    Failure::Words(lower_first(
        &VfsError::from(io::Error::new(error.kind(), error.to_string())).to_string(),
    ))
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A reader that counts what passed through it.
struct Counting<R> {
    inner: R,
    done: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Copies `from` into `to` in pieces, looking at the cancel between them
/// and reporting the source's bytes done. `false` when cancelled.
fn pump(
    from: &mut dyn Read,
    to: &mut dyn Write,
    done: &AtomicU64,
    size: u64,
    watch: &mut Watch<'_>,
) -> io::Result<bool> {
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if (watch.cancelled)() {
            return Ok(false);
        }
        let n = from.read(&mut buffer)?;
        if n == 0 {
            return Ok(true);
        }
        to.write_all(&buffer[..n])?;
        (watch.progress)(done.load(Ordering::Relaxed), size);
    }
}

/// The folder and name of a source.
fn split(source: &Path) -> Result<(PathBuf, String), Failure> {
    let folder = source
        .parent()
        .ok_or_else(|| Failure::Words("it has no folder".to_owned()))?;
    let name = source
        .file_name()
        .ok_or_else(|| Failure::Words("it has no name".to_owned()))?
        .to_string_lossy()
        .into_owned();
    Ok((folder.to_path_buf(), name))
}

/// An output staged by diroxide: its stage's id and the path the plugin
/// writes the whole output to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    /// The stage's id, for the commit.
    pub id: String,
    /// Where the output's bytes go until the commit.
    pub path: PathBuf,
}

/// Where outputs are made: diroxide's `fs/stage` and `fs/commit` (the
/// host, [`crate::host::Host`]), so the made file joins the command's
/// Undo record and the rename never replaces a file.
pub trait Stager {
    /// A stage for an output at `output`, a free name in its folder.
    fn stage(&self, output: &Path) -> Result<Staged, Failure>;

    /// Moves a written stage into place; the made file's path (a name
    /// taken meanwhile is numbered by diroxide).
    fn commit(&self, staged: &Staged) -> Result<PathBuf, Failure>;
}

/// A writer to fill: given back filled to be committed, `None` when
/// cancelled, or the reason it was not filled.
type Fill<'a> = dyn FnMut(Box<dyn Write>) -> Result<Option<Box<dyn Write>>, Failure> + 'a;

/// Removes a stage the plugin wrote and does not commit: its own
/// unfinished file, so nothing half-made is left (diroxide also removes
/// an uncommitted stage when the session ends).
fn discard(staged: &Staged) {
    let _ = std::fs::remove_file(&staged.path);
}

/// Writes a new output named from `wanted` (the first free name) through
/// a stage: `write` fills the stage's file and gives it back to be
/// committed, or says why not; the file is flushed to the disk before
/// the commit.
fn write_new(out: &dyn Stager, folder: &Path, wanted: &str, write: &mut Fill<'_>) -> End {
    let Some(name) = names::free_name(folder, wanted) else {
        return End::Failed(Failure::Words("no free name is left for it".to_owned()));
    };
    let staged = match out.stage(&folder.join(&name)) {
        Ok(staged) => staged,
        Err(failure) => return End::Failed(failure),
    };
    let file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged.path)
    {
        Ok(file) => file,
        Err(error) => return End::Failed(io_words(&error)),
    };
    let filled = write(Box::new(io::BufWriter::with_capacity(CHUNK, file)));
    let mut writer = match filled {
        Ok(Some(writer)) => writer,
        Ok(None) => {
            discard(&staged);
            return End::Cancelled;
        }
        Err(failure) => {
            discard(&staged);
            return End::Failed(failure);
        }
    };
    let flushed = writer.flush();
    drop(writer);
    let synced = flushed.and_then(|()| {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&staged.path)?
            .sync_all()
    });
    if let Err(error) = synced {
        discard(&staged);
        return End::Failed(io_words(&error));
    }
    match out.commit(&staged) {
        Ok(path) => End::Made(path),
        Err(failure) => {
            discard(&staged);
            End::Failed(failure)
        }
    }
}

/// Encrypts one file into `NAME.age` beside it, through `out`'s stage.
pub fn encrypt_file(
    source: &Path,
    lock: &Lock<'_>,
    armor: bool,
    out: &dyn Stager,
    watch: &mut Watch<'_>,
) -> End {
    let (folder, name) = match split(source) {
        Ok(parts) => parts,
        Err(failure) => return End::Failed(failure),
    };
    let size = match std::fs::metadata(source) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(_) => return End::Failed(Failure::Words("it is not a file".to_owned())),
        Err(error) => return End::Failed(io_words(&error)),
    };
    let wanted = names::encrypted_name(&name);
    let mut write = |writer: Box<dyn Write>| {
        let encryptor = match lock {
            Lock::Passphrase(passphrase) => {
                let mut recipient = age::scrypt::Recipient::new((*passphrase).clone());
                recipient.set_work_factor(WORK_FACTOR);
                age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            }
            Lock::Recipients(recipients) => {
                age::Encryptor::with_recipients(recipients.iter().map(|r| r as &dyn age::Recipient))
            }
        }
        .map_err(|e| Failure::Words(lower_first(&e.to_string())))?;
        let file = File::open(source).map_err(|e| io_words(&e))?;
        let done = Arc::new(AtomicU64::new(0));
        let mut reader = Counting {
            inner: BufReader::with_capacity(CHUNK, file),
            done: Arc::clone(&done),
        };
        let format = if armor {
            Format::AsciiArmor
        } else {
            Format::Binary
        };
        let armored = ArmoredWriter::wrap_output(writer, format).map_err(|e| io_words(&e))?;
        let mut stream = encryptor.wrap_output(armored).map_err(|e| io_words(&e))?;
        if !pump(&mut reader, &mut stream, &done, size, watch).map_err(|e| io_words(&e))? {
            return Ok(None);
        }
        let armored = stream.finish().map_err(|e| io_words(&e))?;
        let writer = armored.finish().map_err(|e| io_words(&e))?;
        Ok(Some(writer))
    };
    write_new(out, &folder, &wanted, &mut write)
}

/// Decrypts one file into `NAME` (with `.age` removed) beside it,
/// through `out`'s stage.
pub fn decrypt_file(
    source: &Path,
    unlock: &Unlock<'_>,
    out: &dyn Stager,
    watch: &mut Watch<'_>,
) -> End {
    let (folder, name) = match split(source) {
        Ok(parts) => parts,
        Err(failure) => return End::Failed(failure),
    };
    let size = match std::fs::metadata(source) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(_) => return End::Failed(Failure::Words("it is not a file".to_owned())),
        Err(error) => return End::Failed(io_words(&error)),
    };
    let wanted = names::decrypted_name(&name);
    let mut write = |mut writer: Box<dyn Write>| {
        let file = File::open(source).map_err(|e| io_words(&e))?;
        let done = Arc::new(AtomicU64::new(0));
        let reader = Counting {
            inner: file,
            done: Arc::clone(&done),
        };
        let armored = ArmoredReader::new(BufReader::with_capacity(CHUNK, reader));
        let decryptor = age::Decryptor::new(armored).map_err(|e| decrypt_words(&e))?;
        let mut stream = match unlock {
            Unlock::Passphrase(passphrase) => {
                let mut identity = age::scrypt::Identity::new((*passphrase).clone());
                identity.set_max_work_factor(WORK_CAP);
                decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))
            }
            Unlock::Identities(identities) => {
                decryptor.decrypt(identities.iter().map(|i| i.as_ref() as &dyn age::Identity))
            }
        }
        .map_err(|e| decrypt_words(&e))?;
        if !pump(&mut stream, &mut writer, &done, size, watch).map_err(|e| io_words(&e))? {
            return Ok(None);
        }
        Ok(Some(writer))
    };
    write_new(out, &folder, &wanted, &mut write)
}

/// A decryption's failure in words.
pub fn decrypt_words(error: &age::DecryptError) -> Failure {
    use age::DecryptError as E;
    match error {
        E::DecryptionFailed | E::KeyDecryptionFailed => Failure::WrongPassphrase,
        E::NoMatchingKeys => Failure::NoMatchingKey,
        E::ExcessiveWork { .. } => Failure::TooMuchWork,
        E::InvalidHeader => Failure::NotAge,
        E::InvalidMac => Failure::Words(lines::DAMAGED.to_owned()),
        E::UnknownFormat => Failure::Words(lines::NEWER_FORMAT.to_owned()),
        E::Io(io) => io_words(io),
        _ => Failure::Words(lines::DAMAGED.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    fn scratch() -> tempfile::TempDir {
        let exe = std::env::current_exe().unwrap();
        let base = exe
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("crypt-unit-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix("unit-")
            .tempdir_in(base)
            .unwrap()
    }

    fn quiet() -> (impl Fn() -> bool, impl FnMut(u64, u64)) {
        (|| false, |_, _| {})
    }

    /// diroxide's stage and commit, as the host does them, on the disk:
    /// an unfinished name in the output's folder, then a rename onto the
    /// output's name when it is free.
    struct Here;

    impl Stager for Here {
        fn stage(&self, output: &Path) -> Result<Staged, Failure> {
            let folder = output.parent().unwrap();
            let id = format!(
                "{:08x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos()
            );
            let path = folder.join(format!(".diro-part-{id}.age"));
            let target = output.display().to_string();
            Ok(Staged {
                id: format!("{id}|{target}"),
                path,
            })
        }

        fn commit(&self, staged: &Staged) -> Result<PathBuf, Failure> {
            let target = PathBuf::from(staged.id.split_once('|').unwrap().1);
            if target.exists() {
                return Err(Failure::Words("it exists".to_owned()));
            }
            std::fs::rename(&staged.path, &target).unwrap();
            Ok(target)
        }
    }

    fn made(end: End) -> PathBuf {
        match end {
            End::Made(path) => path,
            other => panic!("not made: {other:?}"),
        }
    }

    #[test]
    fn a_passphrase_round_trip_beside_the_file_keeps_the_original() {
        let dir = scratch();
        let source = dir.path().join("report.pdf");
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &body).unwrap();
        let pass = SecretString::from("thrown-away-unit-pass".to_owned());
        let (cancelled, mut progress) = quiet();
        let mut watch = Watch {
            cancelled: &cancelled,
            progress: &mut progress,
        };
        let out = made(encrypt_file(
            &source,
            &Lock::Passphrase(&pass),
            false,
            &Here,
            &mut watch,
        ));
        assert_eq!(out.file_name().unwrap(), "report.pdf.age");
        assert_eq!(std::fs::read(&source).unwrap(), body);
        let head = std::fs::read(&out).unwrap();
        assert!(head.starts_with(b"age-encryption.org/v1\n-> scrypt "));
        assert_eq!(
            crate::header::peek(&head).scrypt,
            Some(WORK_FACTOR),
            "made at the reference default"
        );
        // Decrypting beside it: report.pdf is taken, so a number is added.
        let back = made(decrypt_file(
            &out,
            &Unlock::Passphrase(&pass),
            &Here,
            &mut watch,
        ));
        assert_eq!(back.file_name().unwrap(), "report (2).pdf");
        assert_eq!(std::fs::read(&back).unwrap(), body);
        // Nothing unfinished is left.
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".diro-part-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn armor_and_keys_and_the_wrong_ones() {
        let dir = scratch();
        let source = dir.path().join("notes.txt");
        std::fs::write(&source, b"hello there").unwrap();
        let identity = age::x25519::Identity::generate();
        let recipients = [identity.to_public()];
        let (cancelled, mut progress) = quiet();
        let mut watch = Watch {
            cancelled: &cancelled,
            progress: &mut progress,
        };
        let out = made(encrypt_file(
            &source,
            &Lock::Recipients(&recipients),
            true,
            &Here,
            &mut watch,
        ));
        assert!(
            std::fs::read(&out)
                .unwrap()
                .starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----")
        );
        let other: Vec<Box<dyn age::Identity + Send + Sync>> =
            vec![Box::new(age::x25519::Identity::generate())];
        assert_eq!(
            decrypt_file(&out, &Unlock::Identities(&other), &Here, &mut watch),
            End::Failed(Failure::NoMatchingKey)
        );
        let pass = SecretString::from("nope".to_owned());
        assert!(matches!(
            decrypt_file(&out, &Unlock::Passphrase(&pass), &Here, &mut watch),
            End::Failed(_)
        ));
        let mine: Vec<Box<dyn age::Identity + Send + Sync>> = vec![Box::new(identity)];
        std::fs::remove_file(&source).unwrap();
        let back = made(decrypt_file(
            &out,
            &Unlock::Identities(&mine),
            &Here,
            &mut watch,
        ));
        assert_eq!(back, source);
        assert_eq!(std::fs::read(&back).unwrap(), b"hello there");
        assert_eq!(pass.expose_secret(), "nope");
    }

    #[test]
    fn a_wrong_passphrase_a_hostile_factor_and_a_damaged_file() {
        let dir = scratch();
        let source = dir.path().join("a.txt");
        std::fs::write(&source, vec![1u8; 300_000]).unwrap();
        let pass = SecretString::from("thrown-away-unit-pass".to_owned());
        let (cancelled, mut progress) = quiet();
        let mut watch = Watch {
            cancelled: &cancelled,
            progress: &mut progress,
        };
        let out = made(encrypt_file(
            &source,
            &Lock::Passphrase(&pass),
            false,
            &Here,
            &mut watch,
        ));
        let wrong = SecretString::from("thrown-away-wrong".to_owned());
        assert_eq!(
            decrypt_file(&out, &Unlock::Passphrase(&wrong), &Here, &mut watch),
            End::Failed(Failure::WrongPassphrase)
        );
        // A header asking for work factor 21 is refused before any work.
        let mut bytes = std::fs::read(&out).unwrap();
        let text = String::from_utf8_lossy(&bytes[..120]).into_owned();
        let at = text.find(" 18\n").unwrap();
        bytes[at + 1..at + 3].copy_from_slice(b"21");
        let hostile = dir.path().join("hostile.age");
        std::fs::write(&hostile, &bytes).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(
            decrypt_file(&hostile, &Unlock::Passphrase(&pass), &Here, &mut watch),
            End::Failed(Failure::TooMuchWork)
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        // A byte changed in the body: the stream's check fails and nothing
        // is left.
        let mut bytes = std::fs::read(&out).unwrap();
        let last = bytes.len() - 100;
        bytes[last] ^= 0x55;
        let damaged = dir.path().join("damaged.age");
        std::fs::write(&damaged, &bytes).unwrap();
        assert_eq!(
            decrypt_file(&damaged, &Unlock::Passphrase(&pass), &Here, &mut watch),
            End::Failed(Failure::Words(lines::DAMAGED.to_owned()))
        );
        assert!(!dir.path().join("damaged").exists());
        // Not age at all.
        assert_eq!(
            decrypt_file(&source, &Unlock::Passphrase(&pass), &Here, &mut watch),
            End::Failed(Failure::NotAge)
        );
    }

    #[test]
    fn a_cancel_leaves_nothing_behind() {
        let dir = scratch();
        let source = dir.path().join("big.bin");
        std::fs::write(&source, vec![9u8; 3 * CHUNK + 5]).unwrap();
        let identity = age::x25519::Identity::generate();
        let recipients = [identity.to_public()];
        let seen = std::cell::Cell::new(0u32);
        let cancelled = || {
            seen.set(seen.get() + 1);
            seen.get() > 2
        };
        let mut reports = Vec::new();
        let mut progress = |done: u64, size: u64| reports.push((done, size));
        let mut watch = Watch {
            cancelled: &cancelled,
            progress: &mut progress,
        };
        assert_eq!(
            encrypt_file(
                &source,
                &Lock::Recipients(&recipients),
                false,
                &Here,
                &mut watch
            ),
            End::Cancelled
        );
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["big.bin"]);
        assert!(!reports.is_empty());
        assert!(
            reports
                .iter()
                .all(|(_, size)| *size == 3 * CHUNK as u64 + 5)
        );
    }
}
