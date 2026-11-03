//! The window (the `gui` feature): one Masonry window on winit, driven by
//! the app's loop through [`Frontend`].
//!
//! - [`run`] builds the event loop, sets the runtime's waker to post a
//!   winit user event, carries out the app's start (its announcements
//!   held for the live region), opens the window titled with the active
//!   place, and runs until the app quits. Closing the window runs the
//!   app's quit rule: when jobs still run, the app asks first and the
//!   window stays.
//! - A ticker thread posts one event when the app's next timer is due
//!   ([`Frontend::next_deadline`]): the filter's pause, the quick view's
//!   wait, the viewer's slices, the key test's end.
//! - The fonts are compiled in and registered when the window starts.
//! - A window that cannot start is a [`GuiError`], never a panic: no
//!   display is the event loop's error, and the window runner's graphics
//!   failure, a panic inside it that textweaver's patch names
//!   ([`masonry_winit::app::GRAPHICS_FAILURE`]), is caught and said.
//!
//! Patterns adapted from textweaver's `crates/textweaver-xilem/src/gui.rs`
//! and `src/safety.rs` (the owner's own project).

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use diroxide_app::commands::ids;
use diroxide_app::{App, Runtime};
use diroxide_core::Command;
use masonry::core::{ErasedAction, WidgetId};
use masonry::dpi::{LogicalSize, PhysicalPosition};
use masonry_winit::app::{
    AppDriver, DriverCtx, EventLoop, EventLoopProxy, GRAPHICS_FAILURE, MasonryState,
    MasonryUserEvent, NewWindow, WindowId,
};
use masonry_winit::winit::window::Window as WinitWindow;

use crate::frontend::Frontend;
use crate::{GuiError, GuiOptions};

pub use crate::frontend::{JOBS_LINE, STATUS_LINE, TITLE_LINE};

/// The window's first size, in logical pixels.
pub const FIRST_SIZE: (f64, f64) = (960.0, 640.0);

/// Where a `--background` window opens: far off every screen.
pub const OFF_SCREEN: (i32, i32) = (-30_000, -30_000);

/// The window's attributes: its title, its first size, and for
/// `--background` off screen, never activated and (on Windows) with no
/// taskbar button.
pub fn attributes(title: &str, background: bool) -> masonry_winit::winit::window::WindowAttributes {
    let mut attributes = WinitWindow::default_attributes()
        .with_title(title)
        .with_inner_size(LogicalSize::new(FIRST_SIZE.0, FIRST_SIZE.1));
    if background {
        attributes = attributes
            .with_active(false)
            .with_position(PhysicalPosition::new(OFF_SCREEN.0, OFF_SCREEN.1));
        #[cfg(windows)]
        {
            use masonry_winit::winit::platform::windows::WindowAttributesExtWindows;
            attributes = attributes.with_skip_taskbar(true);
        }
    }
    attributes
}

/// The message the event loop gets when a worker has a result.
#[derive(Debug)]
struct Wake;

/// The message the event loop gets when the app's next timer is due.
#[derive(Debug)]
struct Tick;

/// The runtime's waker: posts one [`Wake`] to the event loop until the
/// loop has handled it, so a busy worker does not flood the queue.
fn waker(
    proxy: EventLoopProxy,
    window: WindowId,
    pending: Arc<AtomicBool>,
) -> diroxide_app::runtime::Waker {
    let proxy = Mutex::new(proxy);
    Arc::new(move || {
        if pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let proxy = proxy.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = proxy.send_event(MasonryUserEvent::AsyncAction(window, Box::new(Wake)));
    })
}

/// The ticker's shared state: the next deadline, and whether to stop.
#[derive(Debug, Default)]
struct TickerState {
    deadline: Option<Instant>,
    stop: bool,
}

/// A thread that posts one [`Tick`] when the deadline it was given comes.
struct Ticker {
    shared: Arc<(Mutex<TickerState>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(proxy: EventLoopProxy, window: WindowId) -> Self {
        let shared = Arc::new((Mutex::new(TickerState::default()), Condvar::new()));
        let theirs = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("diroxide-gui-ticker".to_owned())
            .spawn(move || {
                let (lock, wakeup) = &*theirs;
                let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    if state.stop {
                        return;
                    }
                    match state.deadline {
                        None => {
                            state = wakeup.wait(state).unwrap_or_else(PoisonError::into_inner);
                        }
                        Some(at) => {
                            let now = Instant::now();
                            if now >= at {
                                state.deadline = None;
                                let _ = proxy.send_event(MasonryUserEvent::AsyncAction(
                                    window,
                                    Box::new(Tick),
                                ));
                            } else {
                                state = wakeup
                                    .wait_timeout(state, at - now)
                                    .unwrap_or_else(PoisonError::into_inner)
                                    .0;
                            }
                        }
                    }
                }
            })
            .ok();
        Self { shared, thread }
    }

    /// Sets the next deadline (or none).
    fn set(&self, deadline: Option<Instant>) {
        let (lock, wakeup) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if state.deadline != deadline {
            state.deadline = deadline;
            wakeup.notify_one();
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        {
            let (lock, wakeup) = &*self.shared;
            let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
            state.stop = true;
            wakeup.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The message the event loop gets from the native menu bar.
#[cfg(windows)]
#[derive(Debug)]
struct MenuMessage(crate::platform::menu::MenuEvent);

/// The window's native menu bar (Windows): built from the app's menu
/// model, built again when the model's tree changes, shown always or
/// only while a key has it open (`auto_hide_menu`).
#[cfg(windows)]
#[derive(Default)]
struct NativeMenus {
    hwnd: Option<isize>,
    bar: Option<crate::platform::menu::Bar>,
    fingerprint: u64,
    watching: bool,
}

#[cfg(windows)]
impl NativeMenus {
    /// The window's handle, from winit.
    fn hwnd_of(window: &WinitWindow) -> Option<isize> {
        use masonry_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        }
    }

    /// Brings the bar in line with the app after a step, then carries out
    /// what the menu keys asked.
    fn sync(
        &mut self,
        window: &WinitWindow,
        frontend: &mut Frontend,
        proxy: &EventLoopProxy,
        window_id: WindowId,
    ) {
        if !crate::menus::native(frontend.app()) {
            self.bar = None;
            frontend.set_menu_bar(false);
            let _ = frontend.take_menu_requests();
            return;
        }
        if self.hwnd.is_none() {
            self.hwnd = Self::hwnd_of(window);
        }
        let Some(hwnd) = self.hwnd else {
            return;
        };
        if !self.watching {
            let proxy = Mutex::new(proxy.clone());
            self.watching = crate::platform::menu::watch(
                hwnd,
                Box::new(move |event| {
                    let proxy = proxy.lock().unwrap_or_else(PoisonError::into_inner);
                    let _ = proxy.send_event(MasonryUserEvent::AsyncAction(
                        window_id,
                        Box::new(MenuMessage(event)),
                    ));
                }),
            );
        }
        let tree = crate::menus::tree(frontend.app());
        let fingerprint = crate::menus::fingerprint(&tree);
        if self.bar.is_none() || fingerprint != self.fingerprint {
            let was_shown = self
                .bar
                .as_ref()
                .is_some_and(crate::platform::menu::Bar::shown);
            self.bar = crate::platform::menu::Bar::build(&tree);
            self.fingerprint = fingerprint;
            if was_shown && let Some(bar) = self.bar.as_mut() {
                let _ = bar.attach(hwnd);
            }
        }
        let auto_hide = frontend.app().settings().auto_hide_menu;
        if let Some(bar) = self.bar.as_mut()
            && !auto_hide
            && !bar.shown()
        {
            let _ = bar.attach(hwnd);
        }
        frontend.set_menu_bar(self.bar.is_some());
        for request in frontend.take_menu_requests() {
            if let Some(bar) = self.bar.as_mut()
                && !bar.shown()
            {
                let _ = bar.attach(hwnd);
            }
            let letter = match request {
                crate::frontend::MenuEnter::Bar => None,
                crate::frontend::MenuEnter::Menu(c) => Some(c),
            };
            crate::platform::menu::enter(hwnd, letter);
        }
    }

    /// A message from the bar: a chosen item's command, or the menu
    /// closed (a bar a key showed hides again, saying nothing).
    fn message(
        &mut self,
        event: crate::platform::menu::MenuEvent,
        frontend: &Frontend,
    ) -> Option<String> {
        use crate::platform::menu::MenuEvent;
        match event {
            MenuEvent::Chosen(id) => self
                .bar
                .as_ref()
                .and_then(|bar| bar.command(id))
                .map(str::to_owned),
            MenuEvent::Closed => {
                if frontend.app().settings().auto_hide_menu
                    && let Some(bar) = self.bar.as_mut()
                {
                    bar.detach();
                }
                None
            }
        }
    }
}

/// The driver: the frontend, and what the window last showed.
struct Gui {
    /// The frontend, until the driver is dropped and gives its app to
    /// `slot`.
    frontend: Option<Frontend>,
    window: WindowId,
    title: String,
    pending: Arc<AtomicBool>,
    /// The runtime's waker, kept to wake the loop once the window exists.
    wake: diroxide_app::runtime::Waker,
    ticker: Ticker,
    slot: Rc<RefCell<Option<App>>>,
    /// The event loop, for the native menu's messages.
    #[cfg(windows)]
    proxy: EventLoopProxy,
    #[cfg(windows)]
    menus: NativeMenus,
}

impl Gui {
    /// One step of the frontend on the window's widgets, then the title,
    /// the next timer and the quit.
    fn step(
        &mut self,
        ctx: &mut DriverCtx<'_>,
        step: impl FnOnce(&mut Frontend, &mut masonry::app::RenderRoot),
    ) {
        let Some(frontend) = self.frontend.as_mut() else {
            return;
        };
        step(frontend, ctx.render_root(self.window));
        let title = frontend.window_title();
        if title != self.title {
            ctx.window(self.window).handle().set_title(&title);
            // AT-SPI and macOS read the title from the window's node.
            ctx.render_root(self.window)
                .set_window_label(title.as_str());
            self.title = title;
        }
        #[cfg(windows)]
        {
            let window = ctx.window(self.window).handle();
            self.menus.sync(window, frontend, &self.proxy, self.window);
        }
        self.ticker.set(frontend.next_deadline());
        if frontend.wants_quit() {
            ctx.exit();
        }
    }
}

impl AppDriver for Gui {
    fn on_action(
        &mut self,
        _window: WindowId,
        ctx: &mut DriverCtx<'_>,
        _widget: WidgetId,
        action: ErasedAction,
    ) {
        self.step(ctx, |frontend, root| {
            frontend.action(root, action, Instant::now());
        });
    }

    fn on_start(&mut self, state: &mut MasonryState) {
        for root in state.roots() {
            for blob in crate::fonts::blobs() {
                let _ = root.register_fonts(blob);
            }
        }
        // The window exists now. A wake posted before it did was dropped
        // as meant for an unknown window, so post one again: the first
        // step names the window's node and shows the held messages.
        self.pending.store(false, Ordering::Release);
        (self.wake)();
    }

    fn on_async_action(
        &mut self,
        _window: WindowId,
        ctx: &mut DriverCtx<'_>,
        action: ErasedAction,
    ) {
        if action.is::<Wake>() {
            self.pending.store(false, Ordering::Release);
            self.step(ctx, |frontend, root| frontend.wake(root, Instant::now()));
        } else if action.is::<Tick>() {
            self.step(ctx, |frontend, root| frontend.tick(root, Instant::now()));
        } else {
            #[cfg(windows)]
            if let Ok(message) = action.downcast::<MenuMessage>() {
                let command = match self.frontend.as_ref() {
                    Some(frontend) => self.menus.message(message.0, frontend),
                    None => None,
                };
                if let Some(command) = command {
                    self.step(ctx, |frontend, root| {
                        frontend.menu_command(root, &command, Instant::now());
                    });
                }
            }
        }
    }

    fn on_close_requested(&mut self, _window: WindowId, ctx: &mut DriverCtx<'_>) {
        // The app's quit rule: the session and the history are saved, and
        // a running job is asked about first (the window stays then).
        self.step(ctx, |frontend, root| {
            frontend.command(root, Command::new(ids::QUIT), Instant::now());
        });
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        // The app goes back to `run`'s caller however the loop ended.
        if let Some(frontend) = self.frontend.take() {
            *self.slot.borrow_mut() = Some(frontend.into_app());
        }
    }
}

/// The cause in a caught panic's payload, in words.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "no description".to_owned())
}

/// The error for a panic that stopped the window: the graphics failure the
/// window runner names, or an internal error.
pub fn error_for_panic(text: &str) -> GuiError {
    match text.split_once(GRAPHICS_FAILURE) {
        Some((_, cause)) => {
            let cause = cause.trim_start_matches(':').trim();
            GuiError::Graphics(if cause.is_empty() {
                "no reason given".to_owned()
            } else {
                cause.to_owned()
            })
        }
        None => GuiError::Stopped(text.lines().next().unwrap_or(text).trim().to_owned()),
    }
}

/// The error for an event loop that could not be built: no display, in
/// words.
fn error_for_event_loop(error: &masonry_winit::winit::error::EventLoopError) -> GuiError {
    use masonry_winit::winit::error::EventLoopError;
    match error {
        EventLoopError::NotSupported(_) | EventLoopError::Os(_) => {
            GuiError::NoDisplay(error.to_string())
        }
        other => GuiError::EventLoop(other.to_string()),
    }
}

/// Masonry installs a trace subscriber that prints to the terminal when
/// none is set. Unless a developer asks for traces (`RUST_LOG`, or
/// `MASONRY_DENSE_LOG_DIR`, which textweaver's patch makes the only place
/// the dense log is written, so it never lands in the temporary folder on
/// the system drive), a quiet one is set first.
fn quiet_traces() {
    let asked = ["RUST_LOG", "MASONRY_DENSE_LOG_DIR"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()));
    if !asked {
        let _ =
            tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default());
    }
}

/// The panic hook while the window runs: the window runner's graphics
/// failure is said by `diro` in one sentence after the window is gone, so
/// the default report of it (a thread, a source line, a note about
/// backtraces) would only stand in front of that sentence in the
/// terminal. Every other panic keeps the default report. Returns what
/// puts the previous hook back.
fn hush_graphics_failure() -> impl FnOnce() {
    type Hook = dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync;
    let previous: Arc<Hook> = Arc::from(std::panic::take_hook());
    let report = Arc::clone(&previous);
    std::panic::set_hook(Box::new(move |info| {
        if !info.to_string().contains(GRAPHICS_FAILURE) {
            report(info);
        }
    }));
    move || {
        let _ = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| previous(info)));
    }
}

/// Runs the window until the app quits; see [`crate::run`].
pub fn run(app: App, mut runtime: Runtime, options: GuiOptions) -> Result<App, GuiError> {
    quiet_traces();
    let event_loop = EventLoop::with_user_event()
        .build()
        .map_err(|e| error_for_event_loop(&e))?;
    let proxy = event_loop.create_proxy();
    let window_id = WindowId::next();
    let pending = Arc::new(AtomicBool::new(false));
    let wake = waker(proxy.clone(), window_id, Arc::clone(&pending));
    runtime.set_waker(Arc::clone(&wake));
    let mut frontend = Frontend::new(app, runtime, options.title.clone());
    frontend.start();
    let title = frontend.window_title();
    let window = NewWindow::new_with_id(
        window_id,
        attributes(&title, options.background),
        frontend.root_widget().erased(),
    );
    let slot = Rc::new(RefCell::new(None));
    let gui = Gui {
        frontend: Some(frontend),
        window: window_id,
        // Empty, so the first step names the window's accessibility node
        // too (AT-SPI and macOS read the title from it).
        title: String::new(),
        pending: Arc::clone(&pending),
        wake,
        ticker: Ticker::start(proxy.clone(), window_id),
        slot: Rc::clone(&slot),
        #[cfg(windows)]
        proxy,
        #[cfg(windows)]
        menus: NativeMenus::default(),
    };
    // The console a double-click opened was let go by diro before the app
    // was built (`crate::console::release_for_window`), and is given back
    // by it if this returns an error.
    let restore_hook = hush_graphics_failure();
    let ran = catch_unwind(AssertUnwindSafe(|| {
        masonry_winit::app::run_with(
            event_loop,
            vec![window],
            gui,
            masonry::theme::default_property_set(),
        )
    }));
    restore_hook();
    match ran {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(GuiError::EventLoop(e.to_string())),
        Err(payload) => return Err(error_for_panic(&panic_text(payload.as_ref()))),
    }
    slot.borrow_mut()
        .take()
        .ok_or_else(|| GuiError::Stopped("the app was not handed back".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graphics_failure_is_said_and_other_panics_are_internal() {
        let text = format!("{GRAPHICS_FAILURE}: no compatible WGPU device found");
        assert_eq!(
            error_for_panic(&text),
            GuiError::Graphics("no compatible WGPU device found".to_owned())
        );
        assert_eq!(
            error_for_panic("index out of bounds: the len is 0\nbacktrace"),
            GuiError::Stopped("index out of bounds: the len is 0".to_owned())
        );
    }

    /// The model's tree as a Win32 menu, read back as UI Automation reads
    /// it: the bar's menus with their access keys, each item's label and,
    /// after a tab, its key (AcceleratorKey); no window needed.
    #[cfg(windows)]
    #[test]
    fn the_native_menu_holds_the_models_items_and_keys() {
        use diroxide_app::AppOptions;
        use diroxide_core::Place;
        let here = Place::parse(env!("CARGO_MANIFEST_DIR")).unwrap();
        let app = App::new(AppOptions::new(here.clone(), here));
        let tree = crate::menus::tree(&app);
        let bar = crate::platform::menu::Bar::build(&tree).unwrap();
        let texts = bar.texts();
        let bar_menus: Vec<&str> = texts
            .iter()
            .filter(|(depth, _)| *depth == 0)
            .map(|(_, text)| text.as_str())
            .collect();
        let expected: Vec<&str> = tree.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(bar_menus, expected);
        assert_eq!(bar_menus[0], "&File");
        // Every item of the File menu, in order, with its key after a tab.
        let file: Vec<String> = tree[0]
            .entries
            .iter()
            .map(crate::menus::Entry::windows_text)
            .collect();
        let held: Vec<&str> = texts
            .iter()
            .skip(1)
            .take_while(|(depth, _)| *depth >= 1)
            .filter(|(depth, _)| *depth == 1)
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(held, file);
        assert!(held.iter().any(|t| t.contains('\t')), "{held:?}");
        // The first item's id gives its command back.
        assert!(bar.command(1).is_some());
        assert!(bar.command(0).is_none());
        assert!(!bar.shown());
    }

    #[test]
    fn a_background_window_is_off_screen_and_inactive() {
        let hidden = attributes("Documents - diroxide", true);
        assert_eq!(hidden.title, "Documents - diroxide");
        assert!(!hidden.active);
        assert!(hidden.position.is_some());
        let shown = attributes("Documents - diroxide", false);
        assert!(shown.active);
        assert!(shown.position.is_none());
    }
}
