//! Subcommands that print and return: `ls`, `completions`, search and
//! plugin tooling. W1e owns this folder; W3b owns `plugin.rs`; W1d adds
//! `keys.rs` and `init.rs`; W2g adds `archive.rs`, `split.rs` and
//! `join.rs`; W3d adds `diff.rs`.
//! `join.rs`; W3s owns `search.rs`; W3t owns `watch.rs`; W4c owns
//! `components.rs`; W4b owns `plugin.rs` and `run.rs`. Alpha 4's
//! contract step (P4) adds the route stubs `doctor.rs` (the registry's
//! lane), `convert.rs` (the conversions lane), `info.rs` and `sum.rs`
//! (the metadata-in-the-app lane) and `watch_task.rs` (the watched-folders
//! lane), each built by its lane in alpha 4.

pub mod archive;
pub mod completions;
pub mod components;
pub mod convert;
pub mod diff;
pub mod doctor;
pub mod dups;
pub mod info;
pub mod init;
pub mod join;
pub mod keys;
pub mod ls;
pub mod plugin;
pub mod rename;
pub mod run;
pub mod search;
pub mod split;
pub mod stdin_list;
pub mod sum;
pub mod sync;
pub mod watch;
pub mod watch_task;
