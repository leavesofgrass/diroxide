// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Preview a theme: its contrast report, the color each role gets at every
//! level of terminal color support, and (with `--swatch`) sample lines in
//! the terminal's colors.
//!
//! ```text
//! cargo run -p diroxide-theme --example preview -- sepia
//! cargo run -p diroxide-theme --example preview -- path/to/mine.toml --swatch
//! cargo run -p diroxide-theme --example preview -- --all
//! ```
//!
//! The report is plain text and reads well with a screen reader; the swatch
//! is escape sequences for sighted checking.

use std::path::Path;
use std::time::Instant;

use diroxide_theme::{
    ColorRole, ColorSupport, Registry, StyleRole, TermColor, TermStyle, TerminalTheme, Theme, check,
};

fn describe(c: Option<TermColor>) -> String {
    match c {
        Some(TermColor::Rgb(rgb)) => rgb.hex(),
        Some(TermColor::Indexed(i)) => format!("index {i}"),
        None => "default".into(),
    }
}

fn sgr(s: TermStyle) -> String {
    let mut codes: Vec<String> = Vec::new();
    for (on, code) in [
        (s.attrs.bold, "1"),
        (s.attrs.italic, "3"),
        (s.attrs.underline, "4"),
        (s.attrs.reverse, "7"),
    ] {
        if on {
            codes.push(code.into());
        }
    }
    for (c, base) in [(s.fg, 38), (s.bg, 48)] {
        match c {
            Some(TermColor::Rgb(rgb)) => {
                codes.push(format!("{base};2;{};{};{}", rgb.r, rgb.g, rgb.b))
            }
            Some(TermColor::Indexed(i)) => codes.push(format!("{base};5;{i}")),
            None => {}
        }
    }
    format!("\x1b[0;{}m", codes.join(";"))
}

fn swatch(theme: &Theme, support: ColorSupport) {
    let tt = TerminalTheme::new(theme, support);
    let page = tt.page();
    let line = |parts: &[(TermStyle, &str)]| {
        let mut s = String::new();
        for (st, text) in parts {
            s.push_str(&sgr(page.patch(*st)));
            s.push_str(text);
        }
        s.push_str(&sgr(page));
        s.push_str(&" ".repeat(4));
        println!("{s}\x1b[0m");
    };
    println!("{support:?}:");
    line(&[(
        tt.style(StyleRole::StatusBar),
        " Status bar: Chapter 1, 12% ",
    )]);
    line(&[(tt.color(ColorRole::Heading1), "Heading 1  ")]);
    line(&[
        (tt.color(ColorRole::Heading2), "Heading 2  "),
        (tt.color(ColorRole::Heading3), "Heading 3  "),
        (tt.color(ColorRole::Heading4), "Heading 4"),
    ]);
    line(&[
        (TermStyle::default(), "Body text with a "),
        (tt.color(ColorRole::Link), "link"),
        (TermStyle::default(), ", "),
        (tt.color(ColorRole::Code), "code"),
        (TermStyle::default(), ", and "),
        (tt.color(ColorRole::DimText), "dim text"),
        (TermStyle::default(), "."),
    ]);
    let sentence = tt.style(StyleRole::SpokenSentence);
    line(&[
        (sentence, "The sentence being read, with the "),
        (sentence.patch(tt.style(StyleRole::SpokenWord)), "word"),
        (sentence, " spoken now."),
    ]);
    line(&[
        (tt.style(StyleRole::FindHit), "match"),
        (TermStyle::default(), " "),
        (tt.style(StyleRole::CurrentFindHit), "current match"),
        (TermStyle::default(), " "),
        (tt.style(StyleRole::Selection), "selected"),
        (TermStyle::default(), " "),
        (tt.style(StyleRole::Bookmark), "bookmark"),
        (TermStyle::default(), " "),
        (tt.style(StyleRole::Note), "note"),
    ]);
    let mut hl: Vec<(TermStyle, String)> = Vec::new();
    for (i, name) in tt.user_highlight_names().enumerate() {
        if let Some(s) = tt.user_highlight(i) {
            hl.push((s, name.to_owned()));
            hl.push((TermStyle::default(), " ".into()));
        }
    }
    let parts: Vec<(TermStyle, &str)> = hl.iter().map(|(s, t)| (*s, t.as_str())).collect();
    line(&parts);
    line(&[(tt.color(ColorRole::Error), "Error: example message.")]);
}

fn report(theme: &Theme) {
    let r = check(theme);
    println!(
        "{} ({}), {} theme. {}",
        theme.meta.display_name,
        theme.name(),
        theme.kind().label(),
        theme.meta.description
    );
    println!("{}", r.summary());
    let levels = [
        ColorSupport::TrueColor,
        ColorSupport::Ansi256,
        ColorSupport::Ansi16,
    ];
    let tts: Vec<TerminalTheme> = levels
        .iter()
        .map(|&l| TerminalTheme::new(theme, l))
        .collect();
    println!("Role: truecolor; 256 colors; 16 colors.");
    let page: Vec<String> = tts.iter().map(|t| describe(t.page().bg)).collect();
    println!("Page background: {}.", page.join("; "));
    for &role in ColorRole::ALL {
        let v: Vec<String> = tts
            .iter()
            .map(|t| {
                let s = t.page().patch(t.color(role));
                describe(if role.is_text() { s.fg } else { s.bg })
            })
            .collect();
        println!("{}: {}.", capital(role.label()), v.join("; "));
    }
    for &role in StyleRole::ALL {
        let v: Vec<String> = tts
            .iter()
            .map(|t| {
                let s = t.page().patch(t.style(role));
                format!("{} on {}{}", describe(s.fg), describe(s.bg), attrs(s))
            })
            .collect();
        println!("{}: {}.", capital(role.label()), v.join("; "));
    }
}

fn attrs(s: TermStyle) -> String {
    let n = s.attrs.names();
    if n.is_empty() {
        String::new()
    } else {
        format!(", {}", n.join(" "))
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show_swatch = args.iter().any(|a| a == "--swatch");
    let reg = Registry::builtin();
    let themes: Vec<Theme> = if args.iter().any(|a| a == "--all") {
        reg.themes().to_vec()
    } else {
        let name = args
            .iter()
            .find(|a| !a.starts_with("--"))
            .map_or("galaxy", String::as_str);
        if Path::new(name).extension().is_some_and(|e| e == "toml") {
            vec![Registry::load_file(Path::new(name))?]
        } else {
            vec![
                reg.get(name)
                    .ok_or_else(|| format!("no theme named {name}"))?
                    .clone(),
            ]
        }
    };
    let start = Instant::now();
    for t in &themes {
        report(t);
        if show_swatch {
            for level in [
                ColorSupport::TrueColor,
                ColorSupport::Ansi256,
                ColorSupport::Ansi16,
                ColorSupport::NoColor,
            ] {
                swatch(t, level);
            }
        }
        println!();
    }
    eprintln!(
        "{} theme(s) in {:.1} ms.",
        themes.len(),
        start.elapsed().as_secs_f64() * 1e3
    );
    Ok(())
}
