//! "Open in textweaver" (commit 54): the shown file handed to `tw`, the
//! owner's reader, for full reading with speech, found through the tools
//! registry's entry and never required. The app runs the program with an
//! argument list and no shell ([`QuickViewRequest::RunProgram`]); when it
//! is not there, the registry's own words say how to install it.

use std::ffi::OsString;

use diroxide_core::Message;
use diroxide_tools::{ToolRegistry, ToolStatus};

use super::{Outcome, QuickView, QuickViewRequest};

/// The tools registry's id for textweaver.
pub const TEXTWEAVER: &str = "tw";

impl QuickView {
    /// Hands the shown file to textweaver when the tools registry finds
    /// `tw`: "Opening quarter.md in textweaver", and the app is asked to
    /// run it. When it is not installed: "Could not open quarter.md in
    /// textweaver: textweaver is not installed. Install textweaver from
    /// its releases page or with cargo." A file inside an archive or on
    /// another machine is not handed over, since textweaver opens files on
    /// this computer.
    pub fn open_in_textweaver(&self, tools: &ToolRegistry) -> Outcome {
        let Some(shown) = self.shown.as_ref() else {
            return vec![Message::status("Nothing shown to open in textweaver")].into();
        };
        let object = format!("{} in textweaver", shown.name);
        let Some(path) = shown.place.local_path() else {
            return vec![Message::could_not(
                "open",
                &object,
                "textweaver opens files on this computer only",
                "Copy it out first",
            )]
            .into();
        };
        let hint = tools
            .spec(TEXTWEAVER)
            .map_or("Install textweaver.", |spec| spec.install_hint);
        match tools.find(TEXTWEAVER) {
            Ok(ToolStatus::Found(program)) => Outcome {
                said: vec![Message::info(format!("Opening {object}"))],
                request: Some(QuickViewRequest::RunProgram {
                    program,
                    args: vec![OsString::from(path)],
                }),
            },
            Ok(ToolStatus::Disabled) => vec![Message::could_not(
                "open",
                &object,
                "textweaver is turned off in tools.toml",
                "Turn it on in tools.toml",
            )]
            .into(),
            Ok(ToolStatus::NotFound) | Err(_) => vec![Message::could_not(
                "open",
                &object,
                "textweaver is not installed",
                hint,
            )]
            .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use diroxide_config::{ToolOverride, ToolsConfig};

    use super::super::tests::{Bench, texts};
    use super::*;
    use crate::quickview::QuickViewOptions;

    fn shown_bench() -> Bench {
        let mut bench = Bench::new(QuickViewOptions::default());
        let place = bench.file("quarter.md", b"# Quarter\n");
        bench.on(&place);
        bench
    }

    #[test]
    fn found_it_hands_the_file_over_with_no_shell() {
        let bench = shown_bench();
        let folder = tempfile::tempdir().unwrap();
        let program = if cfg!(windows) { "tw.exe" } else { "tw" };
        std::fs::write(folder.path().join(program), b"").unwrap();
        let tools = ToolRegistry::builtin().with_path(Some(folder.path().as_os_str().to_owned()));
        let out = bench.view.open_in_textweaver(&tools);
        assert_eq!(texts(&out.said), ["Opening quarter.md in textweaver"]);
        let place = bench.view.shown().unwrap().place.clone();
        match out.request {
            Some(QuickViewRequest::RunProgram { program, args }) => {
                assert!(program.starts_with(folder.path()), "{program:?}");
                assert_eq!(args, [OsString::from(place.local_path().unwrap())]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_it_says_how_to_install() {
        let bench = shown_bench();
        let empty = tempfile::tempdir().unwrap();
        let tools = ToolRegistry::builtin().with_path(Some(empty.path().as_os_str().to_owned()));
        let out = bench.view.open_in_textweaver(&tools);
        assert_eq!(
            texts(&out.said),
            [
                "Could not open quarter.md in textweaver: textweaver is not installed. \
                 Install textweaver from its releases page or with cargo."
            ]
        );
        assert_eq!(out.request, None);
        let mut overrides = ToolsConfig::default();
        overrides.tools.insert(
            TEXTWEAVER.to_owned(),
            ToolOverride {
                path: None,
                disabled: true,
            },
        );
        let off = bench
            .view
            .open_in_textweaver(&ToolRegistry::builtin().with_overrides(overrides));
        assert_eq!(
            texts(&off.said),
            [
                "Could not open quarter.md in textweaver: textweaver is turned off in \
                 tools.toml. Turn it on in tools.toml."
            ]
        );
    }

    #[test]
    fn nothing_shown_or_not_on_this_computer_is_not_handed_over() {
        let bench = Bench::new(QuickViewOptions::default());
        let tools = ToolRegistry::builtin().with_path(None);
        let out = bench.view.open_in_textweaver(&tools);
        assert_eq!(texts(&out.said), ["Nothing shown to open in textweaver"]);
        let mut remote = Bench::new(QuickViewOptions::default());
        let fake = remote.log.fake.clone();
        let mut registry = diroxide_vfs::Registry::new();
        registry.register("fake", std::sync::Arc::new(fake.clone()));
        remote.view = QuickView::new(registry, QuickViewOptions::default());
        let place = diroxide_core::Place::parse("fake://host/share/notes.md").unwrap();
        fake.add_folder(&place.parent().unwrap());
        fake.add_file(&place, b"# Notes\n");
        let entry = fake.entry(&place).unwrap();
        let at = remote.at(0);
        remote.view.turn_on(Some((&place, &entry)), at);
        remote.view.settle(at, std::time::Duration::from_secs(10));
        let out = remote.view.open_in_textweaver(&tools);
        assert_eq!(
            texts(&out.said),
            [
                "Could not open notes.md in textweaver: textweaver opens files on this \
                 computer only. Copy it out first."
            ]
        );
        assert_eq!(out.request, None);
    }
}
