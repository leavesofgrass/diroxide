//! The scripted terminal test (research/alpha1-plan.md part five, commit
//! 67) and the braille walk on the drawn screen (commit 68): `diro`'s
//! own start sequence over a provider whose trash is a test folder, drawn
//! on ratatui's test backend, read line by line as a screen reader and a
//! 40-cell display get it.

// Test helpers: a failure is the test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diroxide_app::{App, AppEvent, AppOptions, Runtime};
use diroxide_cli::OpenOptions;
use diroxide_config::Paths;
use diroxide_core::format::{BRAILLE_CELLS, first_cells, within_cells};
use diroxide_core::{
    Entry, FixedClock, JobId, JobKind, JobState, Key, Place, Timestamp, UtcOffset,
};
use diroxide_ops::JobProgress;
use diroxide_theme::ColorSupport;
use diroxide_tui::{Screen, Session, TuiOptions};
use diroxide_vfs::{
    AttributeChange, Capabilities, CopyHint, FileWriter, LinkRequest, LocalProvider, OpContext,
    Provider, ReadSeek, Registry, SpaceInfo, TrashCheck, TrashGate, VfsError, VolumeInfo,
    WatchHandle, WatchSink, WriteMode,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A provider over the local disk whose trash is a test folder.
#[derive(Debug)]
struct TestTrash {
    local: LocalProvider,
    bin: PathBuf,
    held: Mutex<HashMap<String, Vec<PathBuf>>>,
    counter: Mutex<u32>,
}

impl TestTrash {
    fn new(bin: PathBuf) -> Self {
        fs::create_dir_all(&bin).unwrap();
        Self {
            local: LocalProvider::new(),
            bin,
            held: Mutex::new(HashMap::new()),
            counter: Mutex::new(0),
        }
    }

    fn count(&self) -> usize {
        fs::read_dir(&self.bin).unwrap().count()
    }
}

impl Provider for TestTrash {
    fn scheme(&self) -> &str {
        "file"
    }
    fn capabilities(&self, place: &Place) -> Capabilities {
        let mut caps = self.local.capabilities(place);
        caps.trash = true;
        caps.restore = true;
        caps.watch = false;
        caps
    }
    fn list(&self, place: &Place, ctx: &OpContext) -> Result<Vec<Entry>, VfsError> {
        self.local.list(place, ctx)
    }
    fn stat(&self, place: &Place, ctx: &OpContext) -> Result<Entry, VfsError> {
        self.local.stat(place, ctx)
    }
    fn read(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
        self.local.read(place, ctx)
    }
    fn open_seekable(&self, place: &Place, ctx: &OpContext) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.local.open_seekable(place, ctx)
    }
    fn write(
        &self,
        place: &Place,
        mode: WriteMode,
        ctx: &OpContext,
    ) -> Result<Box<dyn FileWriter>, VfsError> {
        self.local.write(place, mode, ctx)
    }
    fn create_dir(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.create_dir(place, ctx)
    }
    fn create_link(
        &self,
        place: &Place,
        request: &LinkRequest,
        ctx: &OpContext,
    ) -> Result<(), VfsError> {
        self.local.create_link(place, request, ctx)
    }
    fn rename(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.rename(from, to, ctx)
    }
    fn remove(&self, place: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.remove(place, ctx)
    }
    fn trash_gate(&self, _place: &Place, _ctx: &OpContext) -> Result<TrashGate, VfsError> {
        Ok(TrashGate::Allowed)
    }
    fn trash(&self, place: &Place, _ctx: &OpContext) -> Result<TrashCheck, VfsError> {
        let from = PathBuf::from(place.local_path().ok_or(VfsError::NotFound)?);
        let n = {
            let mut counter = self.counter.lock().unwrap();
            *counter += 1;
            *counter
        };
        let to = self.bin.join(format!("item-{n}"));
        fs::rename(&from, &to)?;
        self.held
            .lock()
            .unwrap()
            .entry(place.to_string())
            .or_default()
            .push(to);
        Ok(TrashCheck::InTrash)
    }
    fn restore(&self, original: &Place, _ctx: &OpContext) -> Result<(), VfsError> {
        let held = self
            .held
            .lock()
            .unwrap()
            .get_mut(&original.to_string())
            .and_then(Vec::pop)
            .ok_or(VfsError::NotFound)?;
        let to = PathBuf::from(original.local_path().ok_or(VfsError::NotFound)?);
        if to.exists() {
            return Err(VfsError::AlreadyExists);
        }
        fs::rename(held, to)?;
        Ok(())
    }
    fn set_attributes(
        &self,
        place: &Place,
        change: &AttributeChange,
        ctx: &OpContext,
    ) -> Result<AttributeChange, VfsError> {
        self.local.set_attributes(place, change, ctx)
    }
    fn copy_hint(&self, from: &Place, to: &Place) -> CopyHint {
        self.local.copy_hint(from, to)
    }
    fn copy_within(&self, from: &Place, to: &Place, ctx: &OpContext) -> Result<(), VfsError> {
        self.local.copy_within(from, to, ctx)
    }
    fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
        self.local.volume(place, ctx)
    }
    fn space(&self, place: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
        self.local.space(place, ctx)
    }
    fn watch(&self, _place: &Place, _sink: WatchSink) -> Result<Option<WatchHandle>, VfsError> {
        Ok(None)
    }
}

fn place(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

/// The owner's check folder in small: plain names, accents, Japanese, an
/// emoji, a Backup folder with an older report, and a Photos folder.
fn fixture(root: &Path) -> PathBuf {
    let folder = root.join("work");
    fs::create_dir_all(folder.join("Backup")).unwrap();
    fs::create_dir_all(folder.join("Photos")).unwrap();
    fs::write(folder.join("report.pdf"), vec![b'r'; 2100]).unwrap();
    fs::write(folder.join("notes.txt"), b"notes").unwrap();
    fs::write(folder.join("budget.xlsx"), vec![b'b'; 300]).unwrap();
    fs::write(folder.join("R\u{e9}sum\u{e9}.txt"), b"cv").unwrap();
    fs::write(folder.join("\u{65e5}\u{672c}\u{8a9e}.txt"), b"ja").unwrap();
    fs::write(folder.join("raccoon \u{1f99d}.txt"), b"jimothy").unwrap();
    fs::write(folder.join("Photos").join("crow.jpg"), b"caw").unwrap();
    fs::write(folder.join("Backup").join("report.pdf"), b"old").unwrap();
    folder
}

/// The config file for the tests: no settle time, so y answers at once.
fn write_config(paths: &Paths) {
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(
        paths.config_file(Paths::SETTINGS),
        "dialog_settle_ms = 0\nscreen_reader_mode = \"on\"\n",
    )
    .unwrap();
}

/// The session's time: each step waits a fifth of a second, as a person
/// pressing keys does, so a repeated message's blank has ended.
struct Run {
    clock: std::cell::Cell<Instant>,
}

impl Run {
    fn new() -> Self {
        Self {
            clock: std::cell::Cell::new(Instant::now()),
        }
    }

    fn now(&self) -> Instant {
        self.clock.get()
    }

    fn settle(&self, s: &mut Session<'_, TestBackend>) {
        s.settle(self.now(), Duration::from_secs(20));
        s.tick(self.now());
        s.draw(self.now()).unwrap();
        self.clock.set(self.now() + Duration::from_millis(200));
        s.tick(self.now());
        s.draw(self.now()).unwrap();
    }

    fn press(&self, s: &mut Session<'_, TestBackend>, key: &str) -> Screen {
        s.key(Key::parse(key).unwrap(), self.now());
        self.settle(s);
        s.screen()
    }

    fn type_text(&self, s: &mut Session<'_, TestBackend>, text: &str) {
        for c in text.chars() {
            s.key(
                Key::new(
                    diroxide_core::KeyCode::Char(c),
                    diroxide_core::Modifiers::NONE,
                ),
                self.now(),
            );
        }
        self.settle(s);
    }
}

fn session<'a>(app: &'a mut App, runtime: &'a mut Runtime) -> Session<'a, TestBackend> {
    let terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let options = TuiOptions {
        screen_reader: app.screen_reader(),
        color: Some(ColorSupport::NoColor),
        ..TuiOptions::default()
    };
    Session::new(app, runtime, terminal, options)
}

/// The focused row's line on the screen, by the cursor.
fn cursor_row(screen: &Screen) -> &str {
    &screen.lines[screen.cursor_line.unwrap()]
}

/// The left panel's half of a side-by-side line, as a 40-cell display
/// shows the start of the line.
fn left_half(line: &str) -> String {
    first_cells(line, 39).trim_end().to_owned()
}

/// Asserts the cursor sits at the first cell of the active panel's
/// focused row, whose name begins the line.
fn on_focus(s: &Session<'_, TestBackend>) {
    let screen = s.screen();
    let backend = s.terminal().backend();
    let at = backend.cursor_position();
    assert!(backend.cursor_visible());
    assert_eq!(usize::from(at.y), screen.cursor_line.unwrap());
    assert_eq!(usize::from(at.x), screen.cursor_column);
    let label = s
        .app()
        .active_panel()
        .list()
        .focused()
        .map(|r| r.label.clone())
        .unwrap();
    let row = cursor_row(&screen);
    let start: String = row.chars().skip(screen.cursor_column).collect();
    assert!(
        start.starts_with(&label) || start.starts_with(&format!("* {label}")),
        "the cursor's row \"{row}\" does not start with {label}"
    );
}

/// Asserts a line says `fact` within its first 40 cells.
fn fact(line: &str, fact: &str) {
    assert!(
        within_cells(line, fact),
        "\"{fact}\" is not in the first {BRAILLE_CELLS} cells of \"{line}\""
    );
}

#[test]
fn a_scripted_session_moves_marks_copies_renames_trashes_undoes_and_comes_back() {
    let dir = tempfile::Builder::new()
        .prefix("cli-it-")
        .tempdir()
        .unwrap();
    let work = fixture(dir.path());
    let backup = work.join("Backup");
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    write_config(&paths);
    let trash = Arc::new(TestTrash::new(dir.path().join("bin")));
    let registry = || {
        let mut registry = Registry::new();
        registry.set_local(trash.clone());
        Arc::new(registry)
    };
    let options = OpenOptions {
        places: vec![
            work.to_str().unwrap().to_owned(),
            backup.to_str().unwrap().to_owned(),
        ],
        ..OpenOptions::default()
    };
    let here = place(dir.path());
    let run = Run::new();
    {
        let (mut app, mut runtime) =
            diroxide_cli::open::build_on(&options, Some(&paths), &here, registry()).unwrap();
        let mut s = session(&mut app, &mut runtime);
        s.start(run.now());
        run.settle(&mut s);
        let screen = s.screen();
        // Open: the title says the place first, the panel before the
        // parent; the cursor is on the first row.
        fact(&screen.lines[0], "work, left panel, in ");
        on_focus(&s);
        // Move by every key.
        for key in [
            "down", "down", "up", "end", "home", "pagedown", "pageup", "down",
        ] {
            run.press(&mut s, key);
            on_focus(&s);
        }
        // Into a folder and back by Right and Left, by Enter and
        // Backspace, then back and forward in the history.
        assert!(s.app_mut().focus_name(diroxide_core::Side::Left, "Photos"));
        for key in [
            "right",
            "left",
            "enter",
            "backspace",
            "alt+left",
            "alt+right",
        ] {
            run.press(&mut s, key);
            on_focus(&s);
        }
        assert_eq!(s.app().active_panel().place(), &place(&work));
        // Tab to the other panel and back: one message each, said first.
        let screen = run.press(&mut s, "tab");
        fact(&screen.lines[22], "Right panel: Backup, 1 item");
        on_focus(&s);
        let screen = run.press(&mut s, "tab");
        fact(&screen.lines[22], "Left panel: work, 8 items");
        // Mark three rows with Space.
        assert!(
            s.app_mut()
                .focus_name(diroxide_core::Side::Left, "budget.xlsx")
        );
        let screen = run.press(&mut s, "space");
        fact(&screen.lines[22], "Selected budget.xlsx. 1 selected.");
        run.press(&mut s, "space");
        let screen = run.press(&mut s, "space");
        fact(
            &screen.lines[22],
            "Selected raccoon \u{1f99d}.txt. 3 selected.",
        );
        let marked: Vec<&String> = screen
            .lines
            .iter()
            .filter(|l| l.starts_with("* "))
            .collect();
        assert_eq!(marked.len(), 3, "{:#?}", screen.lines);
        for line in &marked {
            assert!(within_cells(line, ", selected"), "{line}");
        }
        let screen = run.press(&mut s, "ctrl+d");
        fact(&screen.lines[22], "3 selected, 312 bytes. work, 8 items");
        // Copy to the other panel: the question starts on No, y answers.
        let screen = run.press(&mut s, "f5");
        fact(&screen.lines[1], "Copy 3 items to Backup, 312 bytes?");
        assert_eq!(screen.cursor_line, Some(1));
        let screen = run.press(&mut s, "y");
        fact(&screen.lines[22], "Copy finished: 3 done");
        assert_eq!(screen.lines[23], "No jobs");
        for name in ["budget.xlsx", "notes.txt", "raccoon \u{1f99d}.txt"] {
            assert!(backup.join(name).exists(), "{name} was not copied");
        }
        // Rename: the prompt says the name first, the cursor before the
        // extension.
        assert!(
            s.app_mut()
                .focus_name(diroxide_core::Side::Left, "report.pdf")
        );
        let screen = run.press(&mut s, "f2");
        let prompt = screen
            .lines
            .iter()
            .position(|l| l.starts_with("Rename report.pdf to:"))
            .unwrap();
        assert_eq!(screen.cursor_line, Some(prompt));
        assert_eq!(screen.cursor_column, "Rename report.pdf to: report".len());
        run.type_text(&mut s, "-2026");
        let screen = run.press(&mut s, "enter");
        fact(&screen.lines[22], "Renamed report.pdf to report-2026.pdf");
        assert!(work.join("report-2026.pdf").exists());
        on_focus(&s);
        assert!(cursor_row(&s.screen()).starts_with("report-2026.pdf"));
        // Delete to the test trash.
        let screen = run.press(&mut s, "f8");
        fact(&screen.lines[1], "Move 1 item to the trash, 2.1 KB?");
        run.press(&mut s, "y");
        assert!(!work.join("report-2026.pdf").exists());
        assert_eq!(trash.count(), 1);
        // Undo brings it back.
        let screen = run.press(&mut s, "ctrl+z");
        fact(&screen.lines[22], "Undid trash: 1 item is back");
        assert!(work.join("report-2026.pdf").exists());
        assert!(
            s.app_mut()
                .focus_name(diroxide_core::Side::Left, "report-2026.pdf")
        );
        run.settle(&mut s);
        on_focus(&s);
        // Quit: nothing runs, so it does not ask.
        run.press(&mut s, "ctrl+q");
        assert!(s.wants_quit());
    }
    // Relaunch with the same state folder: the cursor is restored.
    let (mut app, mut runtime) =
        diroxide_cli::open::build_on(&OpenOptions::default(), Some(&paths), &here, registry())
            .unwrap();
    let mut s = session(&mut app, &mut runtime);
    s.start(run.now());
    run.settle(&mut s);
    let screen = s.screen();
    assert_eq!(s.app().active_panel().place(), &here);
    // "diro" alone opens the current folder on the left and restores the
    // right panel; the session's left folder is the same place here, so
    // go to it as the owner would.
    assert_eq!(
        s.app().panel(diroxide_core::Side::Right).place(),
        &place(&backup)
    );
    let _ = screen;
    let (mut app, mut runtime) = diroxide_cli::open::build_on(
        &OpenOptions {
            places: vec![work.to_str().unwrap().to_owned()],
            ..OpenOptions::default()
        },
        Some(&paths),
        &here,
        registry(),
    )
    .unwrap();
    let mut s = session(&mut app, &mut runtime);
    s.start(run.now());
    run.settle(&mut s);
    let screen = s.screen();
    assert!(
        cursor_row(&screen).starts_with("report-2026.pdf"),
        "{:#?}",
        screen.lines
    );
    on_focus(&s);
}

#[test]
fn the_drawn_jobs_line_changes_no_more_often_than_the_setting_says() {
    let dir = tempfile::Builder::new()
        .prefix("cli-it-")
        .tempdir()
        .unwrap();
    let here = place(dir.path());
    let clock = Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ));
    let mut options = AppOptions::new(here.clone(), here);
    options.clock = clock.clone();
    let mut app = App::new(options);
    let mut runtime = Runtime::with_local();
    let mut s = session(&mut app, &mut runtime);
    let run = Run::new();
    s.start(run.now());
    run.settle(&mut s);
    assert!(s.app().screen_reader());
    let job = JobId::new(5);
    let mut progress = JobProgress::start(JobKind::Copy, 100, None);
    progress.state = JobState::Running;
    let mut drawn = vec![s.screen().lines[23].clone()];
    for second in 0..30u64 {
        progress.items_done = second * 3;
        s.event(
            AppEvent::JobProgress {
                job,
                progress: progress.clone(),
            },
            run.now(),
        );
        s.draw(run.now()).unwrap();
        let line = s.screen().lines[23].clone();
        if drawn.last() != Some(&line) {
            drawn.push(line);
        }
        clock.advance(1);
    }
    // "No jobs", the start (a change of state), then one change every ten
    // seconds; each inside 40 cells, percent first.
    assert_eq!(drawn.len(), 4, "{drawn:?}");
    for line in &drawn[1..] {
        fact(line, "Copying ");
        assert!(line.chars().count() <= BRAILLE_CELLS, "{line}");
    }
    // A change of state is drawn at once.
    progress.state = JobState::Paused;
    s.event(AppEvent::JobProgress { job, progress }, run.now());
    s.draw(run.now()).unwrap();
    fact(&s.screen().lines[23], "Copying paused at");
}

#[test]
fn the_lines_of_part_three_read_on_a_40_cell_display() {
    let dir = tempfile::Builder::new()
        .prefix("cli-it-")
        .tempdir()
        .unwrap();
    let work = fixture(dir.path());
    let paths = Paths::new(dir.path().join("config"), dir.path().join("state"));
    write_config(&paths);
    let trash = Arc::new(TestTrash::new(dir.path().join("bin")));
    let mut registry = Registry::new();
    registry.set_local(trash);
    let options = OpenOptions {
        places: vec![work.to_str().unwrap().to_owned()],
        ..OpenOptions::default()
    };
    let (mut app, mut runtime) = diroxide_cli::open::build_on(
        &options,
        Some(&paths),
        &place(dir.path()),
        Arc::new(registry),
    )
    .unwrap();
    let mut s = session(&mut app, &mut runtime);
    let run = Run::new();
    s.start(run.now());
    run.settle(&mut s);
    // The first run: the welcome and the layout's key, from the keymap.
    let said: Vec<String> = s.said().iter().map(|m| m.text.clone()).collect();
    let welcome = said
        .iter()
        .find(|l| l.starts_with("diroxide alpha 1."))
        .unwrap();
    fact(welcome, "diroxide alpha 1. Shift+F1 for help.");
    let layout = said.iter().find(|l| l.starts_with("Layout:")).unwrap();
    fact(layout, "Layout: side by side. Ctrl+P, Layout.");
    let screen = s.screen();
    // The title line, and every row on the 40-cell line from its first
    // cell: the name first.
    fact(&screen.lines[0], "work, left panel, in ");
    let names: Vec<String> = s
        .app()
        .active_panel()
        .list()
        .rows()
        .iter()
        .map(|r| r.label.clone())
        .collect();
    for (i, name) in names.iter().enumerate() {
        let line = &screen.lines[2 + i];
        assert!(left_half(line).starts_with(name.as_str()), "{line}");
    }
    // A sort change, said first; the cursor stays on its entry.
    assert!(
        s.app_mut()
            .focus_name(diroxide_core::Side::Left, "notes.txt")
    );
    run.settle(&mut s);
    let screen = run.press(&mut s, "ctrl+f6");
    fact(&screen.lines[22], "Sorted by size, largest first");
    assert!(cursor_row(&screen).starts_with("notes.txt"));
    fact(&screen.lines[0], "work, left panel, in ");
    assert!(screen.lines[0].ends_with(", sorted by size"));
    let screen = run.press(&mut s, "ctrl+f6");
    fact(&screen.lines[22], "Sorted by size, smallest first");
    let screen = run.press(&mut s, "ctrl+f3");
    fact(&screen.lines[22], "Sorted by name, A to Z");
    // Say Status without marks.
    let screen = run.press(&mut s, "ctrl+d");
    fact(&screen.lines[22], "work, 8 items, no selection");
    // Say Other Panel.
    let screen = run.press(&mut s, "alt+o");
    fact(&screen.lines[22], "Other panel: work, 8 items");
    // Make folder: the prompt, then the result.
    let screen = run.press(&mut s, "f7");
    assert!(
        screen
            .lines
            .iter()
            .any(|l| l.starts_with("New folder in work:")),
        "{:#?}",
        screen.lines
    );
    run.type_text(&mut s, "Reports");
    let screen = run.press(&mut s, "enter");
    fact(&screen.lines[22], "Created folder Reports");
    assert!(cursor_row(&screen).starts_with("Reports, folder"));
    // A name that needs care is refused in words.
    let screen = run.press(&mut s, "f2");
    assert!(
        screen
            .lines
            .iter()
            .any(|l| l.starts_with("Rename Reports to:"))
    );
    run.type_text(&mut s, " ");
    let screen = run.press(&mut s, "enter");
    fact(&screen.lines[22], "Name needs care: ends in a dot or space");
    run.press(&mut s, "escape");
    // Layout: the palette's way.
    run.press(&mut s, "ctrl+p");
    run.type_text(&mut s, "layout");
    let screen = run.press(&mut s, "enter");
    fact(&screen.lines[22], "Layout: stacked");
    // Nothing to undo after an undo of nothing.
    let screen = run.press(&mut s, "ctrl+z");
    assert!(
        within_cells(&screen.lines[22], "Undid")
            || within_cells(&screen.lines[22], "Nothing to undo"),
        "{}",
        screen.lines[22]
    );
    // The idle jobs line.
    assert_eq!(screen.lines[23], "No jobs");
}
