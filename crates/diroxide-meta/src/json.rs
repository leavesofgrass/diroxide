//! A small JSON reader for ExifTool's `-json` output (this crate has no
//! JSON dependency, and its manifest is the contract step's): values,
//! arrays and objects, strings with their escapes, numbers kept as the
//! text they were written as. Bounded: at most 32 levels deep and the
//! caller's byte cap; it never panics on any input.

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, as written.
    Number(String),
    /// A string.
    Text(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, its members in order.
    Object(Vec<(String, Json)>),
}

/// The deepest nesting read.
const MOST_DEPTH: usize = 32;

/// Reads one JSON value from `text`, with only white space around it.
pub fn parse(text: &str) -> Option<Json> {
    let mut reader = Reader {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = reader.value(0)?;
    reader.space();
    (reader.at == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.at) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> Option<()> {
        self.space();
        (self.bytes.get(self.at) == Some(&byte)).then(|| self.at += 1)
    }

    fn word(&mut self, word: &[u8]) -> Option<()> {
        let end = self.at.checked_add(word.len())?;
        (self.bytes.get(self.at..end)? == word).then(|| self.at = end)
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > MOST_DEPTH {
            return None;
        }
        self.space();
        match *self.bytes.get(self.at)? {
            b'n' => self.word(b"null").map(|()| Json::Null),
            b't' => self.word(b"true").map(|()| Json::Bool(true)),
            b'f' => self.word(b"false").map(|()| Json::Bool(false)),
            b'"' => self.text().map(Json::Text),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(Json::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b']')?;
                    return Some(Json::Array(items));
                }
            }
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(Json::Object(members));
                }
                loop {
                    self.space();
                    let key = self.text()?;
                    self.eat(b':')?;
                    members.push((key, self.value(depth + 1)?));
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b'}')?;
                    return Some(Json::Object(members));
                }
            }
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') =
                    self.bytes.get(self.at)
                {
                    self.at += 1;
                }
                let number = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
                Some(Json::Number(number.to_owned()))
            }
            _ => None,
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let end = self.at.checked_add(4)?;
        let digits = std::str::from_utf8(self.bytes.get(self.at..end)?).ok()?;
        let value = u32::from_str_radix(digits, 16).ok()?;
        self.at = end;
        Some(value)
    }

    fn text(&mut self) -> Option<String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return None;
        }
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            match byte {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let escape = *self.bytes.get(self.at)?;
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first)
                                && self.bytes.get(self.at..self.at + 2) == Some(b"\\u")
                            {
                                self.at += 2;
                                let second = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return None;
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else {
                                first
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return None,
                    };
                    let mut buffer = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
                other => out.push(other),
            }
        }
    }
}

impl Json {
    /// The value as words: text as it is, numbers as written, `yes` and
    /// `no`, arrays joined with commas; `None` for null and objects.
    pub fn words(&self) -> Option<String> {
        match self {
            Json::Null | Json::Object(_) => None,
            Json::Bool(b) => Some(if *b { "yes" } else { "no" }.to_owned()),
            Json::Number(n) => Some(n.clone()),
            Json::Text(t) => Some(t.clone()),
            Json::Array(items) => {
                let parts: Vec<String> = items.iter().filter_map(Json::words).collect();
                (!parts.is_empty()).then(|| parts.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exiftools_output_reads() {
        let text = r#"[{
  "SourceFile": "C:/photos/a b.jpg",
  "EXIF:Make": "SpikeCam",
  "EXIF:ISO": 100,
  "GPS:GPSLatitude": "45 deg 29' 54.96\" N",
  "XMP:Subject": ["sea", "sky"],
  "File:Comment": "caf\u00e9 \ud83d\ude00\n",
  "Flag": true, "Empty": null, "Nested": {"a": 1}
}]"#;
        let Json::Array(items) = parse(text).unwrap() else {
            panic!("an array")
        };
        let Json::Object(members) = &items[0] else {
            panic!("an object")
        };
        let words: Vec<(String, Option<String>)> = members
            .iter()
            .map(|(k, v)| (k.clone(), v.words()))
            .collect();
        assert_eq!(
            words[1],
            ("EXIF:Make".to_owned(), Some("SpikeCam".to_owned()))
        );
        assert_eq!(words[2].1.as_deref(), Some("100"));
        assert_eq!(words[3].1.as_deref(), Some("45 deg 29' 54.96\" N"));
        assert_eq!(words[4].1.as_deref(), Some("sea, sky"));
        assert_eq!(words[5].1.as_deref(), Some("caf\u{e9} \u{1F600}\n"));
        assert_eq!(words[6].1.as_deref(), Some("yes"));
        assert_eq!(words[7].1, None);
        assert_eq!(words[8].1, None);
    }

    #[test]
    fn bad_and_deep_input_is_refused() {
        for bad in [
            "",
            "[",
            "{\"a\" 1}",
            "[1,]",
            "\"\\q\"",
            "\"\\ud800\\u0041\"",
            "tru",
            "[1] x",
            "\"open",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        let deep = format!("{}{}", "[".repeat(40), "]".repeat(40));
        assert_eq!(parse(&deep), None);
        let fine = format!("{}{}", "[".repeat(30), "]".repeat(30));
        assert!(parse(&fine).is_some());
    }
}
