//! What a file is, by diroxide's own detection: the conversion engine asks
//! the app's file-type registry through [`Typing`] (this crate cannot see
//! the registry), and fills `{typed_input}` from the answer so ImageMagick
//! reads a file as what it is, whatever its name says (spike V1: `PNG:`
//! read a PNG named `.jpg` where `JPEG:` refused it).

/// The type of one file, as the type registry found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileKind {
    /// The type's id in the registry: `png`, `jpeg`, `markdown`; `None`
    /// when no type matched.
    pub type_id: Option<String>,
    /// The groups the file is in: `images`, `screenshots`.
    pub groups: Vec<String>,
}

/// Finds a file's type from its name and its first bytes (at most 64,
/// the registry's head). The app implements it over its type registry;
/// [`ByExtension`] serves tests and a caller without one.
pub trait Typing: Send + Sync {
    /// The type and groups of a file named `name` whose first bytes are
    /// `head`.
    fn kind_of(&self, name: &str, head: &[u8]) -> FileKind;
}

/// How many bytes of a file the planner reads for [`Typing::kind_of`]:
/// the type registry's head.
pub const HEAD_BYTES: usize = 64;

/// Types by the name's extension alone, with a small table of groups:
/// for tests, and for a caller with no type registry. The app uses its
/// registry, which also reads the contents.
#[derive(Clone, Copy, Debug, Default)]
pub struct ByExtension;

/// Extensions and the type id each names, where the two differ.
const ALIASES: [(&str, &str); 9] = [
    ("jpg", "jpeg"),
    ("jpe", "jpeg"),
    ("tif", "tiff"),
    ("heif", "heic"),
    ("htm", "html"),
    ("md", "markdown"),
    ("markdown", "markdown"),
    ("mkv", "mkv"),
    ("m4v", "mp4"),
];

/// The groups of the built-in table, by type id: enough for the recipes'
/// `from-group`.
const GROUPS: [(&str, &[&str]); 5] = [
    (
        "images",
        &[
            "png", "jpeg", "gif", "webp", "bmp", "tiff", "heic", "avif", "jxl", "ico", "psd",
        ],
    ),
    (
        "music",
        &[
            "mp3", "flac", "ogg", "opus", "wav", "m4a", "aac", "aiff", "wma",
        ],
    ),
    (
        "video",
        &["mp4", "mkv", "webm", "mov", "avi", "mpeg", "wmv", "ogv"],
    ),
    (
        "documents",
        &["pdf", "docx", "odt", "epub", "rtf", "doc", "pptx", "xlsx"],
    ),
    (
        "text",
        &["text", "markdown", "org", "rst", "csv", "html", "mediawiki"],
    ),
];

impl Typing for ByExtension {
    fn kind_of(&self, name: &str, _head: &[u8]) -> FileKind {
        let Some((_, extension)) = name.rsplit_once('.') else {
            return FileKind::default();
        };
        let extension = extension.to_ascii_lowercase();
        let type_id = ALIASES
            .iter()
            .find(|(e, _)| *e == extension)
            .map_or(extension.as_str(), |(_, t)| *t)
            .to_owned();
        let type_id = if type_id == "txt" {
            "text".to_owned()
        } else {
            type_id
        };
        let groups = GROUPS
            .iter()
            .filter(|(_, ids)| ids.contains(&type_id.as_str()))
            .map(|(group, _)| (*group).to_owned())
            .collect();
        FileKind {
            type_id: Some(type_id),
            groups,
        }
    }
}

/// Type ids and ImageMagick's name for the format, where the two differ
/// beyond letter case.
const MAGICK_FORMATS: [(&str, &str); 4] = [
    ("jpeg", "JPEG"),
    ("camera-raw", ""),
    ("svgz", "SVG"),
    ("postscript", "PS"),
];

/// ImageMagick's format name for a file: from the type registry's id in
/// capitals (`png` to `PNG`, `heic` to `HEIC`), or for camera raw files,
/// whose one type covers many makers, the extension in capitals (`CR2`,
/// `NEF`, `DNG`). `None` when neither says.
pub fn magick_format(kind: &FileKind, extension: Option<&str>) -> Option<String> {
    let from_extension = || {
        extension
            .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .map(str::to_ascii_uppercase)
    };
    let Some(type_id) = kind.type_id.as_deref() else {
        return from_extension();
    };
    match MAGICK_FORMATS.iter().find(|(t, _)| *t == type_id) {
        Some((_, "")) => from_extension(),
        Some((_, format)) => Some((*format).to_owned()),
        None if type_id.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            Some(type_id.to_ascii_uppercase())
        }
        None => from_extension(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_extension_types_and_groups_a_few_names() {
        let kind = ByExtension.kind_of("Photo.JPG", b"");
        assert_eq!(kind.type_id.as_deref(), Some("jpeg"));
        assert_eq!(kind.groups, ["images"]);
        assert_eq!(ByExtension.kind_of("song.flac", b"").groups, ["music"]);
        assert_eq!(
            ByExtension.kind_of("notes.txt", b"").type_id.as_deref(),
            Some("text")
        );
        assert_eq!(ByExtension.kind_of("Makefile", b""), FileKind::default());
    }

    #[test]
    fn imagemagick_reads_the_format_diroxide_found() {
        let kind = |id: &str| FileKind {
            type_id: Some(id.to_owned()),
            groups: Vec::new(),
        };
        assert_eq!(
            magick_format(&kind("png"), Some("jpg")).as_deref(),
            Some("PNG")
        );
        assert_eq!(
            magick_format(&kind("jpeg"), Some("jpg")).as_deref(),
            Some("JPEG")
        );
        assert_eq!(
            magick_format(&kind("heic"), Some("heif")).as_deref(),
            Some("HEIC")
        );
        assert_eq!(
            magick_format(&kind("camera-raw"), Some("nef")).as_deref(),
            Some("NEF")
        );
        assert_eq!(magick_format(&kind("svgz"), None).as_deref(), Some("SVG"));
        assert_eq!(
            magick_format(&FileKind::default(), Some("tga")).as_deref(),
            Some("TGA")
        );
        assert_eq!(magick_format(&FileKind::default(), None), None);
    }
}
