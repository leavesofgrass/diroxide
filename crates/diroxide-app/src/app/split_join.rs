//! Split and join (plan commit 65): Split file offers the presets from the
//! settings as a list, each row its use first and its size after
//! ("Email attachment, 18 MB, Parts of 18,000,000 bytes"), then equal
//! parts and a custom size; the plan's rows say the outcome before
//! anything is written ("4 parts of 18 MB, the last 6.0 MB"). Join files
//! starts from any part; its verdict is said after the run ("Joined
//! big.bin: checksum matches.") and a verified join offers to trash the
//! parts, starting on No. Undo trashes what either made. The Settings
//! list shows the split presets as a list to read.

use diroxide_core::{
    Command, Effect, JobKind, JobRequest, ListModel, ListRow, Message, PartSize, Place, PlaceKind,
    Side,
};

use super::{App, Purpose, Then};
use crate::commands::ids;
use crate::dialog::{Dialog, with_default_no};
use crate::entry::{CompletionSource, EntryField};

/// A size typed with its unit: "20MB" (powers of 1,000), "100MiB" or
/// "100M" (powers of 1,024), "1.5GB", "4096" (bytes); a space before the
/// unit is allowed. `None` for zero or anything else.
pub(crate) fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let at = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(at);
    let unit_bytes: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" | "bytes" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "k" | "kib" => 1024.0,
        "m" | "mib" => 1_048_576.0,
        "g" | "gib" => 1_073_741_824.0,
        _ => return None,
    };
    let value: f64 = number.parse().ok()?;
    let bytes = (value * unit_bytes).round();
    (1.0..1.8e19).contains(&bytes).then_some(bytes as u64)
}

impl App {
    /// Split file: the presets as a list, then equal parts and a custom
    /// size, for the focused file or each marked file, into the other
    /// panel.
    pub(super) fn split_dialog(&mut self) -> Vec<Effect> {
        let side = self.active;
        let Some((sources, note)) = self.frozen(side) else {
            let empty = self.panel(side).list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        let files: Vec<Place> = sources
            .into_iter()
            .filter(|p| {
                p.name()
                    .and_then(|n| self.panel(side).entry(n))
                    .is_some_and(|e| {
                        !e.is_folder_like() || e.kind == diroxide_core::EntryKind::Archive
                    })
            })
            .collect();
        if files.is_empty() {
            return vec![self.say(Message::info("Split works on files. No file is chosen."))];
        }
        let destination = self.panel(side.other()).place().clone();
        if destination.kind() == PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        let mut rows: Vec<ListRow> = self
            .settings
            .split_presets
            .iter()
            .enumerate()
            .map(|(i, preset)| {
                ListRow::new(format!("preset:{i}"), preset.row_text())
                    .with_details([preset.details_line()])
            })
            .collect();
        rows.push(ListRow::new("equal", "Equal parts, a number you choose"));
        rows.push(ListRow::new("custom", "Custom size, with its unit"));
        let mut list = ListModel::new("Split into").with_rows(rows);
        list.set_help("Up and Down choose a size, Enter plans the split, Escape closes.");
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SPLIT),
            },
            Purpose::SplitSize {
                side,
                files,
                destination,
                note,
            },
        )
    }

    /// A size chosen: a preset plans the split; equal parts and a custom
    /// size ask first.
    pub(super) fn split_size_chosen(
        &mut self,
        side: Side,
        files: Vec<Place>,
        destination: Place,
        note: Option<String>,
        row: &str,
    ) -> Vec<Effect> {
        let mut effects = self.close_top();
        if let Some(index) = row
            .strip_prefix("preset:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            let Some(preset) = self.settings.split_presets.get(index) else {
                return effects;
            };
            let size = PartSize::Bytes(preset.part_bytes());
            effects.extend(self.plan_split(side, files, destination, note, size));
            return effects;
        }
        let (label, equal) = match row {
            "equal" => ("Number of parts", true),
            _ => ("Part size, such as 20MB or 100MiB", false),
        };
        let field = EntryField::new(label, CompletionSource::None);
        effects.extend(self.open_dialog(
            Dialog::edit(field),
            Purpose::SplitTyped {
                side,
                files,
                destination,
                note,
                equal,
            },
        ));
        effects
    }

    /// The number of parts or the part size typed.
    pub(super) fn split_typed(
        &mut self,
        side: Side,
        files: Vec<Place>,
        destination: Place,
        note: Option<String>,
        equal: bool,
        text: &str,
    ) -> Vec<Effect> {
        let size = if equal {
            text.trim()
                .parse::<u32>()
                .ok()
                .filter(|n| *n >= 2)
                .map(PartSize::EqualParts)
        } else {
            parse_size(text).map(PartSize::Bytes)
        };
        let mut effects = self.close_top();
        match size {
            Some(size) => effects.extend(self.plan_split(side, files, destination, note, size)),
            None => {
                let line = if equal {
                    "Type a whole number of parts, 2 or more. Nothing was split."
                } else {
                    "Type a size with its unit, such as 20MB. Nothing was split."
                };
                effects.push(self.say(Message::error(line)));
            }
        }
        effects
    }

    fn plan_split(
        &mut self,
        side: Side,
        files: Vec<Place>,
        destination: Place,
        note: Option<String>,
        size: PartSize,
    ) -> Vec<Effect> {
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Split, files)
            .to(destination)
            .split_into(size);
        self.plan_job(side, request, note, Then::Report, true)
    }

    /// Join files: from the focused part, the whole into the other panel.
    pub(super) fn join_files(&mut self) -> Vec<Effect> {
        let side = self.active;
        let panel = self.panel(side);
        let (Some(entry), Some(place)) = (panel.focused_entry(), panel.focused_place()) else {
            let empty = panel.list().empty_text().to_owned();
            return vec![self.say(Message::status(empty))];
        };
        if diroxide_core::names::split_part(&entry.name).is_none() {
            let text = format!(
                "Not a part: {}. Join starts from a numbered part, such as report.pdf.001.",
                entry.display_name()
            );
            return vec![self.say(Message::info(text))];
        }
        let destination = self.panel(side.other()).place().clone();
        if destination.kind() == PlaceKind::Archive {
            return vec![self.say(Message::error(super::archive::READ_ONLY))];
        }
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Join, vec![place]).to(destination);
        self.plan_job(side, request, None, Then::Report, true)
    }

    /// After a verified join, the offer to trash its parts, starting on
    /// No ("Move the 4 parts to the trash? No is the default. y or n").
    pub(super) fn offer_trash_parts(&mut self, outcome: &diroxide_ops::JoinOutcome) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(offer) = outcome.trash_offer() {
            effects.extend(
                self.open_dialog(
                    Dialog::Confirm {
                        question: with_default_no(offer),
                        listing: outcome
                            .parts
                            .iter()
                            .map(|p| diroxide_core::names::spell_out(&p.to_string()).into_owned())
                            .collect(),
                        on_yes: Command::new(ids::YES),
                    },
                    Purpose::TrashParts(outcome.parts.clone()),
                ),
            );
        }
        effects
    }

    /// Yes to the offer: the parts to the trash, as a job of its own.
    pub(super) fn trash_parts(&mut self, parts: Vec<Place>) -> Vec<Effect> {
        let side = self.active;
        let job = self.next_job_id();
        let request = JobRequest::new(job, JobKind::Trash, parts);
        self.plan_job(side, request, None, Then::Report, false)
    }

    /// The split presets row of the Settings list: the presets as a list
    /// to read, each its use first and its exact bytes after.
    pub(super) fn split_presets_list(&mut self) -> Vec<Effect> {
        let rows: Vec<ListRow> = self
            .settings
            .split_presets
            .iter()
            .enumerate()
            .map(|(i, preset)| {
                ListRow::new(format!("line-{i}"), preset.row_text())
                    .with_details([preset.details_line()])
            })
            .collect();
        let title = format!(
            "Split presets, {}",
            diroxide_core::format::count(rows.len() as u64, "preset", "presets")
        );
        let mut list = ListModel::new(title).with_rows(rows);
        list.set_help(
            "Up and Down read. Add your own to config.toml as split_presets, each a name and its bytes. Escape closes.",
        );
        self.open_dialog(
            Dialog::Choose {
                list,
                on_enter: Command::new(ids::SETTINGS),
            },
            Purpose::Read,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_read_with_their_units() {
        assert_eq!(parse_size("20MB"), Some(20_000_000));
        assert_eq!(parse_size("100MiB"), Some(104_857_600));
        assert_eq!(parse_size("100 m"), Some(104_857_600));
        assert_eq!(parse_size("1.5GB"), Some(1_500_000_000));
        assert_eq!(parse_size("4096"), Some(4096));
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("ten"), None);
        assert_eq!(parse_size("5 parsecs"), None);
    }
}
