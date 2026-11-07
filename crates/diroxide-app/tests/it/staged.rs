//! Outputs a plugin staged and committed (alpha 4, lane AE, E's request
//! 1): the host hands out a staging path beside the output, moves the
//! file into place when the plugin commits it, and the app keeps it in
//! the undo record of the plugin's running command, so Undo would put it
//! in the trash. Undo itself is not run here: the trash is the real one.
//! Every file is under the test's own folder.

use diroxide_core::{Command, CommandId, JobKind, JobOrigin};

use crate::plugins::{Act, MANIFEST, TestLauncher, enable, session, world};
use crate::support::place;

#[test]
fn a_committed_output_joins_the_commands_undo_record() {
    let w = world(MANIFEST);
    enable(&w.paths);
    let output = w.work.join("notes.txt.out");
    let text: &'static str = Box::leak(output.to_str().unwrap().to_owned().into_boxed_str());
    let mut s = session(&w, TestLauncher::with(&[("ask", Act::Stage(text))]));
    let before = s.app.undo_log().last().cloned();
    let said = s.run_command(Command::new(CommandId::new("line-counter.ask").unwrap()));
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"made by the test plugin\n",
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("Host said") && l.contains("\"committed\":true")),
        "{said:?}"
    );
    // No staging file is left beside it.
    let left: Vec<String> = std::fs::read_dir(&w.work)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".diro-part-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    let record = s.app.undo_log().last().cloned().expect("an undo record");
    assert_ne!(Some(&record), before.as_ref());
    assert_eq!(record.kind, Some(JobKind::PluginCommand));
    assert_eq!(record.created, [place(&output)]);
    assert_eq!(record.origin, JobOrigin::Plugin("Line counter".to_owned()));
}
