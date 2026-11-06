//! The job engine's door to the tools (alpha.7, R30 commit 19): the
//! registry implements `diroxide-vfs`'s [`ToolRunner`], so a conversion,
//! the hot-folder runner, a signature check or the 7z reader runs a tool
//! by its id through the one runner ([`run_limited`]): found in the
//! lookup's order, held to the request's limits, stopped with every child
//! on the context's cancel, and every failure said in words for the house
//! style's "Could not VERB OBJECT: CAUSE.".
//!
//! The app and the command line put the registry in the context they
//! make: `OpContext::new().with_tools(Arc::new(registry))`.

use std::fs::File;
use std::io::Cursor;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use diroxide_core::format::human_size;
use diroxide_vfs::{
    CancelToken, Split, ToolInput, ToolOutput, ToolRequest, ToolRunError, ToolRunner, ToolStream,
};

use crate::run::{Caps, Cut, Emit, Finished, Limited, RunFailure, Streaming, run_limited};
use crate::{Location, ProcessSpec, ToolRegistry, system_words};

impl ToolRunner for ToolRegistry {
    fn has(&self, tool: &str) -> bool {
        ToolRegistry::has(self, tool)
    }

    fn name_of(&self, tool: &str) -> String {
        ToolRegistry::name_of(self, tool)
    }

    fn run(&self, request: &ToolRequest, cancel: &CancelToken) -> Result<ToolOutput, ToolRunError> {
        let name = ToolRegistry::name_of(self, &request.tool);
        let (spec, limited) = self.prepare(request, &name)?;
        let done = run_limited(spec, limited, &|| cancel.is_cancelled())
            .map_err(|failure| run_error(&name, failure))?;
        finished(&name, request, done)
    }

    fn run_streaming(
        &self,
        request: &ToolRequest,
        cancel: &CancelToken,
        record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
    ) -> Result<ToolOutput, ToolRunError> {
        let name = ToolRegistry::name_of(self, &request.tool);
        let (spec, limited) = self.prepare(request, &name)?;
        // The pumps hand records over on their own threads; the caller's
        // `record` runs here, on this one, while the program runs on a
        // thread of the scope.
        let (send, receive) = mpsc::channel::<(ToolStream, Vec<u8>)>();
        let emit: Emit = Arc::new(move |stream, bytes| {
            let _ = send.send((stream, bytes.to_vec()));
        });
        let limited = limited.streaming(Streaming {
            stdout: true,
            stderr: true,
            cut: cut_of(request.split),
            emit,
        });
        let stopped = AtomicBool::new(false);
        let outcome = thread::scope(|scope| {
            let running = scope.spawn(|| {
                run_limited(spec, limited, &|| {
                    cancel.is_cancelled() || stopped.load(Ordering::SeqCst)
                })
            });
            // The channel ends when the run has ended and its pumps with
            // it; records after a Break are read and passed over.
            for (stream, bytes) in receive {
                if stopped.load(Ordering::SeqCst) {
                    continue;
                }
                if record(stream, &record_text(request.split, &bytes)).is_break() {
                    stopped.store(true, Ordering::SeqCst);
                }
            }
            running.join()
        });
        let outcome = outcome.map_err(|_| ToolRunError::NotStarted {
            name: name.clone(),
            message: "its runner stopped unexpectedly".to_owned(),
        })?;
        if stopped.load(Ordering::SeqCst) && !cancel.is_cancelled() {
            // The caller asked it to stop: not a failure.
            return Ok(ToolOutput::new(None, Vec::new(), Vec::new()));
        }
        let done = outcome.map_err(|failure| run_error(&name, failure))?;
        finished(&name, request, done)
    }
}

impl ToolRegistry {
    /// The program of a request, found in the lookup's order, and its
    /// limits and input.
    fn prepare(
        &self,
        request: &ToolRequest,
        name: &str,
    ) -> Result<(ProcessSpec, Limited), ToolRunError> {
        let path = match self.locate(&request.tool) {
            Ok(Location::Found { path, .. }) => path,
            Ok(Location::BuiltIn) => {
                return Err(ToolRunError::NotStarted {
                    name: name.to_owned(),
                    message: "it is built into diro and runs inside it".to_owned(),
                });
            }
            Ok(Location::NotFound | Location::PathMissing | Location::TurnedOff) | Err(_) => {
                return Err(ToolRunError::NotInstalled {
                    tool: request.tool.clone(),
                    name: name.to_owned(),
                });
            }
        };
        let mut spec = ProcessSpec::new(path)
            .args(request.args.iter().cloned())
            .diroxide_environment();
        if let Some(folder) = &request.folder {
            spec = spec.cwd(folder);
        }
        let mut limited = Limited::new(
            Some(request.limits.timeout),
            Caps::shared(request.limits.max_output_bytes as u64),
        );
        match &request.input {
            ToolInput::Closed => {}
            ToolInput::Bytes(bytes) => {
                limited = limited.with_input(Box::new(Cursor::new(bytes.clone())));
            }
            ToolInput::File(path) => {
                let file = File::open(path).map_err(|e| ToolRunError::NotStarted {
                    name: name.to_owned(),
                    message: format!(
                        "its input could not be read: {}",
                        lower_first(&e.to_string())
                    ),
                })?;
                limited = limited.with_input(Box::new(file));
            }
        }
        Ok((spec, limited))
    }
}

/// A finished run as the trait gives it back: a run stopped at its cap is
/// an error that names the cap.
fn finished(name: &str, request: &ToolRequest, done: Finished) -> Result<ToolOutput, ToolRunError> {
    if done.stopped_at_cap {
        return Err(ToolRunError::TooMuchOutput {
            name: name.to_owned(),
            limit: human_size(request.limits.max_output_bytes as u64),
        });
    }
    Ok(ToolOutput::new(done.status, done.stdout, done.stderr))
}

/// A run's failure in the trait's words.
fn run_error(name: &str, failure: RunFailure) -> ToolRunError {
    match failure {
        RunFailure::Start(error) => ToolRunError::NotStarted {
            name: name.to_owned(),
            message: lower_first(&system_words(&error)),
        },
        RunFailure::Cancelled => ToolRunError::Cancelled,
        RunFailure::TimedOut(limit) => ToolRunError::TimedOut {
            name: name.to_owned(),
            seconds: limit.as_secs(),
        },
    }
}

/// Where the trait's split cuts a record.
fn cut_of(split: Split) -> Cut {
    match split {
        Split::Lines => Cut::Byte(b'\n'),
        Split::CarriageReturn => Cut::CarriageReturn,
    }
}

/// A record's text: lossy UTF-8, and for lines without the carriage
/// return before the cut, as `Split::records` does.
fn record_text(split: Split, bytes: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if split == Split::Lines && text.ends_with('\r') {
        text.pop();
    }
    text
}

/// The system's words with their first letter in lowercase, for after a
/// colon.
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    use diroxide_vfs::{OpContext, RunLimits};

    use super::*;

    /// This test binary run again as a fake pandoc in a mode given among
    /// its arguments; an ordinary test run does nothing here.
    #[test]
    fn helper() {
        let Some(mode) = env::args().find_map(|a| a.strip_prefix("mode=").map(str::to_owned))
        else {
            return;
        };
        let mut out = std::io::stdout();
        let mut err = std::io::stderr();
        match mode.as_str() {
            "echo" => {
                for arg in env::args().filter(|a| a.starts_with("arg=")) {
                    writeln!(out, "{arg}").unwrap();
                }
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text).unwrap();
                writeln!(out, "read:{}", text.trim()).unwrap();
                writeln!(err, "a warning").unwrap();
            }
            "lines" => {
                out.write_all(b"one\r\ntwo\nthree").unwrap();
            }
            "progress" => {
                err.write_all(
                    b"Receiving objects:  50% (1/2)\rReceiving objects: 100% (2/2)\r\ndone.\n",
                )
                .unwrap();
            }
            "big" => {
                let line = vec![b'x'; 1024 * 1024];
                for _ in 0..100 {
                    if out.write_all(&line).is_err() {
                        break;
                    }
                }
            }
            "forever" => {
                writeln!(out, "started").unwrap();
                out.flush().unwrap();
                thread::sleep(Duration::from_secs(600));
            }
            "fail" => {
                out.flush().unwrap();
                std::process::exit(3);
            }
            _ => {}
        }
        out.flush().unwrap();
        err.flush().unwrap();
        std::process::exit(0);
    }

    /// A folder with this test binary as `pandoc`, and a registry that
    /// looks only there.
    fn fake_pandoc() -> (tempfile::TempDir, ToolRegistry) {
        let folder = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "pandoc.exe"
        } else {
            "pandoc"
        };
        std::fs::copy(env::current_exe().unwrap(), folder.path().join(name)).unwrap();
        let tools = ToolRegistry::builtin().with_path(Some(folder.path().as_os_str().to_owned()));
        (folder, tools)
    }

    fn request(mode: &str, extra: &[&str]) -> ToolRequest {
        let mut args = vec![
            "--exact".to_owned(),
            "runner::tests::helper".to_owned(),
            "--nocapture".to_owned(),
            "--test-threads".to_owned(),
            "1".to_owned(),
            format!("mode={mode}"),
        ];
        args.extend(extra.iter().map(|a| (*a).to_owned()));
        ToolRequest::new("pandoc", args)
    }

    /// Through the context, as the job engine calls it: the arguments
    /// whole, the input given, both streams and the exit code back.
    #[test]
    fn the_context_runs_a_tool_by_its_id() {
        let (_folder, tools) = fake_pandoc();
        let ctx = OpContext::new().with_tools(Arc::new(tools));
        assert!(ctx.tools.has("pandoc"));
        assert_eq!(ctx.tools.name_of("pandoc"), "pandoc");
        assert!(ctx.tools.has("carta"));
        let done = ctx
            .tools
            .run(
                &request("echo", &["arg=two words", "arg=$HOME"])
                    .with_input(ToolInput::Bytes(b"hello".to_vec())),
                &ctx.cancel,
            )
            .unwrap();
        assert!(done.succeeded());
        let text = String::from_utf8_lossy(&done.stdout);
        assert!(text.contains("arg=two words"), "{text}");
        assert!(text.contains("arg=$HOME"), "{text}");
        assert!(text.contains("read:hello"), "{text}");
        assert!(String::from_utf8_lossy(&done.stderr).contains("a warning"));
        let failed = ctx.tools.run(&request("fail", &[]), &ctx.cancel).unwrap();
        assert_eq!(failed.status, Some(3));
    }

    /// Each failure in words, the program's name first.
    #[test]
    fn failures_say_the_tool_and_the_limit() {
        let (_folder, registry) = fake_pandoc();
        let tools: &dyn ToolRunner = &registry;
        let cancel = CancelToken::new();
        let missing = tools
            .run(&ToolRequest::new("magick", ["a.png"]), &cancel)
            .unwrap_err();
        assert_eq!(missing.to_string(), "ImageMagick is not installed");
        let started = Instant::now();
        let timed = tools
            .run(
                &request("forever", &[])
                    .with_limits(RunLimits::new(Duration::from_secs(2), 1 << 20)),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(
            timed.to_string(),
            "pandoc took longer than 2 seconds and was stopped"
        );
        assert!(started.elapsed() < Duration::from_secs(20));
        let capped = tools
            .run(
                &request("big", &[])
                    .with_limits(RunLimits::new(Duration::from_secs(60), 2_000_000)),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(
            capped.to_string(),
            "pandoc wrote more than 2.0 MB and was stopped"
        );
        let built_in = tools
            .run(&ToolRequest::new("carta", ["a.org"]), &cancel)
            .unwrap_err();
        assert_eq!(
            built_in.to_string(),
            "carta could not be started: it is built into diro and runs inside it"
        );
        for line in [missing.to_string(), timed.to_string(), capped.to_string()] {
            assert!(
                diroxide_core::format::within_cells(&line, line.split(' ').next().unwrap()),
                "{line}"
            );
        }
    }

    #[test]
    fn the_cancel_stops_the_program_while_it_runs() {
        let (_folder, tools) = fake_pandoc();
        let cancel = CancelToken::new();
        let raiser = {
            let cancel = cancel.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(500));
                cancel.cancel();
            })
        };
        let error = ToolRunner::run(&tools, &request("forever", &[]), &cancel).unwrap_err();
        raiser.join().unwrap();
        assert_eq!(error, ToolRunError::Cancelled);
    }

    /// Records arrive as they are made: lines lose the carriage return
    /// before their cut and keep an unfinished last line; `cr` gives each
    /// rewrite of a progress line.
    #[test]
    fn records_stream_cut_by_the_split() {
        let (_folder, tools) = fake_pandoc();
        let cancel = CancelToken::new();
        let mut lines = Vec::new();
        let done = tools
            .run_streaming(&request("lines", &[]), &cancel, &mut |stream, text| {
                if stream == ToolStream::Stdout && !text.starts_with("running ") && !text.is_empty()
                {
                    lines.push(text.to_owned());
                }
                ControlFlow::Continue(())
            })
            .unwrap();
        assert!(done.stdout.is_empty());
        let ours: Vec<&str> = lines
            .iter()
            .map(String::as_str)
            .filter(|l| ["one", "two", "three"].iter().any(|w| l.ends_with(w)))
            .collect();
        assert_eq!(ours.len(), 3, "{lines:?}");
        assert!(lines.iter().all(|l| !l.ends_with('\r')), "{lines:?}");
        let mut progress = Vec::new();
        tools
            .run_streaming(
                &request("progress", &[]).split_by(Split::CarriageReturn),
                &cancel,
                &mut |stream, text| {
                    if stream == ToolStream::Stderr {
                        progress.push(text.to_owned());
                    }
                    ControlFlow::Continue(())
                },
            )
            .unwrap();
        assert_eq!(
            progress,
            [
                "Receiving objects:  50% (1/2)",
                "Receiving objects: 100% (2/2)",
                "done."
            ]
        );
    }

    /// A Break from the caller stops the program at once, and is not a
    /// failure.
    #[test]
    fn a_break_stops_the_program() {
        let (_folder, tools) = fake_pandoc();
        let started = Instant::now();
        let done = tools
            .run_streaming(
                &request("forever", &[]),
                &CancelToken::new(),
                &mut |_, text| {
                    if text.ends_with("started") {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                },
            )
            .unwrap();
        assert_eq!(done.status, None);
        assert!(started.elapsed() < Duration::from_secs(30));
    }
}
