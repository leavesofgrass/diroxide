//! Helpers for driving the app without a terminal: a session that feeds
//! keys and commands through the runtime and keeps what was said, fixture
//! folders made at test time under the build's temporary folder, and a
//! provider that trashes into a test folder, never the system trash.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diroxide_app::{
    App, AppEvent, AppOptions, Dialog, Launcher, Opener, Program, Runtime, Startup,
};
use diroxide_config::{Paths, Settings};
use diroxide_core::{
    Command, CommandId, Effect, Entry, FixedClock, Key, Message, Place, Side, Timestamp, UtcOffset,
};
use diroxide_vfs::{
    AttributeChange, Capabilities, CopyHint, FileWriter, LinkRequest, LocalProvider, OpContext,
    Provider, ReadSeek, Registry, SpaceInfo, TrashCheck, TrashGate, VfsError, VolumeInfo,
    WatchHandle, WatchSink, WriteMode,
};

/// A place for a path.
pub fn place(path: &Path) -> Place {
    Place::parse(path.to_str().unwrap()).unwrap()
}

/// A temporary folder for one test, under the system temporary folder
/// (which the test run points at the build's own folder).
pub fn temp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("app-it-")
        .tempdir()
        .unwrap()
}

/// The fixture: a folder of plain names, accents, Japanese and an emoji,
/// with a subfolder and a `Backup` folder holding an older report.
pub fn fixture(root: &Path) -> PathBuf {
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

/// A provider over the local disk whose trash is a test folder: an item
/// moves there, and restore brings back the newest one for a place.
#[derive(Debug)]
pub struct TestTrash {
    local: LocalProvider,
    bin: PathBuf,
    held: Mutex<HashMap<String, Vec<PathBuf>>>,
    counter: Mutex<u32>,
}

impl TestTrash {
    /// A provider trashing into `bin`.
    pub fn new(bin: PathBuf) -> Self {
        fs::create_dir_all(&bin).unwrap();
        Self {
            local: LocalProvider::new(),
            bin,
            held: Mutex::new(HashMap::new()),
            counter: Mutex::new(0),
        }
    }

    /// How many items the test trash holds.
    pub fn count(&self) -> usize {
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

/// The test clock: Friday, October 16, 2026, 9:00 UTC.
pub fn clock() -> Arc<FixedClock> {
    Arc::new(FixedClock::new(
        Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
        UtcOffset::UTC,
    ))
}

/// A tools runner for tests (alpha 4, lane VA): it finds only the tools
/// a test names, records every run, and answers each with the output set
/// for its tool, so no test starts a real program (ffprobe, pdftotext,
/// ExifTool, tesseract).
#[derive(Default)]
pub struct FakeTools {
    /// The tools it finds, by registry id.
    pub found: Mutex<Vec<String>>,
    /// The output each tool gives, by registry id.
    pub answers: Mutex<HashMap<String, diroxide_vfs::ToolOutput>>,
    /// The output a tool gives when its first argument is this one, by
    /// registry id and that argument (`tesseract --list-langs`).
    pub by_first: Mutex<HashMap<(String, String), diroxide_vfs::ToolOutput>>,
    /// Every run asked for, in order.
    pub runs: Mutex<Vec<diroxide_vfs::ToolRequest>>,
}

impl FakeTools {
    /// Makes a tool found, answering its runs with this standard output.
    pub fn answer(&self, tool: &str, stdout: &[u8]) {
        self.answer_with(
            tool,
            diroxide_vfs::ToolOutput::new(Some(0), stdout.to_vec(), Vec::new()),
        );
    }

    /// Makes a tool found, answering its runs with this output.
    pub fn answer_with(&self, tool: &str, output: diroxide_vfs::ToolOutput) {
        self.found.lock().unwrap().push(tool.to_owned());
        self.answers.lock().unwrap().insert(tool.to_owned(), output);
    }

    /// Makes a tool found, answering its runs whose first argument is
    /// `first` with this standard output.
    pub fn answer_first(&self, tool: &str, first: &str, stdout: &[u8]) {
        self.found.lock().unwrap().push(tool.to_owned());
        self.by_first.lock().unwrap().insert(
            (tool.to_owned(), first.to_owned()),
            diroxide_vfs::ToolOutput::new(Some(0), stdout.to_vec(), Vec::new()),
        );
    }

    /// The tools run so far, by id, in order.
    pub fn ran(&self) -> Vec<String> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.tool.clone())
            .collect()
    }
}

impl diroxide_vfs::ToolRunner for FakeTools {
    fn has(&self, tool: &str) -> bool {
        self.found.lock().unwrap().iter().any(|t| t == tool)
    }

    fn run(
        &self,
        request: &diroxide_vfs::ToolRequest,
        _cancel: &diroxide_vfs::CancelToken,
    ) -> Result<diroxide_vfs::ToolOutput, diroxide_vfs::ToolRunError> {
        self.runs.lock().unwrap().push(request.clone());
        let first = request
            .args
            .first()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(output) = self
            .by_first
            .lock()
            .unwrap()
            .get(&(request.tool.clone(), first))
        {
            return Ok(output.clone());
        }
        self.answers
            .lock()
            .unwrap()
            .get(&request.tool)
            .cloned()
            .ok_or_else(|| diroxide_vfs::ToolRunError::NotInstalled {
                tool: request.tool.clone(),
                name: request.tool.clone(),
            })
    }

    fn run_streaming(
        &self,
        request: &diroxide_vfs::ToolRequest,
        cancel: &diroxide_vfs::CancelToken,
        _record: &mut dyn FnMut(diroxide_vfs::ToolStream, &str) -> std::ops::ControlFlow<()>,
    ) -> Result<diroxide_vfs::ToolOutput, diroxide_vfs::ToolRunError> {
        self.run(request, cancel)
    }
}

/// A session: an app and its runtime, and everything said so far.
pub struct Session {
    /// The app.
    pub app: App,
    /// The tools runner its tool-backed work goes through.
    pub tools: Arc<FakeTools>,
    /// The runtime.
    pub runtime: Runtime,
    /// The clock, moved by the tests.
    pub clock: Arc<FixedClock>,
    /// Every message said, in order.
    pub said: Vec<Message>,
    /// Every screen effect, in order.
    pub effects: Vec<Effect>,
}

/// Settings for tests: no settle time, so a key answers at once, unless a
/// test sets one; the first-run components list taken as shown, so a
/// fresh state folder is not a first start's list unless a test asks
/// (lane A3).
pub fn settings() -> Settings {
    Settings {
        dialog_settle_ms: 0,
        components_first_run_shown: true,
        ..Settings::default()
    }
}

/// Records the programs a session runs and the files it opens, so no
/// test starts a real program.
#[derive(Default)]
pub struct Recorder {
    /// The programs run.
    pub programs: Mutex<Vec<Program>>,
    /// The files opened.
    pub opened: Mutex<Vec<PathBuf>>,
    /// The web and mail addresses opened.
    pub addresses: Mutex<Vec<String>>,
    /// The programs Open with started, by name, and the files (lane TA).
    pub started_with: Mutex<Vec<(String, PathBuf)>>,
    /// The files handed to the system's Open with dialog (lane TA).
    pub chosen_another: Mutex<Vec<PathBuf>>,
}

/// Open with's two programs in tests: Reader, the default, and Writer.
pub fn open_with_programs() -> diroxide_app::typing::OpenWithRows {
    let row = |name: &str, default: bool| diroxide_app::typing::OpenWithRow {
        name: name.to_owned(),
        id: name.to_lowercase(),
        default,
    };
    diroxide_app::typing::OpenWithRows {
        rows: vec![row("Reader", true), row("Writer", false)],
        choose_another: true,
        unread: None,
    }
}

impl diroxide_app::OpenWithPrograms for Recorder {
    fn list(&self, _file: &Path, _mime: Option<&str>) -> diroxide_app::typing::OpenWithRows {
        open_with_programs()
    }

    fn start(&self, id: &str, file: &Path, _mime: Option<&str>) -> Result<(), String> {
        self.started_with
            .lock()
            .unwrap()
            .push((id.to_owned(), file.to_owned()));
        Ok(())
    }

    fn choose_another(&self, file: &Path) -> Result<(), String> {
        self.chosen_another.lock().unwrap().push(file.to_owned());
        Ok(())
    }
}

impl Launcher for Recorder {
    fn run(&self, program: &Program) -> Result<Option<i32>, String> {
        self.programs.lock().unwrap().push(program.clone());
        Ok(Some(0))
    }
    fn shell(&self) -> (OsString, Vec<OsString>) {
        (OsString::from("testshell"), vec![OsString::from("-c")])
    }
    fn editor(&self) -> OsString {
        OsString::from("testeditor")
    }
}

impl Opener for Recorder {
    fn open(&self, path: &Path) -> Result<(), String> {
        self.opened.lock().unwrap().push(path.to_owned());
        Ok(())
    }

    fn open_address(&self, address: &str) -> Result<(), String> {
        self.addresses.lock().unwrap().push(address.to_owned());
        Ok(())
    }
}

/// How a session is set up beyond its places and registry.
pub struct Setup {
    /// The settings.
    pub settings: Settings,
    /// The config and state folders, when the session uses them.
    pub paths: Option<Paths>,
    /// The home folder.
    pub home: Option<Place>,
    /// Records programs and opened files.
    pub recorder: Arc<Recorder>,
    /// Starts programs in the test's own way in place of the recorder,
    /// never a real one (lane A5: a launcher that does not wait).
    pub launcher: Option<Arc<dyn Launcher>>,
    /// Starts plugins in the test's own way, never a real process (W4d).
    pub plugin_launcher: Option<Arc<dyn diroxide_plugins::host::Launcher>>,
    /// Starts with plugins off, as `--no-plugins` does (W4d).
    pub no_plugins: bool,
    /// Fetches components in the test's own way (W4d).
    pub component_fetcher: Option<Arc<dyn diroxide_components::Fetcher>>,
    /// Runs the helper programs in the test's own way, finding none
    /// unless the test names them (alpha 4).
    pub tools: Arc<FakeTools>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            settings: settings(),
            paths: None,
            home: None,
            recorder: Arc::new(Recorder::default()),
            launcher: None,
            plugin_launcher: None,
            no_plugins: false,
            component_fetcher: None,
            tools: Arc::new(FakeTools::default()),
        }
    }
}

impl Session {
    /// A session on two places over a registry, with a state folder when
    /// given, started and settled.
    pub fn start(
        left: Place,
        right: Place,
        registry: Registry,
        paths: Option<&Paths>,
        settings: Settings,
    ) -> Self {
        Self::open(
            left,
            right,
            registry,
            Setup {
                settings,
                paths: paths.cloned(),
                ..Setup::default()
            },
        )
    }

    /// A session set up in full, with the default keys.
    pub fn open(left: Place, right: Place, registry: Registry, setup: Setup) -> Self {
        Self::open_with_keymap(
            left,
            right,
            registry,
            setup,
            diroxide_app::Keymap::defaults(),
        )
    }

    /// A session set up in full, with these keys.
    pub fn open_with_keymap(
        left: Place,
        right: Place,
        registry: Registry,
        setup: Setup,
        keymap: diroxide_app::Keymap,
    ) -> Self {
        let paths = setup.paths.as_ref();
        let mut startup = match paths {
            Some(paths) => {
                let config = diroxide_config::load(paths);
                diroxide_app::startup::load(
                    paths,
                    &config,
                    diroxide_app::keymap::Platform::current(),
                )
            }
            None => Startup::default(),
        };
        if setup.home.is_some() {
            startup.home = setup.home.clone();
        }
        let clock = clock();
        let mut options = AppOptions::new(left, right);
        options.settings = setup.settings;
        options.clock = clock.clone();
        options.startup = startup;
        options.keymap = keymap;
        let mut app = App::new(options);
        app.set_tool_runner(setup.tools.clone());
        let tools = setup.tools.clone();
        let mut runtime = Runtime::new(Arc::new(registry))
            .with_launcher(setup.recorder.clone())
            .with_opener(setup.recorder.clone())
            .with_open_with(setup.recorder.clone())
            .with_components_offline();
        if let Some(paths) = paths {
            runtime = runtime.with_paths(paths.clone());
        }
        if let Some(launcher) = setup.launcher.clone() {
            runtime = runtime.with_launcher(launcher);
        }
        if let Some(launcher) = setup.plugin_launcher.clone() {
            runtime = runtime.with_plugin_launcher(launcher);
        }
        if setup.no_plugins {
            runtime = runtime.without_plugins();
        }
        if let Some(fetcher) = setup.component_fetcher.clone() {
            runtime = runtime.with_component_fetcher(fetcher);
        }
        let mut session = Self {
            app,
            tools,
            runtime,
            clock,
            said: Vec::new(),
            effects: Vec::new(),
        };
        let start = session.app.start();
        session.run(start);
        session
    }

    /// Carries out effects until the runtime is idle; returns what was
    /// said meanwhile.
    pub fn run(&mut self, effects: Vec<Effect>) -> Vec<String> {
        let screen = self.runtime.run_until_idle(&mut self.app, effects);
        let mut said = Vec::new();
        for effect in screen {
            if let Effect::Announce(message) = &effect {
                said.push(message.text.clone());
                self.said.push(message.clone());
            }
            self.effects.push(effect);
        }
        said
    }

    /// Keeps what effects say without carrying anything out, so the
    /// requests the app queued stay for the test to read.
    pub fn run_quiet(&mut self, effects: Vec<Effect>) -> Vec<String> {
        let mut said = Vec::new();
        for effect in effects {
            if let Effect::Announce(message) = &effect {
                said.push(message.text.clone());
                self.said.push(message.clone());
            }
            self.effects.push(effect);
        }
        said
    }

    /// Presses a key without carrying out its effects.
    pub fn key_quiet(&mut self, text: &str) -> Vec<String> {
        let effects = self.app.handle(AppEvent::Key(Key::parse(text).unwrap()));
        self.run_quiet(effects)
    }

    /// Presses a key; returns what was said.
    pub fn key(&mut self, text: &str) -> Vec<String> {
        let effects = self.app.handle(AppEvent::Key(Key::parse(text).unwrap()));
        self.run(effects)
    }

    /// Types text, one key a character.
    pub fn type_text(&mut self, text: &str) -> Vec<String> {
        let mut said = Vec::new();
        for c in text.chars() {
            let key = Key::new(
                diroxide_core::KeyCode::Char(c),
                diroxide_core::Modifiers::NONE,
            );
            let effects = self.app.handle(AppEvent::Key(key));
            said.extend(self.run(effects));
        }
        said
    }

    /// Runs a command by id; returns what was said.
    pub fn command(&mut self, id: CommandId) -> Vec<String> {
        self.run_command(Command::new(id))
    }

    /// Runs a command; returns what was said.
    pub fn run_command(&mut self, command: Command) -> Vec<String> {
        let effects = self.app.handle(AppEvent::Command(command));
        self.run(effects)
    }

    /// The name of the bank the key bar shows, after checking that
    /// `App::shown_bank` is the bank `App::key_bar` draws (lane A4: the
    /// GUI's toolbar takes the bank from there).
    pub fn shown_bank(&self) -> String {
        let bank = self.app.shown_bank().expect("a bank is shown");
        assert_eq!(bank.key_bar(self.app.keymap()), self.app.key_bar());
        bank.name
    }

    /// The focused row's id in the active panel.
    pub fn focused(&self) -> String {
        self.app
            .active_panel()
            .list()
            .focused()
            .map(|r| r.id.clone())
            .unwrap_or_default()
    }

    /// Moves the focus to a name in the active panel by type-ahead free
    /// means: Home, then Down until it is there.
    pub fn focus(&mut self, name: &str) {
        self.key("home");
        for _ in 0..self.app.active_panel().list().len() {
            if self.focused() == name {
                return;
            }
            self.key("down");
        }
        assert_eq!(self.focused(), name, "{name} is not in the panel");
    }

    /// The top dialog's title, if one is open.
    pub fn dialog_title(&self) -> Option<String> {
        self.app.dialog().map(|(_, d)| d.title())
    }

    /// The top dialog.
    pub fn dialog(&self) -> Option<&Dialog> {
        self.app.dialog().map(|(_, d)| d)
    }

    /// The names in a panel, in row order.
    pub fn names(&self, side: Side) -> Vec<String> {
        self.app
            .panel(side)
            .list()
            .rows()
            .iter()
            .map(|r| r.id.clone())
            .collect()
    }
}

/// A registry whose local provider trashes into `bin`.
pub fn registry_with_trash(bin: &Path) -> (Registry, Arc<TestTrash>) {
    let trash = Arc::new(TestTrash::new(bin.to_owned()));
    let mut registry = Registry::new();
    registry.set_local(trash.clone());
    (registry, trash)
}

/// The start line's first sentence, from the package version: "diroxide
/// alpha 2." for 0.1.0-alpha.2, so a version bump leaves the tests alone.
pub fn hello() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let number = version.rsplit("alpha.").next().unwrap_or(version);
    format!("diroxide alpha {number}.")
}

/// A small PNG made in memory, never fetched (alpha 4, lane VA): a
/// header, one stored image block of `width` by `height` RGBA pixels, all
/// white, and the end chunk, each with its CRC, as the `png` crate reads
/// them.
pub fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }
    let mut raw = Vec::new();
    for _ in 0..height {
        raw.push(0u8);
        raw.extend(std::iter::repeat_n(0xffu8, width as usize * 4));
    }
    // A zlib stream of one stored deflate block, then its Adler-32.
    let mut zlib = vec![0x78, 0x01, 0x01];
    let len = raw.len() as u16;
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.extend(chunk(b"IHDR", &header));
    png.extend(chunk(b"IDAT", &zlib));
    png.extend(chunk(b"IEND", &[]));
    png
}
