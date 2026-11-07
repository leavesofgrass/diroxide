//! The pins of the helper files a publisher makes (alpha 4, R30 commit
//! 109; the owner's answers 116 and 117).
//!
//! A helper's file in `packaging/components/components.toml` has the
//! publisher's own address and no `pack`: diroxide hosts no copy. Its size
//! and SHA-256 come from [`PINS`], which `cargo xtask components --pin
//! ID` writes, and which nobody types. The pin job runs it on CI's runner
//! (`.github/workflows/pin.yml`, by hand and once a month): for each file
//! it
//!
//! 1. downloads the file from its address into a work folder, with the
//!    project's neutral User-Agent and nothing else;
//! 2. compares the bytes with the publisher's own digest where one is
//!    shown: GitHub's for a release asset (read from GitHub's API, with
//!    the job's token when it has one, and the token goes to that API
//!    alone), or a line of the publisher's checksums file (`checksums`);
//!    a digest that differs, or one that cannot be read, fails the pin;
//! 3. unpacks it with the components crate's own `unpack_in`, the file's
//!    `keep`, `rename` and `strip` applied, says each member left out with
//!    why (spike T1 for the archives no lane downloaded), and fails the pin
//!    when one of its programs is not among what was unpacked;
//! 4. writes the size, the SHA-256, the bytes the kept members unpack to
//!    and what was compared into [`PINS`].
//!
//! A pin applies to a file while the component, the file's name and its
//! address are the same, so a new version (a new address) needs a new
//! pin. `cargo xtask components --addresses` asks each pinned address for
//! its headers and fails when one answers 404 or 410: BtbN keeps a daily
//! FFmpeg build 14 days and the last of each month two years, and
//! exiftool.org keeps only its newest version.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use diroxide_components::messages;
use diroxide_components::pin::{is_plain_path, is_sha256};
use diroxide_components::receipt::Receipt;
use diroxide_components::unpack::is_archive;
use diroxide_components::{
    Action, Component, Fetcher, FilePin, Request, USER_AGENT, hash_file, is_https, is_plain_name,
    unpack_in,
};
use diroxide_core::Secret;
use diroxide_vfs::{CancelToken, OpContext};
use serde::Deserialize;

/// The pin file, from the root.
pub const PINS: &str = "packaging/components/pins.toml";

/// The most bytes one pinned file may have.
pub const MAX_FILE: u64 = 2 << 30;

/// The most bytes a digest page or a checksums file may have.
const MAX_TEXT: u64 = 16 << 20;

/// The format of the pin file.
const FORMAT: u32 = 1;

/// One file's pin, as the pin file keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The component's id.
    pub component: String,
    /// The file's name.
    pub name: String,
    /// The address it was downloaded from.
    pub url: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
    /// The bytes its kept members unpack to; 0 for a file placed as it is.
    #[serde(default)]
    pub unpacked: u64,
    /// What it was compared with, in words: "GitHub's digest".
    #[serde(default)]
    pub checked: String,
}

impl Pin {
    /// Whether the pin applies to this file of this component at this
    /// address.
    pub fn applies(&self, component: &str, name: &str, url: &str) -> bool {
        self.component == component && self.name == name && self.url == url
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinsFile {
    format: u32,
    #[serde(default)]
    pin: Vec<Pin>,
}

/// Reads the pin file at `path`; a file that is not there holds no pins.
/// Every pin is checked: plain names, an `https` address, a size above
/// zero, 64 hex digits.
pub fn read(path: &Path) -> Result<Vec<Pin>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("{PINS} could not be read")),
    };
    let file: PinsFile =
        toml::from_str(&text).with_context(|| format!("{PINS} is not as expected"))?;
    if file.format != FORMAT {
        bail!("{PINS} has format {}, not {FORMAT}", file.format);
    }
    for pin in &file.pin {
        if !is_plain_name(&pin.component)
            || !is_plain_name(&pin.name)
            || !is_https(&pin.url)
            || pin.size == 0
            || !is_sha256(&pin.sha256)
            || pin.checked.chars().any(char::is_control)
        {
            bail!(
                "{PINS}: the pin of {} {} is not as expected",
                pin.component,
                pin.name
            );
        }
    }
    Ok(file.pin)
}

/// The pin file's text: the pins sorted by component and name, every
/// field in a fixed order, so the file changes only when a pin does.
pub fn to_text(pins: &[Pin]) -> String {
    let mut pins = pins.to_vec();
    pins.sort_by(|a, b| (&a.component, &a.name).cmp(&(&b.component, &b.name)));
    let mut out = String::new();
    out.push_str("# The pins of the helper files their publishers make (plan section 5.14,\n");
    out.push_str("# the owner's answers 116 and 117). Written by cargo xtask components\n");
    out.push_str("# --pin, which the pin job runs on CI's runner\n");
    out.push_str("# (.github/workflows/pin.yml); never typed. A pin applies while the\n");
    out.push_str("# component, the file's name and its address are the same.\n");
    out.push_str(&format!("format = {FORMAT}\n"));
    for pin in &pins {
        out.push_str("\n[[pin]]\n");
        out.push_str(&format!("component = {}\n", quoted(&pin.component)));
        out.push_str(&format!("name = {}\n", quoted(&pin.name)));
        out.push_str(&format!("url = {}\n", quoted(&pin.url)));
        out.push_str(&format!("size = {}\n", pin.size));
        out.push_str(&format!("sha256 = {}\n", quoted(&pin.sha256)));
        out.push_str(&format!("unpacked = {}\n", pin.unpacked));
        out.push_str(&format!("checked = {}\n", quoted(&pin.checked)));
    }
    out
}

/// A TOML basic string, with `"` and `\` escaped; control characters
/// never reach here.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Puts `pin` in `pins` in place of the pin for the same file of the same
/// component.
pub fn replace(pins: &mut Vec<Pin>, pin: Pin) {
    pins.retain(|p| !(p.component == pin.component && p.name == pin.name));
    pins.push(pin);
}

/// One file to pin: the component as the list will read it, holding this
/// file alone (its size and hash not known yet), its address, and the
/// publisher's checksums file when there is one.
pub struct Wanted {
    /// The component, with `files` holding this one file.
    pub component: Component,
    /// The publisher's checksums file, an `https` address.
    pub checksums: Option<String>,
}

impl Wanted {
    fn file(&self) -> &FilePin {
        &self.component.files[0]
    }
}

/// Pins files: the fetcher that downloads, the token for GitHub's API,
/// and the work folder the files go to while they are checked.
pub struct Pinner<'a> {
    /// Fetches every address.
    pub fetcher: &'a dyn Fetcher,
    /// The job's token, sent to GitHub's API alone.
    pub token: Option<&'a Secret>,
    /// Where each file is downloaded and unpacked: a folder of its own
    /// per component under it.
    pub work: &'a Path,
}

impl Pinner<'_> {
    /// Downloads, compares and unpacks one file, saying each step in
    /// `lines`; the pin, or the line that says why there is none.
    pub fn pin(&self, wanted: &Wanted, lines: &mut Vec<String>) -> Result<Pin, String> {
        let file = wanted.file();
        let id = &wanted.component.id;
        let name = &file.name;
        let url = file
            .url
            .as_deref()
            .ok_or_else(|| format!("{id}: {name} has no address"))?;
        let folder = self.work.join(id);
        fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let path = folder.join(name);
        self.download(url, &path)
            .map_err(|reason| format!("{id}: {name} not downloaded: {reason}"))?;
        let size = fs::metadata(&path).map_or(0, |m| m.len());
        let sha256 = hash_file(&path).map_err(|e| format!("{id}: {name}: {e}"))?;
        let checked = self.compare(url, name, wanted.checksums.as_deref(), size, &sha256)?;
        lines.push(format!("{id}: {name}, {size} bytes, {checked}."));
        let unpacked = self.unpack(wanted, &folder, size, &sha256, lines)?;
        Ok(Pin {
            component: id.clone(),
            name: name.clone(),
            url: url.to_owned(),
            size,
            sha256,
            unpacked,
            checked,
        })
    }

    /// Downloads `url` to `path`, at most [`MAX_FILE`] bytes.
    fn download(&self, url: &str, path: &Path) -> Result<(), String> {
        let cancel = CancelToken::new();
        let request = Request {
            address: url,
            from: 0,
            if_range: None,
            limit: MAX_FILE,
            token: None,
            cancel: &cancel,
        };
        let fetched = self.fetcher.open(&request).map_err(|e| e.reason())?;
        let mut out = File::create(path).map_err(|e| e.to_string())?;
        let copied = io::copy(&mut fetched.reader.take(MAX_FILE + 1), &mut out)
            .map_err(|e| diroxide_components::FetchError::from_io(&e).reason())?;
        out.flush().map_err(|e| e.to_string())?;
        if copied > MAX_FILE {
            return Err("it is larger than 2 GB".to_owned());
        }
        Ok(())
    }

    /// The text at `url`, at most [`MAX_TEXT`] bytes.
    fn text(&self, url: &str, token: Option<&Secret>) -> Result<String, String> {
        let cancel = CancelToken::new();
        let request = Request {
            address: url,
            from: 0,
            if_range: None,
            limit: MAX_TEXT,
            token,
            cancel: &cancel,
        };
        let fetched = self.fetcher.open(&request).map_err(|e| e.reason())?;
        let mut bytes = Vec::new();
        fetched
            .reader
            .take(MAX_TEXT)
            .read_to_end(&mut bytes)
            .map_err(|e| diroxide_components::FetchError::from_io(&e).reason())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Compares the file with the publisher's digest, where one is shown;
    /// the words for the pin, or the line that fails it.
    fn compare(
        &self,
        url: &str,
        name: &str,
        checksums: Option<&str>,
        size: u64,
        sha256: &str,
    ) -> Result<String, String> {
        if let Some(release) = GithubRelease::parse(url) {
            let api = release.api();
            let text = self
                .text(&api, self.token)
                .map_err(|reason| format!("{name}: GitHub's digest not read: {reason}"))?;
            let shown = github_digest(&text, &release.asset)
                .map_err(|reason| format!("{name}: GitHub's digest not read: {reason}"))?;
            return match shown {
                None => Ok("GitHub shows no digest".to_owned()),
                Some((digest, shown_size)) if digest == sha256 && shown_size == size => {
                    Ok("GitHub's digest matches".to_owned())
                }
                Some(_) => Err(format!("{name} does not match GitHub's digest")),
            };
        }
        if let Some(address) = checksums {
            let text = self
                .text(address, None)
                .map_err(|reason| format!("{name}: the checksums file not read: {reason}"))?;
            return match listed_sha256(&text, name) {
                Some(digest) if digest == sha256 => {
                    Ok("the publisher's checksum matches".to_owned())
                }
                Some(_) => Err(format!("{name} does not match the publisher's checksum")),
                None => Err(format!("{name} is not in the publisher's checksums file")),
            };
        }
        Ok("no digest from the publisher".to_owned())
    }

    /// Unpacks an archive the component unpacks, as the program will:
    /// says each member left out, fails when a program is missing, and
    /// gives the bytes written.
    fn unpack(
        &self,
        wanted: &Wanted,
        folder: &Path,
        size: u64,
        sha256: &str,
        lines: &mut Vec<String>,
    ) -> Result<u64, String> {
        let mut component = wanted.component.clone();
        let id = component.id.clone();
        let file = &mut component.files[0];
        let name = file.name.clone();
        file.size = size;
        file.sha256 = sha256.to_owned();
        let pin = file.clone();
        if component.action != Action::Unpack || !is_archive(&name) {
            let missing = pin.programs().find(|p| *p != name);
            return match missing {
                Some(program) => Err(format!(
                    "{id}: {name} is not an archive, so {program} is not in it"
                )),
                None => Ok(0),
            };
        }
        component.unpacked_size = MAX_FILE * 4;
        let left_out = unpack_in(&component, folder, &OpContext::new())
            .map_err(|e| format!("{id}: {name} not unpacked: {e}"))?;
        for (member, why) in &left_out {
            lines.push(format!("  {}", messages::left_out(member, why)));
        }
        let receipt =
            Receipt::read(folder, &pin).ok_or_else(|| format!("{id}: {name} left no receipt"))?;
        let unpacked: u64 = receipt.files.iter().map(|f| f.size).sum();
        lines.push(format!(
            "  Kept {} files, {unpacked} bytes.",
            receipt.files.len()
        ));
        for program in pin.programs() {
            let there = is_plain_path(program) && receipt.files.iter().any(|f| f.path == program);
            if !there {
                return Err(format!(
                    "{id}: {name} holds no {program} after keep and rename"
                ));
            }
        }
        Ok(unpacked)
    }
}

/// A GitHub release asset's address, in parts.
#[derive(Debug, PartialEq, Eq)]
pub struct GithubRelease {
    owner: String,
    repo: String,
    tag: String,
    asset: String,
}

impl GithubRelease {
    /// `https://github.com/OWNER/REPO/releases/download/TAG/ASSET`, read.
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("https://github.com/")?;
        let parts: Vec<&str> = rest.split('/').collect();
        let [owner, repo, "releases", "download", tag, asset] = parts.as_slice() else {
            return None;
        };
        let fine = |part: &str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && !matches!(b, b'?' | b'#' | b'%'))
        };
        [owner, repo, tag, asset]
            .iter()
            .all(|p| fine(p))
            .then(|| Self {
                owner: (*owner).to_owned(),
                repo: (*repo).to_owned(),
                tag: (*tag).to_owned(),
                asset: (*asset).to_owned(),
            })
    }

    /// The API address of its release.
    pub fn api(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            self.owner, self.repo, self.tag
        )
    }
}

/// The digest and size GitHub's release page shows for `asset`: `None`
/// when it shows no digest; an error when the page is not a release or
/// does not list the asset.
pub fn github_digest(json: &str, asset: &str) -> Result<Option<(String, u64)>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "the answer is not GitHub's release".to_owned())?;
    let assets = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "the answer is not GitHub's release".to_owned())?;
    let found = assets
        .iter()
        .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some(asset))
        .ok_or_else(|| format!("the release does not list {asset}"))?;
    let size = found
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(found
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|d| is_sha256(d))
        .map(|d| (d.to_owned(), size)))
}

/// The SHA-256 a checksums file gives for `name`: the 64-hex-digit word on
/// a line that names the file, in any of the usual forms
/// (`SHA2-256(NAME)= HEX`, `SHA256 (NAME) = HEX`, `HEX  NAME`).
pub fn listed_sha256(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let names_it = line
            .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '*' | '='))
            .any(|word| word == name);
        if !names_it {
            return None;
        }
        line.split(|c: char| !c.is_ascii_hexdigit())
            .find(|word| word.len() == 64)
            .map(str::to_ascii_lowercase)
    })
}

/// What one address answered to a request for its headers.
pub type Probe = dyn Fn(&str) -> Result<u16, String>;

/// Asks each pinned address for its headers, through `probe`. A 404 or a
/// 410 fails; any other trouble is said and not counted. Returns the
/// lines and whether it passed.
pub fn check_addresses(pins: &[Pin], probe: &Probe) -> (Vec<String>, bool) {
    if pins.is_empty() {
        return (
            vec!["components: Pass. No address is pinned yet.".to_owned()],
            true,
        );
    }
    let mut lines = Vec::new();
    let mut gone = 0;
    for pin in pins {
        match probe(&pin.url) {
            Ok(404 | 410) => {
                gone += 1;
                lines.push(format!(
                    "components: Fail. {} is gone (404): {}",
                    pin.name, pin.url
                ));
            }
            Ok(code) if (200..400).contains(&code) => {}
            Ok(code) => lines.push(format!(
                "components: Note. {} answers {code}; not counted.",
                pin.name
            )),
            Err(reason) => lines.push(format!(
                "components: Note. {} not reached: {reason}; not counted.",
                pin.name
            )),
        }
    }
    let passed = gone == 0;
    let total = pins.len();
    let summary = match (passed, total, gone) {
        (true, 1, _) => "components: Pass. The pinned address answers.".to_owned(),
        (true, _, _) => format!("components: Pass. All {total} pinned addresses answer."),
        (false, _, 1) => format!("components: Fail. 1 of {total} pinned addresses is gone."),
        (false, _, _) => format!("components: Fail. {gone} of {total} pinned addresses are gone."),
    };
    lines.insert(0, summary);
    (lines, passed)
}

/// Asks `url` for its headers with curl, following redirects over HTTPS
/// only, with the project's User-Agent: the last answer's status.
pub fn curl_probe(url: &str) -> Result<u16, String> {
    let output = Command::new("curl")
        .args([
            "-q",
            "--silent",
            "--show-error",
            "--location",
            "--head",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            "60",
            "--user-agent",
            USER_AGENT,
            "--write-out",
            "\n%{http_code}",
            url,
        ])
        .output()
        .map_err(|e| format!("curl could not be run: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let code = text
        .lines()
        .last()
        .and_then(|line| line.trim().parse::<u16>().ok())
        .filter(|code| *code > 0);
    match code {
        Some(code) => Ok(code),
        None => Err(String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("no answer")
            .trim()
            .to_owned()),
    }
}

/// The work folder for pins under the build output: `<target>/components/pins`.
pub fn work_folder(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("components")
        .join("pins")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use diroxide_components::fake::{FakeAnswer, FakeFetcher};
    use diroxide_components::{Kind, sha256_hex};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    const ASSET: &str = "https://github.com/example/tool/releases/download/v1.2/tool-1.2.zip";
    const API: &str = "https://api.github.com/repos/example/tool/releases/tags/v1.2";

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn tool_zip() -> Vec<u8> {
        zip_of(&[
            ("tool-1.2/bin/tool", b"program"),
            ("tool-1.2/bin/tool(1).dat", b"odd"),
            ("tool-1.2/doc/manual.html", b"manual"),
            ("tool-1.2/LICENSE", b"MIT"),
        ])
    }

    fn wanted(url: &str, checksums: Option<&str>) -> Wanted {
        let mut pin = FilePin::new("tool-1.2.zip", 1, "0".repeat(64));
        pin.url = Some(url.to_owned());
        pin.strip = 1;
        pin.keep = vec!["bin/*".into(), "LICENSE".into()];
        pin.exec = Some("bin/tool".into());
        let mut component = Component::new("tool", "Tool", vec![pin]);
        component.kind = Kind::Helper;
        component.action = Action::Unpack;
        Wanted {
            component,
            checksums: checksums.map(str::to_owned),
        }
    }

    fn release_json(digest: &str, size: usize) -> String {
        format!(
            r#"{{"tag_name": "v1.2", "assets": [
                {{"name": "other.zip", "size": 3, "digest": "sha256:{}"}},
                {{"name": "tool-1.2.zip", "size": {size}, "digest": "{digest}"}}
            ]}}"#,
            "a".repeat(64)
        )
    }

    #[test]
    fn a_release_asset_is_pinned_against_githubs_digest_and_unpacked() {
        let bytes = tool_zip();
        let fake = FakeFetcher::new();
        fake.serve(ASSET, FakeAnswer::Bytes(bytes.clone()));
        let digest = format!("sha256:{}", sha256_hex(&bytes));
        fake.serve(
            API,
            FakeAnswer::Bytes(release_json(&digest, bytes.len()).into_bytes()),
        );
        let work = tempfile::tempdir().unwrap();
        let token = Secret::new("job-token".to_owned());
        let pinner = Pinner {
            fetcher: &fake,
            token: Some(&token),
            work: work.path(),
        };
        let mut lines = Vec::new();
        let pin = pinner.pin(&wanted(ASSET, None), &mut lines).unwrap();
        assert_eq!(pin.size, bytes.len() as u64);
        assert_eq!(pin.sha256, sha256_hex(&bytes));
        assert_eq!(pin.checked, "GitHub's digest matches");
        assert_eq!(pin.unpacked, 10, "program and MIT");
        assert_eq!(
            lines,
            [
                format!(
                    "tool: tool-1.2.zip, {} bytes, GitHub's digest matches.",
                    bytes.len()
                ),
                "  Left out tool-1.2/bin/tool(1).dat: tool(1).dat is not a plain name.".to_owned(),
                "  Kept 2 files, 10 bytes.".to_owned(),
            ]
        );
        // The token went to GitHub's API alone, never to the file.
        let requests = fake.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            requests.iter().all(|r| r.token == (r.address == API)),
            "{requests:?}"
        );
    }

    #[test]
    fn a_digest_that_differs_or_a_missing_program_gives_no_pin() {
        let bytes = tool_zip();
        let fake = FakeFetcher::new();
        fake.serve(ASSET, FakeAnswer::Bytes(bytes.clone()));
        let wrong = format!("sha256:{}", "b".repeat(64));
        fake.serve(
            API,
            FakeAnswer::Bytes(release_json(&wrong, bytes.len()).into_bytes()),
        );
        let work = tempfile::tempdir().unwrap();
        let pinner = Pinner {
            fetcher: &fake,
            token: None,
            work: work.path(),
        };
        let error = pinner
            .pin(&wanted(ASSET, None), &mut Vec::new())
            .unwrap_err();
        assert_eq!(error, "tool-1.2.zip does not match GitHub's digest");
        // GitHub shows no digest: said, and the program must still be there.
        fake.serve(
            API,
            FakeAnswer::Bytes(release_json("", bytes.len()).into_bytes()),
        );
        let mut no_program = wanted(ASSET, None);
        no_program.component.files[0].exec = Some("bin/other".into());
        let error = pinner.pin(&no_program, &mut Vec::new()).unwrap_err();
        assert_eq!(
            error,
            "tool: tool-1.2.zip holds no bin/other after keep and rename"
        );
        let pin = pinner.pin(&wanted(ASSET, None), &mut Vec::new()).unwrap();
        assert_eq!(pin.checked, "GitHub shows no digest");
        // An address that answers 404 is said.
        let gone = "https://github.com/example/tool/releases/download/v1.2/gone.zip";
        let error = pinner
            .pin(&wanted(gone, None), &mut Vec::new())
            .unwrap_err();
        assert_eq!(
            error,
            "tool: tool-1.2.zip not downloaded: the file is not there"
        );
    }

    #[test]
    fn another_publisher_is_compared_with_its_checksums_file() {
        let bytes = tool_zip();
        let address = "https://downloads.example.invalid/tool-1.2.zip/download";
        let sums = "https://example.invalid/checksums-1.2.txt";
        let fake = FakeFetcher::new();
        fake.serve(address, FakeAnswer::Bytes(bytes.clone()));
        let text = format!(
            "SHA1(tool-1.2.zip)= {}\nSHA2-256(tool-1.2.zip)= {}\nMD5 (tool-1.2.zip) = {}\n",
            "c".repeat(40),
            sha256_hex(&bytes),
            "d".repeat(32)
        );
        fake.serve(sums, FakeAnswer::Bytes(text.into_bytes()));
        let work = tempfile::tempdir().unwrap();
        let pinner = Pinner {
            fetcher: &fake,
            token: None,
            work: work.path(),
        };
        let pin = pinner
            .pin(&wanted(address, Some(sums)), &mut Vec::new())
            .unwrap();
        assert_eq!(pin.checked, "the publisher's checksum matches");
        let pin = pinner.pin(&wanted(address, None), &mut Vec::new()).unwrap();
        assert_eq!(pin.checked, "no digest from the publisher");
    }

    #[test]
    fn digests_are_read_in_their_usual_forms() {
        let hex = "ab75b37515385a753aa60a58f68ffffe1ff81e43b319df9540978fb19e0580e5";
        let name = "exiftool-13.61_64.zip";
        for text in [
            format!("SHA2-256({name})= {hex}"),
            format!("SHA256 ({name}) = {hex}"),
            format!("{hex}  {name}"),
            format!("{hex} *{name}"),
        ] {
            assert_eq!(listed_sha256(&text, name).as_deref(), Some(hex), "{text}");
        }
        assert_eq!(listed_sha256(&format!("{hex}  other.zip"), name), None);
        let sha1 = format!("SHA1({name})= {}", "c".repeat(40));
        assert_eq!(listed_sha256(&sha1, name), None);
        let release = GithubRelease::parse(ASSET).unwrap();
        assert_eq!(release.api(), API);
        for bad in [
            "https://github.com/example/tool/archive/v1.2.zip",
            "https://example.invalid/example/tool/releases/download/v1.2/a.zip",
            "https://github.com/example/tool/releases/download/v1.2/a.zip?x=1",
        ] {
            assert_eq!(GithubRelease::parse(bad), None, "{bad}");
        }
        assert!(github_digest("not json", "a").is_err());
        assert_eq!(
            github_digest(r#"{"assets": []}"#, "a").unwrap_err(),
            "the release does not list a"
        );
    }

    #[test]
    fn pins_write_back_the_same_and_replace_their_own() {
        let pin = |component: &str, name: &str, size: u64| Pin {
            component: component.to_owned(),
            name: name.to_owned(),
            url: format!("https://example.invalid/{name}"),
            size,
            sha256: "e".repeat(64),
            unpacked: 7,
            checked: "GitHub's digest matches".to_owned(),
        };
        let mut pins = vec![pin("tool", "b.zip", 1), pin("tool", "a.zip", 2)];
        replace(&mut pins, pin("tool", "b.zip", 3));
        assert_eq!(pins.len(), 2);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pins.toml");
        fs::write(&path, to_text(&pins)).unwrap();
        let read_back = read(&path).unwrap();
        assert_eq!(read_back[0], pin("tool", "a.zip", 2), "sorted by name");
        assert_eq!(read_back[1], pin("tool", "b.zip", 3));
        assert!(read_back[1].applies("tool", "b.zip", "https://example.invalid/b.zip"));
        assert!(!read_back[1].applies("tool", "b.zip", "https://example.invalid/new/b.zip"));
        assert!(read(&dir.path().join("none.toml")).unwrap().is_empty());
        fs::write(&path, to_text(&pins).replace("size = 3", "size = 0")).unwrap();
        assert!(read(&path).is_err(), "a pin with no size");
    }

    #[test]
    fn only_a_gone_address_fails() {
        let pins: Vec<Pin> = ["a.zip", "b.zip", "c.zip", "d.zip"]
            .iter()
            .map(|name| Pin {
                component: "tool".to_owned(),
                name: (*name).to_owned(),
                url: format!("https://example.invalid/{name}"),
                size: 1,
                sha256: "e".repeat(64),
                unpacked: 0,
                checked: String::new(),
            })
            .collect();
        let answers = |url: &str| -> Result<u16, String> {
            match url.rsplit('/').next() {
                Some("a.zip") => Ok(200),
                Some("b.zip") => Ok(503),
                Some("c.zip") => Err("could not connect".to_owned()),
                _ => Ok(404),
            }
        };
        let (lines, passed) = check_addresses(&pins, &answers);
        assert!(!passed);
        assert_eq!(
            lines,
            [
                "components: Fail. 1 of 4 pinned addresses is gone.",
                "components: Note. b.zip answers 503; not counted.",
                "components: Note. c.zip not reached: could not connect; not counted.",
                "components: Fail. d.zip is gone (404): https://example.invalid/d.zip",
            ]
        );
        let (lines, passed) = check_addresses(&pins[..1], &answers);
        assert!(passed);
        assert_eq!(lines, ["components: Pass. The pinned address answers."]);
        let (lines, _) = check_addresses(&[pins[0].clone(), pins[0].clone()], &answers);
        assert_eq!(lines, ["components: Pass. All 2 pinned addresses answer."]);
        let (lines, passed) = check_addresses(&[], &answers);
        assert!(passed);
        assert_eq!(lines, ["components: Pass. No address is pinned yet."]);
    }
}
