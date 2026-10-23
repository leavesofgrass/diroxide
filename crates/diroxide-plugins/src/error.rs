//! Errors of the plugin system.

use diroxide_core::Explain;
use thiserror::Error;

/// Why a plugin could not be used. The text names the plugin first.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PluginError {
    /// A message was not valid JSON-RPC 2.0.
    #[error("the message is not valid JSON-RPC: {0}")]
    Protocol(String),
    /// A manifest could not be read.
    #[error("{plugin}'s plugin.toml could not be read: {message}")]
    Manifest {
        /// The plugin's folder or name.
        plugin: String,
        /// What is wrong.
        message: String,
    },
    /// The plugin's program could not be started.
    #[error("{plugin} could not be started: {message}")]
    Start {
        /// The plugin.
        plugin: String,
        /// The system's words.
        message: String,
    },
    /// A size, queue or rate limit was reached; the text says which.
    #[error("{0}")]
    Limit(String),
    /// The plugin did not answer in time.
    #[error("{0} did not answer in time")]
    Timeout(String),
    /// The plugin's program ended.
    #[error("{0} stopped running")]
    Stopped(String),
    /// The plugin answered with an error.
    #[error("{plugin} answered with error {code}: {message}")]
    Remote {
        /// The plugin.
        plugin: String,
        /// The JSON-RPC error code.
        code: i64,
        /// The plugin's message.
        message: String,
    },
    /// No plugin has this name.
    #[error("no plugin is called {0}")]
    Unknown(String),
    /// The plugin is not enabled.
    #[error("{0} is not enabled")]
    NotEnabled(String),
    /// The plugin, or every plugin, is off for this session; the text is
    /// the sentence ("Line counter is off: it stopped 3 times.").
    #[error("{0}")]
    Off(String),
    /// A script command's placeholders could not be filled.
    #[error("the placeholder {0} has nothing to fill it")]
    Placeholder(String),
    /// Reading or writing failed.
    #[error("{0}")]
    Io(String),
    /// This part of the plugin system is not built yet.
    #[error("{0} is not built yet")]
    NotBuilt(String),
}

impl Explain for PluginError {
    fn next_step(&self) -> String {
        match self {
            PluginError::Timeout(_) | PluginError::Stopped(_) => {
                "Read the plugin's log with diro plugin log.".to_owned()
            }
            PluginError::NotEnabled(_) | PluginError::Off(_) => {
                "Enable it in the Plugins menu.".to_owned()
            }
            PluginError::Manifest { .. } => "Fix plugin.toml, or disable the plugin.".to_owned(),
            PluginError::Placeholder(_) => "Select something, or move to an entry.".to_owned(),
            _ => String::new(),
        }
    }
}
