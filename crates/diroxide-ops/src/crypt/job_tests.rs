//! The gpg jobs over a fake gpg that answers with the status words spike
//! ES3 saw from gpg 2.4.9 and 2.4.7, in scratch folders under the
//! build's own folder. No real gpg runs here; the passphrases are made
//! for each test and looked for everywhere they must not be.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diroxide_core::{JobId, JobKind, Place, Secret, SecretKind, SecretRequest};
use diroxide_vfs::{
    CancelToken, Registry, SecretSource, ToolOutput, ToolRequest, ToolRunError, ToolRunner,
    ToolStream,
};

use super::job::{GpgAction, GpgError, GpgOptions, GpgRequest};
use super::lines;
use crate::engine::{Engine, JobControl};
use crate::testing::Script;

fn scratch() -> tempfile::TempDir {
    let exe = std::env::current_exe().unwrap();
    let base = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("crypt-unit-tmp");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("gpg-")
        .tempdir_in(&base)
        .unwrap()
}

fn place(path: &Path) -> Place {
    Place::parse(&path.to_string_lossy()).unwrap()
}

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const KEY_ID: &str = "9D5EB0D1926687A4";
const SIGNER: &str = "0B8D814646FF8E39A6B422008C65CD3C79AB3972";

/// A stand-in for gpg: what it writes and says, by the arguments.
struct FakeGpg {
    installed: bool,
    passphrase: String,
    /// Whether a loopback success is remembered by the agent for later
    /// runs (Linux); on Windows the runner stops the agent (ES3).
    agent_caches: bool,
    cached: Mutex<bool>,
    uncertified: bool,
    runs: Mutex<Vec<Vec<String>>>,
    inputs: Mutex<Vec<Vec<u8>>>,
}

impl FakeGpg {
    fn new(passphrase: &str) -> Self {
        Self {
            installed: true,
            passphrase: passphrase.to_owned(),
            agent_caches: false,
            cached: Mutex::new(false),
            uncertified: false,
            runs: Mutex::new(Vec::new()),
            inputs: Mutex::new(Vec::new()),
        }
    }

    fn modes(&self) -> Vec<String> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|args| {
                args.iter()
                    .position(|a| a == "--pinentry-mode")
                    .map(|i| args[i + 1].clone())
            })
            .collect()
    }

    fn answer(&self, request: &ToolRequest) -> ToolOutput {
        let args: Vec<String> = request
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        self.runs.lock().unwrap().push(args.clone());
        // Every gpg run gets the C locale (E's request 6).
        assert!(
            request.env.iter().any(|(k, v)| k == "LC_ALL" && v == "C"),
            "{:?}",
            request.env
        );
        let stdin = request
            .input
            .bytes()
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        self.inputs.lock().unwrap().push(stdin.clone());
        let has = |word: &str| args.iter().any(|a| a == word);
        let output = args
            .iter()
            .position(|a| a == "--output")
            .map(|i| PathBuf::from(&args[i + 1]));
        let dash = args.iter().position(|a| a == "--").unwrap();
        let input = PathBuf::from(&args[dash + 1]);
        let loopback = args
            .windows(2)
            .any(|w| w == ["--pinentry-mode", "loopback"]);
        let right = stdin == format!("{}\n", self.passphrase).into_bytes();
        let out = |status: &str, code: i32| {
            ToolOutput::new(
                Some(code),
                status.as_bytes().to_vec(),
                b"gpg: some words\n".to_vec(),
            )
        };
        let write = |bytes: &[u8]| {
            let output = output.clone().unwrap();
            assert!(!output.exists(), "gpg never writes over a file");
            std::fs::write(output, bytes).unwrap();
        };
        let data = std::fs::read(&input).unwrap_or_default();
        if has("--encrypt") {
            if self.uncertified {
                return out(
                    "[GNUPG:] KEY_CONSIDERED B5FD 0\n[GNUPG:] INV_RECP 10 B5FD3E56\n[GNUPG:] FAILURE encrypt 53\n",
                    2,
                );
            }
            let mut made = b"gpg-encrypted:".to_vec();
            made.extend_from_slice(&data);
            write(&made);
            return out(
                "[GNUPG:] BEGIN_ENCRYPTION 2 9\n[GNUPG:] END_ENCRYPTION\n",
                0,
            );
        }
        if has("--symmetric") {
            assert!(loopback);
            if stdin.len() <= 1 {
                return out("[GNUPG:] FAILURE encrypt 67108949\n", 2);
            }
            let mut made = b"gpg-encrypted:".to_vec();
            made.extend_from_slice(&data);
            write(&made);
            return out(
                "[GNUPG:] NEED_PASSPHRASE_SYM 9 3 2\n[GNUPG:] BEGIN_ENCRYPTION 2 9\n[GNUPG:] END_ENCRYPTION\n",
                0,
            );
        }
        let cached = *self.cached.lock().unwrap();
        if has("--decrypt") {
            let Some(plain) = data.strip_prefix(b"gpg-encrypted:") else {
                return out(
                    "[GNUPG:] NODATA 1\n[GNUPG:] NODATA 2\n[GNUPG:] FAILURE decrypt 4294967295\n",
                    2,
                );
            };
            let head = format!("[GNUPG:] ENC_TO {KEY_ID} 18 0\n[GNUPG:] KEY_CONSIDERED A 0\n");
            if (loopback && right) || (!loopback && cached) {
                if self.agent_caches {
                    *self.cached.lock().unwrap() = true;
                }
                write(plain);
                return out(
                    &format!(
                        "{head}[GNUPG:] BEGIN_DECRYPTION\n[GNUPG:] DECRYPTION_OKAY\n[GNUPG:] GOODMDC\n[GNUPG:] END_DECRYPTION\n"
                    ),
                    0,
                );
            }
            let code = if loopback { 67108875 } else { 67108949 };
            return out(
                &format!(
                    "{head}[GNUPG:] NO_SECKEY {KEY_ID}\n[GNUPG:] ERROR pkdecrypt_failed {code}\n[GNUPG:] DECRYPTION_FAILED\n[GNUPG:] FAILURE gpg-exit 33554433\n"
                ),
                2,
            );
        }
        if has("--detach-sign") {
            let head = format!("[GNUPG:] KEY_CONSIDERED {SIGNER} 0\n[GNUPG:] BEGIN_SIGNING H10\n");
            if (loopback && right) || (!loopback && cached) {
                write(b"a signature");
                return out(
                    &format!("{head}[GNUPG:] SIG_CREATED D 22 10 00 1 {SIGNER}\n"),
                    0,
                );
            }
            let code = if loopback { 67108875 } else { 67108949 };
            return out(&format!("{head}[GNUPG:] FAILURE sign {code}\n"), 2);
        }
        out("", 2)
    }
}

impl ToolRunner for FakeGpg {
    fn has(&self, tool: &str) -> bool {
        self.installed && tool == "gpg"
    }

    fn run(&self, request: &ToolRequest, cancel: &CancelToken) -> Result<ToolOutput, ToolRunError> {
        if cancel.is_cancelled() {
            return Err(ToolRunError::Cancelled);
        }
        if !self.installed {
            return Err(ToolRunError::NotInstalled {
                tool: "gpg".into(),
                name: "GnuPG".into(),
            });
        }
        Ok(self.answer(request))
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        _record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        self.run(request, cancel)
    }
}

/// The owner's answers to the masked prompt, in order; none when they run
/// out.
#[derive(Default)]
struct Answers {
    answers: Mutex<VecDeque<String>>,
    asked: Mutex<Vec<SecretRequest>>,
}

impl Answers {
    fn of(answers: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.iter().map(|a| (*a).to_owned()).collect()),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl SecretSource for Answers {
    fn ask(&self, request: &SecretRequest, _cancel: &CancelToken) -> Option<Secret> {
        self.asked.lock().unwrap().push(request.clone());
        self.answers.lock().unwrap().pop_front().map(Secret::new)
    }
}

fn canary(tag: &str) -> String {
    format!("canary-{tag}-7f3a-not-a-password")
}

struct Setup {
    dir: tempfile::TempDir,
    gpg: Arc<FakeGpg>,
    engine: Engine,
}

fn setup(gpg: FakeGpg) -> Setup {
    let gpg = Arc::new(gpg);
    let engine = Engine::new(Arc::new(Registry::with_local()))
        .with_tools(Arc::clone(&gpg) as Arc<dyn ToolRunner>);
    Setup {
        dir: scratch(),
        gpg,
        engine,
    }
}

fn files(dir: &Path, names: &[&str]) -> Vec<Place> {
    names
        .iter()
        .map(|n| {
            let path = dir.join(n);
            std::fs::write(&path, format!("the text of {n}")).unwrap();
            place(&path)
        })
        .collect()
}

fn run(
    setup: &Setup,
    sources: Vec<Place>,
    options: GpgOptions,
    secrets: Arc<dyn SecretSource>,
) -> super::job::GpgReport {
    let request = GpgRequest::new(JobId::new(7), sources, options).with_secrets(secrets);
    let control = JobControl::new();
    let plan = setup.engine.plan_gpg(&request, &control).unwrap();
    setup.engine.run_gpg(&plan, &control, &Script::default())
}

fn assert_no_secret(setup: &Setup, report: &super::job::GpgReport, secrets: &[&str]) {
    let args = setup.gpg.runs.lock().unwrap().concat().join(" ");
    let said = report.lines().join(" ");
    let debug = format!("{report:?}");
    for secret in secrets {
        assert!(!args.contains(secret), "a passphrase in an argument");
        assert!(!said.contains(secret) && !debug.contains(secret));
    }
    assert!(
        !names_in(setup.dir.path())
            .iter()
            .any(|n| n.starts_with(".diro-part-")),
        "a staging file was left"
    );
}

#[test]
fn encrypting_to_recipients_writes_beside_each_file_and_asks_nothing() {
    let setup = setup(FakeGpg::new("unused"));
    let sources = files(setup.dir.path(), &["a.txt", "b.txt"]);
    std::fs::write(setup.dir.path().join("b.txt.gpg"), b"someone else's").unwrap();
    let answers = Answers::of(&[]);
    let options = GpgOptions {
        action: GpgAction::Encrypt {
            recipients: vec!["ada@example.invalid".to_owned()],
        },
        armor: false,
    };
    let report = run(&setup, sources, options, answers.clone());
    assert!(answers.asked.lock().unwrap().is_empty());
    assert_eq!(report.job.kind, JobKind::Encrypt);
    assert_eq!(report.job.done.len(), 2);
    assert_eq!(report.job.undo.created.len(), 2);
    assert_eq!(
        names_in(setup.dir.path()),
        ["a.txt", "a.txt.gpg", "b.txt", "b.txt (2).gpg", "b.txt.gpg"]
    );
    assert_eq!(
        std::fs::read(setup.dir.path().join("b.txt.gpg")).unwrap(),
        b"someone else's"
    );
    let lines = report.lines();
    assert!(
        lines.contains(&"Encrypted a.txt to a.txt.gpg".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"Encrypted b.txt to b.txt (2).gpg".to_owned()));
    let runs = setup.gpg.runs.lock().unwrap();
    let first = &runs[0];
    assert_eq!(first[..7], super::gpg::PREFIX);
    assert!(
        first
            .windows(2)
            .any(|w| w == ["--recipient", "ada@example.invalid"])
    );
    let output = &first[first.iter().position(|a| a == "--output").unwrap() + 1];
    assert!(
        Path::new(output)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".diro-part-")
    );
    drop(runs);
    assert_no_secret(&setup, &report, &[]);
}

#[test]
fn a_key_not_certified_stops_the_job_with_its_words() {
    let mut gpg = FakeGpg::new("unused");
    gpg.uncertified = true;
    let setup = setup(gpg);
    let sources = files(setup.dir.path(), &["a.txt", "b.txt"]);
    let options = GpgOptions {
        action: GpgAction::Encrypt {
            recipients: vec!["B5FD3E56".to_owned()],
        },
        armor: true,
    };
    let report = run(&setup, sources, options, Answers::of(&[]));
    assert_eq!(report.job.failed.len(), 1);
    assert_eq!(report.job.failed[0].1, lines::NOT_CERTIFIED);
    assert_eq!(report.job.skipped[0].1, lines::NOT_CERTIFIED);
    assert_eq!(
        setup.gpg.runs.lock().unwrap().len(),
        1,
        "one run, not one a file"
    );
    assert_eq!(names_in(setup.dir.path()), ["a.txt", "b.txt"]);
    assert!(report.lines().contains(&format!(
        "Could not encrypt a.txt: {}",
        lines::NOT_CERTIFIED
    )));
}

#[test]
fn a_passphrase_encryption_asks_a_new_passphrase_once_and_gives_it_on_standard_input() {
    let pass = canary("sym");
    let setup = setup(FakeGpg::new(&pass));
    let sources = files(setup.dir.path(), &["a.txt", "b.txt"]);
    let answers = Answers::of(&[&pass]);
    let options = GpgOptions {
        action: GpgAction::EncryptWithPassphrase,
        armor: true,
    };
    let report = run(&setup, sources, options, answers.clone());
    assert_eq!(report.job.done.len(), 2);
    let asked = answers.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].new_secret, "the masked prompt asks it twice");
    assert_eq!(asked[0].kind, SecretKind::FilePassphrase);
    assert_eq!(
        names_in(setup.dir.path()),
        ["a.txt", "a.txt.asc", "b.txt", "b.txt.asc"]
    );
    let inputs = setup.gpg.inputs.lock().unwrap();
    assert!(
        inputs
            .iter()
            .all(|i| *i == format!("{pass}\n").into_bytes())
    );
    drop(inputs);
    assert_no_secret(&setup, &report, &[&pass]);
}

#[test]
fn decryption_asks_once_for_a_key_even_when_the_agent_forgets() {
    let pass = canary("key");
    let setup = setup(FakeGpg::new(&pass));
    let dir = setup.dir.path();
    std::fs::write(dir.join("a.txt.gpg"), b"gpg-encrypted:first").unwrap();
    std::fs::write(dir.join("b.txt.gpg"), b"gpg-encrypted:second").unwrap();
    let answers = Answers::of(&[&pass]);
    let options = GpgOptions {
        action: GpgAction::Decrypt,
        armor: false,
    };
    let report = run(
        &setup,
        vec![place(&dir.join("a.txt.gpg")), place(&dir.join("b.txt.gpg"))],
        options,
        answers.clone(),
    );
    assert_eq!(report.job.kind, JobKind::Decrypt);
    assert_eq!(report.job.done.len(), 2, "{:?}", report.lines());
    let asked = answers.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].kind, SecretKind::ServerPrompt);
    assert_eq!(
        asked[0].prompt_line(),
        "Passphrase for key ending 87A4 (hidden): "
    );
    assert_eq!(asked[0].prompt_line().trim_end().chars().count(), 40);
    drop(asked);
    assert_eq!(
        setup.gpg.modes(),
        ["error", "loopback", "error", "loopback"]
    );
    assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"first");
    let lines = report.lines();
    assert!(lines.contains(&"Decrypted a.txt.gpg to a.txt.".to_owned()));
    assert_eq!(lines.last().unwrap(), lines::NOT_PROOF);
    assert_no_secret(&setup, &report, &[&pass]);
}

#[test]
fn a_key_the_agent_holds_never_asks() {
    let pass = canary("held");
    let mut gpg = FakeGpg::new(&pass);
    gpg.agent_caches = true;
    *gpg.cached.lock().unwrap() = true;
    let setup = setup(gpg);
    let dir = setup.dir.path();
    std::fs::write(dir.join("a.gpg"), b"gpg-encrypted:x").unwrap();
    let answers = Answers::of(&[]);
    let options = GpgOptions {
        action: GpgAction::Decrypt,
        armor: false,
    };
    let report = run(
        &setup,
        vec![place(&dir.join("a.gpg"))],
        options,
        answers.clone(),
    );
    assert_eq!(report.job.done.len(), 1);
    assert!(answers.asked.lock().unwrap().is_empty());
    assert_eq!(setup.gpg.modes(), ["error"]);
}

#[test]
fn a_wrong_passphrase_is_asked_again_three_times_at_most() {
    let pass = canary("right");
    let setup = setup(FakeGpg::new(&pass));
    let dir = setup.dir.path();
    std::fs::write(dir.join("a.gpg"), b"gpg-encrypted:x").unwrap();
    let wrong = canary("wrong");
    let answers = Answers::of(&[&wrong, &pass]);
    let options = GpgOptions {
        action: GpgAction::Decrypt,
        armor: false,
    };
    let report = run(
        &setup,
        vec![place(&dir.join("a.gpg"))],
        options.clone(),
        answers.clone(),
    );
    assert_eq!(report.job.done.len(), 1);
    let attempts: Vec<u32> = answers
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.attempt)
        .collect();
    assert_eq!(attempts, [1, 2]);
    assert_no_secret(&setup, &report, &[&pass, &wrong]);

    let setup = self::setup(FakeGpg::new(&pass));
    let dir = setup.dir.path();
    std::fs::write(dir.join("a.gpg"), b"gpg-encrypted:x").unwrap();
    let answers = Answers::of(&[&wrong, &wrong, &wrong, &pass]);
    let report = run(
        &setup,
        vec![place(&dir.join("a.gpg"))],
        options.clone(),
        answers.clone(),
    );
    assert_eq!(answers.asked.lock().unwrap().len(), 3);
    assert_eq!(report.job.failed[0].1, "wrong passphrase 3 times");
    assert_eq!(names_in(dir), ["a.gpg"]);

    // No answer: skipped, said.
    let setup = self::setup(FakeGpg::new(&pass));
    let dir = setup.dir.path();
    std::fs::write(dir.join("a.gpg"), b"gpg-encrypted:x").unwrap();
    let report = run(
        &setup,
        vec![place(&dir.join("a.gpg"))],
        options,
        Answers::of(&[]),
    );
    assert_eq!(report.job.skipped[0].1, lines::NO_PASSPHRASE);
}

#[test]
fn a_file_that_is_not_openpgp_says_so() {
    let setup = setup(FakeGpg::new("x"));
    let sources = files(setup.dir.path(), &["photo.jpg"]);
    let options = GpgOptions {
        action: GpgAction::Decrypt,
        armor: false,
    };
    let report = run(&setup, sources, options, Answers::of(&[]));
    assert_eq!(report.job.failed[0].1, lines::NOT_OPENPGP);
    assert!(
        report
            .lines()
            .contains(&"Could not decrypt photo.jpg: it is not an OpenPGP file".to_owned())
    );
}

#[test]
fn signing_names_the_key_and_writes_a_sig_beside_the_file() {
    let pass = canary("sign");
    let setup = setup(FakeGpg::new(&pass));
    let sources = files(setup.dir.path(), &["report.pdf"]);
    let answers = Answers::of(&[&pass]);
    let options = GpgOptions {
        action: GpgAction::Sign {
            key: Some(SIGNER.to_owned()),
        },
        armor: false,
    };
    let request = GpgRequest::new(JobId::new(9), sources, options).with_secrets(answers.clone());
    let control = JobControl::new();
    let plan = setup.engine.plan_gpg(&request, &control).unwrap();
    assert_eq!(plan.summary(), "Sign 1 file with key ending 3972");
    assert_eq!(plan.question(), "Sign 1 file with key ending 3972? y or n");
    assert_eq!(plan.lines()[1], "report.pdf to report.pdf.sig");
    let report = setup.engine.run_gpg(&plan, &control, &Script::default());
    assert_eq!(report.job.kind, JobKind::Sign);
    assert!(
        report
            .lines()
            .contains(&"Signed report.pdf in report.pdf.sig".to_owned())
    );
    assert_eq!(
        answers.asked.lock().unwrap()[0].title(),
        "Passphrase for key ending 3972"
    );
    assert_no_secret(&setup, &report, &[&pass]);
}

#[test]
fn the_plan_refuses_what_gpg_cannot_do() {
    let mut gpg = FakeGpg::new("x");
    gpg.installed = false;
    let setup = setup(gpg);
    let sources = files(setup.dir.path(), &["a.txt"]);
    let control = JobControl::new();
    let decrypt = GpgOptions {
        action: GpgAction::Decrypt,
        armor: false,
    };
    let missing = setup
        .engine
        .plan_gpg(
            &GpgRequest::new(JobId::new(1), sources.clone(), decrypt.clone()),
            &control,
        )
        .unwrap_err();
    assert_eq!(missing, GpgError::Missing);
    assert_eq!(missing.to_string(), "gpg not found. Install GnuPG first.");

    let setup = self::setup(FakeGpg::new("x"));
    let sources = files(setup.dir.path(), &["a.txt"]);
    let bad = GpgOptions {
        action: GpgAction::Encrypt {
            recipients: vec!["--trust-model=always".to_owned()],
        },
        armor: false,
    };
    assert!(matches!(
        setup.engine.plan_gpg(
            &GpgRequest::new(JobId::new(1), sources.clone(), bad),
            &control
        ),
        Err(GpgError::BadRecipient(_))
    ));
    let none = GpgOptions {
        action: GpgAction::Encrypt { recipients: vec![] },
        armor: false,
    };
    assert_eq!(
        setup
            .engine
            .plan_gpg(&GpgRequest::new(JobId::new(1), sources, none), &control)
            .unwrap_err(),
        GpgError::NoRecipient
    );
    // A folder is refused in the plan, with its reason.
    std::fs::create_dir(setup.dir.path().join("Photos")).unwrap();
    let plan = setup
        .engine
        .plan_gpg(
            &GpgRequest::new(
                JobId::new(1),
                vec![place(&setup.dir.path().join("Photos"))],
                decrypt,
            ),
            &control,
        )
        .unwrap();
    assert!(plan.items.is_empty());
    assert_eq!(plan.lines()[1], "Refused: Photos, it is not a file");
}

#[test]
fn a_cancel_before_the_run_makes_nothing() {
    let setup = setup(FakeGpg::new("x"));
    let sources = files(setup.dir.path(), &["a.txt"]);
    let options = GpgOptions {
        action: GpgAction::Encrypt {
            recipients: vec!["4E7B".to_owned()],
        },
        armor: false,
    };
    let request = GpgRequest::new(JobId::new(3), sources, options);
    let control = JobControl::new();
    let plan = setup.engine.plan_gpg(&request, &control).unwrap();
    control.cancel.cancel();
    let report = setup.engine.run_gpg(&plan, &control, &Script::default());
    assert!(report.job.cancelled);
    assert_eq!(names_in(setup.dir.path()), ["a.txt"]);
}

#[test]
fn output_names_follow_the_action() {
    let encrypt = GpgOptions {
        action: GpgAction::EncryptWithPassphrase,
        armor: false,
    };
    assert_eq!(encrypt.output_name("a.txt"), "a.txt.gpg");
    let sign = GpgOptions {
        action: GpgAction::Sign { key: None },
        armor: true,
    };
    assert_eq!(sign.output_name("a.txt"), "a.txt.asc");
    assert_eq!(super::job::decrypted_name("a.txt.GPG"), "a.txt");
    assert_eq!(super::job::decrypted_name("a.pgp"), "a");
    assert_eq!(super::job::decrypted_name(".gpg"), ".gpg.decrypted");
    assert_eq!(super::job::decrypted_name("blob"), "blob.decrypted");
    let _ = OsString::new();
}
