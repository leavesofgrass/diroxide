//! Integration tests of the tools registry's contract.

use diroxide_tools::{BUILTIN, ToolRegistry};

#[test]
fn every_tool_has_words_for_settings() {
    let tools = ToolRegistry::builtin().with_path(None);
    for spec in BUILTIN {
        assert!(!spec.purpose.is_empty() && spec.install_hint.ends_with('.'));
        assert!(tools.describe(spec.id).starts_with(spec.name));
    }
}

#[test]
fn the_downloader_fallbacks_and_textweaver_are_known() {
    let tools = ToolRegistry::builtin();
    for id in ["tw", "curl", "wget"] {
        assert!(tools.spec(id).is_some(), "{id}");
    }
    assert_eq!(diroxide_tools::CURL_SAFE_ARGS[0], "-q");
}
