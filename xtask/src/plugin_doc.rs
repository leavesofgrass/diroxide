//! `docs/dev/plugin-protocol.md`, generated from the methods module of
//! `diroxide-plugins`. `cargo xtask regen` writes it and `cargo xtask regen
//! --check` checks it; nobody edits it by hand.

/// The page, relative to the workspace root.
pub const PAGE: &str = diroxide_plugins::protocol::reference::PAGE;

/// The page's text.
pub fn generate() -> String {
    diroxide_plugins::protocol::reference::page()
}
