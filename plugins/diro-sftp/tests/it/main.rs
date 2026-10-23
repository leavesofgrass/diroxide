//! Integration tests of the SFTP places plugin: the program in both its
//! modes, the conformance suite on it, and the spikes that ran on the
//! owner's system (ignored elsewhere).

// Test helpers may unwrap: a failure is the test's failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod askpass;
mod codec;
mod conformance;
#[cfg(feature = "fake-ssh")]
mod connect;
#[cfg(feature = "fake-ssh")]
mod contract;
#[cfg(feature = "fake-ssh")]
mod copy;
#[cfg(feature = "fake-ssh")]
mod dialogs;
#[cfg(feature = "fake-ssh")]
mod doctor;
#[cfg(feature = "fake-ssh")]
mod errors;
#[cfg(feature = "fake-ssh")]
mod fake_server;
#[cfg(feature = "fake-ssh")]
mod fakes;
#[cfg(feature = "fake-ssh")]
mod host;
#[cfg(feature = "fake-ssh")]
mod hosts;
#[cfg(feature = "fake-ssh")]
mod launch;
#[cfg(feature = "fake-ssh")]
mod list;
#[cfg(feature = "fake-ssh")]
mod locate;
mod modes;
#[cfg(feature = "fake-ssh")]
mod ops;
#[cfg(feature = "fake-ssh")]
mod read;
#[cfg(feature = "fake-ssh")]
mod reconnect;
#[cfg(feature = "fake-ssh")]
mod secrets;
mod session;
#[cfg(feature = "fake-ssh")]
mod sites;
#[cfg(all(windows, feature = "fake-ssh"))]
mod spikes;
#[cfg(feature = "fake-ssh")]
mod write;

use std::path::{Path, PathBuf};

/// The plugin's program, as cargo built it for these tests.
pub fn plugin_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diro-sftp"))
}

/// The plain ssh configuration the tests give the plugin: no `Include`,
/// so a test never reads the owner's own configuration.
pub fn plain_ssh_config() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sftp/ssh-config/plain/config")
}

/// The plugin's own manifest with its program replaced by `exe`, in a
/// folder of its own under `root`; its `ssh_config_file` setting names
/// the plain fixture configuration, so the Sites list and
/// `provider/places` never read the owner's.
pub fn plugin_folder(root: &Path, exe: &Path) -> PathBuf {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin.toml")).unwrap();
    let command = format!("['{}']", exe.display());
    let setting = "key = \"ssh_config_file\"\ntype = \"path\"\ndefault = \"\"";
    assert!(manifest.contains(setting), "the manifest's config setting");
    let manifest = manifest
        .replace("[\"./diro-sftp.exe\"]", &command)
        .replace("[\"./diro-sftp\"]", &command)
        .replace(
            setting,
            &format!(
                "key = \"ssh_config_file\"\ntype = \"path\"\ndefault = '{}'",
                plain_ssh_config().display()
            ),
        );
    let folder = root.join("sftp");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("plugin.toml"), manifest).unwrap();
    folder
}
