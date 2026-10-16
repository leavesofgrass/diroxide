//! Integration tests of the command line, and the scripted terminal test
//! with the braille walk on the drawn screen (`screen`).

mod screen;

use std::ffi::OsString;

#[test]
fn help_lists_every_command_meaning_first() {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = diroxide_cli::main_with(
        vec![OsString::from("--help")],
        &mut std::io::empty(),
        &mut out,
        &mut err,
    );
    assert_eq!(code, 0);
    let text = String::from_utf8(out).unwrap();
    assert!(text.lines().any(|l| l.starts_with("diro serve --stdio")));
    assert!(err.is_empty());
}
