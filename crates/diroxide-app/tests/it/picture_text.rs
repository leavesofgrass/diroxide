//! Read text in picture (alpha 4, lane VA, R30 commit 44): Tesseract
//! through a recording runner, never the real program; the text in the
//! viewer under its machine-read title, the details row, and the lines
//! said when Tesseract is missing or the file is not a picture.

use std::fs;
use std::path::Path;

use diroxide_app::commands::ids;
use diroxide_vfs::Registry;

use crate::support::{Session, Setup, place, png_bytes, temp};

/// Spike OCR1's answer from Tesseract 5.5.0: two lines, nine words.
const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t940\t240\t-1\t\n\
4\t1\t1\t1\t1\t0\t25\t27\t374\t28\t-1\t\n\
5\t1\t1\t1\t1\t1\t25\t28\t105\t27\t96.75\tCrows\n\
5\t1\t1\t1\t1\t2\t146\t29\t89\t26\t96.67\tmeet\n\
5\t1\t1\t1\t1\t3\t249\t29\t33\t26\t94.78\tat\n\
5\t1\t1\t1\t1\t4\t296\t27\t103\t28\t96.56\tdawn.\n\
4\t1\t1\t1\t2\t0\t21\t70\t524\t34\t-1\t\n\
5\t1\t1\t1\t2\t1\t21\t70\t137\t34\t92.84\tJimothy\n\
5\t1\t1\t1\t2\t2\t173\t70\t95\t28\t96.46\treads\n\
5\t1\t1\t1\t2\t3\t285\t71\t40\t26\t96.62\t14\n\
5\t1\t1\t1\t2\t4\t340\t70\t103\t28\t96.43\twords\n\
5\t1\t1\t1\t2\t5\t459\t70\t86\t28\t96.07\there.\n";

const LANGS: &[u8] =
    b"List of available languages in \"/usr/share/tesseract-ocr/5/tessdata/\" (2):\neng\nosd\n";

fn session(root: &Path) -> (Session, std::path::PathBuf) {
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("sign.png"), png_bytes(8, 4)).unwrap();
    fs::write(work.join("notes.txt"), "/elsewhere/secret.png\n").unwrap();
    let s = Session::open(
        place(&work),
        place(&work),
        Registry::with_local(),
        Setup::default(),
    );
    (s, work)
}

#[test]
fn the_text_in_a_picture_opens_in_the_viewer_as_machine_read() {
    let dir = temp();
    let (mut s, work) = session(dir.path());
    s.tools.answer_first("tesseract", "--list-langs", LANGS);
    s.tools.answer("tesseract", TSV.as_bytes());
    s.focus("sign.png");
    let said = s.command(ids::READ_PICTURE_TEXT);
    assert_eq!(said[0], "Reading the text in sign.png", "{said:?}");
    assert_eq!(
        said[1], "Text in picture (machine read), sign.png, 9 words",
        "{said:?}"
    );
    let viewer = s.app.viewer().unwrap();
    assert_eq!(
        viewer.title(),
        "Text in picture (machine read), sign.png, 9 words"
    );
    // One run of --list-langs, then the picture by its path, English.
    assert_eq!(s.tools.ran(), ["tesseract", "tesseract"]);
    let run = s.tools.runs.lock().unwrap()[1].clone();
    let args: Vec<String> = run
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            work.join("sign.png").to_string_lossy().as_ref(),
            "stdout",
            "-l",
            "eng",
            "--psm",
            "3",
            "tsv"
        ]
    );
    assert_eq!(run.limits.timeout, std::time::Duration::from_secs(120));
    assert_eq!(run.limits.max_output_bytes, 32 * 1024 * 1024);
    // Reload does not read the picture's bytes as text.
    let said = s.command(ids::VIEWER_RELOAD);
    assert_eq!(
        said,
        ["This text was read from the picture. Read text in picture reads it again."]
    );
    s.command(ids::CLOSE_VIEWER);
    assert!(s.app.viewer().is_none());
    s.focus("sign.png");
    s.command(ids::DETAILS);
    let rows: Vec<String> = match s.dialog() {
        Some(diroxide_app::Dialog::Choose { list, .. }) => {
            list.rows().iter().map(|r| r.label.clone()).collect()
        }
        other => panic!("{other:?}"),
    };
    assert!(
        rows.contains(&"Text in picture (machine read): 9 words".to_owned()),
        "{rows:?}"
    );
}

/// The reading is a job in the jobs line: Cancel stops it in words, and
/// the runtime stops Tesseract with its children.
#[test]
fn reading_is_a_job_that_cancel_stops() {
    let dir = temp();
    let (mut s, _) = session(dir.path());
    s.focus("sign.png");
    let effects = s
        .app
        .dispatch(&diroxide_core::Command::new(ids::READ_PICTURE_TEXT));
    let said = s.run_quiet(effects);
    assert_eq!(said, ["Reading the text in sign.png"]);
    assert_eq!(s.app.jobs().len(), 1);
    let effects = s.app.dispatch(&diroxide_core::Command::new(ids::CANCEL));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, diroxide_core::Effect::CancelJob(_))),
        "{effects:?}"
    );
    let said = s.run_quiet(effects);
    assert_eq!(said, ["Read text in picture: cancelling"]);
}

#[test]
fn without_tesseract_nothing_runs_and_it_says_so() {
    let dir = temp();
    let (mut s, _) = session(dir.path());
    s.focus("sign.png");
    let said = s.command(ids::READ_PICTURE_TEXT);
    assert_eq!(said.last().unwrap(), "No text: tesseract not found");
    assert!(s.tools.ran().is_empty());
    assert!(s.app.viewer().is_none());
}

/// Tesseract reads a file that is not a picture as a list of picture
/// names and opens each (spike OCR1): such a file never goes in.
#[test]
fn a_file_that_is_not_a_picture_never_reaches_tesseract() {
    let dir = temp();
    let (mut s, _) = session(dir.path());
    s.tools.answer_first("tesseract", "--list-langs", LANGS);
    s.tools.answer("tesseract", TSV.as_bytes());
    s.focus("notes.txt");
    let said = s.command(ids::READ_PICTURE_TEXT);
    assert_eq!(
        said.last().unwrap(),
        "No text: not a picture Tesseract reads"
    );
    assert!(s.tools.ran().is_empty());
}
