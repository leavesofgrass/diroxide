//! Open with on macOS (R30's commit 28), without Launch Services: the
//! application folders' bundles are scanned, each `Contents/Info.plist`
//! read for the document types it declares (`CFBundleDocumentTypes`: its
//! extensions, media types and content types), and the chosen one is
//! started with `open -a BUNDLE FILE`.
//!
//! The property lists are read as XML by hand, with no crate; a binary
//! property list (`bplist00`) is skipped and counted, so the list can say
//! how many programs it could not read. A document type whose role is
//! `None` or whose rank is `None` is left out. The system's own default is
//! not read here (it lives in Launch Services' binary preferences): the
//! "Default program" row, the system opener, stands for it.
//!
//! This file is compiled on macOS and in the tests everywhere; on macOS
//! itself it has not been tried by a person yet.

use std::path::{Path, PathBuf};

use super::open_with::{ArgPart, Launch, OpenCommand, OpenWithApp, OpenWithList};

/// How deep a property list may nest before it is refused.
const MAX_DEPTH: usize = 64;

/// The largest `Info.plist` read.
const MAX_PLIST_BYTES: u64 = 4 * 1024 * 1024;

/// A property list value, as far as Open with needs one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    /// `<dict>`: keys and values in order.
    Dict(Vec<(String, Value)>),
    /// `<array>`.
    Array(Vec<Value>),
    /// `<string>`.
    Text(String),
    /// `<true/>` or `<false/>`.
    Bool(bool),
    /// Anything else (`<integer>`, `<real>`, `<date>`, `<data>`).
    Other,
}

impl Value {
    /// A dictionary's value for a key.
    fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A string's text.
    fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    /// An array of strings' texts, lowercase.
    fn texts(&self) -> Vec<String> {
        match self {
            Value::Array(items) => items
                .iter()
                .filter_map(Value::text)
                .map(str::to_lowercase)
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Why a property list was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlistError {
    /// A binary property list, which this reader does not read.
    Binary,
    /// Not a property list it can read.
    Unreadable,
}

/// XML text with its five entities and character references undone.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(end) = after.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A reader over a property list's XML: tags and the text between them.
struct Reader<'a> {
    text: &'a str,
    at: usize,
}

/// One piece of the XML.
#[derive(Debug, PartialEq, Eq)]
enum Piece<'a> {
    /// `<name>`, or `<name/>` when `empty`.
    Open { name: &'a str, empty: bool },
    /// `</name>`.
    Close(&'a str),
    /// Text between tags.
    Text(&'a str),
}

impl<'a> Reader<'a> {
    /// The next piece, skipping the declaration, the document type,
    /// comments and white space between tags; `None` at the end.
    fn next(&mut self) -> Option<Result<Piece<'a>, PlistError>> {
        loop {
            let rest = &self.text[self.at..];
            if rest.is_empty() {
                return None;
            }
            if let Some(after) = rest.strip_prefix("<!--") {
                let end = after.find("-->")?;
                self.at += 4 + end + 3;
                continue;
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = rest.find('>')?;
                self.at += end + 1;
                continue;
            }
            if let Some(after) = rest.strip_prefix('<') {
                let Some(end) = after.find('>') else {
                    return Some(Err(PlistError::Unreadable));
                };
                self.at += end + 2;
                let tag = &after[..end];
                if let Some(name) = tag.strip_prefix('/') {
                    return Some(Ok(Piece::Close(name.trim())));
                }
                let (tag, empty) = match tag.strip_suffix('/') {
                    Some(tag) => (tag, true),
                    None => (tag, false),
                };
                let name = tag.split_whitespace().next().unwrap_or("");
                return Some(Ok(Piece::Open { name, empty }));
            }
            let end = rest.find('<').unwrap_or(rest.len());
            self.at += end;
            let text = &rest[..end];
            if text.trim().is_empty() {
                continue;
            }
            return Some(Ok(Piece::Text(text)));
        }
    }

    /// The text inside an element up to its closing tag.
    fn text_until(&mut self, name: &str) -> Result<String, PlistError> {
        let rest = &self.text[self.at..];
        let end = rest.find('<').ok_or(PlistError::Unreadable)?;
        let text = unescape(&rest[..end]);
        self.at += end;
        match self.next() {
            Some(Ok(Piece::Close(close))) if close == name => Ok(text),
            _ => Err(PlistError::Unreadable),
        }
    }

    /// One value whose opening tag was just read.
    fn value(&mut self, name: &str, empty: bool, depth: usize) -> Result<Value, PlistError> {
        if depth > MAX_DEPTH {
            return Err(PlistError::Unreadable);
        }
        match (name, empty) {
            ("true", true) => return Ok(Value::Bool(true)),
            ("false", true) => return Ok(Value::Bool(false)),
            ("string", true) => return Ok(Value::Text(String::new())),
            ("dict", true) => return Ok(Value::Dict(Vec::new())),
            ("array", true) => return Ok(Value::Array(Vec::new())),
            (_, true) => return Ok(Value::Other),
            _ => {}
        }
        match name {
            "string" => self.text_until("string").map(Value::Text),
            "dict" => {
                let mut entries = Vec::new();
                loop {
                    match self.next() {
                        Some(Ok(Piece::Close("dict"))) => return Ok(Value::Dict(entries)),
                        Some(Ok(Piece::Open { name: "key", empty })) => {
                            let key = if empty {
                                String::new()
                            } else {
                                self.text_until("key")?
                            };
                            let value = match self.next() {
                                Some(Ok(Piece::Open { name, empty })) => {
                                    self.value(name, empty, depth + 1)?
                                }
                                _ => return Err(PlistError::Unreadable),
                            };
                            entries.push((key, value));
                        }
                        _ => return Err(PlistError::Unreadable),
                    }
                }
            }
            "array" => {
                let mut items = Vec::new();
                loop {
                    match self.next() {
                        Some(Ok(Piece::Close("array"))) => return Ok(Value::Array(items)),
                        Some(Ok(Piece::Open { name, empty })) => {
                            items.push(self.value(name, empty, depth + 1)?);
                        }
                        _ => return Err(PlistError::Unreadable),
                    }
                }
            }
            other => {
                // An element this reader does not need: its text skipped.
                self.text_until(other).map(|_| Value::Other)
            }
        }
    }
}

/// A property list's top value, read from its bytes.
pub(crate) fn read_plist(bytes: &[u8]) -> Result<Value, PlistError> {
    if bytes.starts_with(b"bplist") {
        return Err(PlistError::Binary);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PlistError::Unreadable)?;
    let mut reader = Reader {
        text: text.trim_start_matches('\u{feff}'),
        at: 0,
    };
    match reader.next() {
        Some(Ok(Piece::Open {
            name: "plist",
            empty: false,
        })) => {}
        _ => return Err(PlistError::Unreadable),
    }
    match reader.next() {
        Some(Ok(Piece::Open { name, empty })) => reader.value(name, empty, 0),
        _ => Err(PlistError::Unreadable),
    }
}

/// The content types a file of this extension and media type may carry,
/// the specific first: `com.adobe.pdf` for a PDF, then the family
/// (`public.image` for any picture).
fn content_types(extension: &str, mime: Option<&str>) -> Vec<&'static str> {
    let specific: &[&str] = match extension {
        "txt" | "text" | "log" => &["public.plain-text", "public.utf8-plain-text"],
        "md" | "markdown" => &["net.daringfireball.markdown"],
        "pdf" => &["com.adobe.pdf"],
        "jpg" | "jpeg" | "jpe" => &["public.jpeg"],
        "png" => &["public.png"],
        "gif" => &["com.compuserve.gif"],
        "heic" | "heif" => &["public.heic", "public.heif"],
        "tif" | "tiff" => &["public.tiff"],
        "html" | "htm" => &["public.html"],
        "rtf" => &["public.rtf"],
        "csv" => &["public.comma-separated-values-text"],
        "json" => &["public.json"],
        "xml" => &["public.xml"],
        "mp3" => &["public.mp3"],
        "m4a" => &["com.apple.m4a-audio"],
        "wav" => &["com.microsoft.waveform-audio"],
        "mp4" | "m4v" => &["public.mpeg-4"],
        "mov" => &["com.apple.quicktime-movie"],
        "zip" => &["public.zip-archive"],
        "docx" => &["org.openxmlformats.wordprocessingml.document"],
        "xlsx" => &["org.openxmlformats.spreadsheetml.sheet"],
        "pptx" => &["org.openxmlformats.presentationml.presentation"],
        "epub" => &["org.idpf.epub-container"],
        _ => &[],
    };
    let family: &[&str] = match mime.and_then(|m| m.split('/').next()) {
        Some("image") => &["public.image"],
        Some("text") => &["public.text", "public.plain-text"],
        Some("audio") => &["public.audio", "public.audiovisual-content"],
        Some("video") => &["public.movie", "public.audiovisual-content"],
        _ => &[],
    };
    specific.iter().chain(family).copied().collect()
}

/// Whether a bundle's `Info.plist` declares a document type for the file:
/// its extension, its media type, or one of its content types, in a role
/// and rank that open it.
pub(crate) fn declares(info: &Value, extension: &str, mime: Option<&str>) -> bool {
    let Some(Value::Array(types)) = info.get("CFBundleDocumentTypes") else {
        return false;
    };
    let wanted = content_types(extension, mime);
    let mime = mime.map(str::to_lowercase);
    types.iter().any(|doc| {
        let role = doc.get("CFBundleTypeRole").and_then(Value::text);
        let rank = doc.get("LSHandlerRank").and_then(Value::text);
        if role == Some("None") || rank == Some("None") {
            return false;
        }
        let by_extension = doc
            .get("CFBundleTypeExtensions")
            .is_some_and(|e| e.texts().iter().any(|x| x == extension));
        let by_mime = mime.as_ref().is_some_and(|m| {
            doc.get("CFBundleTypeMIMETypes")
                .is_some_and(|t| t.texts().contains(m))
        });
        let by_content = doc
            .get("LSItemContentTypes")
            .is_some_and(|t| t.texts().iter().any(|c| wanted.contains(&c.as_str())));
        by_extension || by_mime || by_content
    })
}

/// A bundle's shown name: `CFBundleDisplayName`, `CFBundleName`, else
/// the folder's name without `.app`.
fn bundle_name(info: &Value, bundle: &Path) -> String {
    info.get("CFBundleDisplayName")
        .and_then(Value::text)
        .or_else(|| info.get("CFBundleName").and_then(Value::text))
        .filter(|n| !n.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            bundle
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

/// The bundles in these folders (`NAME.app`, one level down, and
/// `Utilities` inside each) that declare the file's type, by name; the
/// count of bundles whose `Info.plist` is binary goes in
/// [`OpenWithList::unread`].
pub(crate) fn apps_in(folders: &[PathBuf], extension: &str, mime: Option<&str>) -> OpenWithList {
    let mut list = OpenWithList::default();
    let mut bundles = Vec::new();
    for folder in folders {
        for dir in [folder.clone(), folder.join("Utilities")] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "app") && path.is_dir() {
                    bundles.push(path);
                }
            }
        }
    }
    for bundle in bundles {
        let plist = bundle.join("Contents").join("Info.plist");
        let Ok(meta) = std::fs::metadata(&plist) else {
            continue;
        };
        if meta.len() > MAX_PLIST_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&plist) else {
            continue;
        };
        let info = match read_plist(&bytes) {
            Ok(info) => info,
            Err(PlistError::Binary) => {
                list.unread += 1;
                continue;
            }
            Err(PlistError::Unreadable) => continue,
        };
        if !declares(&info, extension, mime) {
            continue;
        }
        let id = info
            .get("CFBundleIdentifier")
            .and_then(Value::text)
            .map_or_else(|| bundle.to_string_lossy().into_owned(), str::to_owned);
        list.push(OpenWithApp {
            name: bundle_name(&info, &bundle),
            launch: Launch::Command(OpenCommand {
                program: "open".to_owned(),
                args: vec![
                    vec![ArgPart::Text("-a".to_owned())],
                    vec![ArgPart::Text(bundle.to_string_lossy().into_owned())],
                    vec![ArgPart::File],
                ],
            }),
            id,
            default: false,
        });
    }
    list.apps.sort_by_key(|a| a.name.to_lowercase());
    list
}

/// The programs this Mac's application folders declare for a file.
#[cfg(target_os = "macos")]
pub(crate) fn open_with_list(file: &Path, mime: Option<&str>) -> OpenWithList {
    let extension = file
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut folders = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        folders.push(PathBuf::from(home).join("Applications"));
    }
    apps_in(&folders, &extension, mime)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::test_support::scratch;

    const PREVIEW: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<!-- A viewer of pictures and PDF documents. -->
	<key>CFBundleIdentifier</key>
	<string>com.example.viewer</string>
	<key>CFBundleName</key>
	<string>Viewer &amp; Reader</string>
	<key>LSRequiresNativeExecution</key>
	<true/>
	<key>CFBundleVersion</key>
	<integer>42</integer>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeRole</key>
			<string>Viewer</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>com.adobe.pdf</string>
				<string>public.image</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeRole</key>
			<string>None</string>
			<key>CFBundleTypeExtensions</key>
			<array><string>txt</string></array>
		</dict>
	</array>
	<key>Empty</key>
	<string/>
</dict>
</plist>
"#;

    #[test]
    fn xml_property_lists_read_and_binary_ones_are_refused() {
        let info = read_plist(PREVIEW.as_bytes()).unwrap();
        assert_eq!(
            info.get("CFBundleName").and_then(Value::text),
            Some("Viewer & Reader")
        );
        assert_eq!(
            info.get("LSRequiresNativeExecution"),
            Some(&Value::Bool(true))
        );
        assert_eq!(info.get("CFBundleVersion"), Some(&Value::Other));
        assert_eq!(info.get("Empty").and_then(Value::text), Some(""));
        assert!(declares(&info, "pdf", Some("application/pdf")));
        assert!(declares(&info, "png", Some("image/png")), "by the family");
        assert!(!declares(&info, "txt", Some("text/plain")), "role None");
        assert_eq!(read_plist(b"bplist00\x00\x01"), Err(PlistError::Binary));
        assert_eq!(
            read_plist(b"<plist><dict><key>x</key>"),
            Err(PlistError::Unreadable)
        );
        let deep = format!(
            "<plist>{}{}</plist>",
            "<array>".repeat(100),
            "</array>".repeat(100)
        );
        assert_eq!(read_plist(deep.as_bytes()), Err(PlistError::Unreadable));
        assert_eq!(
            unescape("a &lt;b&gt; &#65;&#x42; &bogus; &"),
            "a <b> AB &bogus; &"
        );
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Application folders of their own, under the build's target folder:
    /// a viewer by content type, an editor by extension, one with a
    /// binary list (counted), one declaring nothing for PDF, and one in
    /// Utilities.
    #[test]
    fn the_application_folders_are_scanned() {
        let root = scratch();
        let apps = root.path().join("Applications");
        write(
            &apps.join("Viewer.app/Contents/Info.plist"),
            PREVIEW.as_bytes(),
        );
        let editor = PREVIEW
            .replace("com.example.viewer", "com.example.editor")
            .replace("Viewer &amp; Reader", "")
            .replace(
                "<string>com.adobe.pdf</string>",
                "<string>public.png</string>",
            )
            .replace(
                "<array><string>txt</string></array>",
                "<array><string>PDF</string></array>",
            )
            .replace("<string>None</string>", "<string>Editor</string>");
        write(
            &apps.join("Utilities/Editor.app/Contents/Info.plist"),
            editor.as_bytes(),
        );
        write(
            &apps.join("Packed.app/Contents/Info.plist"),
            b"bplist00\x00",
        );
        let nothing = PREVIEW.replace("CFBundleDocumentTypes", "Unrelated");
        write(
            &apps.join("Other.app/Contents/Info.plist"),
            nothing.as_bytes(),
        );
        write(&apps.join("notes.txt"), b"not a bundle");
        let list = apps_in(std::slice::from_ref(&apps), "pdf", Some("application/pdf"));
        let names: Vec<&str> = list.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Editor", "Viewer & Reader"]);
        assert_eq!(list.unread, 1);
        assert_eq!(
            list.unread_line().as_deref(),
            Some("1 program not listed: binary Info.plist")
        );
        let file = OsStr::new("/Users/ada/My Files/report (1).pdf");
        let Launch::Command(command) = &list.apps[1].launch else {
            panic!("a command");
        };
        let (program, args) = command.for_file(file);
        assert_eq!(program, OsStr::new("open"));
        let viewer = apps.join("Viewer.app");
        let args: Vec<&OsStr> = args.iter().map(|a| a.as_os_str()).collect();
        assert_eq!(
            args,
            [OsStr::new("-a"), viewer.as_os_str(), file],
            "open -a BUNDLE FILE, each whole"
        );
    }
}
