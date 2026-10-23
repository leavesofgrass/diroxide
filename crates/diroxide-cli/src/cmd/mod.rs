//! Subcommands that print and return: `ls`, `completions`, search and
//! plugin tooling. W1e owns this folder; W3b owns `plugin.rs`; W1d adds
//! `keys.rs` and `init.rs`; W2g adds `archive.rs`, `split.rs` and
//! `join.rs`; W3d adds `diff.rs`.
//! `join.rs`; W3s owns `search.rs`; W3t owns `watch.rs`; W4c owns
//! `components.rs`.

pub mod archive;
pub mod completions;
pub mod components;
pub mod diff;
pub mod init;
pub mod join;
pub mod keys;
pub mod ls;
pub mod plugin;
pub mod search;
pub mod split;
pub mod watch;
