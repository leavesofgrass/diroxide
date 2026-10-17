//! The encoding module against one sample per case in
//! `fixtures/viewer/encoding/`: what the viewer decides for each file.

use std::path::PathBuf;

use diroxide_preview::encoding::{Chosen, binary_kind, choose, decode};

fn sample(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/viewer/encoding")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn each_text_sample_decodes_as_text_with_its_encoding() {
    let cafe = "Caf\u{e9} notes.\nSecond line, na\u{ef}ve.\nThird line.\n";
    let cases = [
        ("utf8.txt", "UTF-8", Chosen::Utf8, cafe),
        ("utf8-bom.txt", "UTF-8", Chosen::ByteOrderMark, cafe),
        ("utf16le-bom.txt", "UTF-16LE", Chosen::ByteOrderMark, cafe),
        ("utf16be-bom.txt", "UTF-16BE", Chosen::ByteOrderMark, cafe),
        (
            "utf16le-no-bom.txt",
            "UTF-16LE",
            Chosen::Detected,
            "Hello from a Windows tool.\nSecond line.\n",
        ),
        (
            "windows-1252.txt",
            "windows-1252",
            Chosen::NotUtf8,
            "Quote: \u{201c}caf\u{e9}\u{201d}.\nSecond line.\n",
        ),
    ];
    for (name, encoding, chosen, text) in cases {
        let bytes = sample(name);
        assert_eq!(binary_kind(&bytes), None, "{name} is text");
        let decoded = decode(&bytes, None);
        assert_eq!(
            (decoded.text.as_str(), decoded.encoding),
            (text, encoding),
            "{name}"
        );
        let choice = choose(&bytes, None);
        assert_eq!(choice.chosen, chosen, "{name}");
        assert_eq!(choice.encoding.name(), encoding, "{name}");
    }
}

#[test]
fn utf16_with_a_mark_is_text_and_a_png_is_binary() {
    let utf16 = sample("utf16le-bom.txt");
    assert!(utf16.contains(&0), "the sample has NUL bytes");
    assert_eq!(binary_kind(&utf16), None);
    assert_eq!(
        choose(&utf16, None).line().as_deref(),
        Some("Encoding: UTF-16 (byte-order mark)")
    );
    assert_eq!(binary_kind(&sample("pixel.png")), Some("an image"));
    assert_eq!(binary_kind(&sample("data.bin")), Some("a binary file"));
}
