//! The real terminal: raw mode on the alternate screen (or the inline
//! viewport), restored on every way out the program controls, and the
//! loop that feeds the session.
//!
//! - **Entering.** Raw mode, the alternate screen (not with `--inline`),
//!   mouse capture off, bracketed paste on Unix (on Windows the console
//!   reads keys as key records, where the paste markers would arrive as
//!   typed characters). The keyboard enhancement protocol is asked about
//!   once and the answer goes to the log, never to the screen; diroxide
//!   never turns it on and never relies on it (R4 section 4.3). crossterm
//!   reads the answer itself within two seconds, so the reply never shows
//!   as typed characters.
//! - **Leaving.** On a normal quit, on an error, and on a panic (a panic
//!   hook restores the screen before the message prints) the terminal gets
//!   raw mode off, the main screen back and the cursor shown. A terminate
//!   signal, Ctrl+Break and the window closing need a signal handler,
//!   which waits for a contract change (see [`restore`]).
//! - **Keys.** A reader thread waits for the terminal's events with a
//!   short timeout and passes them on; the runtime's waker wakes the same
//!   loop when a worker has a result. While a program has the terminal
//!   (the editor, a shell), the reader holds off, so the program gets
//!   every key.
//! - **Waiting.** Every wait has a limit: the reader polls in steps of 20
//!   milliseconds, the loop sleeps until its next timer or a second, and
//!   at quit the workers get five seconds to finish their cancellation.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use crossterm::{cursor, execute, terminal};
use diroxide_app::{App, Handover, Runtime};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::session::Session;
use crate::{TuiError, TuiOptions};

/// Whether diroxide has the terminal now: what [`restore`] undoes.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Whether the alternate screen is in use (not with `--inline`).
static ALTERNATE: AtomicBool = AtomicBool::new(false);
/// Whether the panic hook is installed.
static HOOKED: AtomicBool = AtomicBool::new(false);

/// How long the reader waits for an event before it looks again.
const READ_STEP: Duration = Duration::from_millis(20);
/// How long the workers get at quit to finish what they cancel.
const QUIT_WAIT: Duration = Duration::from_secs(5);
/// The inline viewport's height at most.
const INLINE_ROWS: u16 = 24;

fn io_error(error: io::Error) -> TuiError {
    TuiError::Io(error)
}

/// Gives the terminal back: raw mode off, the main screen back, the cursor
/// shown, bracketed paste off. Safe to call more than once and from any
/// thread; it does nothing when diroxide does not have the terminal.
///
/// A signal handler for the terminate signal, Ctrl+Break and the window
/// closing would call this; the workspace has no signal crate yet (the
/// report's contract change request names `ctrlc` with its termination
/// feature).
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    if !cfg!(windows) {
        let _ = execute!(out, event::DisableBracketedPaste);
    }
    if ALTERNATE.load(Ordering::SeqCst) {
        let _ = execute!(out, terminal::LeaveAlternateScreen);
    }
    let _ = execute!(out, cursor::Show);
    let _ = terminal::disable_raw_mode();
    let _ = out.flush();
}

/// Takes the terminal: raw mode, the alternate screen unless inline, the
/// cursor shown.
fn take(inline: bool) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    ACTIVE.store(true, Ordering::SeqCst);
    ALTERNATE.store(!inline, Ordering::SeqCst);
    let mut out = io::stdout();
    if !inline {
        execute!(out, terminal::EnterAlternateScreen)?;
    }
    if !cfg!(windows) {
        execute!(out, event::EnableBracketedPaste)?;
    }
    execute!(out, event::DisableMouseCapture, cursor::Show)?;
    Ok(())
}

/// The thread that draws: a panic there gives the terminal back; a panic
/// on a worker leaves the screen to the loop.
static DRAWER: Mutex<Option<thread::ThreadId>> = Mutex::new(None);

fn install_panic_hook() {
    *DRAWER.lock().unwrap_or_else(PoisonError::into_inner) = Some(thread::current().id());
    if HOOKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let drawer = *DRAWER.lock().unwrap_or_else(PoisonError::into_inner);
        if drawer.is_none_or(|id| id == thread::current().id()) {
            restore();
        }
        previous(info);
    }));
}

/// Gives the terminal back when dropped, whatever way the loop ends.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// What the loop waits for.
enum Incoming {
    /// The terminal sent an event.
    Event(Event),
    /// A worker has a result.
    Wake,
    /// The terminal cannot be read any more, in words.
    Closed(String),
}

/// Holds the reader off while a program has the terminal.
type Gate = Arc<Mutex<bool>>;

/// The reader thread: waits for terminal events in short steps and sends
/// them on, unless the gate is shut.
fn spawn_reader(sender: Sender<Incoming>, gate: Gate, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let shut = gate.lock().unwrap_or_else(PoisonError::into_inner);
            if *shut {
                drop(shut);
                thread::sleep(READ_STEP);
                continue;
            }
            match event::poll(READ_STEP) {
                Ok(false) => {}
                Ok(true) => match event::read() {
                    Ok(found) => {
                        drop(shut);
                        if sender.send(Incoming::Event(found)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Incoming::Closed(error.to_string()));
                        return;
                    }
                },
                Err(error) => {
                    let _ = sender.send(Incoming::Closed(error.to_string()));
                    return;
                }
            }
        }
    })
}

/// The log's lines about the terminal: what the keyboard enhancement
/// query found. Written to the state folder, never shown.
fn probe_lines(inline: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "Terminal: {}, raw mode, mouse capture off.",
        if inline {
            "inline viewport"
        } else {
            "alternate screen"
        }
    )];
    let answer = match terminal::supports_keyboard_enhancement() {
        Ok(true) => "the terminal can report keys exactly; diroxide does not turn it on".to_owned(),
        Ok(false) if cfg!(windows) => {
            "not asked on Windows, where keys arrive as key records".to_owned()
        }
        Ok(false) => "the terminal answered no".to_owned(),
        Err(error) => format!("no answer within two seconds ({error})"),
    };
    lines.push(format!("Keyboard enhancement: {answer}."));
    lines.push(format!(
        "Color: {:?}.",
        diroxide_theme::ColorSupport::detect()
    ));
    lines
}

fn write_log(path: Option<&PathBuf>, lines: &[String]) {
    let Some(path) = path else {
        return;
    };
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let _ = std::fs::write(path, format!("{}\n", lines.join("\n")));
}

/// Runs the app on the real terminal until it quits.
pub(crate) fn run(
    app: &mut App,
    runtime: &mut Runtime,
    options: TuiOptions,
    themes_dir: Option<PathBuf>,
    log: Option<PathBuf>,
) -> Result<(), TuiError> {
    install_panic_hook();
    let inline = options.inline;
    // The query reads the terminal's answer in raw mode itself; it runs
    // before the reader thread starts, so nothing else reads the reply.
    let probe = probe_lines(inline);
    write_log(log.as_ref(), &probe);
    // The guard first: a failure halfway through taking the terminal
    // still gives it back.
    let guard = Guard;
    take(inline).map_err(io_error)?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = if inline {
        let rows = terminal::size().map(|(_, h)| h).unwrap_or(INLINE_ROWS);
        Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(rows.clamp(5, INLINE_ROWS)),
            },
        )
    } else {
        Terminal::new(backend)
    }
    .map_err(io_error)?;

    let (sender, receiver): (Sender<Incoming>, Receiver<Incoming>) = mpsc::channel();
    let gate: Gate = Arc::new(Mutex::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let waker_sender = Mutex::new(sender.clone());
    runtime.set_waker(Arc::new(move || {
        let sender = waker_sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let _ = sender.send(Incoming::Wake);
    }));
    let reader = spawn_reader(sender, Arc::clone(&gate), Arc::clone(&stop));

    let mut session = Session::new(app, runtime, terminal, options)
        .with_themes_dir(themes_dir)
        .with_title_sink(Box::new(|title| {
            let _ = execute!(io::stdout(), terminal::SetTitle(title));
        }));
    let resumed = session.resumed_flag();
    let hook_gate = Arc::clone(&gate);
    session_runtime_hook(&mut session, hook_gate, resumed, inline);

    let result = event_loop(&mut session, &receiver);
    stop.store(true, Ordering::SeqCst);
    // The reader looks at the stop flag every 20 milliseconds.
    let _ = reader.join();
    // Jobs cancelled at quit get a few seconds to leave no half-written
    // file behind.
    session.settle(Instant::now(), QUIT_WAIT);
    drop(session);
    drop(guard);
    if inline {
        println!();
    }
    result
}

/// Sets the runtime's terminal hook: before a program takes the terminal,
/// the reader holds off and the screen is given back; after, the screen is
/// taken again and drawn in full.
fn session_runtime_hook<B: ratatui::backend::Backend>(
    session: &mut Session<'_, B>,
    gate: Gate,
    resumed: Arc<AtomicBool>,
    inline: bool,
) {
    session.set_terminal_hook(Box::new(move |moment| match moment {
        Handover::Suspend => {
            *gate.lock().unwrap_or_else(PoisonError::into_inner) = true;
            restore();
        }
        Handover::Resume => {
            let _ = take(inline);
            resumed.store(true, Ordering::SeqCst);
            *gate.lock().unwrap_or_else(PoisonError::into_inner) = false;
        }
    }));
}

fn event_loop<B: ratatui::backend::Backend>(
    session: &mut Session<'_, B>,
    receiver: &Receiver<Incoming>,
) -> Result<(), TuiError> {
    session.start(Instant::now());
    loop {
        let now = Instant::now();
        session.wake(now);
        session.tick(now);
        session.draw(now)?;
        if session.wants_quit() {
            return Ok(());
        }
        let wait = session
            .next_deadline()
            .map_or(Duration::from_secs(1), |at| {
                at.saturating_duration_since(now)
            })
            .min(Duration::from_secs(1));
        match receiver.recv_timeout(wait) {
            Ok(Incoming::Event(found)) => {
                let now = Instant::now();
                match found {
                    Event::Key(key) => session.key_event(&key, now),
                    Event::Paste(text) => session.paste(&text, now),
                    Event::Resize(_, _) => session.resize(now),
                    Event::FocusGained | Event::FocusLost | Event::Mouse(_) => {}
                }
            }
            Ok(Incoming::Wake) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Incoming::Closed(why)) => {
                return Err(TuiError::Io(io::Error::other(format!(
                    "the terminal could not be read: {why}"
                ))));
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}
