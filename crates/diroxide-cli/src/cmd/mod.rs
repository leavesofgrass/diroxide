//! Subcommands that print and return: `ls`, `completions`, search and
//! plugin tooling. W1e owns this folder; W3b owns `plugin.rs`; W1d adds
//! `keys.rs` and `init.rs`.

pub mod completions;
pub mod init;
pub mod keys;
pub mod ls;
pub mod plugin;
pub mod search;
