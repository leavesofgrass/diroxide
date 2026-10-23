//! `diro-sftp`: the SFTP places plugin, or, when ssh starts it with
//! `DIRO_ASKPASS_ENDPOINT` set, its askpass helper (answer 82).

use std::env;
use std::io;
use std::process::ExitCode;

use diro_sftp::askpass::{self, Mode};
use diro_sftp::plugin::Plugin;
use diro_sftp::rpc;

/// ssh's prompt as bytes: exactly as given on Unix; on Windows, where
/// arguments are text, as UTF-8.
fn prompt_bytes(arg: std::ffi::OsString) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        arg.into_vec()
    }
    #[cfg(not(unix))]
    {
        arg.to_string_lossy().into_owned().into_bytes()
    }
}

fn main() -> ExitCode {
    let code = match askpass::mode(env::var_os(askpass::ENDPOINT_VARIABLE).as_deref()) {
        Mode::Helper => {
            let prompt = env::args_os().nth(1).map(prompt_bytes);
            askpass::helper::run(prompt.as_deref())
        }
        Mode::Plugin => rpc::serve(io::stdin().lock(), Box::new(io::stdout()), Plugin::new()),
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
