// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! sRGB colors and the arithmetic themes need: parsing and printing `#rrggbb`,
//! WCAG 2.x relative luminance and contrast ratio, APCA lightness contrast
//! (reported for information only), and OKLab lightness adjustment for
//! nudging a failing color to the nearest passing one.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An opaque 8-bit sRGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

/// Why a color string could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{input:?} is not a color; write it as #rrggbb, for example #1e1e1e")]
pub struct ColorParseError {
    /// The text that was rejected.
    pub input: String,
}

impl Rgb {
    /// Pure black.
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);
    /// Pure white.
    pub const WHITE: Rgb = Rgb::new(0xff, 0xff, 0xff);

    /// A color from its channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }

    /// A color from `0xRRGGBB`.
    pub const fn from_u32(v: u32) -> Self {
        Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Parses `#rrggbb` or `#rgb` (case-insensitive, surrounding whitespace
    /// ignored).
    pub fn parse(s: &str) -> Result<Self, ColorParseError> {
        let err = || ColorParseError {
            input: s.to_owned(),
        };
        let hex = s.trim().strip_prefix('#').ok_or_else(err)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        let nib = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map_err(|_| err());
        match hex.len() {
            6 => {
                let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
                Ok(Rgb::new(byte(0)?, byte(2)?, byte(4)?))
            }
            3 => Ok(Rgb::new(nib(0)? * 17, nib(1)? * 17, nib(2)? * 17)),
            _ => Err(err()),
        }
    }

    /// `#rrggbb`, lowercase.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// WCAG 2.x relative luminance, 0 (black) to 1 (white): each channel is
    /// linearized with the sRGB transfer function (knee at 0.03928, as the
    /// WCAG definition states) and weighted 0.2126, 0.7152, 0.0722.
    pub fn relative_luminance(self) -> f64 {
        let lin = |c: u8| {
            let v = f64::from(c) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// WCAG 2.x contrast ratio against `other`, from 1 to 21. Symmetric.
    pub fn contrast(self, other: Rgb) -> f64 {
        contrast_ratio(self, other)
    }

    /// True when this color is darker than mid-gray, so text on it should be
    /// light.
    pub fn is_dark(self) -> bool {
        // The luminance at which black and white text have equal contrast.
        self.relative_luminance() < 0.179
    }

    /// Linear interpolation in sRGB: `t = 0` is `self`, `t = 1` is `other`.
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let ch = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Rgb::new(
            ch(self.r, other.r),
            ch(self.g, other.g),
            ch(self.b, other.b),
        )
    }

    pub(crate) fn to_oklab(self) -> [f64; 3] {
        let lin = |c: u8| {
            let v = f64::from(c) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(self.r), lin(self.g), lin(self.b));
        let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
        let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
        let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        [
            0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
            1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
            0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
        ]
    }

    pub(crate) fn from_oklab([lightness, a, b]: [f64; 3]) -> Rgb {
        let l = lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
        let m = lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
        let s = lightness - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        let r = 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s;
        let g = -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s;
        let bl = -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s;
        let enc = |v: f64| {
            let v = v.clamp(0.0, 1.0);
            let e = if v <= 0.003_130_8 {
                12.92 * v
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0).round().clamp(0.0, 255.0) as u8
        };
        Rgb::new(enc(r), enc(g), enc(bl))
    }

    /// Perceptual distance (Euclidean in OKLab); 0 for identical colors,
    /// about 1 between black and white.
    pub fn distance(self, other: Rgb) -> f64 {
        let [l1, a1, b1] = self.to_oklab();
        let [l2, a2, b2] = other.to_oklab();
        ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt()
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl FromStr for Rgb {
    type Err = ColorParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Rgb::parse(s)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// WCAG 2.x contrast ratio `(L_hi + 0.05) / (L_lo + 0.05)`, from 1 to 21.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.relative_luminance(), b.relative_luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// APCA lightness contrast (Lc) of `text` on `background`, per APCA-W3
/// 0.0.98G-4g: positive for dark text on a light background, negative for
/// light text on a dark background, magnitude up to about 108.
///
/// Informational only: WCAG 2.x ratios are what the checks enforce.
pub fn apca_lc(text: Rgb, background: Rgb) -> f64 {
    const BLK_THRS: f64 = 0.022;
    const BLK_CLMP: f64 = 1.414;
    const DELTA_Y_MIN: f64 = 0.0005;
    const LO_CLIP: f64 = 0.1;
    const SCALE: f64 = 1.14;
    const OFFSET: f64 = 0.027;
    let y = |c: Rgb| {
        let ch = |v: u8| (f64::from(v) / 255.0).powf(2.4);
        let y = 0.212_672_9 * ch(c.r) + 0.715_152_2 * ch(c.g) + 0.072_175 * ch(c.b);
        if y > BLK_THRS {
            y
        } else {
            y + (BLK_THRS - y).powf(BLK_CLMP)
        }
    };
    let (yt, yb) = (y(text), y(background));
    if (yb - yt).abs() < DELTA_Y_MIN {
        return 0.0;
    }
    let out = if yb > yt {
        let sapc = (yb.powf(0.56) - yt.powf(0.57)) * SCALE;
        if sapc < LO_CLIP { 0.0 } else { sapc - OFFSET }
    } else {
        let sapc = (yb.powf(0.65) - yt.powf(0.62)) * SCALE;
        if sapc > -LO_CLIP { 0.0 } else { sapc + OFFSET }
    };
    out * 100.0
}

/// The nearest color to `color` (changing only its OKLab lightness, so hue
/// and chroma are kept where the sRGB gamut allows) whose contrast against
/// every color in `against` is at least `min`. Tries both lighter and darker
/// and returns the smaller change, or `None` when neither direction reaches
/// `min`. Returns `color` itself when it already passes.
pub fn adjust_lightness(color: Rgb, against: &[Rgb], min: f64) -> Option<Rgb> {
    let passes = |c: Rgb| against.iter().all(|&a| contrast_ratio(c, a) >= min);
    if passes(color) {
        return Some(color);
    }
    let [l0, a, b] = color.to_oklab();
    const STEP: f64 = 0.001;
    let search = |dir: f64| {
        let mut l = l0;
        for step in 1..=1200 {
            l = l0 + dir * STEP * f64::from(step);
            if !(0.0..=1.0).contains(&l) {
                break;
            }
            let c = Rgb::from_oklab([l, a, b]);
            if passes(c) {
                return Some(((l - l0).abs(), c));
            }
        }
        // The last in-gamut attempt: pure black or white.
        let end = if dir > 0.0 { Rgb::WHITE } else { Rgb::BLACK };
        passes(end).then(|| ((l.clamp(0.0, 1.0) - l0).abs() + 1.0, end))
    };
    match (search(1.0), search(-1.0)) {
        (Some(up), Some(down)) => Some(if up.0 <= down.0 { up.1 } else { down.1 }),
        (Some(one), None) | (None, Some(one)) => Some(one.1),
        (None, None) => None,
    }
}

/// Like [`adjust_lightness`], but only moves away from `from` (lighter when
/// `from` is darker than `color`, darker otherwise): used to nudge a
/// background away from the text on it without passing through it.
pub fn adjust_away(color: Rgb, from: Rgb, min: f64) -> Option<Rgb> {
    if contrast_ratio(color, from) >= min {
        return Some(color);
    }
    let dir = if from.relative_luminance() <= color.relative_luminance() {
        1.0
    } else {
        -1.0
    };
    let [l0, a, b] = color.to_oklab();
    for step in 1..=1200 {
        let l = l0 + dir * 0.001 * f64::from(step);
        if !(0.0..=1.0).contains(&l) {
            break;
        }
        let c = Rgb::from_oklab([l, a, b]);
        if contrast_ratio(c, from) >= min {
            return Some(c);
        }
    }
    let end = if dir > 0.0 { Rgb::WHITE } else { Rgb::BLACK };
    (contrast_ratio(end, from) >= min).then_some(end)
}

/// A ratio as it reads aloud: `4.5 to 1`, one decimal, truncated rather than
/// rounded so a failing 4.49 never reads as a passing 4.5.
pub fn spoken_ratio(ratio: f64) -> String {
    format!("{:.1} to 1", (ratio * 10.0).floor() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        assert_eq!(Rgb::parse("#1E1E1E"), Ok(Rgb::new(0x1e, 0x1e, 0x1e)));
        assert_eq!(Rgb::parse(" #fa0 "), Ok(Rgb::new(0xff, 0xaa, 0x00)));
        assert_eq!(Rgb::from_u32(0x123456).hex(), "#123456");
        for bad in [
            "", "#", "123456", "#12345", "#gg0000", "#1234567", "red", "#+1+2+3",
        ] {
            assert!(Rgb::parse(bad).is_err(), "{bad}");
        }
        let e = Rgb::parse("teal").unwrap_err();
        assert!(e.to_string().contains("#rrggbb"));
    }

    #[test]
    fn wcag_reference_values() {
        assert!((contrast_ratio(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(Rgb::WHITE, Rgb::WHITE) - 1.0).abs() < 1e-9);
        // WebAIM: #767676 on white is the classic 4.54:1 gray.
        let r = contrast_ratio(Rgb::from_u32(0x767676), Rgb::WHITE);
        assert!((r - 4.54).abs() < 0.01, "{r}");
        // #777777 on white is the classic near miss, 4.48:1.
        let r = contrast_ratio(Rgb::from_u32(0x777777), Rgb::WHITE);
        assert!((r - 4.48).abs() < 0.01, "{r}");
    }

    #[test]
    fn apca_reference_values() {
        // Published APCA-W3 0.0.98G values.
        let bow = apca_lc(Rgb::BLACK, Rgb::WHITE);
        assert!((bow - 106.04).abs() < 0.1, "{bow}");
        let wob = apca_lc(Rgb::WHITE, Rgb::BLACK);
        assert!((wob + 107.88).abs() < 0.1, "{wob}");
        let mid = apca_lc(Rgb::from_u32(0x888888), Rgb::WHITE);
        assert!((mid - 63.06).abs() < 0.2, "{mid}");
        assert_eq!(apca_lc(Rgb::WHITE, Rgb::WHITE), 0.0);
    }

    #[test]
    fn oklab_round_trip() {
        for c in [
            0x000000, 0xffffff, 0x1e1e1e, 0xc9b6ff, 0x00ff00, 0x8a4510, 0x002b36,
        ] {
            let c = Rgb::from_u32(c);
            assert_eq!(Rgb::from_oklab(c.to_oklab()), c);
        }
    }

    #[test]
    fn adjust_reaches_the_target_minimally() {
        let bg = Rgb::from_u32(0x1e1e1e);
        let muted = Rgb::from_u32(0x7d7d7d);
        assert!(contrast_ratio(muted, bg) < 4.5);
        let fixed = adjust_lightness(muted, &[bg], 4.5).unwrap();
        assert!(contrast_ratio(fixed, bg) >= 4.5);
        assert!(fixed.relative_luminance() > muted.relative_luminance());
        // Minimal: one step less would fail.
        assert!(contrast_ratio(fixed, bg) < 4.6);
        // Already passing: unchanged.
        assert_eq!(adjust_lightness(Rgb::WHITE, &[bg], 4.5), Some(Rgb::WHITE));
        // Impossible.
        assert_eq!(
            adjust_lightness(Rgb::WHITE, &[Rgb::from_u32(0x777777)], 19.0),
            None
        );
    }

    #[test]
    fn adjust_away_moves_backgrounds_away_from_text() {
        let text = Rgb::from_u32(0xdadada);
        let band = Rgb::from_u32(0x6a6a6a);
        let fixed = adjust_away(band, text, 4.5).unwrap();
        assert!(fixed.relative_luminance() < band.relative_luminance());
        assert!(contrast_ratio(fixed, text) >= 4.5);
    }

    #[test]
    fn spoken_ratio_truncates() {
        assert_eq!(spoken_ratio(4.4999), "4.4 to 1");
        assert_eq!(spoken_ratio(21.0), "21.0 to 1");
        assert_eq!(spoken_ratio(7.06), "7.0 to 1");
    }

    #[test]
    fn serde_as_string() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            c: Rgb,
        }
        let w: W = toml::from_str("c = \"#ABCDEF\"").unwrap();
        assert_eq!(w.c, Rgb::from_u32(0xabcdef));
        assert_eq!(toml::to_string(&w).unwrap().trim(), "c = \"#abcdef\"");
        assert!(toml::from_str::<W>("c = \"blue\"").is_err());
    }
}
