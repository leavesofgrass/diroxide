//! `diro-sftp`: the SFTP places plugin, or, when ssh starts it with
//! `DIRO_ASKPASS_ENDPOINT` set, its askpass helper (answer 82).

use std::env;
use std::io;
use std::process::ExitCode;

use diro_sftp::askpass::{self, Mode};
use diro_sftp::plugin::Plugin;
use diro_sftp::rpc;

fn main() -> ExitCode {
    let code = match askpass::mode(env::var_os(askpass::ENDPOINT_VARIABLE).as_deref()) {
        Mode::Helper => {
            let prompt = env::args_os().nth(1);
            askpass::helper::run(prompt.as_deref().and_then(|p| p.to_str()))
        }
        Mode::Plugin => rpc::serve(io::stdin().lock(), Box::new(io::stdout()), Plugin::new()),
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
