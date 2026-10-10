//! The in-process Lua tier, behind the `lua` feature: `mlua` with a
//! vendored Lua 5.4, for the few things where starting a process is too
//! slow (column formatters, per-row filters, key macros), with the same API
//! names as the protocol so a script can move between tiers.
//!
//! Phase 0: the feature compiles this empty tier and nothing more. Wave 3
//! (W3b) adds `dep:mlua` to the feature and fills it in.

/// The Lua tier. Not built yet.
#[derive(Debug, Default)]
pub struct LuaTier {
    _private: (),
}

impl LuaTier {
    /// Whether the tier can run scripts. Always false in Phase 0.
    pub fn is_available(&self) -> bool {
        false
    }
}
