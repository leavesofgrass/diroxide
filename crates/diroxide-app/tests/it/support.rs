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

/// A session: an app and its runtime, and everything said so far.
pub struct Session {
    /// The app.
    pub app: App,
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
/// test sets one.
pub fn settings() -> Settings {
    Settings {
        dialog_settle_ms: 0,
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
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            settings: settings(),
            paths: None,
            home: None,
            recorder: Arc::new(Recorder::default()),
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
        let app = App::new(options);
        let mut runtime = Runtime::new(Arc::new(registry))
            .with_launcher(setup.recorder.clone())
            .with_opener(setup.recorder.clone());
        if let Some(paths) = paths {
            runtime = runtime.with_paths(paths.clone());
        }
        let mut session = Self {
            app,
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
