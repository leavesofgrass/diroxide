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
