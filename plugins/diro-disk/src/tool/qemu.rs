//! `qemu-img`, narrow and optional (R29 1.5): a container that neither a
//! built-in reader nor 7-Zip opens (a QCOW2 with a backing file, QED, an
//! odd VMDK) is converted to a raw disk in the spool, and only when the
//! size it claims fits the spool budget, so a sparse image claiming 2 TB
//! never becomes a 2 TB file: "it is too large to unpack here: claims 2.2
//! TB, holds 1.1 GB". The form is `qemu-img convert -f FORMAT -O raw -U
//! SOURCE SPOOL/raw.img`, the size first from `qemu-img info
//! --output=json -U SOURCE`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::error::DiskError;
use crate::host::{self, Run};
use crate::image::sniff::Container;
use crate::rpc::Peer;
use crate::spool::Spool;

/// How long a convert may take.
pub const CONVERT_WAIT: Duration = Duration::from_secs(3600);

/// The format word `qemu-img` takes for a container.
pub fn format(container: Container) -> Option<&'static str> {
    Some(match container {
        Container::Qcow2 => "qcow2",
        Container::Vhdx => "vhdx",
        Container::VhdDynamic | Container::VhdFixed | Container::VhdDifferencing => "vpc",
        Container::Vdi => "vdi",
        Container::VmdkSparse | Container::VmdkDescriptor => "vmdk",
        Container::Raw => return None,
    })
}

/// The virtual size `qemu-img info` gives.
pub fn virtual_size(info: &str) -> Option<u64> {
    serde_json::from_str::<Value>(info)
        .ok()?
        .get("virtual-size")?
        .as_u64()
}

/// Converts `source` (a path the host's `qemu-img` reads) into a raw disk
/// in the spool; `None` when `qemu-img` is not there.
pub fn convert(
    peer: &Arc<Peer>,
    spool: &Spool,
    container: Container,
    source: &str,
    held: u64,
) -> Result<Option<std::path::PathBuf>, DiskError> {
    let Some(format) = format(container) else {
        return Ok(None);
    };
    match host::find(peer, "qemu-img") {
        Ok(Some(_)) => {}
        Ok(None) => return Ok(None),
        Err(error) if host::not_offered(&error) => return Ok(None),
        Err(error) => return Err(error),
    }
    let info = host::run(
        peer,
        &Run {
            tool: "qemu-img",
            args: vec![
                "info".into(),
                "--output=json".into(),
                "-U".into(),
                "-f".into(),
                format.into(),
                source.into(),
            ],
            timeout: Some(Duration::from_secs(30)),
            max_output: Some(1024 * 1024),
            ..Run::default()
        },
    )?;
    let size = virtual_size(&String::from_utf8_lossy(&info.stdout))
        .ok_or_else(|| DiskError::Io("qemu-img did not say the disk's size".to_owned()))?;
    spool.reserve(size, held)?;
    let target = spool.name("converted.img");
    let ran = host::run(
        peer,
        &Run {
            tool: "qemu-img",
            args: vec![
                "convert".into(),
                "-U".into(),
                "-f".into(),
                format.into(),
                "-O".into(),
                "raw".into(),
                source.into(),
                target.display().to_string(),
            ],
            timeout: Some(CONVERT_WAIT),
            max_output: Some(64 * 1024),
            ..Run::default()
        },
    )?;
    if ran.code != Some(0) {
        let first = ran
            .stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("it stopped");
        return Err(DiskError::Io(format!(
            "qemu-img: {}",
            crate::error::lowercase_first(first.trim())
        )));
    }
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_gives_the_virtual_size() {
        let info = r#"{"virtual-size": 2199023255552, "filename": "x.qcow2", "format": "qcow2"}"#;
        assert_eq!(virtual_size(info), Some(2_199_023_255_552));
        assert_eq!(virtual_size("not json"), None);
        assert_eq!(format(Container::VhdDynamic), Some("vpc"));
        assert_eq!(format(Container::Raw), None);
    }
}
