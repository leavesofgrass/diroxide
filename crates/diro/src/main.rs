//! `diro`: Directory Oxide, a two-panel file manager. Everything happens
//! in `diroxide-cli`; this is the thin binary that `cargo install diro`
//! installs.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect();
    let code = diroxide_cli::main_with(
        args,
        &mut io::stdin().lock(),
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    );
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
