//! The OpenSSH client as the transport (R24 part 1): finding it and its
//! version, the hosts of the owner's configuration, the arguments it is
//! started with, and the process itself.

pub mod gcmd;
pub mod hosts;
pub mod launch;
pub mod locate;
pub mod options;
pub mod target;
pub mod version;

pub use gcmd::{HostKeyChecking, ResolveError, Resolved, resolve};
pub use hosts::{ConfigHost, ConfigPlaces, HostList, config_hosts};
pub use launch::{AskpassEnv, Launch, LaunchError, SshProcess};
pub use locate::{Found, LocateError, Origin, SearchPlaces, locate};
pub use target::{Target, TargetError};
pub use version::{Flavor, SshVersion};
