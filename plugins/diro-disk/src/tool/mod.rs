//! The tools the plugin uses when they are there and works without
//! (`wants` in its manifest): 7-Zip for what no built-in reader opens,
//! `qemu-img` for a container nothing else reads. Both run through the
//! host's `tools/run`, an argument list, never a shell line, and each
//! writes only into the plugin's own files in the spool.

pub mod qemu;
pub mod sevenz;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::error::DiskError;
use crate::host::{self, Run};
use crate::image::fs::{Facts, FileSystem, Node};
use crate::lines::{self, cause};
use crate::rpc::Peer;
use crate::spool::Spool;
use sevenz::{Tree, Version, parse_slt};

/// The most a member may be to come back in one `x -so` answer: the
/// host's line is 8 MiB, and base64 makes 6 bytes of 8.
pub const MAX_PIPED: u64 = 6 * 1024 * 1024 - 64 * 1024;

/// The most a listing's text may be in one answer.
pub const MAX_LISTING: u64 = 7 * 1024 * 1024;

/// How long 7-Zip may take over one listing or member.
pub const SEVEN_WAIT: Duration = Duration::from_secs(600);

/// 7-Zip as the host finds it, with its version.
#[derive(Clone, Debug)]
pub struct SevenZip {
    peer: Arc<Peer>,
    /// Its version, from its banner.
    pub version: Option<Version>,
}

impl SevenZip {
    /// 7-Zip, when the host finds it.
    pub fn find(peer: &Arc<Peer>) -> Result<Option<Self>, DiskError> {
        let found = match host::find(peer, "7z") {
            Ok(found) => found,
            Err(error) if host::not_offered(&error) => None,
            Err(error) => return Err(error),
        };
        if found.is_none() {
            return Ok(None);
        }
        let banner = host::run(
            peer,
            &Run {
                tool: "7z",
                timeout: Some(Duration::from_secs(10)),
                max_output: Some(64 * 1024),
                ..Run::default()
            },
        )?;
        let text = String::from_utf8_lossy(&banner.stdout);
        Ok(Some(Self {
            peer: Arc::clone(peer),
            version: Version::from_banner(&text),
        }))
    }

    fn run(
        &self,
        args: Vec<String>,
        max: u64,
        stdin: Option<String>,
    ) -> Result<host::Ran, DiskError> {
        host::run(
            &self.peer,
            &Run {
                tool: "7z",
                args,
                stdin,
                timeout: Some(SEVEN_WAIT),
                max_output: Some(max),
            },
        )
    }

    /// Lists an image file 7-Zip can read, by its path; `None` when 7-Zip
    /// cannot open it as anything.
    pub fn list(&self, image: &str) -> Result<Option<(Tree, String)>, DiskError> {
        let ran = self.run(
            vec![
                "l".into(),
                "-slt".into(),
                "-spd".into(),
                "-sccUTF-8".into(),
                "-y".into(),
                "--".into(),
                image.into(),
            ],
            MAX_LISTING,
            Some(String::new()),
        )?;
        if ran.truncated {
            return Err(DiskError::Refused(
                "its 7-Zip listing is over 7 MB, more than one answer holds".to_owned(),
            ));
        }
        let text = String::from_utf8_lossy(&ran.stdout);
        let kind = text
            .lines()
            .find_map(|l| l.trim_end_matches('\r').strip_prefix("Type = "))
            .map(str::to_owned);
        let Some(kind) = kind else {
            let all = format!("{text}{}", ran.stderr);
            if all.contains("Wrong password") || all.contains("encrypted") {
                return Err(DiskError::Needs {
                    cause: cause::PASSWORD.to_owned(),
                    hint: String::new(),
                });
            }
            return Ok(None);
        };
        Ok(Some((Tree::from_items(&parse_slt(&text)), kind)))
    }
}

/// A file system read through 7-Zip: its listing at open, a member when
/// first read.
pub struct SevenZipFs {
    seven: SevenZip,
    image: String,
    kind: String,
    tree: Tree,
    spool: Option<Arc<Spool>>,
    held: u64,
    unpacked: HashMap<Vec<u8>, PathBuf>,
}

impl SevenZipFs {
    /// The file system 7-Zip lists in `image` (a path the host's 7-Zip
    /// reads), or `None` when it opens nothing there.
    pub fn open(
        seven: SevenZip,
        image: String,
        spool: Option<Arc<Spool>>,
        held: u64,
    ) -> Result<Option<Self>, DiskError> {
        let Some((tree, kind)) = seven.list(&image)? else {
            return Ok(None);
        };
        // A container 7-Zip opens and finds nothing in (a QCOW2 whose data
        // is in its backing file) is not read: qemu-img may do better.
        if tree.members.is_empty() && tree.folders.values().all(Vec::is_empty) {
            return Ok(None);
        }
        Ok(Some(Self {
            seven,
            image,
            kind,
            tree,
            spool,
            held,
            unpacked: HashMap::new(),
        }))
    }

    fn member_file(&mut self, key: &[u8], size: u64) -> Result<PathBuf, DiskError> {
        if let Some(path) = self.unpacked.get(key) {
            return Ok(path.clone());
        }
        let member = self
            .tree
            .members
            .get(key)
            .cloned()
            .ok_or(DiskError::NotFound)?;
        let spool = self
            .spool
            .clone()
            .ok_or_else(|| DiskError::Io("the host named no spool folder".to_owned()))?;
        spool.reserve(size, self.held)?;
        let path = if size <= MAX_PIPED {
            let ran = self.seven.run(
                vec![
                    "x".into(),
                    "-so".into(),
                    "-spd".into(),
                    "-y".into(),
                    "--".into(),
                    self.image.clone(),
                    member,
                ],
                MAX_PIPED + 1,
                Some(String::new()),
            )?;
            if ran.code != Some(0) || ran.truncated {
                return Err(seven_failed(&ran.stderr));
            }
            let (path, mut file) = spool.create("member")?;
            std::io::Write::write_all(&mut file, &ran.stdout)
                .map_err(|e| DiskError::Io(e.to_string()))?;
            path
        } else {
            match self.seven.version {
                Some(v) if v >= Version::EXTRACT => {}
                Some(v) => {
                    return Err(DiskError::Needs {
                        cause: cause::old_7zip(&v.words(), &Version::EXTRACT.words()),
                        hint: cause::GET_7ZIP.to_owned(),
                    });
                }
                None => {
                    return Err(DiskError::uses("a 7-Zip whose version it does not say"));
                }
            }
            let folder = spool.folder("unpacked")?;
            let ran = self.seven.run(
                vec![
                    "x".into(),
                    "-spd".into(),
                    "-y".into(),
                    format!("-o{}", folder.display()),
                    "--".into(),
                    self.image.clone(),
                    member.clone(),
                ],
                64 * 1024,
                Some(String::new()),
            )?;
            if ran.code != Some(0) {
                return Err(seven_failed(&ran.stderr));
            }
            let mut path = folder.clone();
            for part in member.split(['/', '\\']).filter(|p| !p.is_empty()) {
                path.push(part);
            }
            // What 7-Zip wrote must be a plain file inside the folder.
            let meta =
                std::fs::symlink_metadata(&path).map_err(|e| DiskError::Io(e.to_string()))?;
            if !meta.is_file() || !path.starts_with(&folder) {
                return Err(DiskError::Refused(
                    "7-Zip did not unpack a plain file".to_owned(),
                ));
            }
            path
        };
        self.unpacked.insert(key.to_vec(), path.clone());
        Ok(path)
    }
}

fn seven_failed(stderr: &str) -> DiskError {
    if stderr.contains("Wrong password") || stderr.contains("encrypted") {
        return DiskError::Needs {
            cause: cause::PASSWORD.to_owned(),
            hint: String::new(),
        };
    }
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("7-Zip stopped");
    DiskError::Io(format!("7-Zip: {}", crate::error::lowercase_first(first)))
}

impl FileSystem for SevenZipFs {
    fn facts(&self) -> Facts {
        Facts {
            words: format!("{}, read with 7-Zip", self.kind),
            label: None,
            case_sensitive: true,
            free: None,
        }
    }

    fn list(&mut self, path: &[Vec<u8>], _folder: &Node) -> Result<Vec<Node>, DiskError> {
        let key = path.join(&b'/');
        self.tree
            .folders
            .get(&key)
            .cloned()
            .ok_or(DiskError::NotFound)
    }

    fn read(
        &mut self,
        path: &[Vec<u8>],
        node: &Node,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, DiskError> {
        let size = node.size.unwrap_or(0);
        if offset >= size {
            return Ok(Vec::new());
        }
        let key = path.join(&b'/');
        let file = self.member_file(&key, size)?;
        let mut file = File::open(file).map_err(|e| DiskError::Io(e.to_string()))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| DiskError::Io(e.to_string()))?;
        let want = usize::try_from((size - offset).min(length as u64)).unwrap_or(0);
        let mut out = vec![0u8; want];
        let mut done = 0;
        while done < want {
            match file
                .read(&mut out[done..])
                .map_err(|e| DiskError::Io(e.to_string()))?
            {
                0 => break,
                n => done += n,
            }
        }
        out.truncate(done);
        Ok(out)
    }
}

/// Says once an image that its listing came from 7-Zip.
pub fn announce_read_with_7zip(peer: &Peer) {
    host::announce(peer, lines::READ_WITH_7ZIP, "status");
}
