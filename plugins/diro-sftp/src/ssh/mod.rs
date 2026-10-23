//! The OpenSSH client as the transport (R24 part 1): finding it and its
//! version, the hosts of the owner's configuration, the arguments it is
//! started with, and the process itself.

pub mod locate;
pub mod version;

pub use locate::{Found, LocateError, Origin, SearchPlaces, locate};
pub use version::{Flavor, SshVersion};
