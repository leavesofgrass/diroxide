//! The window (the `gui` feature): one Masonry window on winit, driven by
//! the app's loop.
//!
//! - [`root_widget`] builds what the window shows, and [`lines`] what each
//!   of its texts says; the in-process harness tests build the same tree
//!   with no display and no graphics card.
//! - [`run`] builds the event loop, sets the runtime's waker to post a
//!   winit user event, carries out the app's start, opens the window titled
//!   with the active place ([`crate::window_title`]), and runs until the
//!   app quits. Closing the window runs the app's quit rule, which saves
//!   the session.
//! - A window that cannot start is a [`GuiError`], never a panic: no
//!   display is the event loop's error, and the window runner's graphics
//!   failure, a panic inside it that textweaver's patch names
//!   ([`masonry_winit::app::GRAPHICS_FAILURE`]), is caught and said.
//!
//! The GUI alpha's foundation lane (G1) adds the panels' list boxes, keys,
//! the live region and the rest; this is the frame they go in. Patterns
//! adapted from textweaver's `crates/textweaver-xilem/src/gui.rs` and
//! `src/safety.rs` (the owner's own project).

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use diroxide_app::commands::ids;
use diroxide_app::{App, Runtime};
use diroxide_core::{Command, Effect};
use masonry::core::{ErasedAction, NewWidget, WidgetId, WidgetTag};
use masonry::dpi::{LogicalSize, PhysicalPosition};
use masonry::widgets::{Flex, Label};
use masonry_winit::app::{
    AppDriver, DriverCtx, EventLoop, EventLoopProxy, GRAPHICS_FAILURE, MasonryUserEvent, NewWindow,
    WindowId,
};
use masonry_winit::winit::window::Window as WinitWindow;

use crate::{GuiError, GuiOptions, console, window_title};

/// The title line's text: the active place first ("Documents, left panel,
/// in D:\work"), the whole line, never cut.
pub const TITLE_LINE: WidgetTag<Label> = WidgetTag::named("diroxide-title-line");

/// The status line's text: the last message, or the active folder and its
/// counts.
pub const STATUS_LINE: WidgetTag<Label> = WidgetTag::named("diroxide-status-line");

/// The window's first size, in logical pixels.
pub const FIRST_SIZE: (f64, f64) = (960.0, 640.0);

/// Where a `--background` window opens: far off every screen.
pub const OFF_SCREEN: (i32, i32) = (-30_000, -30_000);

/// The texts of the window as the app stands: each tag and what it says.
pub fn lines(app: &App) -> [(WidgetTag<Label>, String); 2] {
    [
        (TITLE_LINE, app.title_line()),
        (STATUS_LINE, app.status_line()),
    ]
}

/// What the window shows: the title line above the status line, in a
/// column. G1 puts the panels between them.
pub fn root_widget(app: &App) -> NewWidget<Flex> {
    let [(title_tag, title), (status_tag, status)] = lines(app);
    let column = Flex::column()
        .with_fixed(NewWidget::new(Label::new(title)).with_tag(title_tag))
        .with_fixed(NewWidget::new(Label::new(status)).with_tag(status_tag));
    NewWidget::new(column)
}

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

/// The app's command that quits by the quit rule.
fn quit_command() -> Command {
    Command::new(ids::QUIT)
}

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

/// The driver: the app, its runtime, and what the window last showed.
struct Gui {
    /// The app, until the driver is dropped and gives it to `slot`.
    app: Option<App>,
    runtime: Runtime,
    window: WindowId,
    program: String,
    title: String,
    shown: Vec<String>,
    pending: Arc<AtomicBool>,
    slot: Rc<RefCell<Option<App>>>,
}

impl Gui {
    /// Carries out screen effects: a redraw refreshes the texts, quit
    /// closes the window. Announcements reach the live region in G1; the
    /// status line shows the last message until then.
    fn apply(&mut self, ctx: &mut DriverCtx<'_>, effects: Vec<Effect>) {
        let mut quit = false;
        for effect in effects {
            if effect == Effect::Quit {
                quit = true;
            }
        }
        self.refresh(ctx);
        if quit {
            ctx.exit();
        }
    }

    /// Takes the workers' results, hands them to the app, carries out what
    /// the app asks, until nothing is waiting.
    fn pump(&mut self, ctx: &mut DriverCtx<'_>) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        let mut screen = Vec::new();
        loop {
            let events = self.runtime.poll();
            if events.is_empty() {
                break;
            }
            for event in events {
                let effects = app.handle(event);
                screen.extend(self.runtime.carry_out(app, effects));
            }
        }
        self.apply(ctx, screen);
    }

    /// Puts the app's texts and title on the window where they changed.
    fn refresh(&mut self, ctx: &mut DriverCtx<'_>) {
        let Some(app) = self.app.as_ref() else {
            return;
        };
        let texts = lines(app);
        let root = ctx.render_root(self.window);
        for (index, (tag, text)) in texts.into_iter().enumerate() {
            if self.shown.get(index) != Some(&text) {
                root.edit_widget_with_tag(tag, |mut label| {
                    Label::set_text(&mut label, text.clone())
                });
                if let Some(slot) = self.shown.get_mut(index) {
                    *slot = text;
                } else {
                    self.shown.push(text);
                }
            }
        }
        let title = window_title(app, &self.program);
        if title != self.title {
            ctx.window(self.window).handle().set_title(&title);
            // AT-SPI and macOS read the title from the window's node.
            ctx.render_root(self.window)
                .set_window_label(title.as_str());
            self.title = title;
        }
    }
}

impl AppDriver for Gui {
    fn on_action(
        &mut self,
        _window: WindowId,
        _ctx: &mut DriverCtx<'_>,
        _widget: WidgetId,
        _action: ErasedAction,
    ) {
        // The frame has no controls yet; G1's panels send their actions.
    }

    fn on_async_action(
        &mut self,
        _window: WindowId,
        ctx: &mut DriverCtx<'_>,
        action: ErasedAction,
    ) {
        if action.is::<Wake>() {
            self.pending.store(false, Ordering::Release);
            self.pump(ctx);
        }
    }

    fn on_close_requested(&mut self, _window: WindowId, ctx: &mut DriverCtx<'_>) {
        // The app's quit rule: the session and the history are saved. The
        // frame cannot start a job, so no question stands in the way; G2's
        // dialogs ask when one runs.
        if let Some(app) = self.app.as_mut() {
            let effects = app.dispatch(&quit_command());
            let screen = self.runtime.carry_out(app, effects);
            self.apply(ctx, screen);
        }
        ctx.exit();
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        // The app goes back to `run`'s caller however the loop ended.
        if let Some(app) = self.app.take() {
            *self.slot.borrow_mut() = Some(app);
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

/// Runs the window until the app quits; see [`crate::run`].
pub fn run(app: App, mut runtime: Runtime, options: GuiOptions) -> Result<App, GuiError> {
    quiet_traces();
    let event_loop = EventLoop::with_user_event()
        .build()
        .map_err(|e| error_for_event_loop(&e))?;
    let proxy = event_loop.create_proxy();
    let title = window_title(&app, &options.title);
    let window = NewWindow::new(
        attributes(&title, options.background),
        root_widget(&app).erased(),
    );
    let window_id = window.id;
    let pending = Arc::new(AtomicBool::new(false));
    runtime.set_waker(waker(proxy, window_id, Arc::clone(&pending)));
    let mut app = app;
    let start = app.start();
    let _ = runtime.carry_out(&mut app, start);
    let shown = lines(&app).into_iter().map(|(_, text)| text).collect();
    let slot = Rc::new(RefCell::new(None));
    let gui = Gui {
        app: Some(app),
        runtime,
        window: window_id,
        program: options.title.clone(),
        title,
        shown,
        pending: Arc::clone(&pending),
        slot: Rc::clone(&slot),
    };
    let released = console::release_for_window();
    let ran = catch_unwind(AssertUnwindSafe(|| {
        masonry_winit::app::run_with(
            event_loop,
            vec![window],
            gui,
            masonry::theme::default_property_set(),
        )
    }));
    let outcome = match ran {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(GuiError::EventLoop(e.to_string())),
        Err(payload) => Err(error_for_panic(&panic_text(payload.as_ref()))),
    };
    if outcome.is_err() {
        console::regain_for_terminal(released);
    }
    outcome?;
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
