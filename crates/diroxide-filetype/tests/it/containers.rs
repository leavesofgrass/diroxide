//! The container pattern (spike F2, R30's commit 24): zip-based files,
//! each built in memory with its first member laid out the way a known
//! producer writes it (Word and Excel, pandoc, the `jar` tool, an
//! OpenDocument or EPUB writer), read through the type tree. With its
//! real name, the child always beats zip; with a name that says nothing,
//! the first 64 bytes tell what they can.

use diroxide_config::FileTypesConfig;
use diroxide_core::{Entry, EntryKind, FixedClock, Timestamp, UtcOffset};
use diroxide_filetype::{Probe, Special, TypeRegistry, TypeStage};

/// A zip's first local header: the member's name, its extra field, and
/// the start of its data, as a producer writes them.
fn local_header(member: &str, extra: &[u8], method: u16, data: &[u8]) -> Vec<u8> {
    let mut head = b"PK\x03\x04".to_vec();
    head.extend_from_slice(&20u16.to_le_bytes());
    head.extend_from_slice(&0u16.to_le_bytes());
    head.extend_from_slice(&method.to_le_bytes());
    head.extend_from_slice(&[0x00, 0x20, 0x66, 0x5D]);
    head.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    let size = u32::try_from(data.len()).unwrap_or(0);
    head.extend_from_slice(&size.to_le_bytes());
    head.extend_from_slice(&size.to_le_bytes());
    head.extend_from_slice(&u16::try_from(member.len()).unwrap_or(0).to_le_bytes());
    head.extend_from_slice(&u16::try_from(extra.len()).unwrap_or(0).to_le_bytes());
    head.extend_from_slice(member.as_bytes());
    head.extend_from_slice(extra);
    head.extend_from_slice(data);
    head.extend_from_slice(b"PK\x03\x04\x14\x00");
    head
}

const DEFLATED: &[u8] = b"\xb5\x95\xcbN\xc30\x10E\x7f%\xca\x16%nY";

/// The twenty-four files of spike F2: name, first member, its extra
/// field, how its data starts, the type with the real name, and the type
/// the first 64 bytes alone give.
fn files() -> Vec<(&'static str, Vec<u8>, &'static str, &'static str)> {
    let ooxml = |member: &'static str| local_header(member, &[], 8, DEFLATED);
    let stored = |member: &'static str, data: &'static [u8]| local_header(member, &[], 0, data);
    vec![
        (
            "word-style.docx",
            ooxml("[Content_Types].xml"),
            "docx",
            "ooxml",
        ),
        ("pandoc.docx", ooxml("[Content_Types].xml"), "docx", "ooxml"),
        ("rels-first.docx", ooxml("_rels/.rels"), "docx", "ooxml"),
        (
            "body-first.docx",
            ooxml("word/document.xml"),
            "docx",
            "docx",
        ),
        (
            "excel-style.xlsx",
            ooxml("[Content_Types].xml"),
            "xlsx",
            "ooxml",
        ),
        (
            "docprops-first.xlsx",
            ooxml("docProps/app.xml"),
            "xlsx",
            "ooxml",
        ),
        ("sheet-first.xlsx", ooxml("xl/workbook.xml"), "xlsx", "xlsx"),
        (
            "slides-style.pptx",
            ooxml("[Content_Types].xml"),
            "pptx",
            "ooxml",
        ),
        (
            "pandoc.pptx",
            ooxml("ppt/slides/slide1.xml"),
            "pptx",
            "pptx",
        ),
        (
            "fixed-layout.xps",
            ooxml("[Content_Types].xml"),
            "xps",
            "ooxml",
        ),
        (
            "by-hand.odt",
            stored("mimetype", b"application/vnd.oasis.opendocument.text"),
            "odt",
            "opendocument",
        ),
        (
            "by-hand.ods",
            stored(
                "mimetype",
                b"application/vnd.oasis.opendocument.spreadsheet",
            ),
            "ods",
            "opendocument",
        ),
        (
            "by-hand.odp",
            stored(
                "mimetype",
                b"application/vnd.oasis.opendocument.presentation",
            ),
            "odp",
            "opendocument",
        ),
        (
            "by-hand.epub",
            stored("mimetype", b"application/epub+zip"),
            "epub",
            "epub",
        ),
        (
            "mimetype-late.epub",
            ooxml("META-INF/container.xml"),
            "epub",
            "epub",
        ),
        (
            "jar-tool.jar",
            local_header("META-INF/", b"\xFE\xCA\x00\x00", 0, b""),
            "jar",
            "jar",
        ),
        (
            "manifest-first.jar",
            ooxml("META-INF/MANIFEST.MF"),
            "jar",
            "jar",
        ),
        (
            "signed-old.apk",
            ooxml("META-INF/MANIFEST.MF"),
            "apk",
            "jar",
        ),
        ("app-shaped.apk", ooxml("AndroidManifest.xml"), "apk", "apk"),
        ("plain.zip", ooxml("notes.txt"), "zip", "zip"),
        ("readme-first.zip", ooxml("readme.txt"), "zip", "zip"),
        ("comic.cbz", ooxml("page01.png"), "cbz", "zip"),
        (
            "streamed.zip",
            stored("stream.txt", b"written as a stream\n"),
            "zip",
            "zip",
        ),
        (
            "empty.zip",
            b"PK\x05\x06\x00\x00\x00\x00".to_vec(),
            "zip",
            "zip",
        ),
    ]
}

/// The zip family as the shipped table has it.
const FAMILY: &str = r#"
[[type]]
id = "zip"
name = "zip archive"
short = "zip"
extensions = ["zip"]
magic = [{ at = 0, hex = "50 4B 03 04" }, { at = 0, hex = "50 4B 05 06" }, { at = 0, hex = "50 4B 07 08" }]

[[type]]
id = "ooxml"
name = "Office Open XML file"
short = "OOXML"
parent = "zip"
priority = -1
magic = [{ at = 30, hex = "5B 43 6F 6E 74 65 6E 74 5F 54 79 70 65 73 5D 2E 78 6D 6C" }, { at = 30, hex = "5F 72 65 6C 73 2F" }, { at = 30, hex = "64 6F 63 50 72 6F 70 73 2F" }]

[[type]]
id = "docx"
name = "Word document"
short = "DOCX"
parent = "zip"
extensions = ["docx"]
magic = [{ at = 30, hex = "77 6F 72 64 2F" }]

[[type]]
id = "xlsx"
name = "Excel workbook"
short = "XLSX"
parent = "zip"
extensions = ["xlsx"]
magic = [{ at = 30, hex = "78 6C 2F" }]

[[type]]
id = "pptx"
name = "PowerPoint presentation"
short = "PPTX"
parent = "zip"
extensions = ["pptx"]
magic = [{ at = 30, hex = "70 70 74 2F" }]

[[type]]
id = "xps"
name = "XPS document"
short = "XPS"
parent = "zip"
extensions = ["xps", "oxps"]

[[type]]
id = "opendocument"
name = "OpenDocument file"
short = "ODF"
parent = "zip"
priority = -1
magic = [{ at = 30, hex = "6D 69 6D 65 74 79 70 65 61 70 70 6C 69 63 61 74 69 6F 6E 2F 76 6E 64 2E 6F 61 73 69 73 2E 6F 70 65 6E" }]

[[type]]
id = "odt"
name = "OpenDocument text"
short = "ODT"
parent = "zip"
extensions = ["odt"]

[[type]]
id = "ods"
name = "OpenDocument spreadsheet"
short = "ODS"
parent = "zip"
extensions = ["ods"]

[[type]]
id = "odp"
name = "OpenDocument presentation"
short = "ODP"
parent = "zip"
extensions = ["odp"]

[[type]]
id = "epub"
name = "EPUB book"
short = "EPUB"
parent = "zip"
extensions = ["epub"]
magic = [{ at = 30, hex = "6D 69 6D 65 74 79 70 65 61 70 70 6C 69 63 61 74 69 6F 6E 2F 65 70 75 62 2B 7A 69 70" }, { at = 30, hex = "4D 45 54 41 2D 49 4E 46 2F 63 6F 6E 74 61 69 6E 65 72 2E 78 6D 6C" }]

[[type]]
id = "jar"
name = "Java archive"
short = "JAR"
parent = "zip"
priority = -1
extensions = ["jar"]
magic = [{ at = 30, hex = "4D 45 54 41 2D 49 4E 46 2F FE CA 00 00" }, { at = 30, hex = "4D 45 54 41 2D 49 4E 46 2F 4D 41 4E 49 46 45 53 54 2E 4D 46" }]

[[type]]
id = "apk"
name = "Android package"
short = "APK"
parent = "zip"
extensions = ["apk"]
magic = [{ at = 30, hex = "41 6E 64 72 6F 69 64 4D 61 6E 69 66 65 73 74 2E 78 6D 6C" }]

[[type]]
id = "cbz"
name = "comic book archive"
short = "CBZ"
parent = "zip"
extensions = ["cbz"]
"#;

fn clock() -> FixedClock {
    let now = Timestamp::from_civil(2026, 11, 6, 4, 0, 0, UtcOffset::UTC).unwrap();
    FixedClock::new(now, UtcOffset::UTC)
}

fn type_of(types: &TypeRegistry, name: &str, head: &[u8]) -> Result<(String, TypeStage), Special> {
    let entry = Entry::new(name, EntryKind::File).with_size(4_096);
    let head = &head[..head.len().min(64)];
    types
        .check(&Probe::new(&entry, Some(head)), &clock())
        .map(|found| (found.type_id, found.stage))
}

/// The family read by the same tree as the shipped table, from text.
pub(crate) fn family() -> TypeRegistry {
    let config = FileTypesConfig::from_toml(FAMILY).unwrap();
    let types = TypeRegistry::fallback().with_config(&config);
    assert!(types.problems().is_empty(), "{:?}", types.problems());
    types
}

#[test]
fn the_child_wins_every_time() {
    check_family(&family());
}

/// Every file with its real name, then with a name that says nothing.
pub(crate) fn check_family(types: &TypeRegistry) {
    let files = files();
    assert_eq!(files.len(), 24);
    for (name, head, by_name, by_head) in files {
        let (found, _) = type_of(types, name, &head).unwrap();
        assert_eq!(found, by_name, "{name}");
        let (found, stage) = type_of(types, "file.bin", &head).unwrap();
        assert_eq!(found, by_head, "{name} named file.bin");
        assert_eq!(stage, TypeStage::Signature, "{name} named file.bin");
    }
}

/// A name and contents that disagree: a presentation named `.docx`
/// reads as a presentation, its member saying so in the first 64 bytes;
/// bytes that are no zip at all leave zip's children untested.
#[test]
fn the_contents_speak_where_the_name_is_wrong() {
    let types = family();
    let slides = local_header("ppt/presentation.xml", &[], 8, DEFLATED);
    assert_eq!(type_of(&types, "report.docx", &slides).unwrap().0, "pptx");
    assert_eq!(
        type_of(&types, "report.docx", b"%PDF-1.7\n").unwrap().0,
        "pdf"
    );
    assert_eq!(
        type_of(&types, "report.docx", b"plain words"),
        Err(Special::Unknown)
    );
    let pending = Entry::new("report.docx", EntryKind::File).with_size(4_096);
    let found = types.check(&Probe::new(&pending, None), &clock()).unwrap();
    assert_eq!(
        (found.type_id.as_str(), found.stage),
        ("docx", TypeStage::Name)
    );
}
