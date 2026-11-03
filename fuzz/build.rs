//! The fuzz targets' link step on Windows. With the MSVC toolchain,
//! cargo-fuzz gives every crate it builds the linker argument
//! `/include:main`, so the linker takes libFuzzer's `main` from
//! libfuzzer-sys. One dependency of the archive readers, sevenz-rust2,
//! is also built as a DLL, and a DLL given that argument fails to link
//! with "unresolved external symbol main". So on Windows the targets are
//! built with `cargo +nightly fuzz build --no-include-main-msvc`, and
//! this script gives the argument to the fuzz targets alone
//! (docs/dev/fuzzing.md). On every other system it does nothing.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-link-arg-bins=/include:main");
    }
}
