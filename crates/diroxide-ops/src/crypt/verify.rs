//! Signatures and the chain around a checksum list (R29 3.4 and 3.5,
//! R30 commit 73): one [`verify_signature`] over gpg (`.asc`, `.sig`,
//! `.gpg`, or a file signed inline) and minisign (`.minisig`), giving a
//! [`SignatureVerdict`] with its lines; [`signature_sidecars`], the
//! signature files to look for beside a file or a list; and the chain's
//! sentences ([`chain_lines`]) over a verdict and the list's counts,
//! which the app composes with the checksum list's report.
//!
//! The words never overclaim: a good signature means the key fits, not
//! who holds it; a match without one shows only that the files are not
//! damaged.

use std::path::PathBuf;
use std::time::Duration;

use diroxide_core::Place;
use diroxide_vfs::{CancelToken, RunLimits, ToolRequest, ToolRunError, ToolRunner};

use super::gpg::{self, GPG, Trust, Verified};
use super::lines::{self, chain, verdict};
use super::minisign::{self, MinisignKey};

/// How long gpg may take to check one signature: it reads the whole file.
pub const VERIFY_LIMIT: Duration = Duration::from_secs(6 * 3600);

/// Which program or library checked a signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signer {
    /// gpg, an OpenPGP signature.
    Gpg,
    /// minisign-verify, a minisign signature.
    Minisign,
}

/// What a signature check came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureVerdict {
    /// The signature fits the key.
    Good {
        /// What checked it.
        signer: Signer,
        /// The key's last four hexadecimal digits.
        key: String,
        /// gpg's user id of the key, or minisign's trusted comment: the
        /// signer's own text, cleaned before it is said.
        name: Option<String>,
        /// Whether the owner certified the key (gpg); minisign's key is
        /// the one the owner gave, so it counts as chosen.
        certified: bool,
    },
    /// The signature does not fit: the file was changed, or it is not
    /// this file's.
    Bad {
        /// The key's ending, when known.
        key: Option<String>,
    },
    /// gpg has no public key for it; diroxide never fetches one.
    MissingKey {
        /// The key's ending, when gpg said it.
        key: Option<String>,
    },
    /// Good, but the key expired.
    KeyExpired {
        /// The key's ending.
        key: Option<String>,
    },
    /// Good, but the signature itself expired.
    SignatureExpired {
        /// The key's ending.
        key: Option<String>,
    },
    /// The key is revoked.
    Revoked {
        /// The key's ending.
        key: Option<String>,
    },
    /// A minisign signature made by another key than the one given.
    OtherKey,
    /// The minisign public key could not be read.
    KeyUnreadable,
    /// No minisign public key was given.
    NoKeyGiven,
    /// The signature file could not be read or checked.
    Unreadable,
    /// The signed file could not be read.
    DataUnreadable,
    /// gpg is not installed.
    GpgMissing,
    /// The check was cancelled.
    Cancelled,
}

impl SignatureVerdict {
    /// Whether the signature is good and current: the only verdict that
    /// signs a list.
    pub fn is_good(&self) -> bool {
        matches!(self, SignatureVerdict::Good { .. })
    }

    /// The verdict's line, meaning first: "Signature good: key ending
    /// 4E7B" (31).
    pub fn line(&self) -> String {
        match self {
            SignatureVerdict::Good { key, .. } => verdict::good(key),
            SignatureVerdict::Bad { .. } => verdict::BAD.to_owned(),
            SignatureVerdict::MissingKey { .. } => verdict::NO_PUBLIC_KEY.to_owned(),
            SignatureVerdict::KeyExpired { .. } => verdict::KEY_EXPIRED.to_owned(),
            SignatureVerdict::SignatureExpired { .. } => verdict::SIGNATURE_EXPIRED.to_owned(),
            SignatureVerdict::Revoked { .. } => verdict::REVOKED.to_owned(),
            SignatureVerdict::OtherKey => verdict::OTHER_KEY.to_owned(),
            SignatureVerdict::KeyUnreadable => verdict::BAD_PUBLIC_KEY.to_owned(),
            SignatureVerdict::NoKeyGiven => "Cannot check: no public key given".to_owned(),
            SignatureVerdict::Unreadable => verdict::UNREADABLE.to_owned(),
            SignatureVerdict::DataUnreadable => "Cannot check: the file is unreadable".to_owned(),
            SignatureVerdict::GpgMissing => lines::GPG_NOT_FOUND.to_owned(),
            SignatureVerdict::Cancelled => "Signature check cancelled".to_owned(),
        }
    }

    /// The verdict's line, then what follows it: a good signature's key
    /// text after "key ending 4E7B", whether the owner certified it, and
    /// what good means; a missing key's "diroxide does not fetch keys."
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![self.line()];
        match self {
            SignatureVerdict::Good {
                signer,
                key,
                name,
                certified,
            } => {
                if let Some(name) = name.as_deref().filter(|n| !n.trim().is_empty()) {
                    out.push(match signer {
                        Signer::Gpg => verdict::named(key, name),
                        Signer::Minisign => verdict::comment(name),
                    });
                }
                if !certified {
                    out.push(verdict::NOT_CERTIFIED.to_owned());
                }
                out.push(verdict::GOOD_MEANS.to_owned());
            }
            SignatureVerdict::MissingKey { key } => {
                if let Some(key) = key {
                    out.push(format!("The key ends {key}."));
                }
                out.push(verdict::NO_FETCH.to_owned());
            }
            _ => {}
        }
        out
    }
}

impl From<Verified> for SignatureVerdict {
    fn from(verified: Verified) -> Self {
        match verified {
            Verified::Good { key, name, trust } => SignatureVerdict::Good {
                signer: Signer::Gpg,
                key,
                name: Some(name),
                certified: trust == Trust::Certified,
            },
            Verified::Bad { key } => SignatureVerdict::Bad { key },
            Verified::MissingKey { key } => SignatureVerdict::MissingKey { key },
            Verified::KeyExpired { key } => SignatureVerdict::KeyExpired { key },
            Verified::SignatureExpired { key } => SignatureVerdict::SignatureExpired { key },
            Verified::Revoked { key } => SignatureVerdict::Revoked { key },
            Verified::Unreadable => SignatureVerdict::Unreadable,
        }
    }
}

/// What to check: a signature, the file it signs (none for a file signed
/// inline), and for minisign the public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureCheck {
    /// The signature file: `report.pdf.asc`, `SHA256SUMS.minisig`.
    pub signature: Place,
    /// The signed file; `None` for an inline signature (gpg only).
    pub data: Option<Place>,
    /// minisign's public key, from a file or pasted.
    pub minisign_key: Option<MinisignKey>,
}

impl SignatureCheck {
    /// A detached signature and its file.
    pub fn detached(signature: Place, data: Place) -> Self {
        Self {
            signature,
            data: Some(data),
            minisign_key: None,
        }
    }

    /// The same check with minisign's public key.
    pub fn with_minisign_key(mut self, key: MinisignKey) -> Self {
        self.minisign_key = Some(key);
        self
    }

    /// Whether the signature is minisign's, by its name.
    pub fn is_minisign(&self) -> bool {
        self.signature
            .name()
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".minisig"))
    }
}

/// Checks one signature: minisign in process for a `.minisig`, gpg
/// through `tools` for anything else, `gpg --verify` with the batch
/// prefix and only its status words read. Blocking; both files must be
/// on this computer.
pub fn verify_signature(
    tools: &dyn ToolRunner,
    check: &SignatureCheck,
    cancel: &CancelToken,
) -> SignatureVerdict {
    let Some(signature) = check.signature.local_path().map(PathBuf::from) else {
        return SignatureVerdict::Unreadable;
    };
    let data = match &check.data {
        Some(place) => match place.local_path() {
            Some(path) => Some(PathBuf::from(path)),
            None => return SignatureVerdict::DataUnreadable,
        },
        None => None,
    };
    if check.is_minisign() {
        let Some(data) = data else {
            return SignatureVerdict::DataUnreadable;
        };
        let Some(key) = &check.minisign_key else {
            return SignatureVerdict::NoKeyGiven;
        };
        return minisign::verify(&signature, &data, key, cancel);
    }
    if !tools.has(GPG) {
        return SignatureVerdict::GpgMissing;
    }
    let mut request = ToolRequest::new(GPG, gpg::verify_args(&signature, data.as_deref()))
        .with_limits(RunLimits::new(VERIFY_LIMIT, 1024 * 1024))
        .with_env(gpg::GPG_LOCALE.0, gpg::GPG_LOCALE.1);
    if let Some(folder) = signature.parent() {
        request = request.in_folder(folder);
    }
    match tools.run(&request, cancel) {
        Ok(output) => gpg::read_verify(&output.stdout).into(),
        Err(ToolRunError::Cancelled) => SignatureVerdict::Cancelled,
        Err(ToolRunError::NotInstalled { .. }) => SignatureVerdict::GpgMissing,
        Err(_) => SignatureVerdict::Unreadable,
    }
}

/// A signature file to look for beside a file or a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sidecar {
    /// Its name: `report.pdf.asc`.
    pub name: String,
    /// The file it signs: the file itself, or `SHA256SUMS` for the list
    /// that may name it.
    pub signs: String,
    /// What checks it.
    pub signer: Signer,
}

/// The signature files to look for beside `name`, in order: `NAME.asc`,
/// `NAME.sig`, `NAME.minisig`, then `SHA256SUMS.asc` (a signed list that
/// may name the file), unless `name` is that list.
pub fn signature_sidecars(name: &str) -> Vec<Sidecar> {
    let mut out: Vec<Sidecar> = [
        ("asc", Signer::Gpg),
        ("sig", Signer::Gpg),
        ("minisig", Signer::Minisign),
    ]
    .into_iter()
    .map(|(extension, signer)| Sidecar {
        name: format!("{name}.{extension}"),
        signs: name.to_owned(),
        signer,
    })
    .collect();
    if !name.eq_ignore_ascii_case("SHA256SUMS") {
        out.push(Sidecar {
            name: "SHA256SUMS.asc".to_owned(),
            signs: "SHA256SUMS".to_owned(),
            signer: Signer::Gpg,
        });
    }
    out
}

/// The sidecars of `name` that are among `siblings`, compared without
/// case, each with the sibling's own spelling, in the sidecars' order.
pub fn sidecars_present(name: &str, siblings: &[&str]) -> Vec<Sidecar> {
    signature_sidecars(name)
        .into_iter()
        .filter_map(|mut sidecar| {
            let found = siblings
                .iter()
                .find(|s| s.eq_ignore_ascii_case(&sidecar.name))?;
            sidecar.name = (*found).to_owned();
            Some(sidecar)
        })
        .collect()
}

/// The chain's sentences (R29 3.5, step 6) after a checksum list was
/// verified, `matched` of its `files` matching: with a good signature,
/// "List is signed: 12 of 12 files match" and the signature's lines;
/// with none, "List not signed: 12 of 12 match" and what that shows; with
/// one that is not good or cannot be checked, its verdict first, then
/// "List not verified: 12 of 12 match".
pub fn chain_lines(
    signature: Option<&SignatureVerdict>,
    matched: usize,
    files: usize,
) -> Vec<String> {
    match signature {
        None => vec![
            chain::not_signed(matched, files),
            chain::ONLY_DAMAGE.to_owned(),
        ],
        Some(verdict) if verdict.is_good() => {
            let mut out = vec![chain::signed(matched, files)];
            out.extend(verdict.lines());
            out
        }
        Some(verdict) => {
            let mut out = verdict.lines();
            out.insert(1, chain::not_good(matched, files));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::ControlFlow;
    use std::sync::Mutex;

    use diroxide_vfs::{ToolOutput, ToolStream};

    struct FakeVerify {
        status: &'static str,
        args: Mutex<Vec<String>>,
    }

    impl ToolRunner for FakeVerify {
        fn has(&self, tool: &str) -> bool {
            tool == GPG
        }
        fn run(&self, request: &ToolRequest, _: &CancelToken) -> Result<ToolOutput, ToolRunError> {
            *self.args.lock().unwrap() = request
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            Ok(ToolOutput::new(
                Some(0),
                self.status.as_bytes().to_vec(),
                Vec::new(),
            ))
        }
        fn run_streaming(
            &self,
            request: &ToolRequest,
            cancel: &CancelToken,
            _: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
        ) -> Result<ToolOutput, ToolRunError> {
            self.run(request, cancel)
        }
    }

    fn local(text: &str) -> Place {
        Place::parse(&if cfg!(windows) {
            format!("D:{}", text.replace('/', "\\"))
        } else {
            text.to_owned()
        })
        .unwrap()
    }

    #[test]
    fn gpg_checks_a_detached_signature_with_its_words() {
        let fake = FakeVerify {
            status: "[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG 168B6F650E9C8469 Ada Example <ada@example.invalid>\n\
[GNUPG:] VALIDSIG 78B5 x\n[GNUPG:] TRUST_UNDEFINED 0 pgp\n",
            args: Mutex::new(Vec::new()),
        };
        let check = SignatureCheck::detached(local("/w/report.pdf.asc"), local("/w/report.pdf"));
        let verdict = verify_signature(&fake, &check, &CancelToken::new());
        assert_eq!(
            verdict.lines(),
            [
                "Signature good: key ending 8469",
                "Key ending 8469 is named Ada Example <ada@example.invalid>",
                "You have not certified this key",
                "Good means the key fits, not who holds it.",
            ]
        );
        let args = fake.args.lock().unwrap();
        assert_eq!(args[..7], gpg::PREFIX);
        assert_eq!(args[7..9], ["--verify", "--"]);
    }

    #[test]
    fn minisign_needs_its_key_and_never_runs_gpg() {
        let fake = FakeVerify {
            status: "",
            args: Mutex::new(Vec::new()),
        };
        let check =
            SignatureCheck::detached(local("/w/SHA256SUMS.minisig"), local("/w/SHA256SUMS"));
        assert!(check.is_minisign());
        assert_eq!(
            verify_signature(&fake, &check, &CancelToken::new()),
            SignatureVerdict::NoKeyGiven
        );
        assert!(fake.args.lock().unwrap().is_empty());
    }

    #[test]
    fn sidecars_in_order_and_as_spelled() {
        let names: Vec<String> = signature_sidecars("report.pdf")
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            names,
            [
                "report.pdf.asc",
                "report.pdf.sig",
                "report.pdf.minisig",
                "SHA256SUMS.asc"
            ]
        );
        let list: Vec<String> = signature_sidecars("SHA256SUMS")
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            list,
            ["SHA256SUMS.asc", "SHA256SUMS.sig", "SHA256SUMS.minisig"]
        );
        let present = sidecars_present("report.pdf", &["REPORT.PDF.SIG", "sha256sums.asc", "x"]);
        assert_eq!(present.len(), 2);
        assert_eq!(present[0].name, "REPORT.PDF.SIG");
        assert_eq!(present[1].signs, "SHA256SUMS");
        assert_eq!(present[0].signer, Signer::Gpg);
    }

    #[test]
    fn the_chain_says_what_a_signature_shows_and_no_more() {
        assert_eq!(
            chain_lines(None, 12, 12),
            [
                "List not signed: 12 of 12 match",
                "This shows the files are not damaged. It does not show who made them.",
            ]
        );
        let good = SignatureVerdict::Good {
            signer: Signer::Minisign,
            key: "E81F".to_owned(),
            name: Some("timestamp:1556193335\tfile:SHA256SUMS".to_owned()),
            certified: true,
        };
        assert_eq!(
            chain_lines(Some(&good), 12, 12),
            [
                "List is signed: 12 of 12 files match",
                "Signature good: key ending E81F",
                "Trusted comment: timestamp:1556193335 file:SHA256SUMS",
                "Good means the key fits, not who holds it.",
            ]
        );
        let bad = SignatureVerdict::Bad { key: None };
        assert_eq!(
            chain_lines(Some(&bad), 12, 12),
            [
                "Signature bad: the file was changed",
                "List not verified: 12 of 12 match",
            ]
        );
        let missing = SignatureVerdict::MissingKey {
            key: Some("8469".to_owned()),
        };
        assert_eq!(
            chain_lines(Some(&missing), 2, 3),
            [
                "Cannot check: public key is missing",
                "List not verified: 2 of 3 match",
                "The key ends 8469.",
                "diroxide does not fetch keys.",
            ]
        );
        for verdict in [
            SignatureVerdict::NoKeyGiven,
            SignatureVerdict::DataUnreadable,
            SignatureVerdict::Cancelled,
            SignatureVerdict::GpgMissing,
        ] {
            assert!(verdict.line().chars().count() <= 40, "{}", verdict.line());
        }
    }
}
