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
use diroxide_core::{Key, KeyCode, Message, Place, Side};
use diroxide_gui::frontend::{self, Frontend, Host};
use diroxide_gui::keys;
use diroxide_gui::widgets::root::Root;
use masonry::accesskit::{Action, ActionRequest, Live, NodeId, Role, TreeId};
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

    fn set_window_label(&mut self, label: &str) {
        self.0.set_window_label(label);
    }

    fn set_default_properties(&mut self, props: masonry::core::DefaultProperties) {
        self.0.set_default_properties(std::sync::Arc::new(props));
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
        Self::sized_on(app, runtime, platform, SIZE)
    }

    /// The window at `size` in physical pixels (spike S4's 1080p frame).
    pub fn sized(app: App, runtime: Runtime, size: (u32, u32)) -> Self {
        Self::sized_on(app, runtime, Platform::current(), size)
    }

    fn sized_on(app: App, runtime: Runtime, platform: Platform, size: (u32, u32)) -> Self {
        let mut f = Frontend::new(app, runtime, "diroxide").with_platform(platform);
        f.start();
        let root = f.root_widget();
        let h = TestHarness::create_with_size(masonry::theme::default_property_set(), root, size);
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
            .settle(&mut HostOf(&mut self.h), self.now, Duration::from_secs(20));
        // The app's timers, as the terminal's scripted test runs them: a
        // tick now, the clock 200 ms on, a tick again, and the workers'
        // results that brought.
        self.f.tick(&mut HostOf(&mut self.h), self.now);
        self.now += Duration::from_millis(200);
        self.f.tick(&mut HostOf(&mut self.h), self.now);
        self.f
            .settle(&mut HostOf(&mut self.h), self.now, Duration::from_secs(20));
        self.redraw();
    }

    /// Settles until `done` holds, the clock a tenth of a second on each
    /// round, for at most ten seconds of the real clock: for what a
    /// worker or a watcher brings after the runtime looks idle (a folder
    /// listed again after a rename). A state, never a time, so a loaded
    /// machine only makes it slower.
    pub fn settle_until(&mut self, mut done: impl FnMut(&Self) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if done(self) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
            self.now += Duration::from_millis(100);
            self.settle();
        }
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
        let event = keys::press(&on_this_system(key), Platform::current());
        let _ = self.h.process_text_event(TextEvent::Keyboard(event));
        self.settle();
    }

    /// Types text into the open field, one key at a time.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = Key::new(KeyCode::Char(c), diroxide_core::Modifiers::NONE);
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

    /// What the live region announced since the start, in order: what a
    /// screen reader was told, beside what the focus says.
    pub fn spoken(&self) -> Vec<String> {
        self.f.spoken().iter().map(|m| m.text.clone()).collect()
    }

    /// Runs a command by its id, as a menu or the palette does.
    pub fn run(&mut self, id: &str) {
        let id = diroxide_core::CommandId::new(id.to_owned()).unwrap();
        let now = self.now;
        self.f.command(
            &mut HostOf(&mut self.h),
            diroxide_core::Command::new(id),
            now,
        );
        self.settle();
    }

    /// The rows of the open dialog's list in the app (a choice, a
    /// question's choices, a field's candidates), by id, with the focus.
    pub fn dialog_rows(&self) -> (Vec<String>, Option<usize>) {
        use diroxide_app::Dialog;
        let list = match self.f.app().dialog().map(|(_, d)| d) {
            Some(Dialog::Choose { list, .. }) => list,
            Some(Dialog::Question { choices, .. }) => choices,
            Some(Dialog::Edit {
                candidates: Some(list),
                ..
            }) => list,
            _ => return (Vec::new(), None),
        };
        (
            list.rows().iter().map(|r| r.id.clone()).collect(),
            list.focus_index(),
        )
    }

    /// Moves the open dialog's list to the row with `id` by its keys
    /// (Home, then Down), as a person would.
    pub fn focus_dialog_row(&mut self, id: &str) {
        let (rows, _) = self.dialog_rows();
        let index = rows
            .iter()
            .position(|r| r == id)
            .unwrap_or_else(|| panic!("no row {id} in {rows:?}"));
        self.press("home");
        for _ in 0..index {
            self.press("down");
        }
        assert_eq!(self.dialog_rows().1, Some(index));
    }

    /// The number of dialog nodes in the tree (only the top dialog is).
    pub fn dialogs_in_tree(&self) -> usize {
        self.walk()
            .iter()
            .filter(|(role, _)| *role == Role::Dialog)
            .count()
    }

    /// Puts the app's state on the widgets after a test changed it
    /// directly (a focus by name), and draws the tree again.
    pub fn refresh(&mut self) {
        self.f.refresh(&mut HostOf(&mut self.h));
        self.redraw();
    }

    /// The open dialog's name in the tree, if a dialog is shown.
    pub fn dialog_name(&self) -> Option<String> {
        self.walk()
            .into_iter()
            .find(|(role, _)| *role == Role::Dialog)
            .map(|(_, name)| name)
    }

    /// The focused node's role, name, and keyboard shortcut (what a
    /// screen reader says after the name when asked for keys).
    pub fn focus_key(&self) -> (Role, String, Option<String>) {
        let state = self.h.access_tree().state();
        let focused = state.focus_in_tree();
        let node = focused.active_descendant().unwrap_or(focused);
        let key = node.data().keyboard_shortcut().map(str::to_owned);
        (node.role(), name_of(&node), key)
    }

    /// The open dialog's description (read after its name), if any.
    pub fn dialog_description(&self) -> Option<String> {
        let state = self.h.access_tree().state();
        let mut stack = vec![state.root()];
        while let Some(node) = stack.pop() {
            if node.role() == Role::Dialog {
                return node.data().description().map(str::to_owned);
            }
            stack.extend(node.children());
        }
        None
    }

    /// The list box that holds the focused option, by name, with its
    /// real size.
    pub fn focus_list(&self) -> Option<(String, usize)> {
        let state = self.h.access_tree().state();
        let focused = state.focus_in_tree();
        let list = if focused.role() == Role::ListBox {
            focused
        } else {
            focused.parent()?
        };
        Some((name_of(&list), list.size_of_set().unwrap_or(0)))
    }

    /// The open dialog's nodes, depth first, each with its role and name.
    pub fn dialog_walk(&self) -> Vec<(Role, String)> {
        let state = self.h.access_tree().state();
        let mut stack = vec![state.root()];
        let mut dialog = None;
        while let Some(node) = stack.pop() {
            if node.role() == Role::Dialog {
                dialog = Some(node);
                break;
            }
            stack.extend(node.children());
        }
        let mut found = Vec::new();
        let mut stack: Vec<_> = dialog.into_iter().collect();
        while let Some(node) = stack.pop() {
            found.push((node.role(), name_of(&node)));
            let mut children: Vec<_> = node.children().collect();
            children.reverse();
            stack.extend(children);
        }
        found
    }

    /// The names of the dialog's buttons and toggles, in order.
    pub fn dialog_buttons(&self) -> Vec<(Role, String)> {
        self.dialog_walk()
            .into_iter()
            .filter(|(role, _)| matches!(role, Role::Button | Role::CheckBox))
            .collect()
    }

    /// The node with `role` whose name starts with `name`, inside the open
    /// dialog when one is shown (the window behind it is hidden), else
    /// anywhere.
    pub fn find(&self, role: Role, name: &str) -> Option<NodeId> {
        let state = self.h.access_tree().state();
        let mut stack = vec![state.root()];
        let mut found = None;
        while let Some(node) = stack.pop() {
            if node.is_hidden() {
                continue;
            }
            if node.role() == role && name_of(&node).starts_with(name) {
                found = Some(node.locate().0);
                break;
            }
            let mut children: Vec<_> = node.children().collect();
            children.reverse();
            stack.extend(children);
        }
        found
    }

    /// A screen reader's action on the node with `role` and a name that
    /// starts with `name` (Click, Focus).
    pub fn act(&mut self, action: Action, role: Role, name: &str) {
        let node = self
            .find(role, name)
            .unwrap_or_else(|| panic!("no {role:?} named {name:?} in {:?}", self.walk()));
        self.h.process_access_event(ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: node,
            data: None,
        });
        self.settle();
    }

    /// A mouse click (`count` 2 for a double click) on the middle of the
    /// node with `role` and a name that starts with `name`.
    pub fn click(&mut self, role: Role, name: &str, count: u8) {
        let at = {
            let state = self.h.access_tree().state();
            let mut stack = vec![state.root()];
            // The node named exactly `name` wins over one whose name only
            // starts with it.
            let mut at = None;
            let mut exact = None;
            while let Some(node) = stack.pop() {
                let text = name_of(&node);
                if node.role() == role && text.starts_with(name) && !node.is_hidden() {
                    let middle = node.bounding_box().map(|b| {
                        masonry::kurbo::Point::new((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)
                    });
                    if text == name {
                        exact = exact.or(middle);
                    }
                    at = middle;
                }
                if !node.is_hidden() {
                    stack.extend(node.children());
                }
            }
            exact
                .or(at)
                .unwrap_or_else(|| panic!("no {role:?} named {name:?} with bounds"))
        };
        self.press_at(masonry::kurbo::Point::new(at.x, at.y), count);
    }

    /// A mouse press and release at a window point.
    pub fn press_at(&mut self, at: masonry::kurbo::Point, count: u8) {
        use masonry::core::{PointerButton, PointerButtonEvent, PointerEvent, PointerState};
        use masonry::dpi::PhysicalPosition;
        self.h.mouse_move(at);
        let state = PointerState {
            position: PhysicalPosition::new(at.x, at.y),
            count,
            ..PointerState::default()
        };
        let _ = self
            .h
            .process_pointer_event(PointerEvent::Down(PointerButtonEvent {
                pointer: masonry_testing::PRIMARY_MOUSE,
                button: Some(PointerButton::Primary),
                state: state.clone(),
            }));
        let _ = self
            .h
            .process_pointer_event(PointerEvent::Up(PointerButtonEvent {
                pointer: masonry_testing::PRIMARY_MOUSE,
                button: Some(PointerButton::Primary),
                state,
            }));
        self.settle();
    }

    /// Presses a key as a held key's repeat: the window drops it unless a
    /// repeat means something there (the arrows, typing in a field).
    pub fn press_repeat(&mut self, key: &str) {
        let key = Key::parse(key).unwrap_or_else(|e| panic!("{key}: {e}"));
        let mut event = keys::press(&on_this_system(&key), Platform::current());
        event.repeat = true;
        let _ = self.h.process_text_event(TextEvent::Keyboard(event));
        self.settle();
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

/// A key as the tests write it ("ctrl+shift+p", the Windows and Linux
/// chord) as the window's keymap means it on this system: on macOS the
/// GUI column's Control chords are Command chords
/// (`Keymap::for_platform`), so the same test presses Command+Shift+P
/// there, as a person on a Mac would. Ctrl+Tab and Ctrl+Shift+Tab stay on
/// Control, as the keymap keeps them: Command+Tab switches apps there.
pub fn on_this_system(key: &Key) -> Key {
    let mut key = *key;
    if Platform::current() == Platform::MacOs && key.mods.ctrl && key.code != KeyCode::Tab {
        key.mods.ctrl = false;
        key.mods.meta = true;
    }
    key
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
    app_with(left, right, |_| {})
}

/// The same app, with `change` made to its options first (a clock, a
/// settle time, a state folder).
pub fn app_with(left: &Path, right: &Path, change: impl FnOnce(&mut AppOptions)) -> (App, Runtime) {
    let left = Place::parse(left.to_str().unwrap()).unwrap();
    let right = Place::parse(right.to_str().unwrap()).unwrap();
    let mut options = AppOptions::new(left, right);
    options.settings.dialog_settle_ms = 0;
    // The window's keys: the keymap's GUI column, as diro builds it for
    // the window.
    options.keymap = diroxide_app::Keymap::defaults_for(diroxide_core::KeyColumn::Gui)
        .for_platform(Platform::current());
    change(&mut options);
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
