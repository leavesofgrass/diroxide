//! Volumes: what kind of drive a place is on, and how much room it has.
//! The trash gate asks the volume kind of the path's own volume root, not
//! of its drive letter alone, so a folder mounted from another drive is
//! judged by the drive it is really on (spike six).
//!
//! The volumes list (R25 3.1) is one model over three sources: the local
//! drives and mounts, each plugin's `provider/volumes` rows, and the
//! devices the volume-actions plugin reports ([`volume_rows`]). Rows say
//! their state first ("Mounted: Backup, removable, 412 GB free"). Free
//! space and labels are asked on worker threads with a short limit
//! ([`ask_spaces`], [`SPACE_LIMIT`]); a volume that does not answer in
//! time says "free space not known", and nothing but the volume and its
//! free space is ever asked, so a drive's media is never read.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use diroxide_core::Place;
use diroxide_core::format::human_size;
use serde::Deserialize;

use crate::cancel::{CancelToken, OpContext};
use crate::provider::Provider;

/// What kind of drive a volume is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VolumeKind {
    /// A fixed local disk.
    Fixed,
    /// A removable drive: a USB stick, a card.
    Removable,
    /// A network drive or share.
    Network,
    /// Not known: treated as having no trash.
    #[default]
    Unknown,
}

impl VolumeKind {
    /// The kind in words, for the drive list and messages: "disk",
    /// "removable drive", "network drive", "drive".
    pub const fn word(self) -> &'static str {
        match self {
            VolumeKind::Fixed => "disk",
            VolumeKind::Removable => "removable drive",
            VolumeKind::Network => "network drive",
            VolumeKind::Unknown => "drive",
        }
    }
}

/// The volume a place is on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VolumeInfo {
    /// The volume's root: `D:\`, a mount point, a share, `/`.
    pub root: Place,
    /// What kind of drive it is.
    pub kind: VolumeKind,
    /// The volume's label, when it has one.
    pub label: Option<String>,
    /// The file system's name, such as "NTFS" or "exFAT", when known.
    pub file_system: Option<String>,
}

impl VolumeInfo {
    /// A volume of unknown kind at a root.
    pub fn unknown(root: Place) -> Self {
        Self {
            root,
            kind: VolumeKind::Unknown,
            label: None,
            file_system: None,
        }
    }

    /// The same volume with a kind.
    pub fn with_kind(mut self, kind: VolumeKind) -> Self {
        self.kind = kind;
        self
    }

    /// The top of a place: the volume root a provider with no better
    /// answer reports (a drive, a share, `/`, a remote root, the folder
    /// that holds an archive's top).
    pub fn top_of(place: &Place) -> Place {
        let mut top = place.clone();
        while let Some(parent) = top.parent() {
            top = parent;
        }
        top
    }
}

/// What a place in the places list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlaceRole {
    /// A Windows drive.
    Drive,
    /// The Unix root, `/`.
    Root,
    /// The home folder.
    Home,
    /// A mounted file system on Linux or macOS.
    Mount,
}

/// A place for the go-to-place list: a drive, the root, the home folder or
/// a mount, with its kind and its words, meaning first ("Drive E:,
/// removable drive"; "Home folder, /home/ada"; "My Stick, removable
/// drive, /media/ada/My Stick"). Listing the places never reads a drive's
/// media.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalPlace {
    /// The place itself.
    pub place: Place,
    /// What it is.
    pub role: PlaceRole,
    /// The kind of drive it is on.
    pub kind: VolumeKind,
    /// Its row in words, meaning first.
    pub words: String,
}

impl LocalPlace {
    /// A place with its words made from its role and kind.
    pub fn new(place: Place, role: PlaceRole, kind: VolumeKind) -> Self {
        let words = match role {
            PlaceRole::Drive => {
                let root = place.to_string();
                let letter = root.trim_end_matches('\\');
                format!("Drive {letter}, {}", kind.word())
            }
            PlaceRole::Root => "Root folder /".to_owned(),
            PlaceRole::Home => format!("Home folder, {place}"),
            PlaceRole::Mount => {
                let name = place.name().unwrap_or("mount");
                format!("{name}, {}, {place}", kind.word())
            }
        };
        Self {
            place,
            role,
            kind,
            words,
        }
    }
}

/// Free space and size limits at a place (research/file-operations.md
/// C7). `None` means not known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpaceInfo {
    /// The bytes the user may still write (quotas counted).
    pub free_to_user: Option<u64>,
    /// The volume's size in bytes.
    pub total: Option<u64>,
    /// The largest file the file system takes (4 GiB less one byte on
    /// FAT32).
    pub max_file_size: Option<u64>,
}

// ----- the volumes list (R25 3.1, commit 84) -----

/// How long the volumes list waits for a volume's free space and label
/// before the row says "free space not known": a network volume that
/// never answers never holds the list (R25 3.1).
pub const SPACE_LIMIT: Duration = Duration::from_secs(2);

/// What a row of the volumes list says first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VolumeState {
    /// A local volume that can be opened.
    Mounted,
    /// A device the system knows that is not mounted: Mount opens it.
    NotMounted,
    /// A plugin's storage, reached through its connection.
    Connected,
}

impl VolumeState {
    /// The state in words, said first on its row: "Mounted", "Not
    /// mounted", "Connected".
    pub const fn word(self) -> &'static str {
        match self {
            VolumeState::Mounted => "Mounted",
            VolumeState::NotMounted => "Not mounted",
            VolumeState::Connected => "Connected",
        }
    }
}

/// Where a row of the volumes list came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VolumeSource {
    /// A drive or mount of this computer ([`LocalPlace`]).
    Local,
    /// A plugin's `provider/volumes` row, by the plugin's scheme.
    Plugin(String),
    /// A device the volume-actions plugin reported, not mounted.
    Device,
}

/// What the list knows of a volume's free space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpaceAnswer {
    /// Asked, not answered yet.
    #[default]
    Asking,
    /// The bytes free to the user, and the size when known.
    Known {
        /// The bytes the user may still write.
        free: u64,
        /// The volume's size, when known.
        total: Option<u64>,
    },
    /// The volume did not answer within the limit, or cannot say: never
    /// read as zero.
    NotKnown,
}

impl SpaceAnswer {
    /// The answer from a provider's [`SpaceInfo`]: known when it says
    /// the free bytes, else not known.
    pub fn from_info(info: &SpaceInfo) -> Self {
        match info.free_to_user {
            Some(free) => SpaceAnswer::Known {
                free,
                total: info.total,
            },
            None => SpaceAnswer::NotKnown,
        }
    }

    /// The free space in words, for the end of a row: "412 GB free",
    /// "free space not known", "checking free space".
    pub fn words(self) -> String {
        match self {
            SpaceAnswer::Known { free, .. } => format!("{} free", human_size(free)),
            SpaceAnswer::NotKnown => "free space not known".to_owned(),
            SpaceAnswer::Asking => "checking free space".to_owned(),
        }
    }
}

/// A plugin's storage for the volumes list, as the app reads it from the
/// plugin's `provider/volumes` row (the plugins crate's `VolumeRow`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteVolume {
    /// Its name, cleaned: "build".
    pub name: String,
    /// The plugin's scheme, said as its kind: "sftp".
    pub scheme: String,
    /// Its root.
    pub place: Place,
    /// The bytes free, when the row said.
    pub free: Option<u64>,
    /// Its size, when the row said.
    pub total: Option<u64>,
    /// Whether it can be unplugged.
    pub removable: bool,
}

/// A device the volume-actions plugin reported, one line of its
/// `devices` command: mounted or not, with the identity Unmount, Eject and
/// Mount take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceReport {
    /// Its name, meaning first: the label, or the device's own name.
    pub name: String,
    /// The identity the actions take: a device node (`/dev/sdb1`), a disk
    /// identifier (`disk2s1`), or a drive (`E:`).
    pub device: String,
    /// Its size in bytes, when known.
    pub size: Option<u64>,
    /// Whether it can be unplugged.
    pub removable: bool,
    /// Where it is mounted, when it is.
    pub mounted_at: Option<Place>,
}

impl DeviceReport {
    /// Reads the `devices` command's lines: one JSON object a line, with
    /// `name`, `device`, `size`, `removable` and `mounted_at`. A line that
    /// is not one, or names no device, is left out and counted.
    pub fn parse_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> (Vec<Self>, usize) {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            name: String,
            device: String,
            #[serde(default)]
            size: Option<u64>,
            #[serde(default)]
            removable: bool,
            #[serde(default)]
            mounted_at: Option<String>,
        }
        let mut devices = Vec::new();
        let mut left_out = 0;
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(wire) = serde_json::from_str::<Wire>(line) else {
                left_out += 1;
                continue;
            };
            let device = clean(&wire.device);
            if device.is_empty() {
                left_out += 1;
                continue;
            }
            let name = match clean(&wire.name) {
                name if name.is_empty() => device.clone(),
                name => name,
            };
            let mounted_at = wire
                .mounted_at
                .as_deref()
                .filter(|text| !text.is_empty())
                .and_then(|text| Place::parse(text).ok());
            devices.push(DeviceReport {
                name,
                device,
                size: wire.size,
                removable: wire.removable,
                mounted_at,
            });
        }
        (devices, left_out)
    }
}

/// A name from outside, made safe to say: control and other hidden
/// characters dropped, spaces at the ends trimmed, at most 80 characters.
fn clean(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| {
            !c.is_control() && !diroxide_core::names::has_hidden(c.encode_utf8(&mut [0; 4]))
        })
        .take(80)
        .collect();
    kept.trim().to_owned()
}

/// The three sources of the volumes list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeSources {
    /// The local drives and mounts ([`crate::local_places`]), asked
    /// without touching a drive.
    pub local: Vec<LocalPlace>,
    /// Each loaded plugin's storages.
    pub remote: Vec<RemoteVolume>,
    /// The devices the volume-actions plugin reported.
    pub devices: Vec<DeviceReport>,
}

/// What a row of the volumes list can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VolumeAction {
    /// Opens the volume in the active panel.
    Open,
    /// Mounts a device that is not mounted.
    Mount,
    /// Unmounts a volume, never by force.
    Unmount,
    /// Unmounts and powers off a removable drive, never by force.
    Eject,
    /// Leaves the row out of the list (the ignore list in settings).
    Hide,
}

impl VolumeAction {
    /// The action's name, for the actions list: "Open", "Eject".
    pub const fn name(self) -> &'static str {
        match self {
            VolumeAction::Open => "Open",
            VolumeAction::Mount => "Mount",
            VolumeAction::Unmount => "Unmount",
            VolumeAction::Eject => "Eject",
            VolumeAction::Hide => "Hide",
        }
    }
}

/// The volume actions this computer has: which of the volume-actions
/// plugin's commands are installed and enabled here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VolumeTools {
    /// Mount is there (Linux and macOS).
    pub mount: bool,
    /// Unmount is there (Linux and macOS).
    pub unmount: bool,
    /// Eject is there.
    pub eject: bool,
}

/// One action of a row, offered or shown disabled with its reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOffer {
    /// The action.
    pub action: VolumeAction,
    /// Why it cannot run now, in words; `None` when it can.
    pub why_not: Option<String>,
}

impl ActionOffer {
    fn ready(action: VolumeAction) -> Self {
        Self {
            action,
            why_not: None,
        }
    }

    fn needs_plugin(action: VolumeAction) -> Self {
        Self {
            action,
            why_not: Some(format!(
                "{} needs the Volume actions plugin.",
                action.name()
            )),
        }
    }

    /// The action in words for its list: "Eject", or "Eject, not
    /// available: Eject needs the Volume actions plugin."
    pub fn words(&self) -> String {
        match &self.why_not {
            None => self.action.name().to_owned(),
            Some(why) => format!("{}, not available: {why}", self.action.name()),
        }
    }
}

/// One row of the volumes list: the state first, then the name, the kind
/// and the free space ("Mounted: Backup, removable, 412 GB free").
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VolumeRow {
    /// Its name: a label, a drive ("Drive E:"), a mount's folder, a site.
    pub name: String,
    /// Mounted, not mounted or connected.
    pub state: VolumeState,
    /// Its kind in one word: "disk", "removable", "network", or the
    /// plugin's scheme ("sftp").
    pub kind: String,
    /// Whether it can be unplugged.
    pub removable: bool,
    /// Where Open goes; `None` for a device that is not mounted.
    pub place: Option<Place>,
    /// The identity the volume actions take, when known.
    pub device: Option<String>,
    /// The free space, as far as it is known.
    pub space: SpaceAnswer,
    /// A device's size when it is not mounted, from its report.
    pub size: Option<u64>,
    /// Where it came from.
    pub source: VolumeSource,
    /// The role of a local row (a drive, the root, a mount).
    pub role: Option<PlaceRole>,
}

/// The kind word of a local volume, for a row: "disk", "removable",
/// "network", "drive".
fn kind_word(kind: VolumeKind) -> &'static str {
    match kind {
        VolumeKind::Fixed => "disk",
        VolumeKind::Removable => "removable",
        VolumeKind::Network => "network",
        VolumeKind::Unknown => "drive",
    }
}

impl VolumeRow {
    /// A local drive or mount, mounted, its space not asked yet. The home
    /// folder is a place, not a volume: `None`.
    pub fn local(place: &LocalPlace) -> Option<Self> {
        let name = match place.role {
            PlaceRole::Drive => {
                let root = place.place.to_string();
                format!("Drive {}", root.trim_end_matches('\\'))
            }
            PlaceRole::Root => "Root folder /".to_owned(),
            PlaceRole::Mount => place.place.name().unwrap_or("mount").to_owned(),
            PlaceRole::Home => return None,
        };
        Some(Self {
            name,
            state: VolumeState::Mounted,
            kind: kind_word(place.kind).to_owned(),
            removable: place.kind == VolumeKind::Removable,
            place: Some(place.place.clone()),
            device: None,
            space: SpaceAnswer::Asking,
            size: None,
            source: VolumeSource::Local,
            role: Some(place.role),
        })
    }

    /// A plugin's storage, connected; its space as the plugin said, or
    /// still to ask when it said none.
    pub fn remote(volume: &RemoteVolume) -> Self {
        let space = match volume.free {
            Some(free) => SpaceAnswer::Known {
                free,
                total: volume.total,
            },
            None => SpaceAnswer::Asking,
        };
        Self {
            name: volume.name.clone(),
            state: VolumeState::Connected,
            kind: volume.scheme.clone(),
            removable: volume.removable,
            place: Some(volume.place.clone()),
            device: None,
            space,
            size: volume.total,
            source: VolumeSource::Plugin(volume.scheme.clone()),
            role: None,
        }
    }

    /// A device that is not mounted; its space is never asked, so its
    /// media is never read.
    pub fn device(report: &DeviceReport) -> Self {
        Self {
            name: report.name.clone(),
            state: VolumeState::NotMounted,
            kind: if report.removable {
                "removable"
            } else {
                "disk"
            }
            .to_owned(),
            removable: report.removable,
            place: None,
            device: Some(report.device.clone()),
            space: SpaceAnswer::NotKnown,
            size: report.size,
            source: VolumeSource::Device,
            role: None,
        }
    }

    /// The row in words, the state first: "Mounted: Backup, removable,
    /// 412 GB free"; "Not mounted: Backup, removable, 931 GB";
    /// "Connected: build, sftp, 1.2 TB free".
    pub fn words(&self) -> String {
        let head = format!("{}: {}, {}", self.state.word(), self.name, self.kind);
        match self.state {
            VolumeState::NotMounted => match self.size {
                Some(size) => format!("{head}, {}", human_size(size)),
                None => head,
            },
            VolumeState::Mounted | VolumeState::Connected => {
                format!("{head}, {}", self.space.words())
            }
        }
    }

    /// The details line, said after the row: "412 GB free of 1.0 TB,
    /// E:\"; "Device /dev/sdb1".
    pub fn detail(&self) -> String {
        let mut parts = Vec::new();
        if let SpaceAnswer::Known {
            free,
            total: Some(total),
        } = self.space
        {
            parts.push(format!(
                "{} free of {}",
                human_size(free),
                human_size(total)
            ));
        }
        if let Some(place) = &self.place {
            parts.push(place.to_string());
        }
        if let Some(device) = &self.device {
            parts.push(format!("device {device}"));
        }
        let line = parts.join(", ");
        let mut chars = line.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => line,
        }
    }

    /// The key the ignore list keeps for Hide: the device, else the
    /// place, so a stick hidden once stays hidden wherever it mounts.
    pub fn key(&self) -> String {
        match (&self.device, &self.place) {
            (Some(device), _) => device.clone(),
            (None, Some(place)) => place.to_string(),
            (None, None) => self.name.clone(),
        }
    }

    /// Whether the volume actions apply: a removable volume, or one whose
    /// device is known; never the root, a network share or a plugin's
    /// storage.
    fn takes_actions(&self) -> bool {
        self.source == VolumeSource::Device
            || (self.source == VolumeSource::Local
                && self.role != Some(PlaceRole::Root)
                && self.kind != kind_word(VolumeKind::Network)
                && (self.removable || self.device.is_some()))
    }

    /// The row's actions, in order, each offered or disabled with its
    /// reason ("Eject needs the Volume actions plugin."). Windows mounts
    /// removable drives itself, so it has no Mount or Unmount (R25 3.2).
    pub fn actions(&self, tools: VolumeTools) -> Vec<ActionOffer> {
        self.actions_on(tools, cfg!(windows))
    }

    fn actions_on(&self, tools: VolumeTools, on_windows: bool) -> Vec<ActionOffer> {
        let offer = |action: VolumeAction, here: bool| {
            if here {
                ActionOffer::ready(action)
            } else {
                ActionOffer::needs_plugin(action)
            }
        };
        let mut offers = Vec::new();
        match self.state {
            VolumeState::Connected => offers.push(ActionOffer::ready(VolumeAction::Open)),
            VolumeState::Mounted => {
                offers.push(ActionOffer::ready(VolumeAction::Open));
                if self.takes_actions() {
                    if !on_windows {
                        offers.push(offer(VolumeAction::Unmount, tools.unmount));
                    }
                    if self.removable {
                        offers.push(offer(VolumeAction::Eject, tools.eject));
                    }
                }
            }
            VolumeState::NotMounted => {
                if !on_windows {
                    offers.push(offer(VolumeAction::Mount, tools.mount));
                }
                if self.removable {
                    offers.push(offer(VolumeAction::Eject, tools.eject));
                }
            }
        }
        offers.push(ActionOffer::ready(VolumeAction::Hide));
        offers
    }

    /// Takes in a volume's answer: its free space, and its label as the
    /// name of a drive or mount that has one ("Backup E:").
    pub fn apply(&mut self, answer: VolumeAnswer) {
        self.space = answer.space;
        let Some(label) = answer.label.map(|l| clean(&l)).filter(|l| !l.is_empty()) else {
            return;
        };
        match self.role {
            Some(PlaceRole::Drive) if self.name.starts_with("Drive ") => {
                let letter = self.name.trim_start_matches("Drive ").to_owned();
                self.name = format!("{label} {letter}");
            }
            Some(PlaceRole::Mount) => self.name = label,
            _ => {}
        }
    }
}

/// The volumes list from its three sources, in order: the local volumes,
/// the plugins' storages, then the devices that are not mounted. A device
/// mounted where a local row is gives that row its identity (and its
/// name, when the row has only a drive's); a device mounted where no
/// local row is says nothing more; one that is not mounted gets a row of
/// its own. Rows whose [`VolumeRow::key`] is in `hidden` are left out.
pub fn volume_rows(sources: &VolumeSources, hidden: &[String]) -> Vec<VolumeRow> {
    let mut rows: Vec<VolumeRow> = sources.local.iter().filter_map(VolumeRow::local).collect();
    for report in &sources.devices {
        let Some(at) = &report.mounted_at else {
            continue;
        };
        let Some(row) = rows
            .iter_mut()
            .find(|row| row.place.as_ref() == Some(at) && row.source == VolumeSource::Local)
        else {
            continue;
        };
        row.device = Some(report.device.clone());
        if report.removable {
            row.removable = true;
            row.kind = kind_word(VolumeKind::Removable).to_owned();
        }
        if row.role == Some(PlaceRole::Drive) && report.name != report.device {
            let letter = row.name.trim_start_matches("Drive ").to_owned();
            row.name = format!("{} {letter}", report.name);
        }
    }
    rows.extend(sources.remote.iter().map(VolumeRow::remote));
    rows.extend(
        sources
            .devices
            .iter()
            .filter(|report| report.mounted_at.is_none())
            .map(VolumeRow::device),
    );
    rows.retain(|row| !hidden.contains(&row.key()));
    rows
}

/// A volume's answer to the list: its free space and its label.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeAnswer {
    /// The free space, or not known.
    pub space: SpaceAnswer,
    /// The volume's label, when it has one.
    pub label: Option<String>,
}

impl VolumeAnswer {
    /// The answer of a volume that did not answer in time.
    pub fn not_known() -> Self {
        Self {
            space: SpaceAnswer::NotKnown,
            label: None,
        }
    }
}

/// One volume to ask: the row it answers, its root and its provider.
#[derive(Clone, Debug)]
pub struct SpaceRequest {
    /// The row's index in the list.
    pub index: usize,
    /// The volume's root.
    pub place: Place,
    /// The provider that serves it.
    pub provider: Arc<dyn Provider>,
}

/// The rows to ask: every mounted or connected row whose space is still
/// [`SpaceAnswer::Asking`] and whose provider `resolve` finds. A device
/// that is not mounted is never asked, so its media is never read.
pub fn space_requests(
    rows: &[VolumeRow],
    resolve: impl Fn(&Place) -> Option<Arc<dyn Provider>>,
) -> Vec<SpaceRequest> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.state != VolumeState::NotMounted && row.space == SpaceAnswer::Asking)
        .filter_map(|(index, row)| {
            let place = row.place.clone()?;
            let provider = resolve(&place)?;
            Some(SpaceRequest {
                index,
                place,
                provider,
            })
        })
        .collect()
}

/// Asks each volume for its label ([`Provider::volume`]) and free space
/// ([`Provider::space`]), each on a thread of its own, and hands each
/// answer to `answer` as it comes, by row index. Nothing else is asked of
/// a volume: no listing, no read, no stat, so a drive's media is never
/// read. When `limit` passes, every volume not answered gets
/// [`VolumeAnswer::not_known`] and its ask is cancelled; a provider that
/// never returns keeps only its own thread, never the caller. Blocks for
/// at most `limit`: run it off the input thread.
pub fn ask_spaces(
    requests: Vec<SpaceRequest>,
    limit: Duration,
    answer: &mut dyn FnMut(usize, VolumeAnswer),
) {
    let deadline = Instant::now() + limit;
    let (sender, receiver) = mpsc::channel::<(usize, VolumeAnswer)>();
    let mut waiting: Vec<(usize, CancelToken)> = Vec::new();
    for request in requests {
        let cancel = CancelToken::new();
        let sender = sender.clone();
        let token = cancel.clone();
        let index = request.index;
        let spawned = std::thread::Builder::new()
            .name("diroxide-volume-space".to_owned())
            .spawn(move || {
                let ctx = OpContext::new().with_cancel(token);
                let label = request
                    .provider
                    .volume(&request.place, &ctx)
                    .ok()
                    .and_then(|info| info.label);
                let space = request
                    .provider
                    .space(&request.place, &ctx)
                    .map_or(SpaceAnswer::NotKnown, |info| SpaceAnswer::from_info(&info));
                // The list may have stopped waiting: a closed channel is
                // fine.
                let _ = sender.send((request.index, VolumeAnswer { space, label }));
            });
        match spawned {
            Ok(_) => waiting.push((index, cancel)),
            Err(_) => answer(index, VolumeAnswer::not_known()),
        }
    }
    drop(sender);
    while !waiting.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok((index, got)) => {
                if let Some(at) = waiting.iter().position(|(i, _)| *i == index) {
                    waiting.remove(at);
                    answer(index, got);
                }
            }
            Err(_) => break,
        }
    }
    for (index, cancel) in waiting {
        cancel.cancel();
        answer(index, VolumeAnswer::not_known());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::place::PathStyle;

    #[test]
    fn places_say_what_they_are_first() {
        let drive = Place::parse_with("E:\\", PathStyle::Windows).unwrap();
        assert_eq!(
            LocalPlace::new(drive, PlaceRole::Drive, VolumeKind::Removable).words,
            "Drive E:, removable drive"
        );
        let stick = Place::parse_with("/media/ada/My Stick", PathStyle::Unix).unwrap();
        assert_eq!(
            LocalPlace::new(stick, PlaceRole::Mount, VolumeKind::Removable).words,
            "My Stick, removable drive, /media/ada/My Stick"
        );
        let home = Place::parse_with("/home/ada", PathStyle::Unix).unwrap();
        let home = LocalPlace::new(home, PlaceRole::Home, VolumeKind::Fixed);
        assert_eq!(home.words, "Home folder, /home/ada");
        let root = Place::parse_with("/", PathStyle::Unix).unwrap();
        assert_eq!(
            LocalPlace::new(root, PlaceRole::Root, VolumeKind::Fixed).words,
            "Root folder /"
        );
    }

    #[test]
    fn the_top_of_a_place_is_its_drive() {
        let place = Place::parse_with("D:\\docs\\a.zip!x/y", PathStyle::Windows).unwrap();
        assert_eq!(VolumeInfo::top_of(&place).to_string(), "D:\\");
        assert_eq!(VolumeKind::Removable.word(), "removable drive");
        assert_eq!(
            VolumeInfo::unknown(VolumeInfo::top_of(&place))
                .with_kind(VolumeKind::Fixed)
                .kind,
            VolumeKind::Fixed
        );
    }

    // ----- the volumes list -----

    use crate::capabilities::Capabilities;
    use crate::error::VfsError;
    use crate::provider::{AttributeChange, FileWriter, WriteMode};
    use diroxide_core::Entry;
    use diroxide_core::format::within_cells;
    use std::io::Read;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A provider of volumes only, for the list's tests: it answers
    /// `volume` and `space` after a delay (or never, until cancelled),
    /// and counts every call that would read a drive's media.
    #[derive(Debug, Default)]
    struct Volumes {
        delay: Duration,
        never: bool,
        label: Option<String>,
        free: Option<u64>,
        media_reads: AtomicU32,
        asked: Mutex<Vec<String>>,
    }

    impl Volumes {
        fn answering(free: u64, label: Option<&str>) -> Self {
            Self {
                free: Some(free),
                label: label.map(str::to_owned),
                ..Self::default()
            }
        }

        fn wait(&self, ctx: &OpContext) -> Result<(), VfsError> {
            let start = Instant::now();
            while self.never || start.elapsed() < self.delay {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }

        fn read_media(&self) -> VfsError {
            self.media_reads.fetch_add(1, Ordering::SeqCst);
            VfsError::unsupported("test", "read a drive's media")
        }
    }

    impl Provider for Volumes {
        fn scheme(&self) -> &str {
            "file"
        }
        fn capabilities(&self, _: &Place) -> Capabilities {
            Capabilities::read_only()
        }
        fn list(&self, _: &Place, _: &OpContext) -> Result<Vec<Entry>, VfsError> {
            Err(self.read_media())
        }
        fn stat(&self, _: &Place, _: &OpContext) -> Result<Entry, VfsError> {
            Err(self.read_media())
        }
        fn read(&self, _: &Place, _: &OpContext) -> Result<Box<dyn Read + Send>, VfsError> {
            Err(self.read_media())
        }
        fn write(
            &self,
            _: &Place,
            _: WriteMode,
            _: &OpContext,
        ) -> Result<Box<dyn FileWriter>, VfsError> {
            Err(self.read_media())
        }
        fn create_dir(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(self.read_media())
        }
        fn rename(&self, _: &Place, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(self.read_media())
        }
        fn remove(&self, _: &Place, _: &OpContext) -> Result<(), VfsError> {
            Err(self.read_media())
        }
        fn set_attributes(
            &self,
            _: &Place,
            _: &AttributeChange,
            _: &OpContext,
        ) -> Result<AttributeChange, VfsError> {
            Err(self.read_media())
        }
        fn volume(&self, place: &Place, ctx: &OpContext) -> Result<VolumeInfo, VfsError> {
            self.asked.lock().unwrap().push(place.to_string());
            self.wait(ctx)?;
            let mut info = VolumeInfo::unknown(place.clone());
            info.label = self.label.clone();
            Ok(info)
        }
        fn space(&self, _: &Place, ctx: &OpContext) -> Result<SpaceInfo, VfsError> {
            self.wait(ctx)?;
            Ok(SpaceInfo {
                free_to_user: self.free,
                total: self.free.map(|f| f * 2),
                max_file_size: None,
            })
        }
    }

    fn unix(text: &str) -> Place {
        Place::parse_with(text, PathStyle::Unix).unwrap()
    }

    fn stick() -> LocalPlace {
        LocalPlace::new(
            unix("/media/ada/BACKUP"),
            PlaceRole::Mount,
            VolumeKind::Removable,
        )
    }

    fn report(device: &str, mounted_at: Option<&str>) -> DeviceReport {
        DeviceReport {
            name: "Backup".to_owned(),
            device: device.to_owned(),
            size: Some(931_000_000_000),
            removable: true,
            mounted_at: mounted_at.map(unix),
        }
    }

    #[test]
    fn rows_say_their_state_first_within_forty_cells() {
        let mut mounted = VolumeRow::local(&stick()).unwrap();
        mounted.apply(VolumeAnswer {
            space: SpaceAnswer::Known {
                free: 412_000_000_000,
                total: Some(1_000_000_000_000),
            },
            label: Some("Backup".to_owned()),
        });
        assert_eq!(mounted.words(), "Mounted: Backup, removable, 412 GB free");
        assert_eq!(mounted.detail(), "412 GB free of 1.0 TB, /media/ada/BACKUP");
        let unmounted = VolumeRow::device(&report("/dev/sdb1", None));
        assert_eq!(unmounted.words(), "Not mounted: Backup, removable, 931 GB");
        assert_eq!(unmounted.detail(), "Device /dev/sdb1");
        let site = VolumeRow::remote(&RemoteVolume {
            name: "build".to_owned(),
            scheme: "sftp".to_owned(),
            place: Place::parse("sftp://build.example/").unwrap(),
            free: Some(1_200_000_000_000),
            total: None,
            removable: false,
        });
        assert_eq!(site.words(), "Connected: build, sftp, 1.2 TB free");
        let mut slow = VolumeRow::local(&stick()).unwrap();
        slow.apply(VolumeAnswer::not_known());
        assert_eq!(
            slow.words(),
            "Mounted: BACKUP, removable, free space not known"
        );
        for row in [&mounted, &unmounted, &site, &slow] {
            let state = row.state.word();
            assert!(within_cells(&row.words(), state), "{}", row.words());
            assert!(within_cells(&row.words(), &row.name), "{}", row.words());
        }
        let drive = LocalPlace::new(
            Place::parse_with("E:\\", PathStyle::Windows).unwrap(),
            PlaceRole::Drive,
            VolumeKind::Removable,
        );
        let mut drive = VolumeRow::local(&drive).unwrap();
        assert_eq!(
            drive.words(),
            "Mounted: Drive E:, removable, checking free space"
        );
        drive.apply(VolumeAnswer {
            space: SpaceAnswer::NotKnown,
            label: Some("Backup\u{7}".to_owned()),
        });
        assert_eq!(drive.name, "Backup E:", "a control character is dropped");
    }

    #[test]
    fn one_list_from_local_volumes_plugin_rows_and_devices() {
        let home = LocalPlace::new(unix("/home/ada"), PlaceRole::Home, VolumeKind::Fixed);
        let root = LocalPlace::new(unix("/"), PlaceRole::Root, VolumeKind::Fixed);
        let sources = VolumeSources {
            local: vec![root, home, stick()],
            remote: vec![RemoteVolume {
                name: "build".to_owned(),
                scheme: "sftp".to_owned(),
                place: Place::parse("sftp://build.example/").unwrap(),
                free: None,
                total: None,
                removable: false,
            }],
            devices: vec![
                report("/dev/sdb1", Some("/media/ada/BACKUP")),
                report("/dev/sdc1", None),
            ],
        };
        let rows = volume_rows(&sources, &[]);
        let words: Vec<String> = rows.iter().map(VolumeRow::words).collect();
        assert_eq!(
            words,
            [
                "Mounted: Root folder /, disk, checking free space",
                "Mounted: BACKUP, removable, checking free space",
                "Connected: build, sftp, checking free space",
                "Not mounted: Backup, removable, 931 GB",
            ],
            "the home folder is a place, not a volume"
        );
        assert_eq!(rows[1].device.as_deref(), Some("/dev/sdb1"));
        assert_eq!(rows[1].key(), "/dev/sdb1");
        let hidden = volume_rows(&sources, &["/dev/sdc1".to_owned()]);
        assert_eq!(hidden.len(), 3, "Hide keeps the device out");
    }

    #[test]
    fn actions_need_the_plugin_and_say_why_when_it_is_missing() {
        let mounted = volume_rows(
            &VolumeSources {
                local: vec![stick()],
                devices: vec![report("/dev/sdb1", Some("/media/ada/BACKUP"))],
                ..VolumeSources::default()
            },
            &[],
        )
        .remove(0);
        let words = |offers: Vec<ActionOffer>| -> Vec<String> {
            offers.iter().map(ActionOffer::words).collect()
        };
        assert_eq!(
            words(mounted.actions_on(VolumeTools::default(), false)),
            [
                "Open",
                "Unmount, not available: Unmount needs the Volume actions plugin.",
                "Eject, not available: Eject needs the Volume actions plugin.",
                "Hide",
            ]
        );
        let all = VolumeTools {
            mount: true,
            unmount: true,
            eject: true,
        };
        assert_eq!(
            words(mounted.actions_on(all, true)),
            ["Open", "Eject", "Hide"],
            "Windows mounts and unmounts removable drives itself"
        );
        let unmounted = VolumeRow::device(&report("/dev/sdc1", None));
        assert_eq!(
            words(unmounted.actions_on(all, false)),
            ["Mount", "Eject", "Hide"]
        );
        let root = VolumeRow::local(&LocalPlace::new(
            unix("/"),
            PlaceRole::Root,
            VolumeKind::Fixed,
        ))
        .unwrap();
        assert_eq!(words(root.actions_on(all, false)), ["Open", "Hide"]);
        let reason = ActionOffer::needs_plugin(VolumeAction::Eject);
        assert!(within_cells(
            reason.why_not.as_deref().unwrap(),
            "Eject needs the Volume actions plugin."
        ));
    }

    #[test]
    fn the_devices_lines_are_read_and_bad_lines_counted() {
        let lines = [
            r#"{"name":"Backup","device":"/dev/sdb1","size":931000000000,"removable":true}"#,
            r#"{"name":"","device":"/dev/sdc1","mounted_at":"/media/ada/CARD"}"#,
            "not json",
            r#"{"name":"x","device":"   "}"#,
            "",
        ];
        let (devices, left_out) = DeviceReport::parse_lines(lines);
        assert_eq!(left_out, 2);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].mounted_at, None);
        assert_eq!(devices[1].name, "/dev/sdc1", "no name: the device's own");
        assert_eq!(
            devices[1].mounted_at.is_some(),
            cfg!(not(windows)),
            "a Unix mount point is a place only on Unix"
        );
    }

    #[test]
    fn free_space_is_asked_with_a_limit_and_a_silent_volume_is_not_known() {
        let quick: Arc<dyn Provider> =
            Arc::new(Volumes::answering(412_000_000_000, Some("Backup")));
        let slow: Arc<Volumes> = Arc::new(Volumes {
            delay: Duration::from_millis(150),
            ..Volumes::answering(5_000_000_000, None)
        });
        let silent: Arc<Volumes> = Arc::new(Volumes {
            never: true,
            ..Volumes::default()
        });
        let sources = VolumeSources {
            local: vec![
                stick(),
                LocalPlace::new(unix("/mnt/slow"), PlaceRole::Mount, VolumeKind::Fixed),
                LocalPlace::new(unix("/mnt/nas"), PlaceRole::Mount, VolumeKind::Network),
            ],
            devices: vec![report("/dev/sdc1", None)],
            ..VolumeSources::default()
        };
        let mut rows = volume_rows(&sources, &[]);
        let requests = space_requests(&rows, |place| {
            Some(match place.to_string().as_str() {
                "/media/ada/BACKUP" => Arc::clone(&quick),
                "/mnt/slow" => Arc::clone(&slow) as Arc<dyn Provider>,
                _ => Arc::clone(&silent) as Arc<dyn Provider>,
            })
        });
        assert_eq!(
            requests.len(),
            3,
            "the device that is not mounted is never asked"
        );
        let started = Instant::now();
        let mut order = Vec::new();
        ask_spaces(
            requests,
            Duration::from_millis(600),
            &mut |index, answer| {
                order.push(index);
                rows[index].apply(answer);
            },
        );
        let took = started.elapsed();
        assert!(took < Duration::from_secs(2), "the limit holds: {took:?}");
        assert_eq!(order.last(), Some(&2), "the silent volume answers last");
        let words: Vec<String> = rows.iter().map(VolumeRow::words).collect();
        assert_eq!(
            words,
            [
                "Mounted: Backup, removable, 412 GB free",
                "Mounted: slow, disk, 5.0 GB free",
                "Mounted: nas, network, free space not known",
                "Not mounted: Backup, removable, 931 GB",
            ]
        );
        for provider in [&slow, &silent] {
            assert_eq!(provider.media_reads.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn a_volume_that_cannot_say_its_space_is_not_known_never_zero() {
        assert_eq!(
            SpaceAnswer::from_info(&SpaceInfo::default()),
            SpaceAnswer::NotKnown
        );
        assert_eq!(SpaceAnswer::NotKnown.words(), "free space not known");
        let mut answered = Vec::new();
        ask_spaces(Vec::new(), Duration::from_millis(10), &mut |i, a| {
            answered.push((i, a));
        });
        assert!(answered.is_empty());
    }
}
