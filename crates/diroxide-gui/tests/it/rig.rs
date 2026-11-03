//! The GUI's loop in Masonry's in-process harness: the app and its
//! runtime as `diro` builds them, the window's widgets with no window, no
//! display and no graphics card, and the accessibility tree read through
//! AccessKit's own consumer, the crate the platform adapters use.

// Helpers outside the test functions unwrap as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::path::Path;
use std::time::{Duration, Instant};

use diroxide_app::keymap::Platform;
use diroxide_app::{App, AppOptions, Runtime};
use diroxide_core::{Key, Message, Place, Side};
use diroxide_gui::frontend::{self, Frontend, Host};
use diroxide_gui::keys;
use diroxide_gui::widgets::root::Root;
use masonry::accesskit::{Live, Role};
use masonry::core::{FromDynWidget, TextEvent, Widget, WidgetId, WidgetMut, WidgetTag};
use masonry_testing::TestHarness;

/// The harness as the driver's host.
pub struct HostOf<'a>(pub &'a mut TestHarness<Root>);

impl Host for HostOf<'_> {
    fn edit<W: Widget + FromDynWidget + ?Sized, R>(
        &mut self,
        tag: WidgetTag<W>,
        f: impl FnOnce(WidgetMut<'_, W>) -> R,
    ) -> R {
        self.0.edit_widget(tag, f)
    }

    fn id_of<W: Widget + FromDynWidget + ?Sized>(&self, tag: WidgetTag<W>) -> Option<WidgetId> {
        // Every tag the driver asks for is built with the window.
        Some(self.0.get_widget(tag).id())
    }

    fn focused(&self) -> Option<WidgetId> {
        self.0.focused_widget_id()
    }

    fn focus(&mut self, id: Option<WidgetId>) -> bool {
        if let Some(id) = id {
            let Some(widget) = self.0.try_get_widget(id) else {
                return false;
            };
            if widget.ctx().is_stashed() || widget.ctx().is_disabled() {
                return false;
            }
        }
        self.0.focus_on(id);
        true
    }
}

/// The window's size in the tests, in physical pixels.
pub const SIZE: (u32, u32) = (1100, 780);

/// A running window in the harness.
pub struct Rig {
    pub f: Frontend,
    pub h: TestHarness<Root>,
    pub now: Instant,
}

impl Rig {
    /// The window over an app, started, its listings in.
    pub fn new(app: App, runtime: Runtime) -> Self {
        Self::on(app, runtime, Platform::current())
    }

    /// The same, pressing keys as on `platform`.
    pub fn on(app: App, runtime: Runtime, platform: Platform) -> Self {
        let mut f = Frontend::new(app, runtime, "diroxide").with_platform(platform);
        f.start();
        let root = f.root_widget();
        let h = TestHarness::create_with_size(masonry::theme::default_property_set(), root, SIZE);
        let mut rig = Self {
            f,
            h,
            now: Instant::now(),
        };
        rig.f.refresh(&mut HostOf(&mut rig.h));
        rig.settle();
        rig
    }

    /// Hands the widgets' actions to the driver until none is left.
    pub fn pump(&mut self) {
        let mut guard = 0;
        while let Some((action, _)) = self.h.pop_action_erased() {
            self.f.action(&mut HostOf(&mut self.h), action, self.now);
            guard += 1;
            assert!(guard < 1_000, "the widgets keep sending actions");
        }
    }

    /// Waits for the workers, hands their results to the app, and draws
    /// the tree again.
    pub fn settle(&mut self) {
        self.pump();
        self.f
            .settle(&mut HostOf(&mut self.h), self.now, Duration::from_secs(10));
        self.redraw();
    }

    /// Draws the tree again, handing any action the drawing caused to the
    /// driver (a panel's new page size) and drawing again after it.
    pub fn redraw(&mut self) {
        for _ in 0..4 {
            let _ = self.h.redraw();
            let mut acted = false;
            while let Some((action, _)) = self.h.pop_action_erased() {
                self.f.action(&mut HostOf(&mut self.h), action, self.now);
                acted = true;
            }
            if !acted {
                break;
            }
        }
    }

    /// Presses a key written as the keymap writes it ("ctrl+q", "f5"),
    /// through the window: the key event to the focused widget, the root's
    /// action to the driver, the workers' results, a new tree.
    pub fn press(&mut self, key: &str) {
        let key = Key::parse(key).unwrap_or_else(|e| panic!("{key}: {e}"));
        self.press_key(&key);
    }

    /// Presses a key.
    pub fn press_key(&mut self, key: &Key) {
        let event = keys::press(key, Platform::current());
        let _ = self.h.process_text_event(TextEvent::Keyboard(event));
        self.settle();
    }

    /// Types text into the open field, one key at a time.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = Key::new(
                diroxide_core::KeyCode::Char(c),
                diroxide_core::Modifiers::NONE,
            );
            self.press_key(&key);
        }
    }

    /// The node with the keyboard focus, as a screen reader finds it: the
    /// focused widget's node, or its active descendant.
    pub fn focus_node(&self) -> (Role, String) {
        let state = self.h.access_tree().state();
        let focused = state.focus_in_tree();
        let node = focused.active_descendant().unwrap_or(focused);
        (node.role(), name_of(&node))
    }

    /// Every node's role and name, depth first from the root.
    pub fn walk(&self) -> Vec<(Role, String)> {
        let mut found = Vec::new();
        let mut stack = vec![self.h.access_tree().state().root()];
        while let Some(node) = stack.pop() {
            found.push((node.role(), name_of(&node)));
            let mut children: Vec<_> = node.children().collect();
            children.reverse();
            stack.extend(children);
        }
        found
    }

    /// The texts of the live region's messages now in the tree, with their
    /// politeness.
    pub fn live(&self) -> Vec<(String, Live)> {
        let state = self.h.access_tree().state();
        let mut out = Vec::new();
        let mut stack = vec![state.root()];
        while let Some(node) = stack.pop() {
            if node.live() != Live::Off && node.role() == Role::Label {
                out.push((name_of(&node), node.live()));
            }
            stack.extend(node.children());
        }
        out
    }

    /// What the app announced since the start, in order.
    pub fn said(&self) -> &[Message] {
        self.f.said()
    }

    /// The active panel's side.
    pub fn active(&self) -> Side {
        self.f.app().active_side()
    }

    /// The widget id of a panel's list box.
    pub fn panel_id(&self, side: Side) -> WidgetId {
        self.h.get_widget(frontend::panel_tag(side)).id()
    }
}

/// A node's name as the platform adapters read it: a label's name is its
/// value.
pub fn name_of(node: &accesskit_consumer::NodeRef<'_>) -> String {
    let name = if node.label_comes_from_value() {
        node.value()
    } else {
        node.label()
    };
    name.unwrap_or_default()
}

/// The app over `left` and `right`, as the tests build it: no config, no
/// state folder, the local disk, and no settle time on a dialog (the
/// scripted keys come faster than a held key could).
pub fn app_on(left: &Path, right: &Path) -> (App, Runtime) {
    let left = Place::parse(left.to_str().unwrap()).unwrap();
    let right = Place::parse(right.to_str().unwrap()).unwrap();
    let mut options = AppOptions::new(left, right);
    options.settings.dialog_settle_ms = 0;
    // The window's keys: the keymap's GUI column, as diro builds it for
    // the window.
    options.keymap = diroxide_app::Keymap::defaults_for(diroxide_core::KeyColumn::Gui)
        .for_platform(Platform::current());
    let app = App::new(options);
    (app, Runtime::with_local())
}

/// A folder of `names` (files with a few bytes each) and `folders`, in a
/// new temporary folder on D: under the build folder.
pub fn fixture(names: &[&str], folders: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("gui-it-")
        .tempdir()
        .unwrap();
    for name in names {
        std::fs::write(dir.path().join(name), b"diroxide").unwrap();
    }
    for folder in folders {
        std::fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    dir
}
