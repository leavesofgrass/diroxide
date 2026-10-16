//! Property tests over path normalization as the provider meets it (tag
//! `prop`): places built from plain names round-trip through their text,
//! the extended-length prefix is stripped from anything shown and never
//! counted, and lengths are counted in UTF-16 units.

use diroxide_core::Place;
use diroxide_core::place::strip_extended_prefix;
use diroxide_vfs::limits::{can_open_with_system, can_start_program_in, utf16_units};
use proptest::prelude::*;

fn base() -> &'static str {
    if cfg!(windows) { "D:\\" } else { "/" }
}

/// Plain names: letters, digits, spaces inside, dots, CJK and an emoji;
/// never a separator or a hidden character.
fn plain_name() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9\u{E9}\u{65E5}\u{1F980}][a-zA-Z0-9 ._\u{E9}\u{65E5}-]{0,20}"
        .prop_filter("not a dot name", |name| name != "." && name != "..")
        .prop_map(|name| name.trim_end_matches(['.', ' ']).to_owned())
        .prop_filter("not empty", |name| !name.is_empty())
}

proptest! {
    #[test]
    fn places_from_plain_names_round_trip(names in prop::collection::vec(plain_name(), 1..12)) {
        let mut place = Place::parse(base()).unwrap();
        for name in &names {
            place = place.join(name).unwrap();
        }
        let text = place.to_string();
        prop_assert_eq!(Place::parse(&text).unwrap(), place.clone());
        prop_assert_eq!(place.name(), names.last().map(String::as_str));
        let mut up = place.clone();
        for _ in 0..names.len() {
            up = up.parent().unwrap();
        }
        prop_assert!(up.is_root());
    }

    #[test]
    fn the_prefix_is_never_shown_and_never_counted(names in prop::collection::vec(plain_name(), 1..30)) {
        let mut place = Place::parse(base()).unwrap();
        for name in &names {
            place = place.join(name).unwrap();
        }
        let plain = place.to_string();
        let prefixed = format!(r"\\?\{plain}");
        let stripped = strip_extended_prefix(&prefixed).into_owned();
        let unchanged = strip_extended_prefix(&plain).into_owned();
        prop_assert_eq!(&stripped, &plain);
        prop_assert_eq!(&unchanged, &plain);
        prop_assert_eq!(utf16_units(&prefixed), utf16_units(&plain));
        prop_assert_eq!(utf16_units(&plain), plain.encode_utf16().count());
        if cfg!(windows) {
            prop_assert_eq!(Place::parse(&prefixed).unwrap(), place.clone());
        }
        // The shell-bound limits follow the length alone.
        let units = utf16_units(&plain);
        prop_assert_eq!(can_start_program_in(&place), !cfg!(windows) || units <= 258);
        prop_assert_eq!(can_open_with_system(&place), !cfg!(windows) || units <= 259);
    }
}
