//! The fake ssh, placed where a test wants it: a copy of the program in a
//! folder of its own, with its `fake-ssh.json` beside it.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The fake ssh as cargo built it.
pub const FAKE_SSH: &str = env!("CARGO_BIN_EXE_diro-fake-ssh");

/// The file name ssh has on this system.
pub fn ssh_name() -> &'static str {
    if cfg!(windows) { "ssh.exe" } else { "ssh" }
}

/// Puts a fake ssh in `folder` with `config`, and returns its path.
pub fn fake_ssh_in(folder: &Path, config: &Value) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let program = folder.join(ssh_name());
    std::fs::copy(FAKE_SSH, &program).unwrap();
    std::fs::write(
        folder.join("fake-ssh.json"),
        serde_json::to_string_pretty(config).unwrap(),
    )
    .unwrap();
    program
}

/// The lines a fake wrote to its log, each as JSON.
pub fn log_lines(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
