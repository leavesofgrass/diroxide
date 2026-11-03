// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

//! Configures a suitable default [`tracing`] implementation for a Masonry application.
//!
//! This uses a custom log format specialised for GUI applications,
//! and, in debug mode, will write all logs to a file in the folder named by
//! the `MASONRY_DENSE_LOG_DIR` environment variable (a textweaver patch:
//! with the variable unset, no file is written).
//! This also uses a default filter, which can be overwritten using `RUST_LOG`.
//! This will include all [`DEBUG`](tracing::Level::DEBUG) messages in debug mode,
//! and all [`INFO`](tracing::Level::INFO) level messages in release mode.
//!
//! If a `tracing` backend is already configured, this will not overwrite that.

// TODO - Move this code out of masonry.

use std::error::Error;
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::fs::File;
#[cfg(not(target_arch = "wasm32"))]
use std::time::UNIX_EPOCH;

#[cfg(not(target_arch = "wasm32"))]
use time::macros::format_description;
use tracing::Subscriber;
#[cfg(not(target_arch = "wasm32"))]
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
#[cfg(not(target_arch = "wasm32"))]
use tracing_subscriber::fmt::time::UtcTime;
use tracing_subscriber::prelude::*;

/// The environment variable naming the folder for the full ("dense") log
/// of a debug build (a textweaver patch). Unset or empty, no file is
/// written.
pub const DENSE_LOG_DIR_VAR: &str = "MASONRY_DENSE_LOG_DIR";

#[cfg(not(target_arch = "wasm32"))]
/// The folder [`DENSE_LOG_DIR_VAR`] names, if it is set and not empty.
fn dense_log_dir() -> Option<std::path::PathBuf> {
    dense_log_dir_from(std::env::var_os(DENSE_LOG_DIR_VAR))
}

#[cfg(not(target_arch = "wasm32"))]
fn dense_log_dir_from(value: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    value
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(not(target_arch = "wasm32"))]
/// Get the tracing subscriber we wish to set-up for a non-web platform with the given `default_level`.
///
/// Returns the subscriber, and the error in case of a (recoverable) error.
fn default_tracing_subscriber_native(
    default_level: LevelFilter,
    span_times: Option<SpanTimes>,
) -> (impl Subscriber, Option<Box<dyn Error>>) {
    // Use EnvFilter to allow the user to override the log level without recompiling.
    let env_filter_builder = EnvFilter::builder()
        .with_default_directive(default_level.into())
        .with_env_var("RUST_LOG");
    let err = env_filter_builder
        .from_env()
        .err()
        .map(|err| format!("failed to parse RUST_LOG environment variable: {err:#}").into());
    let env_filter = env_filter_builder.from_env_lossy();

    // This format is more concise than even the 'Compact' default:
    // - We print the time without the date (GUI apps usually run for very short periods).
    // - We print the time with millisecond instead of microsecond precision.
    // - We skip the target. In app code, the target is almost always visual noise. By
    //   default, it only gives you the module a log was defined in. This is rarely useful;
    //   the log message is much more helpful for finding a log's location.
    let timer = UtcTime::new(format_description!(
        // We append a `Z` here to indicate clearly that this is a UTC time
        "[hour repr:24]:[minute]:[second].[subsecond digits:3]Z"
    ));
    // If modifying, also update the module level docs
    let console_layer = tracing_subscriber::fmt::layer()
        .with_timer(timer.clone())
        .with_target(false)
        .with_filter(env_filter);

    // We skip the layer which stores to a file in `--release` mode for performance.
    // textweaver patch: and in debug builds too, unless `MASONRY_DENSE_LOG_DIR`
    // names the folder to write it in. Upstream wrote it to the system's
    // temporary folder on every start and every test program, which filled
    // the system drive; now it goes only where it is asked for.
    let dense_log = if cfg!(debug_assertions) {
        dense_log_dir().and_then(|dir| {
            // TODO - Replace with a more targeted subscriber.
            // See https://github.com/linebender/xilem/issues/1556
            let id = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let path = dir.join(format!("masonry-{id:016}-dense.log"));
            match File::create(&path) {
                Ok(file) => Some((path, file)),
                Err(e) => {
                    #[allow(clippy::print_stderr, reason = "Tracing is not set up yet")]
                    {
                        eprintln!(
                            "Cannot write the full Masonry log to {}: {e}",
                            path.display()
                        );
                    }
                    None
                }
            }
        })
    } else {
        None
    };
    let log_file_layer = if let Some((tmp_path, file)) = dense_log {
        // If modifying, also update the module level docs
        let log_file_layer = tracing_subscriber::fmt::layer()
            .with_timer(timer)
            .with_writer(file)
            // TODO - For some reason, `.with_ansi(false)` still leaves some italics in the output.
            .with_ansi(false);
        // Note that this layer does not use the provided filter, and instead logs all events.

        #[allow(clippy::print_stderr, reason = "Can only use stderr")]
        {
            // We print this message to stderr (rather than through `tracing`), because:
            // 1) Tracing hasn't been set up yet
            // 2) The tracing logs could have been configured to eat this message, and we think this is still important to have visible.
            // 3) This message is only sent in debug mode, so won't be exposed to end-users.
            eprintln!("---");
            eprintln!("Writing full logs to {}", tmp_path.display());
            eprintln!("---");
        }

        Some(log_file_layer)
    } else {
        None
    };

    #[cfg(target_os = "android")]
    let android_trace_layer = tracing_android_trace::AndroidTraceLayer::new();

    // textweaver patch: the span timer, filtered to its one span name, so
    // it turns on no other span or event.
    let span_times_layer = span_times.map(|t| {
        let name = t.name;
        t.with_filter(tracing_subscriber::filter::filter_fn(move |m| {
            m.is_span() && m.name() == name
        }))
    });

    let registry = tracing_subscriber::registry()
        .with(console_layer)
        .with(log_file_layer)
        .with(span_times_layer);

    #[cfg(target_os = "android")]
    let registry = registry.with(android_trace_layer);

    // After the above line because of https://github.com/linebender/android_trace/pull/17
    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());

    (registry, err)
}

#[cfg(target_arch = "wasm32")]
/// Initialise tracing for the web with the given `max_level`.
fn default_tracing_subscriber_wasm(
    max_level: LevelFilter,
) -> (impl Subscriber, Option<Box<dyn Error>>) {
    // Note - tracing-wasm might not work in headless Node.js. Probably doesn't matter anyway,
    // because this is a GUI framework, so wasm targets will virtually always be browsers.

    // Ignored if the panic hook is already set
    console_error_panic_hook::set_once();

    let config = tracing_wasm::WASMLayerConfigBuilder::new()
        .set_max_level(
            max_level
                .into_level()
                .expect("for max_level to be > tracing::LevelFilter::OFF"),
        )
        .build();

    (
        tracing_subscriber::Registry::default().with(tracing_wasm::WASMLayer::new(config)),
        None,
    )
}

/// Constructs a default tracing subscriber with a given `max_level` filter.
pub fn default_tracing_subscriber(
    max_level: LevelFilter,
) -> (impl Subscriber, Option<Box<dyn Error>>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        default_tracing_subscriber_native(max_level, None)
    }

    #[cfg(target_arch = "wasm32")]
    {
        default_tracing_subscriber_wasm(max_level)
    }
}

/// An Error indicating that a tracing subscriber has been set before.
#[derive(Debug)]
pub struct TracingSubscriberHasBeenSetError;

impl fmt::Display for TracingSubscriberHasBeenSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad("A tracing subscriber has been set before.")
    }
}

impl Error for TracingSubscriberHasBeenSetError {}

/// Verify that a tracing subscriber has not been set before or return with an error.
fn verify_subscriber_has_not_been_set() -> Result<(), TracingSubscriberHasBeenSetError> {
    // The tracing_core::dispatcher::has_been_set function is doc(hidden).
    // However, it is guaranteed to remain for the entire tracing_core 1.0 series,
    // as tracing depends on it, and it isn't documented as unsupported.
    if tracing_core::dispatcher::has_been_set() {
        return Err(TracingSubscriberHasBeenSetError);
    }
    Ok(())
}

/// Initialise tracing with a default subscriber for a unit test.
/// This ignores most messages to limit noise (but will still log all messages to a file).
pub fn try_init_test_tracing() -> Result<(), TracingSubscriberHasBeenSetError> {
    // For unit tests we want to suppress most messages.
    let default_level = LevelFilter::WARN;

    verify_subscriber_has_not_been_set()?;

    let (subscriber, err) = default_tracing_subscriber(default_level);

    // We may ignore potential errors here because we already checked that no subscriber has been set.
    let _ = tracing::subscriber::set_global_default(subscriber);
    if let Some(err) = err {
        tracing::error!(err, "Logging init had recoverable error");
    }

    Ok(())
}

/// Initialise tracing with a default subscriber for an end-user application.
pub fn try_init_tracing() -> Result<(), TracingSubscriberHasBeenSetError> {
    init_app_tracing(None)
}

/// Called with how long a span lasted, from its creation to its close.
pub type SpanTimeReport = Box<dyn Fn(std::time::Duration) + Send + Sync>;

/// Like [`try_init_tracing`], and `report` is also called with the length
/// of every span named `name` when it closes (a textweaver patch: the
/// frame times its `--log` reports, from `masonry_winit`'s `redraw` span).
/// On the web, `report` is never called.
pub fn try_init_tracing_with_span_times(
    name: &'static str,
    report: SpanTimeReport,
) -> Result<(), TracingSubscriberHasBeenSetError> {
    init_app_tracing(Some(SpanTimes { name, report }))
}

fn init_app_tracing(span_times: Option<SpanTimes>) -> Result<(), TracingSubscriberHasBeenSetError> {
    // Default level is DEBUG in --dev, INFO in --release, unless a level is passed.
    // DEBUG should print a few logs per low-density event.
    // INFO should only print logs for noteworthy things.
    let default_level = if cfg!(debug_assertions) {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };

    verify_subscriber_has_not_been_set()?;

    #[cfg(not(target_arch = "wasm32"))]
    let (subscriber, err) = default_tracing_subscriber_native(default_level, span_times);
    #[cfg(target_arch = "wasm32")]
    let (subscriber, err) = {
        drop(span_times);
        default_tracing_subscriber(default_level)
    };

    // We may ignore potential errors here because we already checked that no subscriber has been set.
    let _ = tracing::subscriber::set_global_default(subscriber);
    if let Some(err) = err {
        tracing::error!("Initialising logging encountered recoverable error: {err}");
    }

    Ok(())
}

/// The layer that times the spans with one name (a textweaver patch).
struct SpanTimes {
    name: &'static str,
    report: SpanTimeReport,
}

/// When a timed span was created, kept in its extensions.
struct SpanStarted(std::time::Instant);

impl<S> tracing_subscriber::Layer<S> for SpanTimes
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if attrs.metadata().name() == self.name
            && let Some(span) = ctx.span(id)
        {
            span.extensions_mut()
                .insert(SpanStarted(std::time::Instant::now()));
        }
    }

    fn on_close(&self, id: tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = ctx.span(&id)
            && let Some(started) = span.extensions().get::<SpanStarted>()
        {
            (self.report)(started.0.elapsed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_init_test_tracing_errors() {
        let _first_result = try_init_test_tracing();
        let second_result = try_init_test_tracing();
        assert!(second_result.is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn span_times_report_only_their_span() {
        // textweaver patch: the frame times under `--log`.
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&seen);
        let layer = SpanTimes {
            name: "timed",
            report: Box::new(move |d| into.lock().unwrap().push(d)),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let a = tracing::info_span!("timed");
            std::thread::sleep(std::time::Duration::from_millis(2));
            drop(a);
            drop(tracing::info_span!("other"));
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0] >= std::time::Duration::from_millis(2));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dense_log_only_where_asked() {
        // textweaver patch: no folder, no file (never the temporary folder).
        assert_eq!(dense_log_dir_from(None), None);
        assert_eq!(dense_log_dir_from(Some("".into())), None);
        assert_eq!(
            dense_log_dir_from(Some("logs".into())),
            Some(std::path::PathBuf::from("logs"))
        );
    }
}
