//! The cross-checks of listings and headers (research/archives.md section
//! 11): the zip listing against `7z l -slt` (direction one), and packed
//! archives against `unzip -t`, `7z t`, `tar -tvf` with no owner names,
//! `gzip -t` and `xz -t` (direction two); the helpers the whole suite in
//! `cross_suite` shares. Each runs only when `DIRO_CROSS_CHECK=1` is set
//! and its tool is on PATH (the container has 7-Zip 25.01); otherwise it
//! says why it did not run and passes.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;

use diroxide_archive::{ArchiveProvider, Limits};
use diroxide_core::{EntryKind, Place};
use diroxide_vfs::{OpContext, Provider, Registry};

/// Whether the cross-check may run, or why not.
pub fn oracle(tool: &str) -> Result<(), String> {
    if std::env::var_os("DIRO_CROSS_CHECK").is_none_or(|v| v != "1") {
        return Err("set DIRO_CROSS_CHECK=1 to run it".to_owned());
    }
    match Command::new(tool).arg("--help").output() {
        Ok(_) => Ok(()),
        Err(_) => Err(format!("{tool} is not on PATH")),
    }
}

/// `7z l -slt`'s listing: path, size and CRC of each file.
fn seven_z_listing(archive: &std::path::Path) -> BTreeMap<String, (u64, Option<u32>)> {
    let output = Command::new("7z")
        .args(["l", "-slt", "-ba"])
        .arg(archive)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut listing = BTreeMap::new();
    for block in text.split("\n\n").chain(text.split("\r\n\r\n")) {
        let field = |key: &str| {
            block
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key} = ")))
                .map(str::trim)
                .map(str::to_owned)
        };
        let (Some(path), Some(folder)) = (field("Path"), field("Folder")) else {
            continue;
        };
        if folder == "+" {
            continue;
        }
        let size = field("Size").and_then(|s| s.parse().ok()).unwrap_or(0);
        let crc = field("CRC").and_then(|c| u32::from_str_radix(&c, 16).ok());
        listing.insert(path.replace('\\', "/"), (size, crc));
    }
    listing
}

/// The provider's listing of every file, walking its folders.
fn our_listing(
    provider: &ArchiveProvider,
    archive: &Place,
) -> BTreeMap<String, (u64, Option<u32>)> {
    let ctx = OpContext::new();
    let mut listing = BTreeMap::new();
    let mut stack = vec![(archive.clone().into_archive(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for entry in provider.list(&folder, &ctx).unwrap() {
            let path = format!("{prefix}{}", entry.name);
            if entry.kind == EntryKind::Folder {
                stack.push((folder.join(&entry.name).unwrap(), format!("{path}/")));
            } else {
                listing.insert(path, (entry.size.unwrap_or(0), entry.crc32));
            }
        }
    }
    listing
}

#[test]
fn the_zip_listing_matches_seven_z() {
    if let Err(why) = oracle("7z") {
        eprintln!("cross-check against 7z l -slt not run: {why}");
        return;
    }
    let dir = crate::spike_listing::scratch();
    let path = dir.path().join("cross.zip");
    let bytes = crate::build::zip_of(&[
        ("week1/", b""),
        ("week1/notes.txt", b"hello"),
        ("week1/deep/more.txt", &[b'm'; 70_000]),
        ("caf\u{e9}.txt", b"accents"),
        ("empty.txt", b""),
    ]);
    std::fs::write(&path, bytes).unwrap();
    let provider = Arc::new(ArchiveProvider::new(Limits::default()));
    let mut registry = Registry::with_local();
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    let place = Place::parse(&path.to_string_lossy()).unwrap();
    let ours = our_listing(&provider, &place);
    let theirs = seven_z_listing(&path);
    eprintln!("cross-check against 7z l -slt ran: {} files", theirs.len());
    assert_eq!(ours, theirs);
    let _ = provider.capabilities(&place);
}

/// The first line a tool says about its version, for the report.
pub fn version_of(tool: &str, flag: &str) -> String {
    Command::new(tool)
        .arg(flag)
        .output()
        .map(|out| {
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let text = if text.trim().is_empty() {
                String::from_utf8_lossy(&out.stderr).into_owned()
            } else {
                text
            };
            let line = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim();
            // The version, not the sentence or the library list after it.
            let line = line.split("  ").next().unwrap_or(line);
            line.split(" zlib/")
                .next()
                .unwrap_or(line)
                .trim_end_matches('.')
                .to_owned()
        })
        .unwrap_or_default()
}

/// A tree of plain names on disk to pack: text, noise, an empty file, a
/// folder two deep.
pub fn source_tree(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("course");
    std::fs::create_dir_all(root.join("week1").join("deep")).unwrap();
    std::fs::write(root.join("notes.txt"), "the notes\n".repeat(200)).unwrap();
    std::fs::write(root.join("empty.txt"), b"").unwrap();
    std::fs::write(root.join("week1").join("a.txt"), b"week one").unwrap();
    std::fs::write(
        root.join("week1").join("deep").join("noise.bin"),
        crate::build::noise(100_000, 9),
    )
    .unwrap();
    root
}

/// Packs `source` on disk into `name` beside it, through the real local
/// provider, with an optional password; gives the archive's path.
pub fn pack_on_disk(
    source: &std::path::Path,
    name: &str,
    format: diroxide_archive::ArchiveFormat,
    password: Option<&str>,
) -> std::path::PathBuf {
    pack_on_disk_with(source, name, format, password, |_| {})
}

/// [`pack_on_disk`], with the request changed first (solid, level).
pub fn pack_on_disk_with(
    source: &std::path::Path,
    name: &str,
    format: diroxide_archive::ArchiveFormat,
    password: Option<&str>,
    change: impl FnOnce(&mut diroxide_archive::PackRequest),
) -> std::path::PathBuf {
    use diroxide_archive::{JobEnv, PackRequest, plan_pack, run_pack};
    let (registry, archives) = ArchiveProvider::install(Registry::with_local(), Limits::default());
    let engine = diroxide_ops::Engine::new(registry.clone());
    let destination = source.parent().unwrap().join(name);
    let mut request = PackRequest {
        password: password.map(|p| diroxide_core::Secret::new(p.to_owned())),
        encrypt_names: password.is_some(),
        ..PackRequest::new(
            vec![Place::parse(&source.to_string_lossy()).unwrap()],
            Place::parse(&destination.to_string_lossy()).unwrap(),
            format,
        )
    };
    change(&mut request);
    let job = diroxide_core::JobId::new(7);
    let plan = plan_pack(&registry, &request, job, &OpContext::new()).unwrap();
    let control = diroxide_ops::JobControl::new();
    let sink = crate::jobs::Sink::skipping();
    let env = JobEnv::new(&engine, &archives, job, &control, &sink);
    let outcome = run_pack(&plan, request.password.as_ref(), &env);
    assert!(outcome.report.failed.is_empty(), "{:?}", outcome.lines);
    assert!(destination.exists(), "{:?}", outcome.lines);
    destination
}

/// Runs a tool and says whether it succeeded, with what it printed.
pub fn tool_says(tool: &str, args: &[&std::ffi::OsStr]) -> (bool, String) {
    let output = Command::new(tool).args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn diroxide_zips_pass_unzip_and_seven_z() {
    let tools: Vec<&str> = ["unzip", "7z"]
        .into_iter()
        .filter(|tool| match oracle(tool) {
            Ok(()) => true,
            Err(why) => {
                eprintln!("cross-check of packed zips against {tool} not run: {why}");
                false
            }
        })
        .collect();
    if tools.is_empty() {
        return;
    }
    let dir = crate::spike_listing::scratch();
    let source = source_tree(dir.path());
    for password in [None, Some("cross-check-password")] {
        let name = if password.is_some() {
            "locked.zip"
        } else {
            "plain.zip"
        };
        let zip = pack_on_disk(
            &source,
            name,
            diroxide_archive::ArchiveFormat::Zip,
            password,
        );
        for tool in &tools {
            let version = if *tool == "7z" {
                version_of("7z", "i")
            } else {
                version_of("unzip", "-v")
            };
            let mut args: Vec<std::ffi::OsString> = Vec::new();
            match (*tool, password) {
                ("unzip", Some(p)) => args.extend(["-t".into(), "-P".into(), p.into()]),
                ("unzip", None) => args.push("-t".into()),
                (_, Some(p)) => args.extend(["t".into(), format!("-p{p}").into()]),
                (_, None) => args.push("t".into()),
            }
            args.push(zip.clone().into_os_string());
            let args: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
            let (ok, said) = tool_says(tool, &args);
            // Info-ZIP's unzip reads no AES; 7-Zip does.
            if *tool == "unzip" && password.is_some() {
                eprintln!("cross-check: {version} does not read AES zips; 7z checks those");
                continue;
            }
            assert!(ok, "{tool} on {name}: {said}");
            eprintln!("cross-check: {name} passed {tool} ({version})");
        }
    }
}

#[test]
fn diroxide_tars_pass_tar_xz_and_gzip_with_no_owner_names() {
    use diroxide_archive::ArchiveFormat;
    let tar_ok = oracle("tar");
    if let Err(why) = &tar_ok {
        eprintln!("cross-check of packed tars against tar not run: {why}");
        return;
    }
    let dir = crate::spike_listing::scratch();
    let source = source_tree(dir.path());
    let tar_version = version_of("tar", "--version");
    let cases = [
        (ArchiveFormat::Tar, "course.tar", "-tvf", None),
        (ArchiveFormat::TarGz, "course.tar.gz", "-tzvf", Some("gzip")),
        (ArchiveFormat::TarXz, "course.tar.xz", "-tJvf", Some("xz")),
    ];
    for (format, name, flags, filter) in cases {
        if let Some(tool) = filter
            && let Err(why) = oracle(tool)
        {
            eprintln!("cross-check of {name} not run: {why}");
            continue;
        }
        let archive = pack_on_disk(&source, name, format, None);
        let (ok, listing) = tool_says("tar", &[flags.as_ref(), archive.as_os_str()]);
        assert!(ok, "tar {flags} {name}: {listing}");
        let lines: Vec<&str> = listing.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 7, "{listing}");
        for line in &lines {
            // GNU tar says "0/0"; bsdtar (Windows ships it) says "0 0"
            // after the link count.
            let fields: Vec<&str> = line.split_whitespace().collect();
            let owners = if tar_version.contains("bsdtar") {
                fields.get(2..4).map(|f| f.join("/")).unwrap_or_default()
            } else {
                fields.get(1).map(|f| (*f).to_owned()).unwrap_or_default()
            };
            assert_eq!(owners, "0/0", "no owner names: {line}");
        }
        eprintln!("cross-check: {name} passed tar {flags} ({tar_version}), owners 0/0");
        if let Some(tool) = filter {
            let (ok, said) = tool_says(tool, &["-t".as_ref(), archive.as_os_str()]);
            assert!(ok, "{tool} -t {name}: {said}");
            eprintln!(
                "cross-check: {name} passed {tool} -t ({})",
                version_of(tool, "--version")
            );
        }
    }
    if oracle("xz").is_ok() {
        let notes = source.join("notes.txt");
        let archive = pack_on_disk(&notes, "notes.txt.xz", ArchiveFormat::Xz, None);
        let (ok, said) = tool_says("xz", &["-t".as_ref(), archive.as_os_str()]);
        assert!(ok, "xz -t: {said}");
        let (ok, text) = tool_says("xz", &["-dc".as_ref(), archive.as_os_str()]);
        assert!(
            ok && text == "the notes\n".repeat(200),
            "xz -dc gives the file back"
        );
        eprintln!("cross-check: notes.txt.xz passed xz -t and xz -dc");
    }
}
