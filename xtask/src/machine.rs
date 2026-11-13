//! What a bench run records about where it ran (R27's commit 36): the
//! machine in words and the versions of what was measured, beside the
//! date and time from the clock, so a baseline names what it was taken
//! on, as the plan's "Fast and lean" wants (plan section 9).
//!
//! Nothing here names the machine or its owner: no host name, no account,
//! no volume label. The processor, the memory, the system and the disk
//! are read by their models only.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, DiskKind, Disks, MemoryRefreshKind, RefreshKind, System};

/// The machine a run was on, in words.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Machine {
    /// The processor's model: "AMD Ryzen 5 5600G with Radeon Graphics".
    pub processor: String,
    /// Its cores and threads: "6 cores, 12 threads".
    pub cores: String,
    /// The memory: "64 GB".
    pub memory: String,
    /// The system and its build or kernel: "Windows 11 Home, build 26200".
    pub system: String,
    /// The disk that held the bench's trees: "Samsung SSD 990 EVO 2TB,
    /// NVMe" on Windows; its kind and file system elsewhere.
    pub disk: String,
}

/// The versions of what a run measured.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Versions {
    /// diroxide's version, the workspace's: "0.1.0-alpha.2".
    pub diroxide: String,
    /// The commit the bench was built from, said "with changes" when the
    /// tracked files differed from it: "44062eb".
    pub commit: String,
    /// The compiler: "rustc 1.96.0 (...)".
    pub rustc: String,
}

/// What a program wrote, trimmed, when it ran and ended well.
fn output_of(command: &mut Command) -> Option<String> {
    let out = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Bytes as whole gibibytes, the way memory is sold: "64 GB".
pub fn memory_words(bytes: u64) -> String {
    let gib = (bytes as f64 / f64::from(1u32 << 30)).round() as u64;
    format!("{gib} GB")
}

/// Cores and threads in words: "6 cores, 12 threads".
pub fn cores_words(cores: Option<usize>, threads: usize) -> String {
    let threads = if threads == 1 {
        "1 thread".to_owned()
    } else {
        format!("{threads} threads")
    };
    match cores {
        Some(1) => format!("1 core, {threads}"),
        Some(cores) => format!("{cores} cores, {threads}"),
        None => threads,
    }
}

/// The disk's model and bus on Windows, through PowerShell's storage
/// commands, by the folder's drive letter.
fn windows_disk(folder: &Path) -> Option<String> {
    let letter = folder
        .to_str()?
        .chars()
        .next()
        .filter(char::is_ascii_alphabetic)?;
    let script = format!(
        "$d = Get-Partition -DriveLetter {letter} | Get-Disk; if ($d) {{ \"$($d.FriendlyName), $($d.BusType)\" }}"
    );
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    output_of(&mut command)
}

/// The disk's kind and file system, from the mounted disk that holds the
/// folder (the longest mount point that starts its path).
fn mounted_disk(folder: &Path) -> Option<String> {
    let disks = Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter(|d| folder.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())?;
    let kind = match disk.kind() {
        DiskKind::SSD => "an SSD",
        DiskKind::HDD => "a hard disk",
        DiskKind::Unknown(_) => "a disk of unknown kind",
    };
    let file_system = disk.file_system().to_string_lossy();
    Some(if file_system.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind}, {file_system}")
    })
}

/// The machine this run is on; a fact that cannot be read is "unknown".
/// `folder` is where the bench writes its trees.
pub fn this_machine(folder: &Path) -> Machine {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let unknown = || "unknown".to_owned();
    let processor = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .filter(|brand| !brand.is_empty())
        .unwrap_or_else(unknown);
    let threads = std::thread::available_parallelism().map_or(system.cpus().len(), usize::from);
    let name = System::long_os_version()
        .or_else(System::name)
        .unwrap_or_else(|| std::env::consts::OS.to_owned());
    // Windows says its build as the kernel's version; the others their
    // kernel's own.
    let system_words = match System::kernel_version() {
        Some(kernel) if cfg!(windows) => format!("{name}, build {kernel}"),
        Some(kernel) => format!("{name}, kernel {kernel}"),
        None => name,
    };
    let disk = if cfg!(windows) {
        windows_disk(folder).or_else(|| mounted_disk(folder))
    } else {
        mounted_disk(folder)
    };
    Machine {
        processor,
        cores: cores_words(System::physical_core_count(), threads),
        memory: memory_words(system.total_memory()),
        system: system_words,
        disk: disk.unwrap_or_else(unknown),
    }
}

/// The versions of this build: the workspace's version, the commit of
/// `root`, and the compiler that `root`'s toolchain file names.
pub fn these_versions(root: &Path) -> Versions {
    let hash = commit_hash(root);
    let changed = hash.is_some() && has_changes(root);
    Versions {
        diroxide: env!("CARGO_PKG_VERSION").to_owned(),
        commit: commit_words(hash, changed),
        rustc: rustc_version(root),
    }
}

/// The short hash of `root`'s commit, when git answers.
fn commit_hash(root: &Path) -> Option<String> {
    output_of(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--short=7", "HEAD"]),
    )
}

/// Whether `root`'s tracked files differ from its commit. Git may read
/// every tracked file again to answer (a Windows tree mounted in a
/// container makes it do so), which takes minutes on a busy share.
fn has_changes(root: &Path) -> bool {
    output_of(Command::new("git").current_dir(root).args([
        "status",
        "--porcelain",
        "--untracked-files=no",
    ]))
    .is_some()
}

/// The commit in words: the hash, "with changes" after it when the
/// tracked files differed, "unknown" when git did not answer.
fn commit_words(hash: Option<String>, changed: bool) -> String {
    match hash {
        Some(hash) if changed => format!("{hash} with changes"),
        Some(hash) => hash,
        None => "unknown".to_owned(),
    }
}

/// The compiler `root`'s toolchain file names: "rustc 1.96.0 (...)".
fn rustc_version(root: &Path) -> String {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    output_of(Command::new(rustc).current_dir(root).arg("--version"))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The machine in one line, its processor first: "AMD Ryzen 5 5600G with
/// Radeon Graphics, 6 cores, 12 threads, 64 GB, Windows 11 Home, build
/// 26200; the trees on Samsung SSD 990 EVO 2TB, NVMe."
pub fn machine_words(machine: &Machine) -> String {
    format!(
        "{}, {}, {}, {}; the trees on {}.",
        machine.processor, machine.cores, machine.memory, machine.system, machine.disk
    )
}

/// The versions in one line: "diroxide 0.1.0-alpha.2 at 44062eb, rustc
/// 1.96.0 (...)."
pub fn versions_words(versions: &Versions) -> String {
    format!(
        "diroxide {} at {}, {}.",
        versions.diroxide, versions.commit, versions.rustc
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_and_cores_read_in_words() {
        assert_eq!(memory_words(68_719_476_736), "64 GB");
        assert_eq!(memory_words(17_179_869_184), "16 GB");
        assert_eq!(cores_words(Some(6), 12), "6 cores, 12 threads");
        assert_eq!(cores_words(Some(1), 1), "1 core, 1 thread");
        assert_eq!(cores_words(None, 4), "4 threads");
    }

    #[test]
    fn this_machine_is_read_without_a_name() {
        let root = crate::bench::workspace_root().unwrap();
        let machine = this_machine(&root);
        assert!(!machine.processor.is_empty());
        assert!(machine.cores.contains("thread"), "{}", machine.cores);
        assert!(machine.memory.ends_with(" GB"), "{}", machine.memory);
        let words = machine_words(&machine);
        // The machine's own name never goes into a record that is
        // committed and published.
        if let Some(host) = System::host_name().filter(|h| h.len() > 2) {
            assert!(
                !words.to_lowercase().contains(&host.to_lowercase()),
                "the host name is in the record"
            );
        }
        assert!(words.ends_with('.'), "{words}");
    }

    #[test]
    fn the_versions_name_the_workspace_and_the_compiler() {
        // The parts, without the whole-tree status: on a Windows tree
        // mounted in the development container git reads every tracked
        // file again for it, minutes on a busy share, and that is the
        // machine's cost and not this code's.
        let root = crate::bench::workspace_root().unwrap();
        let versions = Versions {
            diroxide: env!("CARGO_PKG_VERSION").to_owned(),
            commit: commit_words(commit_hash(&root), false),
            rustc: rustc_version(&root),
        };
        assert_eq!(versions.diroxide, env!("CARGO_PKG_VERSION"));
        assert!(versions.rustc.starts_with("rustc "), "{}", versions.rustc);
        assert!(!versions.commit.is_empty());
        assert!(versions_words(&versions).starts_with("diroxide "));
    }

    #[test]
    fn the_commit_says_its_changes_in_words() {
        let hash = || Some("44062eb".to_owned());
        assert_eq!(commit_words(hash(), false), "44062eb");
        assert_eq!(commit_words(hash(), true), "44062eb with changes");
        assert_eq!(commit_words(None, true), "unknown");
    }
}
