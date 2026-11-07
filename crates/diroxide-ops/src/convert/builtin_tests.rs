//! The built-in recipes (R30 commit 61): the embedded `conversions.toml`
//! reads with no problem, its words fit a Braille line, and every recipe
//! runs through the engine: with fake tools for ImageMagick, FFmpeg,
//! pandoc and pdftotext, and with carta itself for carta's.

use std::fs;
use std::sync::Arc;

use diroxide_core::format::BRAILLE_CELLS;
use diroxide_vfs::CancelToken;

use super::book::{Availability, BUILT_IN, RecipeBook};
use super::plan::ConvertRequest;
use super::recipe::RecipeSource;
use super::testing::{FakeTools, JOB, engine, place, scratch};
use super::typing::ByExtension;
use crate::engine::JobControl;
use crate::testing::Script;

/// A sample of what a recipe takes: its name and its bytes, real text for
/// carta's readers.
fn sample(from_group: Option<&str>, from: &[String]) -> (String, Vec<u8>) {
    let markdown = b"# Field notes\n\nHello from *Ada Example*.\n\n- one\n  - two\n".to_vec();
    match (from_group, from.first().map(String::as_str)) {
        (Some("images"), _) => ("photo.png".into(), b"PNG".to_vec()),
        (Some("music"), _) => ("song.flac".into(), b"FLAC".to_vec()),
        (Some("video"), _) => ("clip.mp4".into(), b"MP4".to_vec()),
        (_, Some("md")) => ("notes.md".into(), markdown),
        (_, Some("org")) => (
            "notes.org".into(),
            b"* Field notes\n\nHello from Ada Example.\n".to_vec(),
        ),
        (_, Some("rst")) => (
            "notes.rst".into(),
            b"Field notes\n===========\n\nHello from Ada Example.\n".to_vec(),
        ),
        (_, Some("mediawiki")) => (
            "notes.mediawiki".into(),
            b"== Field notes ==\n\nHello from Ada Example.\n".to_vec(),
        ),
        (_, Some(extension)) => (format!("sample.{extension}"), b"bytes".to_vec()),
        _ => ("sample.bin".into(), b"bytes".to_vec()),
    }
}

/// Every tool the built-in recipes name, with every encoder they need.
fn every_tool() -> FakeTools {
    let mut tools = FakeTools::with(&["magick", "ffmpeg", "ffprobe", "pandoc", "pdftotext"]);
    tools.encoders.push_str(" V....D libx264  H.264\n");
    tools
}

#[test]
fn the_built_in_file_reads_with_no_problem_and_its_words_fit() {
    let book = RecipeBook::built_in();
    assert!(book.problems().is_empty(), "{:?}", book.problems());
    assert_eq!(book.recipes().len(), 31);
    let mut ids: Vec<&str> = book.recipes().iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 31, "ids are unique");
    for recipe in book.recipes() {
        assert_eq!(recipe.source, RecipeSource::BuiltIn);
        assert!(
            recipe.name.chars().count() <= BRAILLE_CELLS,
            "{}",
            recipe.name
        );
        assert!(!recipe.about.is_empty(), "{}", recipe.id);
        assert!(recipe.about.ends_with('.'), "{}", recipe.id);
        let ready = Availability::Ready.line(recipe);
        assert!(ready.starts_with(&recipe.name));
    }
    assert!(BUILT_IN.starts_with("# The conversion recipes diroxide ships"));
    let tools: Vec<&str> = book.recipes().iter().map(|r| r.tool.as_str()).collect();
    for tool in ["magick", "ffmpeg", "pandoc", "pdftotext", "carta"] {
        assert!(tools.contains(&tool), "{tool}");
    }
}

/// The recipe list's rows: what is ready, and what each missing tool or
/// encoder keeps from running, its name first.
#[test]
fn availability_says_what_is_missing() {
    let book = RecipeBook::built_in();
    let tools = FakeTools::with(&["magick", "ffmpeg"]);
    let rows = book.availability(&tools, &CancelToken::new());
    let line = |id: &str| {
        rows.iter()
            .find(|(r, _)| r.id == id)
            .map(|(r, a)| a.line(r))
            .unwrap()
    };
    assert_eq!(line("image-to-jpeg"), "Image to JPEG, ready");
    assert_eq!(line("video-to-mp4"), "Video to MP4, needs libx264");
    assert_eq!(line("markdown-to-word"), "Markdown to Word, needs pandoc");
    assert_eq!(
        line("markdown-to-word-built-in"),
        "Markdown to Word, built in, ready"
    );
    assert_eq!(line("pdf-to-text"), "PDF to text, needs pdftotext");
    // Every row fits a Braille line, with every tool missing too.
    let nothing = FakeTools::with(&["ffmpeg"]);
    for (recipe, state) in rows
        .iter()
        .cloned()
        .chain(book.availability(&nothing, &CancelToken::new()))
        .chain(book.availability(&FakeTools::with(&[]), &CancelToken::new()))
    {
        let row = state.line(recipe);
        assert!(row.chars().count() <= BRAILLE_CELLS, "{row}");
    }
    // ffmpeg is asked for its encoders once, whatever the recipes.
    let asked = tools
        .runs
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, args)| args.iter().any(|a| a == "-encoders"))
        .count();
    assert_eq!(asked, 1);
}

/// Every built-in recipe runs through the engine and makes its output
/// under its real name, every placeholder filled.
#[test]
fn every_built_in_recipe_runs_through_the_engine() {
    let book = RecipeBook::built_in();
    for recipe in book.recipes() {
        let root = scratch();
        let input = root.path().join("in");
        let out = root.path().join("out");
        fs::create_dir_all(&input).unwrap();
        fs::create_dir_all(&out).unwrap();
        let (name, bytes) = sample(recipe.from_group.as_deref(), &recipe.from);
        fs::write(input.join(&name), bytes).unwrap();
        let tools = Arc::new(every_tool());
        let engine = engine(tools.clone());
        let request = ConvertRequest::new(
            JOB,
            vec![place(&input.join(&name))],
            place(&out),
            recipe.clone(),
        );
        let plan = engine
            .plan_convert(&request, &ByExtension, &JobControl::new())
            .unwrap_or_else(|e| panic!("{}: {e}", recipe.id));
        assert_eq!(plan.work.len(), 1, "{}: {:?}", recipe.id, plan.lines());
        let report = engine.run_convert(&plan, &JobControl::new(), &Script::default());
        assert_eq!(
            report.made.len(),
            1,
            "{}: {:?}",
            recipe.id,
            report.job.failed
        );
        let stem = name.rsplit_once('.').unwrap().0;
        let made = format!("{stem}.{}", recipe.to);
        assert!(out.join(&made).is_file(), "{}: {made}", recipe.id);
        for (tool, args) in tools.recipe_runs() {
            assert_eq!(tool, recipe.tool);
            assert!(
                args.iter()
                    .all(|a| !a.contains("{input") && !a.contains("{output")),
                "{}: {args:?}",
                recipe.id
            );
        }
    }
}
