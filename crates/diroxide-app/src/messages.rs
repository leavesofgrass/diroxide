//! The app's message catalog: the sentences Wave 1 adds beside the core
//! catalog (`diroxide_core::catalog`), each with its meaning in the first
//! 40 cells, so the Braille walk test can read them all. A sentence that
//! names a key takes the key from the live keymap as an argument.
//!
//! Here: the settings and the startup problems line; the file clipboard;
//! the permissions sentences of research/permissions.md section 8 (R11),
//! for alpha.2's dialog and the column words now; the hot folders' polite
//! summaries (R16) and the compare verdict lines (R18), reserved for their
//! alphas. The secret prompt's words and the honest permanent-delete
//! sentence live in the core catalog; the conflict questions' words live
//! in the job engine (`Conflict::lines` and `choices`), and this module's
//! tests hold them to 40 cells too.

use diroxide_config::{ConfigProblem, problems_line};
use diroxide_core::Message;
use diroxide_core::format::count;

/// A number with thousands grouped by commas: "1,204".
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The startup line when the config files had problems, the files first:
/// "keymap.toml: 2 problems. Config problems in the Help menu lists
/// them." `None` when there were none.
pub fn startup_problems(problems: &[ConfigProblem]) -> Option<Message> {
    let line = problems_line(problems);
    (!line.is_empty()).then(|| {
        Message::warning(format!(
            "{line}. Config problems in the Help menu lists them."
        ))
    })
}

/// A setting changed in the Settings list: "Layout: stacked".
pub fn setting_changed(name: &str, value_words: &str) -> Message {
    Message::info(format!("{name}: {value_words}"))
}

/// A setting put back to its default: "Layout back to side by side".
pub fn setting_reset(name: &str, value_words: &str) -> Message {
    Message::info(format!("{name} back to {value_words}"))
}

/// Cut is pending until a paste: "Cut 3 items. Paste with Ctrl+V.", the
/// paste key from the live keymap; without one, the Edit menu.
pub fn cut_pending(items: u64, paste_key: &str) -> Message {
    let what = count(items, "item", "items");
    if paste_key.is_empty() {
        Message::info(format!("Cut {what}. Paste from the Edit menu."))
    } else {
        Message::info(format!("Cut {what}. Paste with {paste_key}."))
    }
}

/// Copied to the file clipboard: "Copied 3 items. Paste with Ctrl+V."
pub fn copied_to_clipboard(items: u64, paste_key: &str) -> Message {
    let what = count(items, "item", "items");
    if paste_key.is_empty() {
        Message::info(format!("Copied {what}. Paste from the Edit menu."))
    } else {
        Message::info(format!("Copied {what}. Paste with {paste_key}."))
    }
}

/// The permissions sentences of research/permissions.md section 8, words
/// first, the name or path last.
pub mod permissions {
    use diroxide_core::Message;
    use diroxide_core::format::count;

    /// The dialog's title for one entry: "Permissions for report.pdf".
    pub fn title(name: &str) -> String {
        format!("Permissions for {name}")
    }

    /// The dialog's title for several: "Permissions for 3 items".
    pub fn title_many(items: u64) -> String {
        format!("Permissions for {}", count(items, "item", "items"))
    }

    /// The Windows dialog's title: "Attributes for report.pdf".
    pub fn attributes_title(name: &str) -> String {
        format!("Attributes for {name}")
    }

    /// One row of the dialog: "Owner can read: yes", "Group can write:
    /// mixed, unchanged".
    pub fn row(who: &str, can: &str, state: &str) -> String {
        format!("{who} can {can}: {state}")
    }

    /// The octal row: "Octal: 644".
    pub fn octal_row(octal: &str) -> String {
        format!("Octal: {octal}")
    }

    /// The question before a change: "Change permissions of 12 items? y or
    /// n".
    pub fn change_question(items: u64) -> Message {
        Message::question(&format!(
            "Change permissions of {}",
            count(items, "item", "items")
        ))
    }

    /// Losing one's own access: "You would lose read access to 3 items.
    /// Continue? y or n".
    pub fn lose_access_question(right: &str, items: u64) -> Message {
        Message::question(&format!(
            "You would lose {right} access to {}. Continue",
            count(items, "item", "items")
        ))
    }

    /// Setuid: "Let report.sh run as its owner (setuid)? y or n".
    pub fn setuid_question(name: &str) -> Message {
        Message::question(&format!("Let {name} run as its owner (setuid)"))
    }

    /// Open to everyone: "Let everyone change report.pdf? y or n".
    pub fn everyone_question(name: &str) -> Message {
        Message::question(&format!("Let everyone change {name}"))
    }

    /// Done: "Changed permissions of 12 items."
    pub fn changed(items: u64) -> Message {
        Message::done(format!(
            "Changed permissions of {}.",
            count(items, "item", "items")
        ))
    }

    /// Undone: "Undid permission change of 12 items."
    pub fn undid(items: u64) -> Message {
        Message::done(format!(
            "Undid permission change of {}.",
            count(items, "item", "items")
        ))
    }

    /// Nothing to do: "Nothing to change: permissions already match."
    pub fn nothing_to_change() -> Message {
        Message::info("Nothing to change: permissions already match.")
    }

    /// A link: "Could not change permissions of link.txt: links have none.
    /// Change the target instead."
    pub fn links_have_none(name: &str) -> Message {
        Message::could_not(
            "change permissions of",
            name,
            "links have none",
            "Change the target instead",
        )
    }

    /// Not the owner: "Could not change permissions of a.txt: only its
    /// owner (ada) can. Ask the owner, or start diro as administrator."
    pub fn only_owner_can(name: &str, owner: &str) -> Message {
        Message::could_not(
            "change permissions of",
            name,
            &format!("only its owner ({owner}) can"),
            "Ask the owner, or start diro as administrator",
        )
    }

    /// A read-only disk: "Could not change permissions of a.txt: the disk
    /// is read only."
    pub fn disk_read_only(name: &str) -> Message {
        Message::could_not("change permissions of", name, "the disk is read only", "")
    }

    /// A disk without permissions: "Could not change permissions of a.txt:
    /// this disk does not keep them."
    pub fn disk_keeps_none(name: &str) -> Message {
        Message::could_not(
            "change permissions of",
            name,
            "this disk does not keep them",
            "",
        )
    }

    /// Open to everyone in a home or a protected place, refused.
    pub fn everyone_could_write(place: &str) -> Message {
        Message::could_not(
            "change permissions of",
            place,
            "everyone could write there",
            "Choose less access",
        )
    }

    /// A recursive change from a root, refused.
    pub fn too_much_below(place: &str) -> Message {
        Message::error(format!(
            "Could not change permissions below {place}: too much of the system. Choose a smaller folder."
        ))
    }

    /// An owner change that needs an administrator.
    pub fn owner_needs_administrator(name: &str) -> Message {
        Message::could_not(
            "change the owner of",
            name,
            "only an administrator can",
            "Start diro as administrator, then try again",
        )
    }

    /// A group the user is not in.
    pub fn not_in_group(name: &str, group: &str) -> Message {
        Message::could_not(
            "change the group of",
            name,
            &format!("you are not in group {group}"),
            "Choose one of your groups",
        )
    }

    /// A protected place's owner, refused.
    pub fn system_protected(place: &str) -> Message {
        Message::could_not(
            "change the owner of",
            place,
            "system folders are protected",
            "Use the system's own tools",
        )
    }

    /// The change time, which only the system sets.
    pub fn change_time_is_the_systems() -> Message {
        Message::error(
            "Could not set the change time: the system sets it. Set the modified time instead.",
        )
    }

    /// A locked entry met by an operation.
    pub fn locked(name: &str) -> Message {
        Message::error(format!(
            "Could not change {name}: it is locked. Unlock it first."
        ))
    }

    /// A change the disk ignored.
    pub fn ignored() -> Message {
        Message::warning("Permissions did not change: the disk ignored the request.")
    }

    /// A hidden result leaving the list: "report.pdf is hidden now; it
    /// leaves the list."
    pub fn hidden_now(name: &str) -> Message {
        Message::info(format!("{name} is hidden now; it leaves the list."))
    }

    /// Running as administrator: "Administrator mode: permissions do not
    /// limit you."
    pub fn administrator_mode() -> Message {
        Message::warning("Administrator mode: permissions do not limit you.")
    }
}

/// The hot folders' polite lines (R16), reserved for alpha.7: never more
/// than one summary every ten seconds, never a dialog.
pub mod hot {
    use diroxide_core::Message;

    /// The summary: "Scans: 3 done, 1 failed", or "Scans: 3 done".
    pub fn summary(rule: &str, done: u64, failed: u64) -> Message {
        if failed == 0 {
            Message::done(format!("{rule}: {done} done"))
        } else {
            Message::warning(format!("{rule}: {done} done, {failed} failed"))
        }
    }

    /// A paused rule, said once: "Scans paused: pdftotext is not
    /// installed".
    pub fn paused(rule: &str, why: &str) -> Message {
        Message::warning(format!("{rule} paused: {why}"))
    }

    /// A dry run's summary: "Dry run: 3 would convert, 1 skipped".
    pub fn dry_run(would: u64, verb: &str, skipped: u64) -> Message {
        if skipped == 0 {
            Message::info(format!("Dry run: {would} would {verb}"))
        } else {
            Message::info(format!("Dry run: {would} would {verb}, {skipped} skipped"))
        }
    }
}

/// The compare verdicts (R18), reserved for alpha.3: the verdict first,
/// the names last, because they can be long.
pub mod compare {
    use diroxide_core::Message;

    /// Different: "7 changes between report.txt and old.txt".
    pub fn changes(n: u64, a: &str, b: &str) -> Message {
        let noun = if n == 1 { "change" } else { "changes" };
        Message::info(format!("{n} {noun} between {a} and {b}"))
    }

    /// The counts after the verdict: "Changed 4, added 2, removed 1."
    pub fn counts(changed: u64, added: u64, removed: u64) -> String {
        format!("Changed {changed}, added {added}, removed {removed}.")
    }

    /// The same: "Same: report.txt and old.txt".
    pub fn same(a: &str, b: &str) -> Message {
        Message::info(format!("Same: {a} and {b}"))
    }

    /// Binary files that differ: "Binary, 1,204 bytes differ: a.bin and
    /// b.bin".
    pub fn binary(bytes: u64, a: &str, b: &str) -> Message {
        Message::info(format!(
            "Binary, {} bytes differ: {a} and {b}",
            super::grouped(bytes)
        ))
    }

    /// Too large to compare as text.
    pub fn too_large(a: &str, b: &str) -> Message {
        Message::info(format!("Too large to compare as text: {a} and {b}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;
    use diroxide_core::{
        Clock, ConflictKind, Entry, EntryKind, FixedClock, Place, Timestamp, UtcOffset,
    };
    use diroxide_ops::Conflict;

    fn long(n: usize) -> String {
        "x".repeat(n)
    }

    #[test]
    fn every_sentence_says_its_meaning_in_40_cells() {
        let name = long(200);
        let checks: Vec<(String, &str)> = vec![
            (
                startup_problems(&[ConfigProblem::new("keymap.toml", "x")])
                    .unwrap()
                    .text,
                "keymap.toml: 1 problem",
            ),
            (setting_changed("Layout", "stacked").text, "Layout: stacked"),
            (
                setting_reset("Layout", "side by side").text,
                "Layout back to side by side",
            ),
            (cut_pending(3, "Ctrl+V").text, "Cut 3 items"),
            (copied_to_clipboard(1, "").text, "Copied 1 item"),
            (permissions::title(&name), "Permissions for"),
            (permissions::title_many(3), "Permissions for 3 items"),
            (permissions::attributes_title(&name), "Attributes for"),
            (
                permissions::row("Group", "write", "mixed, unchanged"),
                "Group can write: mixed, unchanged",
            ),
            (permissions::octal_row("644"), "Octal: 644"),
            (
                permissions::change_question(12).text,
                "Change permissions of 12 items",
            ),
            (
                permissions::lose_access_question("read", 3).text,
                "You would lose read access to 3 items",
            ),
            (
                permissions::setuid_question("run.sh").text,
                "Let run.sh run as its owner (setuid)",
            ),
            (
                permissions::everyone_question(&name).text,
                "Let everyone change",
            ),
            (
                permissions::changed(12).text,
                "Changed permissions of 12 items",
            ),
            (
                permissions::undid(12).text,
                "Undid permission change of 12 items",
            ),
            (permissions::nothing_to_change().text, "Nothing to change"),
            (
                permissions::links_have_none(&name).text,
                "Could not change permissions of",
            ),
            (
                permissions::only_owner_can(&name, "ada").text,
                "Could not change permissions of",
            ),
            (
                permissions::disk_read_only(&name).text,
                "Could not change permissions of",
            ),
            (
                permissions::disk_keeps_none(&name).text,
                "Could not change permissions of",
            ),
            (
                permissions::everyone_could_write(&name).text,
                "Could not change permissions of",
            ),
            (
                permissions::too_much_below("/").text,
                "Could not change permissions below /",
            ),
            (
                permissions::owner_needs_administrator(&name).text,
                "Could not change the owner of",
            ),
            (
                permissions::not_in_group(&name, "staff").text,
                "Could not change the group of",
            ),
            (
                permissions::system_protected("/usr").text,
                "Could not change the owner of /usr",
            ),
            (
                permissions::change_time_is_the_systems().text,
                "Could not set the change time",
            ),
            (permissions::locked(&name).text, "Could not change"),
            (permissions::ignored().text, "Permissions did not change"),
            (
                permissions::hidden_now("report.pdf").text,
                "report.pdf is hidden now",
            ),
            (permissions::administrator_mode().text, "Administrator mode"),
            (hot::summary("Scans", 3, 1).text, "Scans: 3 done, 1 failed"),
            (hot::summary("Scans", 3, 0).text, "Scans: 3 done"),
            (
                hot::paused("Scans", "pdftotext is not installed").text,
                "Scans paused",
            ),
            (
                hot::dry_run(3, "convert", 1).text,
                "Dry run: 3 would convert, 1 skipped",
            ),
            (
                compare::changes(7, "report.txt", &name).text,
                "7 changes between report.txt and",
            ),
            (compare::counts(4, 2, 1), "Changed 4, added 2, removed 1."),
            (
                compare::same("report.txt", &name).text,
                "Same: report.txt and",
            ),
            (
                compare::binary(1204, "a.bin", &name).text,
                "Binary, 1,204 bytes differ",
            ),
            (
                compare::too_large("a.log", &name).text,
                "Too large to compare as text",
            ),
        ];
        for (text, meaning) in &checks {
            assert!(within_cells(text, meaning), "{text}");
        }
        assert_eq!(startup_problems(&[]), None);
        assert_eq!(
            cut_pending(3, "Ctrl+V").text,
            "Cut 3 items. Paste with Ctrl+V."
        );
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(999), "999");
    }

    #[test]
    fn the_engines_conflict_lines_and_choices_fit_40_cells() {
        let clock = FixedClock::new(
            Timestamp::from_civil(2026, 10, 16, 9, 0, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::UTC,
        );
        let target = Place::parse(if cfg!(windows) {
            r"D:\backup\report.pdf"
        } else {
            "/backup/report.pdf"
        })
        .unwrap();
        let older = Timestamp::from_civil(2026, 10, 9, 9, 0, 0, UtcOffset::UTC).unwrap();
        let source_entry = Entry::new("report.pdf", EntryKind::File)
            .with_size(2_300_000)
            .with_modified(clock.now());
        let target_entry = Entry::new("report.pdf", EntryKind::File)
            .with_size(2_100_000)
            .with_modified(older);
        for kind in [
            ConflictKind::FileOverFile,
            ConflictKind::ReadOnlyTarget,
            ConflictKind::FolderOverFolder,
            ConflictKind::FileOverFolder,
            ConflictKind::FolderOverFile,
            ConflictKind::SameFile,
        ] {
            let conflict = Conflict {
                kind,
                source: target.clone(),
                target: target.clone(),
                source_entry: source_entry.clone(),
                target_entry: target_entry.clone(),
                keep_both_name: "report (2).pdf".to_owned(),
            };
            let lines = conflict.lines(&clock);
            assert!(lines[0].starts_with(kind.situation()), "{lines:?}");
            for line in lines {
                let meaning = line.split(':').next().unwrap_or(&line);
                assert!(within_cells(&line, meaning), "{line}");
            }
            for choice in conflict.choices() {
                assert!(choice.label.chars().count() <= 40, "{}", choice.label);
            }
        }
    }
}
