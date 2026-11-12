//! The readers that run a helper program, always through the tools
//! registry's runner ([`ToolRunner`], the one door R30's commit 7 made),
//! never as a shell string:
//!
//! - **ExifTool, "Show all"** (R12, R28 3.2): every tag ExifTool knows,
//!   on demand. A page of up to [`PAGE`] files is one run, `-@ -` with
//!   the options and the names on standard input (so no command line
//!   passes Windows' 32,767 characters), `-json -G -s` for group names
//!   and short tag names (R28 3.5's batching).
//! - **pdfinfo, the PDF fallback** (R12; the owner's answer 120): when
//!   `lopdf` cannot read a PDF or it is larger than a read may take. Its
//!   `Tagged` line says whether a PDF carries the structure a screen
//!   reader follows.
//!
//! A program that is missing says so in the run's own words ("ExifTool
//! is not installed"); the app offers the component or the package.

use std::path::{Path, PathBuf};
use std::time::Duration;

use diroxide_core::details::DetailGroup;
use diroxide_vfs::{CancelToken, RunLimits, ToolInput, ToolRequest, ToolRunner};

use crate::clean::plain_key;
use crate::field::Field;
use crate::json::{self, Json};
use crate::reader::MetaError;
use crate::words::{grouped, iso_date_words, yes_no};

/// ExifTool's id in the tools registry.
pub const EXIFTOOL: &str = "exiftool";

/// pdfinfo's id in the tools registry.
pub const PDFINFO: &str = "pdfinfo";

/// The most files of one ExifTool run.
pub const PAGE: usize = 50;

/// The limits of one ExifTool page: a minute and 16 MiB of output.
pub const EXIFTOOL_LIMITS: RunLimits = RunLimits::new(Duration::from_secs(60), 16 * 1024 * 1024);

/// The limits of one pdfinfo run: ten seconds and 1 MiB.
pub const PDFINFO_LIMITS: RunLimits = RunLimits::new(Duration::from_secs(10), 1024 * 1024);

/// The groups "Show all" leaves out: ExifTool's own, and the file
/// system's, which the listing already says.
const QUIET_GROUPS: [&str; 2] = ["ExifTool", "System"];

/// The run for one page of files: `exiftool -@ -`, with `-json -G -s`,
/// UTF-8 names, and each name on its own line of standard input. A name
/// holding a line break cannot travel in that list and is refused.
pub fn exiftool_request(paths: &[PathBuf]) -> Result<ToolRequest, MetaError> {
    let mut list =
        String::from("-json\n-G\n-s\n-charset\nfilename=UTF8\n-api\nLargeFileSupport=1\n");
    for path in paths.iter().take(PAGE) {
        let text = path
            .to_str()
            .ok_or_else(|| MetaError::unreadable("a name ExifTool cannot be given"))?;
        if text.contains(['\n', '\r']) {
            return Err(MetaError::unreadable(
                "a name with a line break cannot go to ExifTool",
            ));
        }
        list.push_str(text);
        list.push('\n');
    }
    Ok(ToolRequest::new(EXIFTOOL, ["-@", "-"])
        .with_input(ToolInput::Bytes(list.into_bytes()))
        .with_limits(EXIFTOOL_LIMITS))
}

/// Whether an ExifTool tag can say who, where or with what.
fn private_tag(group: &str, tag: &str) -> bool {
    const WORDS: [&str; 12] = [
        "GPS",
        "Serial",
        "Owner",
        "Artist",
        "Author",
        "Creator",
        "Comment",
        "By-line",
        "Location",
        "City",
        "LastModifiedBy",
        "Company",
    ];
    // A color profile's creator is a maker's code, not a person.
    !group.starts_with("ICC") && (group.starts_with("GPS") || WORDS.iter().any(|w| tag.contains(w)))
}

/// One file's tags from ExifTool's JSON object, as fields keyed
/// `exiftool.GROUP.TAG` with the label "TAG (GROUP)".
fn tags_of(members: &[(String, Json)]) -> Vec<Field> {
    members
        .iter()
        .filter(|(name, _)| name != "SourceFile")
        .filter_map(|(name, value)| {
            let (group, tag) = name.split_once(':').unwrap_or(("", name));
            if QUIET_GROUPS.contains(&group) {
                return None;
            }
            let words = value.words()?;
            let key = format!("exiftool.{}.{}", plain_key(group), plain_key(tag));
            let label = if group.is_empty() {
                tag.to_owned()
            } else {
                format!("{tag} ({group})")
            };
            let field = Field::new(key, label, words, DetailGroup::Technical);
            Some(if private_tag(group, tag) {
                field.private()
            } else {
                field
            })
        })
        .map(Field::cleaned)
        .collect()
}

/// ExifTool's answer for a page, matched to the names asked by its
/// `SourceFile`; a file it did not answer for gets no fields.
pub fn read_exiftool_json(
    stdout: &[u8],
    paths: &[PathBuf],
) -> Result<Vec<(PathBuf, Vec<Field>)>, MetaError> {
    let text = String::from_utf8_lossy(stdout);
    let Json::Array(items) = json::parse(&text)
        .ok_or_else(|| MetaError::unreadable("ExifTool's answer could not be read"))?
    else {
        return Err(MetaError::unreadable("ExifTool's answer could not be read"));
    };
    let same = |a: &str, b: &Path| {
        let b = b.to_string_lossy();
        a == b || a.replace('\\', "/") == b.replace('\\', "/")
    };
    Ok(paths
        .iter()
        .map(|path| {
            let fields = items
                .iter()
                .find_map(|item| match item {
                    Json::Object(members)
                        if members.iter().any(|(k, v)| {
                            k == "SourceFile" && matches!(v, Json::Text(t) if same(t, path))
                        }) =>
                    {
                        Some(tags_of(members))
                    }
                    _ => None,
                })
                .unwrap_or_default();
            (path.clone(), fields)
        })
        .collect())
}

/// "Show all": every tag ExifTool reads, for a page of up to [`PAGE`]
/// local files, in one run through the registry.
pub fn show_all(
    runner: &dyn ToolRunner,
    paths: &[PathBuf],
    cancel: &CancelToken,
) -> Result<Vec<(PathBuf, Vec<Field>)>, MetaError> {
    let paths = &paths[..paths.len().min(PAGE)];
    let request = exiftool_request(paths)?;
    let output = runner
        .run(&request, cancel)
        .map_err(|e| MetaError::Place(e.into()))?;
    if output.stdout.is_empty() && !output.succeeded() {
        let why = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("ExifTool failed")
            .trim()
            .to_owned();
        return Err(MetaError::unreadable(why));
    }
    read_exiftool_json(&output.stdout, paths)
}

/// The words when ExifTool is missing, for the end of "Show all": "More
/// details: ExifTool not found".
pub const EXIFTOOL_MISSING: &str = "More details: ExifTool not found";

/// The run of `pdfinfo -isodates FILE`.
pub fn pdfinfo_request(path: &Path) -> ToolRequest {
    ToolRequest::new(
        PDFINFO,
        [
            std::ffi::OsString::from("-isodates"),
            path.as_os_str().to_owned(),
        ],
    )
    .with_limits(PDFINFO_LIMITS)
}

/// pdfinfo's lines as fields, keyed as the `lopdf` reader keys them, and
/// the facts only it gives: whether the PDF is tagged, holds a form or
/// JavaScript.
pub fn read_pdfinfo(stdout: &[u8]) -> Vec<Field> {
    let text = String::from_utf8_lossy(stdout);
    let mut fields = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let field = match key.trim() {
            "Title" => Field::new("doc.title", "Title", value, DetailGroup::Content),
            "Subject" => Field::new("doc.subject", "Subject", value, DetailGroup::Content),
            "Keywords" => Field::new("doc.keywords", "Keywords", value, DetailGroup::Content),
            "Author" => Field::new("doc.author", "Author", value, DetailGroup::Source).private(),
            "Creator" => Field::new("doc.application", "Made with", value, DetailGroup::Source),
            "Producer" => Field::new("doc.producer", "PDF made by", value, DetailGroup::Source),
            "CreationDate" => match iso_date_words(value) {
                Some(date) => {
                    Field::new("doc.created", "Document created", date, DetailGroup::Dates)
                }
                None => continue,
            },
            "ModDate" => match iso_date_words(value) {
                Some(date) => Field::new(
                    "doc.modified",
                    "Document modified",
                    date,
                    DetailGroup::Dates,
                ),
                None => continue,
            },
            "Pages" => match value.parse::<u64>() {
                Ok(n) => Field::new("doc.pages", "Pages", grouped(n), DetailGroup::Content)
                    .sorted_by_count(n),
                Err(_) => continue,
            },
            "Encrypted" => Field::new(
                "doc.encrypted",
                "Encrypted",
                if value.starts_with("yes") {
                    "yes"
                } else {
                    "no"
                },
                DetailGroup::Flags,
            ),
            "Tagged" => Field::new(
                "doc.tagged",
                "Tagged for screen readers",
                yes_no(value == "yes"),
                DetailGroup::Technical,
            ),
            "Form" => Field::new(
                "doc.form",
                "Form",
                value.to_ascii_lowercase(),
                DetailGroup::Content,
            ),
            "JavaScript" => Field::new(
                "doc.javascript",
                "JavaScript",
                value.to_ascii_lowercase(),
                DetailGroup::Technical,
            ),
            "PDF version" => Field::new(
                "doc.pdf_version",
                "PDF version",
                value,
                DetailGroup::Technical,
            ),
            _ => continue,
        };
        fields.push(field.cleaned());
    }
    fields
}

/// The PDF fallback: `pdfinfo` through the registry, for a PDF `lopdf`
/// could not read or that was too large to read in memory.
pub fn pdf_fallback(
    runner: &dyn ToolRunner,
    path: &Path,
    cancel: &CancelToken,
) -> Result<Vec<Field>, MetaError> {
    let output = runner
        .run(&pdfinfo_request(path), cancel)
        .map_err(|e| MetaError::Place(e.into()))?;
    if !output.succeeded() {
        let why = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("pdfinfo failed")
            .trim()
            .to_owned();
        return Err(MetaError::unreadable(why));
    }
    Ok(read_pdfinfo(&output.stdout))
}

#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;
    use std::sync::Mutex;

    use diroxide_vfs::{ToolOutput, ToolRunError, ToolStream};

    use super::*;

    /// A runner that answers with canned output and keeps what it was
    /// asked.
    struct Canned {
        stdout: Vec<u8>,
        asked: Mutex<Vec<ToolRequest>>,
    }

    impl ToolRunner for Canned {
        fn has(&self, _tool: &str) -> bool {
            true
        }
        fn run(
            &self,
            request: &ToolRequest,
            _cancel: &CancelToken,
        ) -> Result<ToolOutput, ToolRunError> {
            self.asked.lock().unwrap().push(request.clone());
            Ok(ToolOutput::new(Some(0), self.stdout.clone(), Vec::new()))
        }
        fn run_streaming(
            &self,
            request: &ToolRequest,
            cancel: &CancelToken,
            _record: &mut dyn FnMut(ToolStream, &str) -> ControlFlow<()>,
        ) -> Result<ToolOutput, ToolRunError> {
            self.run(request, cancel)
        }
    }

    #[test]
    fn show_all_is_one_run_a_page_with_the_names_on_standard_input() {
        let a = PathBuf::from(if cfg!(windows) {
            r"C:\photos\a.jpg"
        } else {
            "/photos/a.jpg"
        });
        let b = PathBuf::from(if cfg!(windows) {
            r"C:\photos\-b.jpg"
        } else {
            "/photos/-b.jpg"
        });
        let source = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let stdout = format!(
            r#"[{{"SourceFile":"{}","ExifTool:ExifToolVersion":13.25,"System:FileSize":"12 kB","EXIF:Make":"SpikeCam","GPS:GPSLatitude":"45 deg 29' 54.96\" N","IPTC:By-line":"Ada Example","ICC_Profile:ProfileCreator":"none"}}]"#,
            source(&a)
        );
        let runner = Canned {
            stdout: stdout.into_bytes(),
            asked: Mutex::new(Vec::new()),
        };
        let got = show_all(&runner, &[a.clone(), b.clone()], &CancelToken::new()).unwrap();
        let lines: Vec<String> = got[0]
            .1
            .iter()
            .map(|f| {
                format!(
                    "{} = {}{}",
                    f.key,
                    f.line(),
                    if f.private { " (private)" } else { "" }
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                "exiftool.exif.make = Make (EXIF): SpikeCam",
                "exiftool.gps.gpslatitude = GPSLatitude (GPS): 45 deg 29' 54.96\" N (private)",
                "exiftool.iptc.by-line = By-line (IPTC): Ada Example (private)",
                "exiftool.icc_profile.profilecreator = ProfileCreator (ICC_Profile): none",
            ]
        );
        assert!(got[1].1.is_empty());
        let asked = runner.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].tool, "exiftool");
        assert_eq!(asked[0].args, ["-@", "-"]);
        let ToolInput::Bytes(list) = &asked[0].input else {
            panic!("the names go on standard input")
        };
        let list = String::from_utf8(list.clone()).unwrap();
        assert!(list.starts_with("-json\n-G\n-s\n"));
        assert!(list.ends_with(&format!("{}\n{}\n", a.display(), b.display())));
        let newline = PathBuf::from("/a\nb.jpg");
        assert!(exiftool_request(&[newline]).is_err());
        assert!("More details: ExifTool not found".chars().count() <= 40);
        assert_eq!(EXIFTOOL_MISSING, "More details: ExifTool not found");
    }

    #[test]
    fn pdfinfo_lines_become_the_pdf_fields() {
        let out = b"Title:           Field Notes\nAuthor:          Ada Example\nCreator:         Writer 7\nProducer:        diroxide tests\nCreationDate:    2001-01-01T08:30:00Z\nModDate:         2001-02-03T04:05:00-08\nCustom Metadata: no\nTagged:          yes\nForm:            AcroForm\nJavaScript:      no\nPages:           1204\nEncrypted:       no\nPDF version:     1.7\n";
        let lines: Vec<String> = read_pdfinfo(out).iter().map(Field::line).collect();
        assert_eq!(
            lines,
            [
                "Title: Field Notes",
                "Author: Ada Example",
                "Made with: Writer 7",
                "PDF made by: diroxide tests",
                "Document created: 2001-01-01 08:30",
                "Document modified: 2001-02-03 04:05",
                "Tagged for screen readers: yes",
                "Form: acroform",
                "JavaScript: no",
                "Pages: 1,204",
                "Encrypted: no",
                "PDF version: 1.7",
            ]
        );
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line}");
        }
        let request = pdfinfo_request(Path::new("/docs/notes.pdf"));
        assert_eq!(request.tool, "pdfinfo");
        assert_eq!(request.args[0], "-isodates");
    }
}
