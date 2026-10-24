//! Integration tests of the diro binary.

use std::process::Command;

#[test]
fn the_binary_prints_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_diro"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with(concat!("diro ", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn a_usage_mistake_exits_with_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_diro"))
        .arg("--bogus")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.starts_with("Could not start diro: the option --bogus is not known."));
}
