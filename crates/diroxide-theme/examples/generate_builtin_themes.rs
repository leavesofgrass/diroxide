// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Writes the built-in theme files (`themes/*.toml`) from star's palettes
//! and prints the table of contrast adjustments as Markdown.
//!
//! Run: `cargo run -p diroxide-theme --example generate_builtin_themes`

use std::path::PathBuf;

use diroxide_theme::star;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes");
    std::fs::create_dir_all(&dir)?;
    for p in &star::PALETTES {
        std::fs::write(
            dir.join(format!("{}.toml", p.name)),
            star::generated_file(p)?,
        )?;
    }
    println!("{}", star::adjustments_table()?);
    Ok(())
}
