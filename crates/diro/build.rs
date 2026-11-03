//! Links the diroxide mark into `diro.exe` on Windows, so Explorer,
//! shortcuts and the taskbar show it, and the window loads it by id
//! (`ICON_RESOURCE` in `crates/diroxide-gui/src/window.rs`).
//! `assets/logo/icons/diroxide.res` is a compiled resource file written by
//! `cargo xtask icons` (`xtask/src/icons.rs`); the Microsoft linker takes
//! it as an input, so no resource compiler and no C are needed. Other
//! targets, and the GNU toolchain, build without it, and the window falls
//! back to the 64-pixel mark compiled into the GUI. Adapted from
//! textweaver's `crates/textweaver-xilem/build.rs` (the owner's own
//! project).

use std::path::Path;

fn main() {
    // The workspace root, two folders up from this crate's.
    let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) else {
        return;
    };
    let res = root
        .join("assets")
        .join("logo")
        .join("icons")
        .join("diroxide.res");
    println!("cargo:rerun-if-changed={}", res.display());
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc && res.is_file() {
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
