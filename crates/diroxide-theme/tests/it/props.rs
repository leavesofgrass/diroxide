// SPDX-License-Identifier: Apache-2.0 OR MIT
// Ported from textweaver's theme crate (textweaver-theme), relicensed by its
// author for diroxide.
//! Properties of the color arithmetic.

use diroxide_theme::color::{adjust_away, adjust_lightness};
use diroxide_theme::{Rgb, apca_lc, contrast_ratio};
use proptest::prelude::*;

fn rgb() -> impl Strategy<Value = Rgb> {
    any::<(u8, u8, u8)>().prop_map(|(r, g, b)| Rgb::new(r, g, b))
}

proptest! {
    #[test]
    fn hex_round_trips(c in rgb()) {
        prop_assert_eq!(Rgb::parse(&c.hex()), Ok(c));
        prop_assert_eq!(Rgb::parse(&c.hex().to_uppercase()), Ok(c));
    }

    #[test]
    fn contrast_is_symmetric_and_bounded(a in rgb(), b in rgb()) {
        let r = contrast_ratio(a, b);
        prop_assert!((r - contrast_ratio(b, a)).abs() < 1e-12);
        prop_assert!((1.0..=21.0 + 1e-9).contains(&r));
    }

    #[test]
    fn apca_polarity(a in rgb(), b in rgb()) {
        let lc = apca_lc(a, b);
        prop_assert!(lc.abs() <= 108.5);
        if lc > 0.0 {
            // Positive means darker text on a lighter background.
            prop_assert!(a.relative_luminance() < b.relative_luminance());
        }
    }

    #[test]
    fn adjusted_colors_pass(c in rgb(), bg in rgb(), min in 1.5f64..7.0) {
        if let Some(fixed) = adjust_lightness(c, &[bg], min) {
            prop_assert!(contrast_ratio(fixed, bg) >= min);
        }
        if let Some(fixed) = adjust_away(c, bg, min) {
            prop_assert!(contrast_ratio(fixed, bg) >= min);
        }
    }

    #[test]
    fn a_passing_color_is_always_reachable_at_aa(c in rgb(), bg in rgb()) {
        // Black or white always gives 4.5:1 against any background.
        prop_assert!(adjust_lightness(c, &[bg], 4.5).is_some());
    }
}
