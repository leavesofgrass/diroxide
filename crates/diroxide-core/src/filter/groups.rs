//! The built-in file-type groups (plan section 5.6, question 71): seven
//! groups by extension, so `group images` works before the full type
//! model of alpha.7 replaces this table with `filetypes.toml` and the
//! same syntax. No extension is in two groups, and each group says what
//! it holds in a sentence.

use crate::entry::{Entry, EntryKind};

/// A file-type group: its name, what it holds, and its extensions
/// (lowercase, without the dot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    /// The name, as typed and said: "images".
    pub name: &'static str,
    /// What the group holds, in a sentence for the guide and the Groups
    /// list.
    pub description: &'static str,
    /// The extensions that put a file in the group.
    pub extensions: &'static [&'static str],
}

/// The seven built-in groups, in the order the Groups list shows them.
pub const GROUPS: [Group; 7] = [
    Group {
        name: "images",
        description: "Pictures and photos: PNG, JPEG, GIF, WebP, camera raw files and the like.",
        extensions: &[
            "png", "jpg", "jpeg", "jpe", "gif", "bmp", "webp", "tif", "tiff", "svg", "ico", "heic",
            "heif", "avif", "jxl", "psd", "xcf", "raw", "cr2", "cr3", "nef", "arw", "dng", "orf",
            "rw2",
        ],
    },
    Group {
        name: "music",
        description: "Sound and music: MP3, FLAC, Ogg, WAV and the like.",
        extensions: &[
            "mp3", "flac", "ogg", "oga", "opus", "wav", "m4a", "aac", "wma", "aif", "aiff", "alac",
            "ape", "wv", "mid", "midi", "mka",
        ],
    },
    Group {
        name: "video",
        description: "Video: MP4, Matroska, WebM, QuickTime and the like.",
        extensions: &[
            "mp4", "m4v", "mkv", "webm", "avi", "mov", "wmv", "mpg", "mpeg", "flv", "3gp", "ogv",
            "mts", "m2ts", "vob",
        ],
    },
    Group {
        name: "documents",
        description: "Documents, sheets, slides and e-books: PDF, Word, OpenDocument, EPUB.",
        extensions: &[
            "pdf", "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "epub",
            "mobi", "djvu", "xps", "pages", "numbers", "key",
        ],
    },
    Group {
        name: "archives",
        description: "Archives and disk images: zip, 7z, tar and its compressed forms, ISO.",
        extensions: &[
            "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lz",
            "lzma", "lz4", "cab", "iso", "jar", "z", "arj", "lzh", "cpio",
        ],
    },
    Group {
        name: "code",
        description: "Source code, scripts and settings files: Rust, C, Python, HTML, JSON, TOML.",
        extensions: &[
            "rs", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "cs", "java", "kt", "kts", "go", "py",
            "pyw", "rb", "js", "mjs", "cjs", "ts", "tsx", "jsx", "php", "pl", "pm", "lua", "swift",
            "dart", "zig", "nim", "hs", "ml", "mli", "scala", "el", "lisp", "clj", "cljs", "sh",
            "bash", "zsh", "fish", "ps1", "psm1", "bat", "cmd", "sql", "html", "htm", "css",
            "scss", "sass", "less", "xml", "json", "yaml", "yml", "toml", "vue", "svelte", "asm",
            "vim",
        ],
    },
    Group {
        name: "text",
        description: "Plain text, notes and logs: TXT, Markdown, CSV, INI and the like.",
        extensions: &[
            "txt", "md", "markdown", "rst", "log", "csv", "tsv", "ini", "cfg", "conf", "nfo",
            "org", "tex", "adoc", "srt", "vtt", "diff", "patch",
        ],
    },
];

/// The group a name gives, ignoring case and a missing plural `s`:
/// "Images", "image" and "document" all name groups.
pub fn group_named(word: &str) -> Option<&'static Group> {
    let lower = word.to_lowercase();
    GROUPS
        .iter()
        .find(|group| group.name == lower || group.name.strip_suffix('s') == Some(lower.as_str()))
}

/// The group an extension puts a file in, ignoring case and a leading
/// dot.
pub fn group_of_extension(extension: &str) -> Option<&'static Group> {
    let lower = extension.trim_start_matches('.').to_lowercase();
    GROUPS
        .iter()
        .find(|group| group.extensions.contains(&lower.as_str()))
}

/// The group an entry is in, by its extension; folders and items are in
/// none.
pub fn group_of(entry: &Entry) -> Option<&'static Group> {
    match entry.kind {
        EntryKind::Folder | EntryKind::Item => None,
        _ => entry.extension().and_then(group_of_extension),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::format::within_cells;

    #[test]
    fn no_extension_is_in_two_groups() {
        let mut seen = HashSet::new();
        for group in GROUPS {
            for extension in group.extensions {
                assert!(
                    seen.insert(*extension),
                    "{extension} is in two groups, the second {}",
                    group.name
                );
                assert_eq!(*extension, extension.to_lowercase(), "{extension}");
                assert!(!extension.contains('.'), "{extension}");
                assert!(!extension.is_empty());
            }
        }
    }

    #[test]
    fn every_group_is_documented() {
        let mut names = HashSet::new();
        for group in GROUPS {
            assert!(names.insert(group.name));
            assert!(group.description.ends_with('.'), "{}", group.name);
            // The kind of thing comes first, inside one Braille line.
            let first = group.description.split(':').next().unwrap_or_default();
            assert!(within_cells(group.description, first), "{}", group.name);
            assert!(!group.extensions.is_empty());
            assert_eq!(group_named(group.name), Some(&group));
        }
    }

    #[test]
    fn groups_are_found_by_name_and_extension() {
        assert_eq!(group_named("Images").map(|g| g.name), Some("images"));
        assert_eq!(group_named("image").map(|g| g.name), Some("images"));
        assert_eq!(group_named("document").map(|g| g.name), Some("documents"));
        assert_eq!(group_named("music").map(|g| g.name), Some("music"));
        assert_eq!(group_named("pictures"), None);
        assert_eq!(group_of_extension("PNG").map(|g| g.name), Some("images"));
        assert_eq!(group_of_extension(".ts").map(|g| g.name), Some("code"));
        assert_eq!(group_of_extension("m2ts").map(|g| g.name), Some("video"));
        assert_eq!(group_of_extension("exe"), None);
        let png = Entry::new("holiday.PNG", EntryKind::File);
        assert_eq!(group_of(&png).map(|g| g.name), Some("images"));
        let folder = Entry::new("photos.png", EntryKind::Folder);
        assert_eq!(group_of(&folder), None);
        let archive = Entry::new("a.tar.gz", EntryKind::Archive);
        assert_eq!(group_of(&archive).map(|g| g.name), Some("archives"));
    }
}
