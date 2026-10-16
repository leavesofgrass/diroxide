// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Following the operating system's light, dark, and high-contrast setting.
//!
//! The decision is pure ([`theme_for_os_scheme`], [`follow_os`],
//! [`startup_theme`]) and so are the parsers for each platform's settings
//! ([`parse_windows`], [`parse_macos`], [`parse_linux`]); [`probe`] is the
//! small impure part that reads the settings, with a time limit, and never
//! fails (it answers [`OsScheme::Unknown`]).
//!
//! Sources, per platform:
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
//!   `AppsUseLightTheme` (0 dark, 1 light) and `HKCU\Control Panel\Accessibility\HighContrast`
//!   `Flags` (bit 1, `HCF_HIGHCONTRASTON`), read with `reg query`.
//! - macOS: `defaults read -g AppleInterfaceStyle` (`Dark`, or absent for
//!   light) and `defaults read com.apple.universalaccess increaseContrast`.
//! - Linux: `gsettings` `org.gnome.desktop.interface color-scheme`
//!   (`prefer-dark`, `prefer-light`, `default`) and
//!   `org.gnome.desktop.a11y.interface high-contrast`, then `GTK_THEME`
//!   (a `:dark` suffix or a `HighContrast` theme).

use std::time::Duration;

use crate::model::ThemeKind;

/// The operating system's appearance setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OsScheme {
    /// Dark mode.
    Dark,
    /// Light mode.
    Light,
    /// A high-contrast or increased-contrast mode.
    HighContrast,
    /// Not known; keep the saved theme.
    Unknown,
}

impl OsScheme {
    /// `dark`, `light`, `high-contrast`, or `unknown`, as star wrote them.
    pub fn key(self) -> &'static str {
        match self {
            OsScheme::Dark => "dark",
            OsScheme::Light => "light",
            OsScheme::HighContrast => "high-contrast",
            OsScheme::Unknown => "unknown",
        }
    }

    /// Reads a key; anything else is `Unknown`.
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => OsScheme::Dark,
            "light" => OsScheme::Light,
            "high-contrast" | "high_contrast" | "contrast" => OsScheme::HighContrast,
            _ => OsScheme::Unknown,
        }
    }
}

/// star's `theme_for_os_scheme`: dark → `galaxy`, light → `galaxy-light`,
/// high contrast → `high-contrast`, unknown → `None` (leave the saved theme).
pub fn theme_for_os_scheme(scheme: OsScheme) -> Option<&'static str> {
    match scheme {
        OsScheme::Dark => Some("galaxy"),
        OsScheme::Light => Some("galaxy-light"),
        OsScheme::HighContrast => Some("high-contrast"),
        OsScheme::Unknown => None,
    }
}

/// What a theme looks like, for [`follow_os`]: its kind and its light or
/// dark partner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeFacts<'a> {
    /// Light, dark, or high contrast.
    pub kind: ThemeKind,
    /// The partner theme's name, if any.
    pub counterpart: Option<&'a str>,
}

/// The theme to switch to when the OS scheme is `scheme` and the current
/// theme is `current`; `None` to keep it. Improves on star by staying within
/// the current theme's light/dark pair (a `solarized-dark` user in light mode
/// gets `solarized-light`, not `galaxy-light`); themes without a partner
/// fall back to star's mapping. A high-contrast theme is kept in high-contrast
/// mode.
pub fn follow_os(current: Option<ThemeFacts<'_>>, scheme: OsScheme) -> Option<String> {
    let want = match scheme {
        OsScheme::Unknown => return None,
        OsScheme::HighContrast => ThemeKind::HighContrast,
        OsScheme::Dark => ThemeKind::Dark,
        OsScheme::Light => ThemeKind::Light,
    };
    if let Some(cur) = current {
        if cur.kind == want {
            return None;
        }
        if want != ThemeKind::HighContrast
            && cur.kind != ThemeKind::HighContrast
            && let Some(partner) = cur.counterpart
        {
            return Some(partner.to_owned());
        }
    }
    theme_for_os_scheme(scheme).map(str::to_owned)
}

/// star's `_maybe_follow_os_theme`: at startup, follow the OS only when
/// following is on and the user has not picked a theme explicitly. Returns
/// the theme to switch to, or `None` to keep `saved`. Switching this way is
/// not an explicit choice, so the next launch follows again.
pub fn startup_theme(
    saved: Option<ThemeFacts<'_>>,
    follow_os_setting: bool,
    theme_explicit: bool,
    scheme: OsScheme,
) -> Option<String> {
    if !follow_os_setting || theme_explicit {
        return None;
    }
    follow_os(saved, scheme)
}

/// Reads a number from `reg query` output: a `REG_DWORD` (`0x1`, as
/// `AppsUseLightTheme` is stored) or a decimal `REG_SZ` (`126`, as the
/// high-contrast `Flags` are stored).
pub fn parse_reg_number(output: &str, value: &str) -> Option<u32> {
    output.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        if !it.next()?.eq_ignore_ascii_case(value) {
            return None;
        }
        let ty = it.next()?.to_ascii_uppercase();
        let v = it.next()?;
        match ty.as_str() {
            "REG_DWORD" => {
                let hex = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X"))?;
                u32::from_str_radix(hex, 16).ok()
            }
            "REG_SZ" => v.parse().ok(),
            _ => None,
        }
    })
}

/// Windows: `AppsUseLightTheme` and the high-contrast `Flags` value
/// (bit 1 on means high contrast).
pub fn parse_windows(
    apps_use_light_theme: Option<u32>,
    high_contrast_flags: Option<u32>,
) -> OsScheme {
    if high_contrast_flags.is_some_and(|f| f & 1 == 1) {
        return OsScheme::HighContrast;
    }
    match apps_use_light_theme {
        Some(0) => OsScheme::Dark,
        Some(_) => OsScheme::Light,
        None => OsScheme::Unknown,
    }
}

/// macOS: the output of `defaults read -g AppleInterfaceStyle` (`None` when
/// the command failed, which means light mode because the key is absent) and
/// of `defaults read com.apple.universalaccess increaseContrast`.
pub fn parse_macos(interface_style: Option<&str>, increase_contrast: Option<&str>) -> OsScheme {
    if increase_contrast.is_some_and(|s| matches!(s.trim(), "1" | "true" | "YES")) {
        return OsScheme::HighContrast;
    }
    match interface_style.map(str::trim) {
        Some(s) if s.eq_ignore_ascii_case("dark") => OsScheme::Dark,
        _ => OsScheme::Light,
    }
}

/// Linux: `gsettings get org.gnome.desktop.interface color-scheme` output
/// (`'prefer-dark'`), `gsettings get org.gnome.desktop.a11y.interface
/// high-contrast` output (`true`), and the `GTK_THEME` variable.
pub fn parse_linux(
    color_scheme: Option<&str>,
    high_contrast: Option<&str>,
    gtk_theme: Option<&str>,
) -> OsScheme {
    let gtk = gtk_theme.unwrap_or_default().to_ascii_lowercase();
    if high_contrast.is_some_and(|s| s.trim() == "true") || gtk.contains("highcontrast") {
        return OsScheme::HighContrast;
    }
    match color_scheme.map(|s| s.trim().trim_matches('\'')) {
        Some("prefer-dark") => return OsScheme::Dark,
        Some("prefer-light") => return OsScheme::Light,
        _ => {}
    }
    if gtk.ends_with(":dark") || gtk.contains("-dark") {
        return OsScheme::Dark;
    }
    if color_scheme.is_some_and(|s| s.trim().trim_matches('\'') == "default") {
        return OsScheme::Light;
    }
    OsScheme::Unknown
}

/// The interface text scale from Windows' "Text size" (`TextScaleFactor`
/// under `HKCU\SOFTWARE\Microsoft\Accessibility`, 100 to 225 percent):
/// 1.0 to 2.25, and 1.0 when it is not set.
pub fn parse_text_scale_windows(percent: Option<u32>) -> f64 {
    percent.map_or(1.0, |p| f64::from(p.clamp(100, 225)) / 100.0)
}

/// The interface text scale from GNOME's `text-scaling-factor` (the output
/// of `gsettings get org.gnome.desktop.interface text-scaling-factor`,
/// such as `1.25`): 1.0 to 2.25, and 1.0 when it is not set.
pub fn parse_text_scale_gnome(output: Option<&str>) -> f64 {
    output
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite())
        .map_or(1.0, |f| f.clamp(1.0, 2.25))
}

/// Reads the platform's interface text size (Windows' "Text size", GNOME's
/// text scaling factor), as a factor from 1.0 to 2.25; 1.0 elsewhere or
/// when it cannot be read. Takes a few milliseconds (one small process);
/// call it at startup.
pub fn text_scale() -> f64 {
    let limit = Duration::from_millis(500);
    if cfg!(windows) {
        let key = r"HKCU\SOFTWARE\Microsoft\Accessibility";
        let v = run("reg", &["query", key, "/v", "TextScaleFactor"], limit)
            .and_then(|o| parse_reg_number(&o, "TextScaleFactor"));
        parse_text_scale_windows(v)
    } else if cfg!(target_os = "macos") {
        1.0
    } else {
        let out = run(
            "gsettings",
            &["get", "org.gnome.desktop.interface", "text-scaling-factor"],
            limit,
        );
        parse_text_scale_gnome(out.as_deref())
    }
}

/// Runs a command with a time limit and no window, through an argument
/// list and never a shell; its standard output on success, read as UTF-8
/// with anything else replaced. A program still running at the limit is
/// stopped. (textweaver's crate used its core's process helper; diroxide's
/// tools registry has its own, so this small runner keeps the theme crate
/// free of workspace dependencies.)
fn run(program: &str, args: &[&str], limit: Duration) -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The process gets no console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                return status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).into_owned());
            }
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Reads the OS setting. Takes a few milliseconds (it starts one or two
/// small processes) and gives up after about half a second; call it at
/// startup or when the user asks, not per frame.
pub fn probe() -> OsScheme {
    let limit = Duration::from_millis(500);
    if cfg!(windows) {
        let q = |key: &str, value: &str| {
            run("reg", &["query", key, "/v", value], limit)
                .and_then(|o| parse_reg_number(&o, value))
        };
        let hc = q(r"HKCU\Control Panel\Accessibility\HighContrast", "Flags");
        let light = q(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "AppsUseLightTheme",
        );
        parse_windows(light, hc)
    } else if cfg!(target_os = "macos") {
        let style = run("defaults", &["read", "-g", "AppleInterfaceStyle"], limit);
        let contrast = run(
            "defaults",
            &["read", "com.apple.universalaccess", "increaseContrast"],
            limit,
        );
        parse_macos(style.as_deref(), contrast.as_deref())
    } else {
        let scheme = run(
            "gsettings",
            &["get", "org.gnome.desktop.interface", "color-scheme"],
            limit,
        );
        let hc = run(
            "gsettings",
            &["get", "org.gnome.desktop.a11y.interface", "high-contrast"],
            limit,
        );
        let gtk = std::env::var("GTK_THEME").ok();
        parse_linux(scheme.as_deref(), hc.as_deref(), gtk.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_reads_both_platforms() {
        let out = "\r\nHKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Accessibility\r\n    TextScaleFactor    REG_DWORD    0x96\r\n\r\n";
        let v = parse_reg_number(out, "TextScaleFactor");
        assert_eq!(v, Some(150));
        assert!((parse_text_scale_windows(v) - 1.5).abs() < 1e-9);
        assert!((parse_text_scale_windows(None) - 1.0).abs() < 1e-9);
        assert!((parse_text_scale_windows(Some(400)) - 2.25).abs() < 1e-9);
        assert!((parse_text_scale_gnome(Some("1.25\n")) - 1.25).abs() < 1e-9);
        assert!((parse_text_scale_gnome(Some("0.5")) - 1.0).abs() < 1e-9);
        assert!((parse_text_scale_gnome(None) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn star_mapping() {
        assert_eq!(theme_for_os_scheme(OsScheme::Dark), Some("galaxy"));
        assert_eq!(theme_for_os_scheme(OsScheme::Light), Some("galaxy-light"));
        assert_eq!(
            theme_for_os_scheme(OsScheme::HighContrast),
            Some("high-contrast")
        );
        assert_eq!(theme_for_os_scheme(OsScheme::Unknown), None);
        assert_eq!(OsScheme::from_key(" DARK "), OsScheme::Dark);
        assert_eq!(OsScheme::from_key("sepia"), OsScheme::Unknown);
    }

    #[test]
    fn follow_stays_in_the_pair() {
        let sol = ThemeFacts {
            kind: ThemeKind::Dark,
            counterpart: Some("solarized-light"),
        };
        assert_eq!(
            follow_os(Some(sol), OsScheme::Light).as_deref(),
            Some("solarized-light")
        );
        assert_eq!(follow_os(Some(sol), OsScheme::Dark), None);
        assert_eq!(
            follow_os(Some(sol), OsScheme::HighContrast).as_deref(),
            Some("high-contrast")
        );
        let nord = ThemeFacts {
            kind: ThemeKind::Dark,
            counterpart: None,
        };
        assert_eq!(
            follow_os(Some(nord), OsScheme::Light).as_deref(),
            Some("galaxy-light")
        );
        let hc = ThemeFacts {
            kind: ThemeKind::HighContrast,
            counterpart: None,
        };
        assert_eq!(follow_os(Some(hc), OsScheme::HighContrast), None);
        assert_eq!(
            follow_os(Some(hc), OsScheme::Dark).as_deref(),
            Some("galaxy")
        );
        assert_eq!(follow_os(None, OsScheme::Unknown), None);
        assert_eq!(follow_os(None, OsScheme::Dark).as_deref(), Some("galaxy"));
    }

    #[test]
    fn startup_respects_explicit_choice() {
        let g = ThemeFacts {
            kind: ThemeKind::Dark,
            counterpart: Some("galaxy-light"),
        };
        assert_eq!(
            startup_theme(Some(g), true, false, OsScheme::Light).as_deref(),
            Some("galaxy-light")
        );
        assert_eq!(startup_theme(Some(g), true, true, OsScheme::Light), None);
        assert_eq!(startup_theme(Some(g), false, false, OsScheme::Light), None);
    }

    #[test]
    fn windows_parsing() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\r\n    AppsUseLightTheme    REG_DWORD    0x0\r\n\r\n";
        assert_eq!(parse_reg_number(out, "AppsUseLightTheme"), Some(0));
        assert_eq!(parse_reg_number(out, "Other"), None);
        assert_eq!(
            parse_reg_number("    Flags    REG_SZ    126", "Flags"),
            Some(126)
        );
        assert_eq!(
            parse_reg_number("    Flags    REG_BINARY    00", "Flags"),
            None
        );
        assert_eq!(parse_windows(Some(0), Some(126)), OsScheme::Dark);
        assert_eq!(parse_windows(Some(1), Some(127)), OsScheme::HighContrast);
        assert_eq!(parse_windows(Some(1), None), OsScheme::Light);
        assert_eq!(parse_windows(None, None), OsScheme::Unknown);
    }

    #[test]
    fn macos_parsing() {
        assert_eq!(parse_macos(Some("Dark\n"), Some("0\n")), OsScheme::Dark);
        assert_eq!(parse_macos(None, None), OsScheme::Light);
        assert_eq!(parse_macos(Some("Dark"), Some("1")), OsScheme::HighContrast);
    }

    #[test]
    fn linux_parsing() {
        assert_eq!(
            parse_linux(Some("'prefer-dark'\n"), Some("false"), None),
            OsScheme::Dark
        );
        assert_eq!(
            parse_linux(Some("'prefer-light'"), None, None),
            OsScheme::Light
        );
        assert_eq!(parse_linux(Some("'default'"), None, None), OsScheme::Light);
        assert_eq!(
            parse_linux(Some("'default'"), None, Some("Adwaita:dark")),
            OsScheme::Dark
        );
        assert_eq!(
            parse_linux(None, Some("true\n"), None),
            OsScheme::HighContrast
        );
        assert_eq!(
            parse_linux(None, None, Some("HighContrast")),
            OsScheme::HighContrast
        );
        assert_eq!(parse_linux(None, None, None), OsScheme::Unknown);
    }

    #[test]
    #[ignore = "reads this machine's appearance setting (the registry, defaults or gsettings); run by hand"]
    fn probe_never_panics() {
        // Whatever this machine says, the answer is one of the four.
        let s = probe();
        assert!(matches!(
            s,
            OsScheme::Dark | OsScheme::Light | OsScheme::HighContrast | OsScheme::Unknown
        ));
    }
}
