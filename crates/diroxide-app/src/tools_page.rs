//! The Settings Tools page (alpha 4, R30 commit 42; R28 1.3 and 1.9; the
//! owner's answers 121 and 124): each helper program diroxide can use,
//! one a row, its name first, in the tools registry's own words, the
//! lines `diro doctor` says ([`ToolsReport::lines`], lane K):
//! "ImageMagick: 7.1.2-35, on PATH", "pandoc: not found", "carta:
//! 0.0.13, built in", where each was found said in the lookup's order
//! (`tools.toml`, a component, PATH, the known install folders).
//!
//! What the page adds to the report: a tool that is not found and that a
//! helper component brings says what it would download ("FFmpeg: not
//! found, 138 MB to download", 37 cells); a tool with a package command
//! has it as the row's next words ("Install: winget install
//! JohnMacFarlane.Pandoc"). Enter on a row offers the component, through
//! the components' ask-first question that starts on No, or copies the
//! package command for the owner to run, or says where to get the tool.
//! Nothing is downloaded or installed without a yes.

use diroxide_components::{Kind, Status};
use diroxide_core::{ListModel, ListRow};
use diroxide_tools::{NextStep, ToolRegistry, ToolReport, ToolState, ToolsReport};

use crate::components::{Listing, Row};

/// What Enter offers on a tool's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    /// A helper component brings it: its id, for the ask-first question.
    Component(String),
    /// A command that installs it from the system's packages, to copy.
    Command(String),
    /// Where to get it, in words.
    Words(String),
    /// Nothing to offer: it is found, built in, or turned off.
    Nothing,
}

/// One tool's row on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRow {
    /// The tool's id in the registry: `ffmpeg`.
    pub id: String,
    /// Its line, its name first: "FFmpeg: not found, 138 MB to download".
    pub line: String,
    /// The next step, said after the line: "Install: winget install
    /// Gyan.FFmpeg".
    pub next: Option<String>,
    /// What Enter offers.
    pub offer: Offer,
}

/// A tool's line with what a helper component would download: "FFmpeg:
/// not found, 138 MB to download"; the report's own line otherwise.
pub fn line_with_download(tool: &ToolReport, download: Option<&str>) -> String {
    match (&tool.state, download) {
        (ToolState::NotFound { .. }, Some(size)) => {
            format!("{}, {size} to download", tool.line())
        }
        _ => tool.line(),
    }
}

/// The helper component that brings a tool, when one is known and not
/// installed: a component whose programs for this computer include one of
/// the tool's program names.
pub fn component_for<'a>(
    registry: &ToolRegistry,
    id: &str,
    listing: Option<&'a Listing>,
) -> Option<&'a Row> {
    let spec = registry.spec(id)?;
    let names: Vec<String> = spec
        .programs
        .iter()
        .map(|p| crate::components::program_name(p))
        .collect();
    listing?.rows.iter().find(|row| {
        row.kind == Kind::Helper
            && !matches!(row.status, Status::Installed | Status::NoBuild)
            && row.programs.iter().any(|p| names.contains(p))
    })
}

/// Every tool's row, in the report's order.
pub fn rows(
    report: &ToolsReport,
    registry: &ToolRegistry,
    listing: Option<&Listing>,
) -> Vec<ToolRow> {
    report
        .tools
        .iter()
        .map(|tool| {
            let component = match tool.state {
                ToolState::NotFound { .. } => component_for(registry, &tool.id, listing),
                _ => None,
            };
            let offer = match (&tool.state, component) {
                (_, Some(row)) => Offer::Component(row.id.clone()),
                (
                    ToolState::NotFound {
                        install: Some(NextStep::Command(command)),
                    },
                    None,
                ) => Offer::Command(command.clone()),
                (
                    ToolState::NotFound {
                        install: Some(NextStep::Words(words)),
                    },
                    None,
                ) => Offer::Words(words.clone()),
                (ToolState::PathMissing, None) => {
                    Offer::Words("Check its path in tools.toml.".to_owned())
                }
                _ => Offer::Nothing,
            };
            ToolRow {
                id: tool.id.clone(),
                line: line_with_download(tool, component.map(|row| row.size.as_str())),
                next: tool.install_line(),
                offer,
            }
        })
        .collect()
}

/// The page as a list: "Tools: 13 of 22 found" as its title, each tool a
/// row with its next step after its line.
pub fn page(report: &ToolsReport, rows: &[ToolRow]) -> ListModel {
    let list_rows = rows
        .iter()
        .map(|row| {
            let item = ListRow::new(row.id.clone(), row.line.clone());
            match &row.next {
                Some(next) => item.with_details([next.clone()]),
                None => item,
            }
        })
        .collect();
    let mut list = ListModel::new(report.summary()).with_rows(list_rows);
    list.set_help(
        "Up and Down read, Enter offers a missing tool's component or copies its install command, Escape closes.",
    );
    list
}

#[cfg(test)]
mod tests {
    use diroxide_components::messages::AskFirst;
    use diroxide_tools::FoundBy;

    use super::*;

    fn tool(id: &str, name: &str, state: ToolState) -> ToolReport {
        ToolReport {
            id: id.to_owned(),
            name: name.to_owned(),
            state,
        }
    }

    fn helper(id: &str, programs: &[&str], status: Status) -> Row {
        Row {
            id: id.to_owned(),
            title: id.to_owned(),
            line: id.to_owned(),
            status,
            kind: Kind::Helper,
            features: Vec::new(),
            ask: AskFirst {
                lines: Vec::new(),
                question: String::new(),
            },
            ask_for_feature: None,
            no_build: Vec::new(),
            programs: programs.iter().map(|p| (*p).to_owned()).collect(),
            size: "138 MB".to_owned(),
        }
    }

    #[test]
    fn a_missing_tool_a_component_brings_says_its_download() {
        let registry = ToolRegistry::builtin();
        let report = ToolsReport {
            tools: vec![
                tool(
                    "ffmpeg",
                    "FFmpeg",
                    ToolState::NotFound {
                        install: Some(NextStep::Command("winget install Gyan.FFmpeg".to_owned())),
                    },
                ),
                tool(
                    "tesseract",
                    "Tesseract",
                    ToolState::NotFound {
                        install: Some(NextStep::Command(
                            "winget install UB-Mannheim.TesseractOCR".to_owned(),
                        )),
                    },
                ),
                tool(
                    "magick",
                    "ImageMagick",
                    ToolState::Found {
                        version: Some("7.1.2-35".to_owned()),
                        by: FoundBy::Path,
                    },
                ),
                tool(
                    "carta",
                    "carta",
                    ToolState::Found {
                        version: Some("0.0.13".to_owned()),
                        by: FoundBy::BuiltIn,
                    },
                ),
                tool("pdftotext", "pdftotext", ToolState::PathMissing),
            ],
        };
        let listing = Listing {
            rows: vec![helper(
                "ffmpeg-tools",
                &["ffmpeg", "ffprobe"],
                Status::NotInstalled,
            )],
            ..Listing::default()
        };
        let rows = rows(&report, &registry, Some(&listing));
        let lines: Vec<&str> = rows.iter().map(|r| r.line.as_str()).collect();
        assert_eq!(
            lines,
            [
                "FFmpeg: not found, 138 MB to download",
                "Tesseract: not found",
                "ImageMagick: 7.1.2-35, on PATH",
                "carta: 0.0.13, built in",
                "pdftotext: not at its tools.toml path",
            ]
        );
        assert_eq!(rows[0].offer, Offer::Component("ffmpeg-tools".to_owned()));
        assert_eq!(
            rows[1].offer,
            Offer::Command("winget install UB-Mannheim.TesseractOCR".to_owned())
        );
        assert_eq!(
            rows[1].next.as_deref(),
            Some("Install: winget install UB-Mannheim.TesseractOCR")
        );
        assert_eq!(rows[2].offer, Offer::Nothing);
        assert_eq!(
            rows[4].offer,
            Offer::Words("Check its path in tools.toml.".to_owned())
        );
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line}");
        }
        let list = page(&report, &rows);
        assert_eq!(list.title(), "Tools: 2 of 5 found");
        assert_eq!(
            list.rows()[1].spoken(),
            "Tesseract: not found, Install: winget install UB-Mannheim.TesseractOCR"
        );
        // An installed component offers nothing more; the package command
        // stands.
        let installed = Listing {
            rows: vec![helper("ffmpeg-tools", &["ffmpeg"], Status::Installed)],
            ..Listing::default()
        };
        let again = super::rows(&report, &registry, Some(&installed));
        assert_eq!(again[0].line, "FFmpeg: not found");
        assert_eq!(
            again[0].offer,
            Offer::Command("winget install Gyan.FFmpeg".to_owned())
        );
    }

    #[test]
    fn a_programs_name_is_matched_without_its_folder_or_exe() {
        assert_eq!(crate::components::program_name("bin/FFmpeg.exe"), "ffmpeg");
        assert_eq!(crate::components::program_name("ffprobe"), "ffprobe");
        assert_eq!(
            crate::components::program_name("pandoc-3.8\\pandoc.exe"),
            "pandoc"
        );
    }
}
