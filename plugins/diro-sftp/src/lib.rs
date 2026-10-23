//! The SFTP places plugin: places such as `sftp://user@host:22/path`,
//! reached through the OpenSSH client already on the machine (answer 78),
//! so the owner's agent, key files, known hosts and ssh configuration
//! work as they are and no key, host key or cipher is ever diroxide's.
//!
//! One program, two modes (answer 82): the JSON-RPC plugin the host starts
//! ([`rpc`], [`plugin`]), and the askpass helper ssh starts when it needs
//! a password or a yes for a new host key ([`askpass`]).
//!
//! The rules this crate keeps, and its tests check:
//!
//! - It never writes known hosts, never accepts a changed host key, and
//!   never accepts a new one without the owner's typed yes.
//! - It never puts a password on a command line, in an environment
//!   variable meant for ssh, in a file or in its log.
//! - Every argument to ssh is one element of an array, with `--` before the
//!   host; a host that starts with a dash or holds odd characters is
//!   refused in words. ssh never runs through a shell.
//! - Every line it says carries its meaning in the first 40 cells; the
//!   host adds the plugin's name last.
//!
//! The crate lives in a workspace of its own under `plugins/` (plan
//! section 5.8, item 3); it is pure Rust, with no C build.

pub mod askpass;
pub mod lines;
pub mod plugin;
pub mod rpc;
pub mod settings;
pub mod ssh;

/// The default SFTP port.
pub const DEFAULT_PORT: u16 = 22;
