//! The 7z tool backend (alpha 4, R30 commits 80 and 81): RAR and the rest
//! read through 7-Zip. Without the program, each made file says what it
//! needs; with it (the development image's 7-Zip 25.01, or the program
//! `DIRO_TEST_7Z` names), each is listed, its members read through the
//! pipe, and unpacked through the staging road, or refused in words by an
//! older program. Every test file is made at test time under this build's
//! own temporary folder; no password is ever on a command line.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use diroxide_archive::read_tool::{SevenZipVersion, read_with_line, too_old_line};
use diroxide_archive::sniff::{Signature, decide, not_installed_line, sniff_signature};
use diroxide_archive::{
    Allowances, ArchiveFormat, ArchiveProvider, JobEnv, Limits, UnpackRequest, plan_unpack,
    run_unpack,
};
use diroxide_core::{JobId, Place};
use diroxide_ops::{Engine, JobControl};
use diroxide_vfs::{
    CancelToken, OpContext, Registry, ToolInput, ToolOutput, ToolRequest, ToolRunError, ToolRunner,
    ToolStream,
};

use crate::jobs::Sink;
use crate::tool_made::{MEMBERS, ar, arj, cab, cpio, lzh, rar5, rpm};

/// 7-Zip run as a plain process, standing in for the tools registry's
/// runner: the arguments whole, standard input as asked, no window.
pub struct Program(pub PathBuf);

impl ToolRunner for Program {
    fn has(&self, tool: &str) -> bool {
        tool == "7z"
    }

    fn name_of(&self, _tool: &str) -> String {
        "7-Zip".to_owned()
    }

    fn run(
        &self,
        request: &ToolRequest,
        _cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolRunError> {
        let mut command = Command::new(&self.0);
        command
            .args(&request.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(match request.input {
                ToolInput::Closed => Stdio::null(),
                _ => Stdio::piped(),
            });
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| ToolRunError::NotStarted {
            name: "7-Zip".to_owned(),
            message: e.to_string(),
        })?;
        if let (ToolInput::Bytes(bytes), Some(mut stdin)) = (&request.input, child.stdin.take()) {
            let _ = stdin.write_all(bytes);
        }
        let output = child
            .wait_with_output()
            .map_err(|e| ToolRunError::NotStarted {
                name: "7-Zip".to_owned(),
                message: e.to_string(),
            })?;
        Ok(ToolOutput::new(
            output.status.code(),
            output.stdout,
            output.stderr,
        ))
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        _record: &mut dyn FnMut(ToolStream, &str) -> std::ops::ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        self.run(request, cancel)
    }
}

/// The 7-Zip these tests run, with its version: the one `DIRO_TEST_7Z`
/// names, the development image's `7z`, or the program folder's on
/// Windows; `None` when there is none, and the tests that need it say so.
pub fn seven_zip() -> Option<(PathBuf, SevenZipVersion)> {
    let mut tries: Vec<PathBuf> = Vec::new();
    if let Some(named) = std::env::var_os("DIRO_TEST_7Z") {
        tries.push(PathBuf::from(named));
    }
    tries.push(PathBuf::from("7z"));
    if cfg!(windows) {
        tries.push(PathBuf::from(r"C:\Program Files\7-Zip\7z.exe"));
    }
    for program in tries {
        let mut command = Command::new(&program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let Ok(output) = command.output() else {
            continue;
        };
        let banner = String::from_utf8_lossy(&output.stdout);
        if let Some(version) = SevenZipVersion::from_banner(&banner) {
            return Some((program, version));
        }
    }
    None
}

/// A scratch folder under this build's own temporary folder.
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("tool-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// Every made file: its name, its bytes and its signature.
fn made() -> Vec<(&'static str, Vec<u8>, Signature)> {
    vec![
        ("Backup.rar", rar5(&MEMBERS), Signature::Rar),
        ("setup.cab", cab(&MEMBERS), Signature::Cab),
        ("old.arj", arj(&MEMBERS), Signature::Arj),
        ("old.lzh", lzh(&MEMBERS), Signature::Lzh),
        ("made.rpm", rpm(&MEMBERS), Signature::Rpm),
        ("initrd.cpio", cpio(&MEMBERS), Signature::Cpio),
        ("lib.ar", ar(&MEMBERS), Signature::Ar),
    ]
}

#[test]
fn each_made_file_carries_its_signature() {
    for (name, bytes, signature) in made() {
        assert_eq!(sniff_signature(&bytes), Some(signature), "{name}");
        let sniffed = decide(name, &bytes).unwrap();
        assert_eq!(sniffed.format, ArchiveFormat::Tool(signature), "{name}");
        assert!(!sniffed.disagrees(), "{name}");
        // Named as something else, the bytes still win.
        let renamed = decide("data.dat", &bytes).unwrap();
        assert_eq!(renamed.format, ArchiveFormat::Tool(signature), "{name}");
    }
}

/// The provider over the local disk, its spool in `dir`, and a context
/// that runs `tools`.
fn world(
    dir: &Path,
    tools: Arc<dyn ToolRunner>,
) -> (Arc<Registry>, Arc<ArchiveProvider>, OpContext) {
    let (registry, provider) = ArchiveProvider::install(Registry::with_local(), Limits::default());
    let spool = dir.join("spool");
    std::fs::create_dir_all(&spool).unwrap();
    provider.set_spool_dir(Some(spool));
    let ctx = OpContext::new().with_tools(tools);
    (registry, provider, ctx)
}

fn place_of(path: &Path) -> Place {
    Place::parse(&path.to_string_lossy()).unwrap()
}

#[test]
fn without_7_zip_each_format_says_what_it_needs() {
    let dir = scratch();
    let (_registry, provider, ctx) = world(dir.path(), Arc::new(diroxide_vfs::NoTools));
    let path = dir.path().join("Backup.rar");
    std::fs::write(&path, rar5(&MEMBERS)).unwrap();
    let root = place_of(&path).into_archive();
    let error = provider
        .list_folder(&root, &ctx, Allowances::default())
        .unwrap_err();
    assert_eq!(error.to_string(), "it needs 7-Zip");
    assert_eq!(
        error.message("open", "Backup.rar").text,
        "Could not open Backup.rar: it needs 7-Zip. Get it from Manage optional components, then try again."
    );
    assert_eq!(
        not_installed_line(Signature::Rar),
        "RAR needs 7-Zip: not installed"
    );
    assert_eq!(read_with_line("Backup.rar"), "Backup.rar, read with 7-Zip");
}

#[test]
fn seven_zip_lists_reads_and_unpacks_the_made_files() {
    let Some((program, version)) = seven_zip() else {
        eprintln!("7-Zip not found: the 7z tool tests did not run");
        return;
    };
    eprintln!("7-Zip {} at {}", version.words(), program.display());
    let dir = scratch();
    let tools: Arc<dyn ToolRunner> = Arc::new(Program(program));
    let (registry, provider, ctx) = world(dir.path(), tools.clone());
    for (name, bytes, signature) in made() {
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        let root = place_of(&path).into_archive();
        let listed = provider.list_folder(&root, &ctx, Allowances::default());
        let listing = match listed {
            Ok(listing) => listing,
            // Debian's 7-Zip leaves out the RAR code (R7): said in words.
            Err(error) if signature == Signature::Rar => {
                assert_eq!(error.to_string(), "it needs a 7-Zip that reads RAR");
                eprintln!("{name}: {error}");
                continue;
            }
            Err(error) => panic!("{name}: {error}"),
        };
        let names: Vec<String> = listing.entries.iter().map(|e| e.name.clone()).collect();
        eprintln!("{name}: {names:?}, {}", provider.title(&root));
        assert!(
            provider
                .title(&root)
                .starts_with(&format!("{name}, read with 7-Zip")),
            "{}",
            provider.title(&root)
        );
        let opened = provider
            .open_archive(&place_of(&path), &ctx, Allowances::default())
            .unwrap();
        assert!(opened.read_with_tool() && !opened.random_read());
        assert_eq!(opened.tool_version(), Some(version));
        if signature == Signature::Rpm {
            // An RPM lists its payload as one member: the cpio inside.
            assert_eq!(names.len(), 1, "{names:?}");
            continue;
        }
        if signature == Signature::Ar {
            // ar keeps names only: both files at its root.
            assert!(names.contains(&"notes.txt".to_owned()), "{names:?}");
        } else {
            assert!(names.contains(&"docs".to_owned()), "{names:?}");
            let notes = root.join("docs").unwrap().join("notes.txt").unwrap();
            if version.extracts() {
                let mut text = String::new();
                provider
                    .read_member(&notes, &ctx, Allowances::default(), None)
                    .unwrap()
                    .read_to_string(&mut text)
                    .unwrap();
                assert_eq!(text, "the notes, read with 7-Zip", "{name}");
            }
        }
        // A whole unpack: the staging road on 25.01 and later, the words
        // on an older program.
        let out = dir.path().join(format!("out-{}", signature.short()));
        std::fs::create_dir_all(&out).unwrap();
        let request = UnpackRequest::new(place_of(&path), place_of(&out));
        let planned = plan_unpack(
            &provider,
            &registry,
            &request,
            JobId::new(5),
            &ctx,
            Allowances::default(),
        );
        if !version.extracts() {
            let error = planned.unwrap_err();
            assert_eq!(
                error.message("unpack", name).text,
                format!("Could not unpack {name}: {}.", too_old_line(version))
            );
            continue;
        }
        let plan = planned.unwrap();
        let engine = Engine::new(registry.clone());
        let control = JobControl::new();
        let sink = Sink::skipping();
        let env = JobEnv::new(&engine, &provider, JobId::new(5), &control, &sink)
            .with_tools(tools.clone());
        let outcome = run_unpack(&plan, None, Allowances::default(), &env);
        eprintln!("{name}: {:?}", outcome.lines);
        let readme = std::fs::read_to_string(find(&out, "readme.txt")).unwrap();
        assert_eq!(readme, "read me first", "{name}");
        let staging: Vec<_> = std::fs::read_dir(dir.path().join("spool"))
            .unwrap()
            .collect();
        assert!(staging.is_empty(), "the staging folder is gone");
    }
}

/// A file named `name` somewhere under `dir`.
fn find(dir: &Path, name: &str) -> PathBuf {
    let mut stack = vec![dir.to_owned()];
    while let Some(folder) = stack.pop() {
        for item in std::fs::read_dir(&folder).unwrap() {
            let item = item.unwrap();
            if item.file_type().unwrap().is_dir() {
                stack.push(item.path());
            } else if item.file_name() == name {
                return item.path();
            }
        }
    }
    panic!("{name} not under {}", dir.display());
}
