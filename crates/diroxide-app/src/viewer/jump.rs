//! Jumps, reload and follow: going to a line, an offset or a percent and
//! saying where it landed; reading the file again; and following a file
//! that grows, saying how many lines arrived at the progress interval
//! rather than reading each one aloud.

use diroxide_core::Message;
use diroxide_core::format::{count, grouped};
use diroxide_vfs::Registry;

use super::{COMMAND_BUDGET, Landing, RowPos, Task, Viewer, ViewerError, ViewerSource, percent};

/// Where to jump, as typed in the jump prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Jump {
    /// A line, 1-based: "120", "line 120".
    Line(u64),
    /// A percent of the file: "50%", "50 percent".
    Percent(u64),
    /// A byte offset: "0x1f40", "offset 8000", "@8000".
    Offset(u64),
    /// The first line: "top", "start".
    Top,
    /// The last line: "end", "bottom".
    End,
}

impl Jump {
    /// Reads what was typed; the error is the line to say, meaning first:
    /// "Not a line, offset or percent: abc".
    pub fn parse(text: &str) -> Result<Self, String> {
        let typed = text.trim();
        let lower = typed.to_ascii_lowercase();
        let bad = || {
            if typed.is_empty() {
                "Type a line, an offset or a percent".to_owned()
            } else {
                format!("Not a line, offset or percent: {typed}")
            }
        };
        let number = |s: &str| -> Option<u64> { s.trim().replace(',', "").parse().ok() };
        if matches!(lower.as_str(), "top" | "start" | "first") {
            return Ok(Jump::Top);
        }
        if matches!(lower.as_str(), "end" | "bottom" | "last") {
            return Ok(Jump::End);
        }
        if let Some(hex) = lower.strip_prefix("0x") {
            return u64::from_str_radix(hex.trim(), 16)
                .map(Jump::Offset)
                .map_err(|_| bad());
        }
        if let Some(rest) = lower
            .strip_prefix("offset")
            .or_else(|| lower.strip_prefix('@'))
        {
            let rest = rest.trim();
            if let Some(hex) = rest.strip_prefix("0x") {
                return u64::from_str_radix(hex, 16)
                    .map(Jump::Offset)
                    .map_err(|_| bad());
            }
            return number(rest).map(Jump::Offset).ok_or_else(bad);
        }
        if let Some(rest) = lower
            .strip_suffix('%')
            .or_else(|| lower.strip_suffix("percent"))
        {
            return number(rest)
                .map(|p| Jump::Percent(p.min(100)))
                .ok_or_else(bad);
        }
        let line = lower.strip_prefix("line").unwrap_or(&lower);
        number(line).map(Jump::Line).ok_or_else(bad)
    }
}

impl Viewer {
    /// Jumps to what was typed (see [`Jump::parse`]) and says where it
    /// landed: "At line 120 of 4,000". A far line in a large file lands
    /// after [`Viewer::work`].
    pub fn jump(&mut self, typed: &str) -> Vec<Message> {
        match Jump::parse(typed) {
            Ok(jump) => self.jump_to(jump),
            Err(line) => vec![Message::warning(line)],
        }
    }

    /// Jumps to a place in the file.
    pub fn jump_to(&mut self, jump: Jump) -> Vec<Message> {
        let landing = Landing::Jump(jump);
        if self.mode == super::ViewerMode::Hex {
            let len = self.source.available();
            match jump {
                Jump::Offset(offset) => {
                    self.hex_go(offset);
                    let at = grouped(self.hex_cursor);
                    return vec![Message::info(if offset >= len {
                        format!("Past the end: at offset {at}, the last")
                    } else {
                        format!("At offset {at}")
                    })];
                }
                Jump::Percent(p) => {
                    self.hex_go((u128::from(len) * u128::from(p) / 100) as u64);
                    let at = grouped(self.hex_cursor);
                    return vec![Message::info(format!("At offset {at}, {p} percent"))];
                }
                Jump::Top | Jump::End | Jump::Line(_) => {}
            }
        }
        match jump {
            Jump::Top => {
                self.home();
                vec![Message::info(self.landed(jump, 0, false))]
            }
            Jump::End => self.end(),
            Jump::Line(n) => self.start_task(Task::Line {
                line: n.saturating_sub(1),
                landing,
            }),
            Jump::Percent(p) => match self.index.total() {
                Some(total) => {
                    let line = (u128::from(total.saturating_sub(1)) * u128::from(p) / 100) as u64;
                    self.start_task(Task::Line { line, landing })
                }
                None => {
                    let offset = (u128::from(self.source.available()) * u128::from(p) / 100) as u64;
                    self.start_task(Task::Offset { offset, landing })
                }
            },
            Jump::Offset(offset) => {
                if offset >= self.source.available() {
                    self.start_task(Task::Line {
                        line: u64::MAX,
                        landing,
                    })
                } else {
                    self.start_task(Task::Offset { offset, landing })
                }
            }
        }
    }

    /// The words for where a jump landed, meaning first.
    pub(super) fn landed(&self, jump: Jump, line: u64, past: bool) -> String {
        let at = grouped(line + 1);
        let of = match self.index.total() {
            Some(total) => format!(" of {}", grouped(total)),
            None => String::new(),
        };
        if self.mode == super::ViewerMode::Hex && matches!(jump, Jump::Line(_)) {
            let offset = grouped(self.hex_cursor);
            return if past {
                format!("Past the end: at line {at}, offset {offset}")
            } else {
                format!("At line {at}, offset {offset}")
            };
        }
        match jump {
            Jump::Offset(offset) if offset >= self.source.available() => {
                format!("Past the end: at line {at}, the last")
            }
            _ if past => format!("Past the end: at line {at}, the last"),
            Jump::Percent(_) => {
                let whole = self.index.total().unwrap_or(0);
                let p = if whole > 0 {
                    percent(line + 1, whole)
                } else {
                    let shown = self.lines.get(&line).map_or(0, |s| s.offset);
                    percent(shown, self.source.available().max(1))
                };
                format!("At line {at}{of}, {p} percent")
            }
            Jump::Offset(offset) => format!("At line {at}, offset {}", grouped(offset)),
            Jump::Line(_) | Jump::Top | Jump::End => format!("At line {at}{of}"),
        }
    }

    /// Reads the file again through the registry, keeping the cursor's
    /// line where it still exists: "Reloaded notes.txt, 4,012 lines".
    pub fn reload(&mut self, registry: &Registry) -> Vec<Message> {
        match ViewerSource::open(
            registry,
            &self.place,
            self.source.total(),
            self.options.stream_cap(),
        ) {
            Ok(source) => self.reload_with(source),
            Err(error) => vec![error.message(&self.name)],
        }
    }

    /// Reloads from a source the caller opened.
    pub fn reload_with(&mut self, source: ViewerSource) -> Vec<Message> {
        let line = self.cursor.line;
        self.source = source;
        if let Err(error) = self.load() {
            return vec![error.message(&self.name)];
        }
        self.cursor = RowPos::default();
        self.top = RowPos::default();
        if let Ok(Some((found, _))) = self.find_line(line, COMMAND_BUDGET) {
            self.cursor = RowPos {
                line: found,
                part: 0,
            };
        }
        self.keep_cursor_visible();
        let mut said = vec![Message::info(match self.index.total() {
            Some(total) => format!("Reloaded {}, {}", self.name, Viewer::lines_words(total)),
            None => format!("Reloaded {}", self.name),
        })];
        if let Some(policy) = self.policy_line() {
            said.push(Message::info(policy));
        }
        said
    }

    /// Turns follow on or off: on, the viewer goes to the end and stays
    /// there as the file grows. "Follow on".
    pub fn toggle_follow(&mut self) -> Vec<Message> {
        if !self.source.is_seekable() {
            return vec![Message::info("Follow works on files on disk only")];
        }
        self.follow = !self.follow;
        if self.follow {
            let mut said = vec![Message::info("Follow on")];
            self.task = None;
            self.start_task(Task::Line {
                line: u64::MAX,
                landing: Landing::Quiet,
            });
            said.extend(self.finish());
            said
        } else {
            vec![Message::info("Follow off")]
        }
    }

    /// Whether the viewer follows the file.
    pub fn follows(&self) -> bool {
        self.follow
    }

    /// Looks for new lines while following, at the progress interval:
    /// "12 new lines", the cursor on the last; a file that got shorter is
    /// read again from the top.
    pub fn follow_tick(&mut self) -> Vec<Message> {
        if !self.follow {
            return Vec::new();
        }
        let before = self.source.available();
        let lines_before = self.index.lines_known();
        let now = match self.source.refresh() {
            Ok(now) => now,
            Err(error) => return vec![ViewerError::from(error).message(&self.name)],
        };
        if now < before {
            if let Err(error) = self.load() {
                return vec![error.message(&self.name)];
            }
            self.home();
            return vec![Message::info("File got shorter; at line 1")];
        }
        if now == before {
            return Vec::new();
        }
        self.index.reopen();
        self.reset_search();
        // The last line may have grown.
        if let Some((&last, _)) = self.lines.iter().next_back() {
            self.lines.remove(&last);
        }
        self.task = None;
        self.start_task(Task::Line {
            line: u64::MAX,
            landing: Landing::Quiet,
        });
        self.finish();
        let new = self.index.lines_known().saturating_sub(lines_before);
        if new == 0 {
            return Vec::new();
        }
        vec![Message::info(count(new, "new line", "new lines"))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jump_grammar_reads_lines_offsets_and_percents() {
        let table: [(&str, Result<Jump, &str>); 14] = [
            ("120", Ok(Jump::Line(120))),
            ("line 4,000", Ok(Jump::Line(4000))),
            ("50%", Ok(Jump::Percent(50))),
            ("75 percent", Ok(Jump::Percent(75))),
            ("150%", Ok(Jump::Percent(100))),
            ("0x1F40", Ok(Jump::Offset(8000))),
            ("offset 8000", Ok(Jump::Offset(8000))),
            ("offset 0x10", Ok(Jump::Offset(16))),
            ("@512", Ok(Jump::Offset(512))),
            ("top", Ok(Jump::Top)),
            ("END", Ok(Jump::End)),
            ("abc", Err("Not a line, offset or percent: abc")),
            ("", Err("Type a line, an offset or a percent")),
            ("0xZZ", Err("Not a line, offset or percent: 0xZZ")),
        ];
        for (typed, expected) in table {
            assert_eq!(
                Jump::parse(typed),
                expected.map_err(str::to_owned),
                "{typed:?}"
            );
        }
    }
}
