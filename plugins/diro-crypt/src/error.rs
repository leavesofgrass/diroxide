//! What can stop a whole command, as the host hears it. A file that
//! cannot be encrypted or decrypted is not an error here: the command
//! says its cause and goes on with the rest ([`crate::commands`]).

use std::fmt;

use diroxide_plugins::protocol::{ErrorKind, RpcError};

/// A failure of a whole command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CryptError {
    /// The owner cancelled, or the host cancelled the request.
    Cancelled,
    /// A call to the host failed.
    Host(RpcError),
    /// The request could not be read.
    Bad(String),
}

impl CryptError {
    /// The error on the wire.
    pub fn to_rpc(&self) -> RpcError {
        match self {
            CryptError::Cancelled => {
                RpcError::new(ErrorKind::RequestCancelled, "the request was cancelled")
            }
            CryptError::Host(error) => error.clone(),
            CryptError::Bad(why) => RpcError::new(ErrorKind::InvalidParams, why.clone()),
        }
    }
}

impl fmt::Display for CryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rpc().message)
    }
}

impl std::error::Error for CryptError {}

impl From<RpcError> for CryptError {
    fn from(error: RpcError) -> Self {
        if error.code == ErrorKind::RequestCancelled.code() {
            CryptError::Cancelled
        } else {
            CryptError::Host(error)
        }
    }
}
