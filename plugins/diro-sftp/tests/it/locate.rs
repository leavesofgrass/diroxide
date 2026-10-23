//! Finding ssh (R25 commit 48): fake programs in fake folders.

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use diro_sftp::ssh::locate::{Origin, SearchPlaces, locate, probe};
use diro_sftp::ssh::{Flavor, LocateError};
use serde_json::json;

use crate::fakes::{fake_ssh_in, log_lines};

fn places(path: &[&Path], extra: &str) -> SearchPlaces {
    let mut entries: Vec<OsString> = path.iter().map(|p| p.as_os_str().to_owned()).collect();
    if !extra.is_empty() {
        entries.push(OsString::from(extra));
    }
    let separator = if cfg!(windows) { ";" } else { ":" };
    let joined = entries
        .iter()
        .map(|e| e.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator);
    SearchPlaces {
        path: Some(OsString::from(joined)),
        windows: cfg!(windows),
        ..SearchPlaces::default()
    }
}

#[test]
fn the_highest_version_wins_and_the_windows_build_on_a_tie() {
    let root = tempfile::tempdir().unwrap();
    let portable = root.path().join("portable");
    let windows = root.path().join("windows");
    let old = root.path().join("old");
    fake_ssh_in(
        &portable,
        &json!({"version": "OpenSSH_9.5p2, OpenSSL 3.0.13"}),
    );
    fake_ssh_in(
        &windows,
        &json!({"version": "OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2"}),
    );
    fake_ssh_in(
        &old,
        &json!({"version": "OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2"}),
    );
    let found = locate(None, &places(&[&old, &portable, &windows], ""), probe).unwrap();
    assert_eq!(found.path.parent(), Some(windows.as_path()));
    assert_eq!(found.version.flavor, Flavor::Windows);
    assert_eq!(found.origin, Origin::Path);
    assert_eq!(found.status(), "ssh: OpenSSH for Windows 9.5 from PATH");
    let newer = root.path().join("newer");
    fake_ssh_in(&newer, &json!({"version": "OpenSSH_10.3p1, OpenSSL 3.5.6"}));
    let found = locate(
        None,
        &places(&[&old, &portable, &windows, &newer], ""),
        probe,
    )
    .unwrap();
    assert_eq!(found.path.parent(), Some(newer.as_path()));
    assert_eq!(found.status(), "ssh: OpenSSH 10.3 from PATH");
}

#[test]
fn the_setting_is_used_alone_and_must_be_openssh() {
    let root = tempfile::tempdir().unwrap();
    let chosen = fake_ssh_in(
        &root.path().join("chosen"),
        &json!({"version": "OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2"}),
    );
    let newer = root.path().join("newer");
    fake_ssh_in(&newer, &json!({"version": "OpenSSH_10.3p1, OpenSSL 3.5.6"}));
    let found = locate(Some(&chosen), &places(&[&newer], ""), probe).unwrap();
    assert_eq!(found.path, chosen);
    assert_eq!(found.origin, Origin::Setting);
    assert!(!found.version.askpass_force());
    let not_ssh = fake_ssh_in(
        &root.path().join("not-ssh"),
        &json!({"version": "PuTTY Release 0.81"}),
    );
    assert_eq!(
        locate(Some(&not_ssh), &places(&[&newer], ""), probe)
            .unwrap_err()
            .lines(),
        [
            "The ssh program in settings cannot be used.",
            "It did not answer as OpenSSH."
        ]
    );
    let empty = root.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        locate(None, &places(&[&empty], ""), probe),
        Err(LocateError::NotFound { .. })
    ));
}

/// The rule of R24 section 1.1: a planted ssh in the current folder, or
/// reached through a relative PATH entry, is never asked anything; only
/// absolute paths are ever started.
#[test]
fn a_planted_ssh_is_never_run() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("planted.log");
    let planted = root.path().join("planted");
    fake_ssh_in(
        &planted,
        &json!({"version": "OpenSSH_99.9p9, planted", "log": log}),
    );
    let good = root.path().join("good");
    fake_ssh_in(&good, &json!({"version": "OpenSSH_9.9p1, OpenSSL 3.0"}));
    let asked: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
    let relative = if cfg!(windows) {
        ".;planted;;.\\planted"
    } else {
        ".:planted::./planted"
    };
    let found = locate(None, &places(&[&good], relative), |path| {
        asked.borrow_mut().push(path.to_path_buf());
        probe(path)
    })
    .unwrap();
    assert_eq!(found.path.parent(), Some(good.as_path()));
    assert!(asked.borrow().iter().all(|p| p.is_absolute()), "{asked:?}");
    assert!(log_lines(&log).is_empty(), "the planted ssh ran");
}
