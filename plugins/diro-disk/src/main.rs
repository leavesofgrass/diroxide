//! `diro-disk`: the disk-image places plugin, which the host starts and
//! speaks to over standard input and output.

use std::io;
use std::process::ExitCode;

use diro_disk::plugin::Plugin;
use diro_disk::rpc;

fn main() -> ExitCode {
    let code = rpc::serve(io::stdin().lock(), Box::new(io::stdout()), Plugin::new());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
