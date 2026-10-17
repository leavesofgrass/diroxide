//! The archive provider as the app makes it: its limits from the
//! `[archives]` table of the settings, one key to one field (P2's open
//! point), the read-ahead question's threshold and the spool folder (W2a's
//! request 3, P2b), installed in the registry beside the local provider;
//! and the sweep at start of what an interrupted run left behind: spool
//! files in the state folder's spool, and temporary archives a pack left
//! beside its destination (W2b's notes).
//!
//! The archive crate cannot depend on the configuration crate, so the
//! mapping lives here, with a test that the defaults of both agree.

use std::sync::Arc;
use std::time::SystemTime;

use diroxide_archive::{ArchiveProvider, Limits, is_pack_temp, spool};
use diroxide_config::{ArchiveSettings, Paths};
use diroxide_core::place::PathStyle;
use diroxide_core::{EntryKind, Place};
use diroxide_vfs::{OpContext, Registry, VfsError};

/// The archive formats and a pack's request, for a frontend that names a
/// format or makes an archive without a dialog.
pub use diroxide_archive::{ArchiveFormat, PackRequest};

/// Packs files on this computer into a new archive with diroxide's own
/// runner, asking nothing (a password travels in the request): for a
/// frontend's tests and for a demo set made without a dialog. The error
/// says what could not be packed, in words.
pub fn pack_local(request: &PackRequest) -> Result<(), diroxide_core::Message> {
    diroxide_archive::pack(request, &Registry::with_local(), &OpContext::new()).map_err(|error| {
        let name = request
            .destination
            .name()
            .map_or_else(|| request.destination.to_string(), str::to_owned);
        diroxide_core::Message::from_error("pack", &name, &error)
    })
}

/// What the runtime learned about an archive while it listed a place in
/// it, sent to the app before the listing: the words for the title line
/// and the line said on opening it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveFacts {
    /// The place listed.
    pub place: Place,
    /// The archive file the place is inside.
    pub container: Place,
    /// The place's title from the provider, the format named when its
    /// bytes and its name disagree: "a.zip, 7z archive, in D:\docs";
    /// "backup.7z.001, archive of 12 parts".
    pub title: String,
    /// The line said on opening it: "Opened course.zip: 10 items, 48 KB."
    pub opened: String,
    /// Lines said after it: "Over 500,000 entries: listing the first
    /// 500,000".
    pub notes: Vec<String>,
    /// Whether a password was asked for to list it, so the app makes the
    /// provider forget it when the panel leaves the archive.
    pub locked: bool,
}

/// The provider's limits from the `[archives]` table: sizes in GiB and
/// MiB become bytes, counts stay counts.
pub fn limits(settings: &ArchiveSettings) -> Limits {
    let count = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
    let gib = ArchiveSettings::gib;
    let mib = ArchiveSettings::mib;
    Limits {
        max_member_size: gib(settings.max_member_gib),
        ask_total_bytes: gib(settings.ask_total_gib),
        max_total_bytes: gib(settings.max_total_gib),
        max_ratio: settings.max_ratio,
        ratio_floor_bytes: mib(settings.ratio_floor_mib),
        ask_dictionary_bytes: mib(settings.ask_dictionary_mib),
        max_dictionary: mib(settings.max_dictionary_mib),
        max_header_bytes: mib(settings.max_header_mib),
        max_index_bytes: mib(settings.max_index_mib),
        max_entries: count(settings.max_entries),
        max_name_bytes: count(settings.max_name_bytes),
        max_component_bytes: count(settings.max_component_bytes),
        max_link_target_bytes: count(settings.max_link_target_bytes),
        max_memory_bytes: mib(settings.max_memory_mib),
        max_spool_memory_bytes: mib(settings.spool_memory_mib),
        max_spool_disk_bytes: gib(settings.spool_disk_gib),
        max_nesting: count(settings.max_nesting),
    }
}

/// The archive provider the settings describe: the limits, the read-ahead
/// question (`with_cost_question`) and, with the state folder known, the
/// spool folder (`with_spool_dir`), never the system's temporary folder.
pub fn provider(settings: &ArchiveSettings, paths: Option<&Paths>) -> ArchiveProvider {
    let provider =
        ArchiveProvider::new(limits(settings)).with_cost_question(settings.read_ahead_ask_bytes());
    match paths {
        Some(paths) => provider.with_spool_dir(settings.spool_dir(paths)),
        None => provider,
    }
}

/// A registry like `base` with `provider` serving the places inside
/// archives, connected to it: the archive provider reaches archive files
/// through this registry.
pub fn install(
    base: &Registry,
    provider: ArchiveProvider,
) -> (Arc<Registry>, Arc<ArchiveProvider>) {
    let provider = Arc::new(provider);
    let mut registry = base.clone();
    registry.set_archive(provider.clone());
    let registry = Arc::new(registry);
    provider.connect(&registry);
    (registry, provider)
}

/// Whether a registry already serves places inside archives.
pub fn serves_archives(registry: &Registry) -> bool {
    let Ok(probe) = Place::parse_with("/probe.zip!member", PathStyle::Unix) else {
        return false;
    };
    !matches!(registry.resolve(&probe), Err(VfsError::NoProvider(_)))
}

/// Sweeps what an interrupted run left, at start: the spool files in the
/// spool folder, and the pack runner's temporary archives in the folders
/// packs wrote into (the app remembers them). Only regular files named as
/// the archive crate names its own, and older than a day, so a pack that
/// another diroxide is writing right now is never touched. Returns how
/// many were removed; nothing else is ever removed.
pub fn sweep(registry: &Registry, spool_dir: Option<&std::path::Path>, folders: &[Place]) -> usize {
    let mut removed = spool_dir
        .and_then(|dir| spool::sweep(dir).ok())
        .unwrap_or(0);
    let now = SystemTime::now();
    let ctx = OpContext::new();
    for folder in folders {
        let Ok(provider) = registry.resolve(folder) else {
            continue;
        };
        let Ok(entries) = provider.list(folder, &ctx) else {
            continue;
        };
        for entry in entries {
            if entry.kind != EntryKind::File || !is_pack_temp(&entry.name) {
                continue;
            }
            let old = entry
                .modified
                .and_then(|t| {
                    let at = SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(
                        u64::try_from(t.unix_seconds()).ok()?,
                    ))?;
                    now.duration_since(at).ok()
                })
                .is_some_and(|age| age >= spool::SWEEP_AFTER);
            if !old {
                continue;
            }
            if let Ok(place) = folder.join(&entry.name)
                && provider.remove(&place, &ctx).is_ok()
            {
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2's open point: the `[archives]` defaults are the provider's.
    #[test]
    fn the_settings_defaults_are_the_providers_defaults() {
        let settings = ArchiveSettings::default();
        assert_eq!(limits(&settings), Limits::default());
        // The read-ahead question's default, the archive crate's
        // COST_ASK_BYTES: 256 MiB.
        assert_eq!(settings.read_ahead_ask_bytes(), 256 << 20);
        let mut strict = settings.clone();
        strict.max_nesting = 2;
        strict.max_member_gib = 1;
        let mapped = limits(&strict);
        assert_eq!(mapped.max_nesting, 2);
        assert_eq!(mapped.max_member_size, 1 << 30);
    }

    #[test]
    fn a_registry_gets_the_archive_provider_once() {
        let local = Registry::with_local();
        assert!(!serves_archives(&local));
        let (registry, provider) = install(&local, provider(&ArchiveSettings::default(), None));
        assert!(serves_archives(&registry));
        assert_eq!(provider.limits(), Limits::default());
    }

    #[test]
    fn the_sweep_removes_only_old_temporary_archives() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Place::parse(&dir.path().to_string_lossy()).unwrap();
        let fresh = dir.path().join(".diroxide-7.tmp");
        std::fs::write(&fresh, b"x").unwrap();
        let other = dir.path().join("notes.tmp");
        std::fs::write(&other, b"x").unwrap();
        let registry = Registry::with_local();
        assert_eq!(sweep(&registry, None, std::slice::from_ref(&folder)), 0);
        assert!(fresh.exists(), "a fresh one may be another run's");
        assert!(other.exists());
    }
}
