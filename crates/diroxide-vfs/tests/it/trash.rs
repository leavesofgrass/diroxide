//! The trash gate on real paths (spikes four and six), without ever
//! calling the real trash: every test here stops at the gate, at a missing
//! item or at a taken name, before the system trash could be reached. The
//! call and its post-check are tested against a test-only bin folder in
//! the provider's own unit tests.

use std::fs;

use diroxide_core::Place;
use diroxide_vfs::{LocalProvider, OpContext, Provider, TrashGate, VfsError};

use crate::local::place_of;
use crate::scratch;

/// A file in `folder` whose full path is exactly `units` UTF-16 units (all
/// plain ASCII letters).
fn file_of_length(folder: &tempfile::TempDir, units: usize) -> Place {
    let base = folder.path().to_str().unwrap().len() + 1;
    let name = "n".repeat(units - base);
    fs::write(folder.path().join(&name), b"x").unwrap();
    place_of(folder).join(&name).unwrap()
}

#[test]
fn a_missing_item_is_not_found_before_any_call() {
    let folder = scratch();
    let missing = place_of(&folder).join("gone.txt").unwrap();
    assert_eq!(
        LocalProvider::new().trash(&missing, &OpContext::new()),
        Err(VfsError::NotFound)
    );
}

#[test]
fn a_restore_onto_a_taken_name_is_refused_before_the_trash_is_read() {
    let folder = scratch();
    fs::write(folder.path().join("back.txt"), b"b").unwrap();
    let place = place_of(&folder).join("back.txt").unwrap();
    let result = LocalProvider::new().restore(&place, &OpContext::new());
    if cfg!(target_os = "macos") {
        assert!(matches!(result, Err(VfsError::Unsupported { .. })));
    } else {
        assert_eq!(result, Err(VfsError::AlreadyExists));
    }
}

#[cfg(windows)]
#[test]
fn the_gate_takes_259_units_and_refuses_260() {
    let folder = scratch();
    let provider = LocalProvider::new();
    let short = file_of_length(&folder, 259);
    assert_eq!(
        provider.trash_gate(&short, &OpContext::new()),
        Ok(TrashGate::Allowed)
    );
    let long = file_of_length(&folder, 260);
    assert_eq!(
        provider.trash_gate(&long, &OpContext::new()),
        Ok(TrashGate::TooLong { units: 260 })
    );
    // The trash itself refuses before the call: the file stays.
    match provider.trash(&long, &OpContext::new()) {
        Err(VfsError::TrashRefused { why }) => {
            assert_eq!(why, "the path is 260 characters, too long for the trash");
        }
        other => panic!("{other:?}"),
    }
    assert!(fs::metadata(long.local_path().unwrap()).is_ok());
}

#[cfg(windows)]
#[test]
fn a_folder_is_refused_when_a_path_inside_is_too_long() {
    let folder = scratch();
    let provider = LocalProvider::new();
    fs::create_dir(folder.path().join("box")).unwrap();
    let place = place_of(&folder).join("box").unwrap();
    assert_eq!(
        provider.trash_gate(&place, &OpContext::new()),
        Ok(TrashGate::Allowed)
    );
    let base = folder.path().join("box").to_str().unwrap().len() + 1;
    let deep = "d".repeat(270 - base);
    fs::write(folder.path().join("box").join(&deep), b"x").unwrap();
    assert_eq!(
        provider.trash_gate(&place, &OpContext::new()),
        Ok(TrashGate::TooLong { units: 270 })
    );
    assert!(matches!(
        provider.trash(&place, &OpContext::new()),
        Err(VfsError::TrashRefused { .. })
    ));
    assert!(folder.path().join("box").join(&deep).exists());
}

#[cfg(windows)]
#[test]
fn a_path_far_past_the_limit_is_refused_and_kept() {
    // Spike four's shape: a tree of plain 30-character names, past 1,000
    // characters, which the standard library handles and the trash would
    // have removed for good with Ok.
    let folder = scratch();
    let mut path = folder.path().to_path_buf();
    for level in 0..34 {
        path.push(format!("level{level:02}-{}", "x".repeat(22)));
    }
    fs::create_dir_all(&path).unwrap();
    let file = path.join("deep.txt");
    fs::write(&file, b"deep").unwrap();
    let place = Place::parse(file.to_str().unwrap()).unwrap();
    let provider = LocalProvider::new();
    match provider.trash_gate(&place, &OpContext::new()) {
        Ok(TrashGate::TooLong { units }) => assert!(units > 1000, "{units}"),
        other => panic!("{other:?}"),
    }
    assert!(provider.trash(&place, &OpContext::new()).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"deep");
}

#[cfg(unix)]
#[test]
fn unix_has_no_length_limit_at_the_gate() {
    let folder = scratch();
    let long = file_of_length(&folder, 260);
    assert_eq!(
        LocalProvider::new().trash_gate(&long, &OpContext::new()),
        Ok(TrashGate::Allowed)
    );
}
