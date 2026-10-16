//! `diro plugin`: list, new, enable, disable, run and log. W3b owns this
//! file. Phase 0 lists the plugins in the config folder; the rest say they
//! are not built yet.

use std::io::Write;

use diroxide_config::{Paths, PluginSettings};
use diroxide_plugins::Host;

use crate::CliError;

/// A plugin subcommand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginCommand {
    /// List the plugins found.
    List,
    /// Write a working skeleton of a new plugin.
    New {
        /// Its name.
        name: String,
        /// Its language: `python` or `bash` (Lua scripts run through the
        /// Lua runner plugin, like any other language).
        language: String,
    },
    /// Enable a plugin.
    Enable(String),
    /// Disable a plugin.
    Disable(String),
    /// Run a plugin's command.
    Run(String),
    /// Print a plugin's log.
    Log(String),
}

/// Reads the arguments after `plugin`.
pub fn parse(args: &[String]) -> Result<PluginCommand, CliError> {
    let missing = || {
        CliError::Missing(format!(
            "plugin {}",
            args.first().map_or("", String::as_str)
        ))
    };
    let name = || args.get(1).cloned().ok_or_else(missing);
    match args.first().map(String::as_str) {
        Some("list") => Ok(PluginCommand::List),
        Some("new") => {
            let language = match args.get(2..) {
                Some([flag, language]) if flag == "--language" => language.clone(),
                Some([]) => "python".to_owned(),
                _ => return Err(missing()),
            };
            Ok(PluginCommand::New {
                name: name()?,
                language,
            })
        }
        Some("enable") => Ok(PluginCommand::Enable(name()?)),
        Some("disable") => Ok(PluginCommand::Disable(name()?)),
        Some("run") => Ok(PluginCommand::Run(name()?)),
        Some("log") => Ok(PluginCommand::Log(name()?)),
        Some(other) => Err(CliError::UnknownOption(other.to_owned())),
        None => Err(CliError::Missing("plugin".to_owned())),
    }
}

/// Runs a plugin subcommand.
pub fn run(command: &PluginCommand, out: &mut dyn Write) -> Result<i32, CliError> {
    let failed = |e: std::io::Error| CliError::Failed(e.to_string());
    match command {
        PluginCommand::List => {
            let Some(paths) = Paths::from_environment() else {
                return Err(CliError::Failed(
                    "the config folder could not be found".to_owned(),
                ));
            };
            let mut host = Host::new(&PluginSettings::default());
            let problems = host.discover(&paths.plugins_dir());
            if host.plugins().is_empty() {
                writeln!(out, "No plugins in {}", paths.plugins_dir().display()).map_err(failed)?;
            }
            for info in host.plugins() {
                let state = if info.enabled {
                    "enabled"
                } else {
                    "not enabled"
                };
                writeln!(
                    out,
                    "{}, {state}, {}",
                    info.manifest.name, info.manifest.description
                )
                .map_err(failed)?;
            }
            for problem in problems {
                writeln!(out, "{problem}").map_err(failed)?;
            }
            Ok(0)
        }
        _ => Err(CliError::Failed(
            "this plugin command is not built yet".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn subcommands_are_read() {
        assert_eq!(parse(&args(&["list"])), Ok(PluginCommand::List));
        assert_eq!(
            parse(&args(&["new", "thumbs", "--language", "bash"])),
            Ok(PluginCommand::New {
                name: "thumbs".into(),
                language: "bash".into()
            })
        );
        assert_eq!(
            parse(&args(&["new", "thumbs"])),
            Ok(PluginCommand::New {
                name: "thumbs".into(),
                language: "python".into()
            })
        );
        assert_eq!(
            parse(&args(&["enable"])),
            Err(CliError::Missing("plugin enable".into()))
        );
        assert_eq!(
            parse(&args(&["dance"])),
            Err(CliError::UnknownOption("dance".into()))
        );
    }
}
