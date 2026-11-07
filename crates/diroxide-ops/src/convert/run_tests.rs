//! The conversion engine with fake tools over real files in a scratch
//! folder under the build's own folder: the staging name, the rename that
//! never replaces, the policies, the refusals, cancel and progress.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use diroxide_core::format::within_cells;
use diroxide_core::job::ErrorPolicy;
use diroxide_core::{ConflictAction, ConflictAnswer, ConflictPolicy, JobKind, JobOrigin, JobState};

use super::plan::{ConvertError, ConvertRequest};
use super::staging::is_staging_name;
use super::testing::{Act, FakeTools, JOB, engine, place, recipe, scratch};
use super::typing::ByExtension;
use crate::engine::JobControl;
use crate::progress::JobProgress;
use crate::testing::Script;
use crate::tracker::is_ignored;

const IMAGE_TO_JPEG: &str = r#"
[[recipe]]
id = "image-to-jpeg"
name = "Image to JPEG"
from-group = "images"
to = "jpg"
tool = "magick"
args = ["{typed_input}", "-quality", "90", "{output}"]
max-input = "1 KB"
"#;

const AUDIO_TO_OPUS: &str = r#"
[[recipe]]
id = "audio-to-opus"
name = "Audio to Opus"
from-group = "music"
to = "opus"
tool = "ffmpeg"
args = ["-i", "file:{input}", "-vn", "-map_metadata", "0", "-codec:a", "libopus", "{output}"]
"#;

/// Two folders, `in` with `a.png` and `b.png`, and an empty `out`.
fn folders() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = scratch();
    let input = root.path().join("in");
    let output = root.path().join("out");
    fs::create_dir_all(&input).unwrap();
    fs::create_dir_all(&output).unwrap();
    fs::write(input.join("a.png"), b"PNG a").unwrap();
    fs::write(input.join("b.png"), b"PNG b").unwrap();
    (root, input, output)
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn request(sources: &[&Path], out: &Path, text: &str) -> ConvertRequest {
    ConvertRequest::new(
        JOB,
        sources.iter().map(|p| place(p)).collect(),
        place(out),
        recipe(text),
    )
}

#[test]
fn a_picture_is_made_at_a_staging_name_and_renamed_in() {
    let (_root, input, out) = folders();
    let tools = Arc::new(FakeTools::with(&["magick"]));
    let engine = engine(tools.clone());
    let req = request(
        &[&input.join("a.png"), &input.join("b.png")],
        &out,
        IMAGE_TO_JPEG,
    );
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.question().text, "Convert 2 files to JPEG? y or n");
    let mut progress = JobProgress::start(JobKind::Convert, 40, None);
    progress.state = JobState::Running;
    progress.items_done = 11;
    assert_eq!(plan.progress_line(&progress), "Converting 12 of 40 images");
    progress.state = JobState::Paused;
    assert_eq!(
        plan.progress_line(&progress),
        "Converting paused at 12 of 40"
    );
    assert_eq!(plan.summary(), "Convert plan: 2 files");
    assert_eq!(plan.lines()[0], "a.png to a.jpg");
    assert_eq!(plan.lines()[2], "Tool: ImageMagick");
    let sink = Script::default();
    let report = engine.run_convert(&plan, &JobControl::new(), &sink);
    assert_eq!(report.made.len(), 2, "{:?}", report.job.failed);
    assert_eq!(names(&out), ["a.jpg", "b.jpg"]);
    assert_eq!(fs::read(out.join("a.jpg")).unwrap(), b"made from PNG a");
    assert_eq!(report.job.undo.created.len(), 2);
    assert_eq!(report.job.undo.left.len(), 2);
    let runs = tools.recipe_runs();
    assert_eq!(runs.len(), 2);
    let args = &runs[0].1;
    assert_eq!(args[0], "-limit");
    let typed = args.iter().find(|a| a.starts_with("PNG:")).unwrap();
    assert!(
        typed.ends_with("a.png") || typed.ends_with("b.png"),
        "{typed}"
    );
    let staged = args.last().unwrap();
    assert!(is_staging_name(
        Path::new(staged).file_name().unwrap().to_str().unwrap()
    ));
    assert_eq!(Path::new(staged).parent().unwrap(), out.as_path());
}

/// A hot folder watching the destination never sees a half-made output:
/// while the tool works, every name in the folder is one the arrival
/// tracker ignores, and the output appears whole, by a rename.
#[test]
fn a_hot_folder_never_sees_a_half_made_output() {
    let (_root, input, out) = folders();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let watch = seen.clone();
    let tools = Arc::new(FakeTools::with(&["magick"]).during(move |output: &Path| {
        let folder = output.parent().unwrap();
        // The tool's half-written output is in the folder now.
        fs::write(output, b"half").unwrap();
        for name in names(folder) {
            watch.lock().unwrap().push(name);
        }
    }));
    let engine = engine(tools);
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    engine.run_convert(&plan, &JobControl::new(), &Script::default());
    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|name| is_ignored(name)), "{seen:?}");
    assert_eq!(names(&out), ["a.jpg"]);
}

#[test]
fn keep_both_is_the_default_and_nothing_is_ever_overwritten() {
    let (_root, input, out) = folders();
    fs::write(out.join("a.jpg"), b"old").unwrap();
    let engine = engine(Arc::new(FakeTools::with(&["magick"])));
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.lines()[0], "a.png to a (2).jpg");
    engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(fs::read(out.join("a.jpg")).unwrap(), b"old");
    assert_eq!(fs::read(out.join("a (2).jpg")).unwrap(), b"made from PNG a");
    let overwrite = req.clone().conflicts(ConflictPolicy::Overwrite);
    let refused = engine
        .plan_convert(&overwrite, &ByExtension, &JobControl::new())
        .unwrap_err();
    assert_eq!(refused, ConvertError::Overwrite);
    assert_eq!(
        refused.to_string(),
        "conversions never replace a file; choose keep both, skip or newer"
    );
}

#[test]
fn skip_and_newer_leave_an_existing_output() {
    let (_root, input, out) = folders();
    fs::write(out.join("a.jpg"), b"old").unwrap();
    let engine = engine(Arc::new(FakeTools::with(&["magick"])));
    let skip = request(
        &[&input.join("a.png"), &input.join("b.png")],
        &out,
        IMAGE_TO_JPEG,
    )
    .conflicts(ConflictPolicy::Skip);
    let plan = engine
        .plan_convert(&skip, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.lines()[0], "Skipped a.png: output exists");
    assert_eq!(plan.summary(), "Convert plan: 1 file, 1 skipped");
    // The output was made after its source: up to date, so newer skips.
    let newer = skip.clone().conflicts(ConflictPolicy::Newer);
    let plan = engine
        .plan_convert(&newer, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.lines()[0], "Skipped a.png: output is up to date");
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.job.skipped.len(), 1);
    assert_eq!(fs::read(out.join("a.jpg")).unwrap(), b"old");
}

/// A file made at the output's name after the plan is never replaced:
/// the run keeps both.
#[test]
fn a_name_taken_after_the_plan_keeps_both() {
    let (_root, input, out) = folders();
    let racer = out.join("a.jpg");
    let tools = Arc::new(FakeTools::with(&["magick"]).during(move |_: &Path| {
        fs::write(&racer, b"raced").unwrap();
    }));
    let engine = engine(tools);
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.work[0].output.name(), Some("a.jpg"));
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(fs::read(out.join("a.jpg")).unwrap(), b"raced");
    assert_eq!(report.made[0].1.name(), Some("a (2).jpg"));
}

#[test]
fn a_failed_tool_leaves_no_staging_file_and_says_why() {
    let (_root, input, out) = folders();
    let tools =
        Arc::new(FakeTools::with(&["magick"]).acting(Act::Fail(1, "magick: no decode delegate")));
    let engine = engine(tools.clone());
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(
        report.job.failed[0].1,
        "ImageMagick ended with code 1: magick: no decode delegate"
    );
    assert!(names(&out).is_empty());
    assert!(report.job.undo.created.is_empty());
    *tools.act.lock().unwrap() = Act::Nothing;
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.job.failed[0].1, "ImageMagick made no file");
    assert!(names(&out).is_empty());
}

#[test]
fn cancel_stops_the_tool_and_removes_the_staging_file() {
    let (_root, input, out) = folders();
    let engine = engine(Arc::new(FakeTools::with(&["magick"]).acting(Act::Hang)));
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    let control = JobControl::new();
    let report = std::thread::scope(|scope| {
        let running = scope.spawn(|| engine.run_convert(&plan, &control, &Script::default()));
        while names(&out).is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        control.cancel.cancel();
        running.join().unwrap()
    });
    assert!(report.job.cancelled);
    assert!(report.made.is_empty());
    assert!(names(&out).is_empty(), "{:?}", names(&out));
}

/// The plan's rows: refusals first, then skips, then the work, each with
/// its meaning in the first 40 cells.
#[test]
fn refusals_come_first_then_skips_then_work() {
    let (_root, input, _out) = folders();
    fs::write(input.join("a.webp"), b"WEBP a").unwrap();
    fs::write(input.join("big.png"), vec![b'x'; 2000]).unwrap();
    fs::write(input.join("c.jpg"), b"JPEG c").unwrap();
    fs::write(input.join("notes.txt"), b"text").unwrap();
    fs::write(input.join("b.jpg"), b"old").unwrap();
    let engine = engine(Arc::new(FakeTools::with(&["magick"])));
    let sources =
        ["a.png", "a.webp", "big.png", "c.jpg", "notes.txt", "b.png"].map(|name| input.join(name));
    let sources: Vec<&Path> = sources.iter().map(|p| p.as_path()).collect();
    let req = request(&sources, &input, IMAGE_TO_JPEG).conflicts(ConflictPolicy::Skip);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    let lines = plan.lines();
    assert_eq!(
        lines[..6],
        [
            "Refused: a.webp, a.png makes the same a.jpg",
            "Refused: big.png, larger than 1.0 KB, its limit",
            "Refused: c.jpg, its output would take its own name",
            "Refused: notes.txt, Image to JPEG does not take text",
            "Skipped b.png: output exists",
            "a.png to a.jpg",
        ]
    );
    for line in &lines[..5] {
        let head: String = line.chars().take(40).collect();
        assert!(head.contains(':'), "{line}");
    }
    assert!(within_cells(&lines[0], "Refused: a.webp"));
    assert_eq!(plan.summary(), "Convert plan: 1 file, 1 skipped, 4 refused");
}

#[test]
fn ffmpeg_runs_quietly_whitelisted_and_says_its_percent() {
    let root = scratch();
    fs::write(root.path().join("song.flac"), b"FLAC").unwrap();
    let tools = Arc::new(FakeTools::with(&["ffmpeg", "ffprobe"]));
    let engine = engine(tools.clone());
    let req = request(
        &[&root.path().join("song.flac")],
        root.path(),
        AUDIO_TO_OPUS,
    );
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.question().text, "Convert 1 file to Opus? y or n");
    let sink = Script::default();
    let report = engine.run_convert(&plan, &JobControl::new(), &sink);
    assert_eq!(report.made.len(), 1, "{:?}", report.job.failed);
    let args = &tools.recipe_runs()[0].1;
    assert_eq!(
        args[..7],
        [
            "-hide_banner",
            "-nostdin",
            "-nostats",
            "-loglevel",
            "error",
            "-progress",
            "pipe:1"
        ]
    );
    let at = args.iter().position(|a| a == "-i").unwrap();
    assert_eq!(args[at - 2..at], ["-protocol_whitelist", "file"]);
    assert!(args[at + 1].starts_with("file:"));
    assert!(!args.iter().any(|a| a == "-y"));
    let currents: Vec<String> = sink
        .progress
        .lock()
        .unwrap()
        .iter()
        .filter_map(|p| p.current.clone())
        .collect();
    assert!(
        currents.contains(&"song.flac: 25 percent".to_owned()),
        "{currents:?}"
    );
    assert!(
        currents.contains(&"song.flac: 50 percent".to_owned()),
        "{currents:?}"
    );
    let lines: Vec<String> = sink
        .progress
        .lock()
        .unwrap()
        .iter()
        .map(|p| plan.progress_line(p))
        .collect();
    assert!(
        lines.contains(&"Converting song.flac: 50 percent".to_owned()),
        "{lines:?}"
    );
    assert_eq!(lines.last().unwrap(), "Converting finished");
}

#[test]
fn a_missing_tool_or_encoder_stops_the_plan_in_words() {
    let (_root, input, out) = folders();
    let none = engine(Arc::new(FakeTools::with(&[])));
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG);
    let error = none
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap_err();
    assert_eq!(error.to_string(), "ImageMagick is not installed");
    let mp4 = r#"
[[recipe]]
id = "video-to-mp4"
name = "Video to MP4"
from = ["png"]
to = "mp4"
tool = "ffmpeg"
needs-encoder = "libx264"
args = ["-i", "file:{input}", "{output}"]
"#;
    let ffmpeg = engine(Arc::new(FakeTools::with(&["ffmpeg"])));
    let error = ffmpeg
        .plan_convert(
            &request(&[&input.join("a.png")], &out, mp4),
            &ByExtension,
            &JobControl::new(),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "this FFmpeg has no libx264 encoder");
}

#[test]
fn a_format_imagemagick_cannot_read_is_refused_and_a_misread_name_needs_the_spool() {
    let (_root, input, out) = folders();
    fs::write(input.join("photo.heic"), b"HEIC").unwrap();
    let mut tools = FakeTools::with(&["magick"]);
    tools.formats = "      PNG* PNG       rw-   Portable Network Graphics\n".to_owned();
    let engine = engine(Arc::new(tools));
    let req = request(
        &[&input.join("photo.heic"), &input.join("a.png")],
        &out,
        IMAGE_TO_JPEG,
    );
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(
        plan.lines()[0],
        "Refused: photo.heic, ImageMagick cannot open HEIC"
    );
    assert_eq!(plan.work.len(), 1);
}

#[test]
fn a_folder_is_one_level_unless_recursive() {
    let (_root, input, out) = folders();
    fs::create_dir_all(input.join("trip")).unwrap();
    fs::write(input.join("trip").join("c.png"), b"PNG c").unwrap();
    fs::write(input.join(".hidden.png"), b"PNG h").unwrap();
    fs::write(input.join("notes.txt"), b"text").unwrap();
    let engine = engine(Arc::new(FakeTools::with(&["magick"])));
    let flat = request(&[&input], &out, IMAGE_TO_JPEG);
    let plan = engine
        .plan_convert(&flat, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.work.len(), 2);
    assert_eq!(plan.left_out, 1);
    assert!(
        plan.lines()
            .contains(&"Left out: 1 file it does not take".to_owned())
    );
    let deep = flat.recursive(true);
    let plan = engine
        .plan_convert(&deep, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.work.len(), 3);
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(names(&out.join("in")), ["a.jpg", "b.jpg", "trip"]);
    assert_eq!(names(&out.join("in").join("trip")), ["c.jpg"]);
    // The folders it made are in the undo record, parents first.
    assert_eq!(report.job.undo.created[0], place(&out.join("in")));
    assert_eq!(
        report.job.undo.created[1],
        place(&out.join("in").join("trip"))
    );
}

/// With a person there, a taken name is asked about; an answer to replace
/// keeps both and says so, since a conversion never replaces a file.
#[test]
fn the_ask_policy_asks_and_an_answer_to_replace_keeps_both() {
    let (_root, input, out) = folders();
    fs::write(out.join("a.jpg"), b"old").unwrap();
    let engine = engine(Arc::new(FakeTools::with(&["magick"])));
    let req = request(&[&input.join("a.png")], &out, IMAGE_TO_JPEG)
        .conflicts(ConflictPolicy::Ask)
        .from(JobOrigin::User);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert!(plan.work[0].ask);
    let sink = Script::conflicts(vec![ConflictAnswer::once(ConflictAction::Replace)]);
    let report = engine.run_convert(&plan, &JobControl::new(), &sink);
    assert_eq!(sink.asked.lock().unwrap().len(), 1);
    assert_eq!(fs::read(out.join("a.jpg")).unwrap(), b"old");
    assert_eq!(report.made[0].1.name(), Some("a (2).jpg"));
    assert_eq!(
        report.job.warnings[0].1,
        "Conversions never replace a file: kept both"
    );
    // A hot folder's job never asks: keep both, at the plan.
    let unattended = req.from(JobOrigin::HotFolder("scans".into()));
    let plan = engine
        .plan_convert(&unattended, &ByExtension, &JobControl::new())
        .unwrap();
    assert!(!plan.work[0].ask);
    assert_eq!(plan.work[0].output.name(), Some("a (3).jpg"));
}

#[test]
fn stop_at_first_ends_the_job_after_one_failure() {
    let root = scratch();
    fs::write(root.path().join("one.flac"), b"FLAC 1").unwrap();
    fs::write(root.path().join("two.flac"), b"FLAC 2").unwrap();
    let engine = engine(Arc::new(
        FakeTools::with(&["ffmpeg"]).acting(Act::Fail(2, "bad")),
    ));
    let sources = [root.path().join("one.flac"), root.path().join("two.flac")];
    let req = request(&[&sources[0], &sources[1]], root.path(), AUDIO_TO_OPUS)
        .on_errors(ErrorPolicy::StopAtFirst);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert!(report.job.stopped);
    assert_eq!(report.job.failed.len(), 1);
    assert_eq!(report.job.failed[0].1, "FFmpeg ended with code 2: bad");
    assert_eq!(super::run::parallel("ffmpeg"), 1);
    assert_eq!(super::run::parallel("magick"), 4);
}

/// A remote place served by the ops fake, under the `sftp` scheme.
fn remote(path: &str) -> diroxide_core::Place {
    diroxide_core::Place::parse(&format!("sftp://host{path}")).unwrap()
}

/// An engine whose local places are real and whose `sftp` places are the
/// fake's.
fn engine_with_remote(
    tools: Arc<FakeTools>,
    fake: &crate::fake::FakeProvider,
) -> crate::engine::Engine {
    let mut registry = diroxide_vfs::Registry::with_local();
    registry.register("sftp", Arc::new(fake.clone()));
    crate::engine::Engine::new(Arc::new(registry)).with_tools(tools)
}

#[test]
fn a_source_not_on_this_computer_goes_through_the_spool() {
    let root = scratch();
    let out = root.path().join("out");
    let spool = root.path().join("spool");
    fs::create_dir_all(&out).unwrap();
    let fake = crate::fake::FakeProvider::new();
    fake.add_folder(&remote("/data"));
    fake.add_file(&remote("/data/a.png"), b"PNG far");
    let engine = engine_with_remote(Arc::new(FakeTools::with(&["magick"])), &fake);
    let req = ConvertRequest::new(
        JOB,
        vec![remote("/data/a.png")],
        place(&out),
        recipe(IMAGE_TO_JPEG),
    );
    let refused = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(
        refused.lines()[0],
        "Refused: a.png, not on this computer, and no spool folder"
    );
    let req = req.spool(&spool);
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert!(plan.work[0].through_spool);
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.made.len(), 1, "{:?}", report.job.failed);
    assert_eq!(fs::read(out.join("a.jpg")).unwrap(), b"made from PNG far");
    assert!(names(&spool).is_empty(), "{:?}", names(&spool));
}

#[test]
fn an_output_for_a_place_not_on_this_computer_is_made_in_the_spool() {
    let (root, input, _out) = folders();
    let spool = root.path().join("spool");
    let fake = crate::fake::FakeProvider::new();
    fake.add_folder(&remote("/out"));
    fake.add_file(&remote("/out/b.jpg"), b"old");
    let engine = engine_with_remote(Arc::new(FakeTools::with(&["magick"])), &fake);
    let req = request(
        &[&input.join("a.png"), &input.join("b.png")],
        &input,
        IMAGE_TO_JPEG,
    );
    let mut req = req.spool(&spool);
    req.destination = remote("/out");
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    assert_eq!(plan.lines()[1], "b.png to b (2).jpg");
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(report.made.len(), 2, "{:?}", report.job.failed);
    assert_eq!(
        fake.contents(&remote("/out/a.jpg")).unwrap(),
        b"made from PNG a"
    );
    assert_eq!(fake.contents(&remote("/out/b.jpg")).unwrap(), b"old");
    assert_eq!(
        fake.contents(&remote("/out/b (2).jpg")).unwrap(),
        b"made from PNG b"
    );
    assert!(names(&spool).is_empty(), "{:?}", names(&spool));
    // Four pictures run at once, so the order they finish in varies.
    assert!(report.job.undo.created.contains(&remote("/out/a.jpg")));
    assert!(report.job.undo.created.contains(&remote("/out/b (2).jpg")));
}

/// Ctrl+Z after a conversion trashes the files the job made, through the
/// undo record's made list, and leaves the sources and older files.
#[test]
fn undo_trashes_the_files_the_job_made() {
    let (root, input, out) = folders();
    fs::write(out.join("a.jpg"), b"old").unwrap();
    let bin = root.path().join("bin");
    let mut registry = diroxide_vfs::Registry::new();
    registry.set_local(Arc::new(super::testing::ScratchBin::new(bin.clone())));
    let engine = crate::engine::Engine::new(Arc::new(registry))
        .with_tools(Arc::new(FakeTools::with(&["magick"])));
    let req = request(
        &[&input.join("a.png"), &input.join("b.png")],
        &out,
        IMAGE_TO_JPEG,
    );
    let plan = engine
        .plan_convert(&req, &ByExtension, &JobControl::new())
        .unwrap();
    let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
    assert_eq!(names(&out), ["a (2).jpg", "a.jpg", "b.jpg"]);
    let mut log = crate::undo::UndoLog::new(10);
    log.record(report.job.undo.clone());
    let undone = engine
        .undo_last(
            &mut log,
            diroxide_core::JobId::new(8),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert_eq!(undone.done.len(), 2, "{:?}", undone.failed);
    assert_eq!(names(&out), ["a.jpg"]);
    assert_eq!(names(&bin), ["a (2).jpg", "b.jpg"]);
    assert_eq!(names(&input), ["a.png", "b.png"]);
}
