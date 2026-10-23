//! The SFTP protocol layer, ours (R24 part 2; answer 88): the packet
//! codec, the request multiplexer and the session, on threads, with no
//! async runtime, over any pair of byte streams (ssh's pipes, or the fake
//! server's in tests).

pub mod codec;
pub mod errors;
pub mod fake;
pub mod fuzzing;
pub mod mux;
pub mod session;

pub use codec::{Attrs, CodecError, NameEntry, Request, Response};
pub use mux::{Mux, SftpError};
pub use session::{Session, SessionConfig};
