//! The batch rename engine's tests: the rules, the rows and their words,
//! the count line, the clashes, the editor round trip, and runs through
//! the engine over the fake provider, with property tests for swaps,
//! rings and permutations, and a fuzz-style test over rule texts.

use std::collections::BTreeMap;

use diroxide_core::{Entry, EntryKind, JobId, Place, Timestamp, UtcOffset};
use diroxide_vfs::names::NameRules;
use proptest::prelude::*;

use super::*;
use crate::engine::JobControl;
use crate::fake::{FakeProvider, FakeSwitches};
use crate::testing::{self, Script, p};
use crate::undo::UndoLog;

/// Files in one folder, each a source, with that folder's facts: case
/// ignored, Linux-style name rules, and the given other names there.
fn files(
    folder: &str,
    names: &[&str],
    others: &[&str],
) -> (Vec<RenameSource>, RenameContext<'static>) {
    let place = p(folder);
    let sources = names
        .iter()
        .map(|name| {
            let mut entry = Entry::new(*name, EntryKind::File).with_size(2_100_000);
            entry.modified = Some(Timestamp::from_unix(1_780_000_000, 0));
            RenameSource::new(place.join(name).unwrap(), entry)
        })
        .collect();
    let all = names
        .iter()
        .chain(others)
        .map(|n| (*n).to_owned())
        .collect();
    let facts = FolderFacts {
        names: all,
        case_sensitive: false,
        rules: NameRules {
            windows: false,
            max_length: Some(255),
            length_in_utf16: false,
        },
    };
    (sources, RenameContext::new().with_folder(place, facts))
}

fn plan(names: &[&str], recipe: &Recipe) -> RenamePlan {
    let (sources, context) = files("/a", names, &[]);
    plan_rename(&sources, recipe, &context).unwrap()
}

fn new_names(plan: &RenamePlan) -> Vec<String> {
    plan.rows().iter().map(|row| row.new.clone()).collect()
}

fn cells(line: &str) -> usize {
    line.chars().count()
}

#[test]
fn plain_replace_keeps_the_extension_and_every_character_literal() {
    let recipe = Recipe::of(vec![Rule::replace("IMG_", "trip-")]);
    let plan = plan(&["IMG_0001.jpg", "IMG_0002.IMG_", "notes.jpg"], &recipe);
    assert_eq!(
        new_names(&plan),
        ["trip-0001.jpg", "trip-0002.IMG_", "notes.jpg"]
    );
    let dots = Recipe::of(vec![Rule::replace(".", "$1").every_match().on(Part::Name)]);
    assert_eq!(new_names(&self::plan(&["a.b.c"], &dots)), ["a$1b$1c"]);
}

#[test]
fn patterns_reorder_with_groups_and_ignore_case_when_asked() {
    let recipe = Recipe::of(vec![Rule::pattern(r"(\d{2})-(\d{2})-(\d{4})", "$3-$2-$1")]);
    assert_eq!(
        new_names(&plan(&["29-03-2020 trip.jpg"], &recipe)),
        ["2020-03-29 trip.jpg"]
    );
    let any_case = Recipe::of(vec![Rule::pattern("img", "photo").ignoring_case()]);
    assert_eq!(new_names(&plan(&["IMG_1.jpg"], &any_case)), ["photo_1.jpg"]);
    let exact = Recipe::of(vec![Rule::pattern("img", "photo")]);
    assert_eq!(new_names(&plan(&["IMG_1.jpg"], &exact)), ["IMG_1.jpg"]);
}

#[test]
fn insert_remove_tidy_and_case_edit_their_part() {
    let recipe = Recipe::of(vec![
        Rule::Tidy {
            tidy: Tidy::Trim,
            part: Part::Stem,
        },
        Rule::Tidy {
            tidy: Tidy::CollapseSpaces,
            part: Part::Stem,
        },
        Rule::Tidy {
            tidy: Tidy::SpacesTo { with: "_".into() },
            part: Part::Stem,
        },
        Rule::Insert {
            text: "x-".into(),
            at: Position::from_start(0),
            part: Part::Stem,
        },
        Rule::Remove {
            at: Position::from_end(0),
            count: 2,
            part: Part::Stem,
        },
        Rule::case(CaseForm::Upper).on(Part::Extension),
    ]);
    assert_eq!(
        new_names(&plan(&["  my   holiday  01.jpg"], &recipe)),
        ["x-my_holiday_.JPG"]
    );
    let digits = Recipe::of(vec![Rule::Tidy {
        tidy: Tidy::Remove {
            class: CharacterClass::Digits,
        },
        part: Part::Name,
    }]);
    assert_eq!(new_names(&plan(&["a1b2.mp3"], &digits)), ["ab.mp"]);
    let title = Recipe::of(vec![Rule::case(CaseForm::Title)]);
    assert_eq!(
        new_names(&plan(&["the_big-day.TXT"], &title)),
        ["The_Big-Day.TXT"]
    );
}

#[test]
fn templates_count_date_and_size_and_keep_compound_extensions() {
    let recipe =
        Recipe::of(vec![Rule::template("{modified}-{counter}-{old-stem}")]).counting(Counter {
            start: 8,
            step: 2,
            width: 3,
            per_folder: false,
        });
    let plan = plan(&["a.tar.gz", "b.txt"], &recipe);
    assert_eq!(
        new_names(&plan),
        ["2026-05-28-008-a.tar.gz", "2026-05-28-010-b.txt"]
    );
    let size = Recipe::of(vec![Rule::template("{stem} ({size:human})")]);
    assert_eq!(
        new_names(&self::plan(&["c.pdf"], &size)),
        ["c (2.1 MB).pdf"]
    );
    let ext = Recipe::of(vec![Rule::template("{ext}-{stem}").on(Part::Name)]);
    assert_eq!(
        new_names(&self::plan(&["d.txt", "e"], &ext)),
        ["txt-d", "-e"]
    );
    let gone = Recipe::of(vec![Rule::template("").on(Part::Extension)]);
    assert_eq!(new_names(&self::plan(&["f.txt"], &gone)), ["f"]);
}

#[test]
fn counters_restart_per_folder_and_skips_never_renumber() {
    let recipe = Recipe::of(vec![Rule::template("{counter}")]).counting(Counter {
        per_folder: true,
        ..Counter::default()
    });
    let mut sources = files("/a", &["x.txt", "y.txt"], &[]).0;
    sources.extend(files("/b", &["z.txt"], &[]).0);
    let plan = plan_rename(&sources, &recipe, &RenameContext::new()).unwrap();
    assert_eq!(new_names(&plan), ["1.txt", "2.txt", "1.txt"]);
    let mut choices = Choices::new();
    assert!(choices.toggle_skip(0));
    let global = Recipe::of(vec![Rule::template("{counter}")]);
    let (sources, context) = files("/a", &["x.txt", "y.txt"], &[]);
    let skipped = plan_rename_with(&sources, &global, &context, &choices).unwrap();
    assert_eq!(new_names(&skipped), ["x.txt", "2.txt"]);
    assert_eq!(skipped.rows()[0].line(), "Skipped by you: x.txt");
}

#[test]
fn a_missing_value_or_a_negative_counter_is_that_rows_problem() {
    struct Exif;
    impl FieldLookup for Exif {
        fn field(&self, source: &RenameSource, key: &str) -> Option<String> {
            (key == "exif.date" && source.entry.name.starts_with("IMG"))
                .then(|| "2026-11-01".into())
        }
    }
    let recipe = Recipe::of(vec![Rule::template("{meta:exif.date} {stem}")]);
    let (sources, context) = files("/a", &["IMG_1.jpg", "scan.jpg"], &[]);
    let lookup = Exif;
    let plan = plan_rename(&sources, &recipe, &context.with_lookup(&lookup)).unwrap();
    assert_eq!(
        plan.rows()[1].line(),
        "Problem, no value for {meta:exif.date}: scan.jpg"
    );
    assert_eq!(plan.rows()[0].new, "2026-11-01 IMG_1.jpg");
    assert_eq!(plan.count_line(), "1 problem. 1 of 2 files will change.");
    let down = Recipe::of(vec![Rule::template("{counter}")]).counting(Counter {
        start: 0,
        step: -1,
        ..Counter::default()
    });
    let plan = self::plan(&["a.txt", "b.txt"], &down);
    assert_eq!(
        plan.rows()[1].line(),
        "Problem, counter went below zero: b.txt"
    );
}

#[test]
fn bad_recipes_are_refused_whole_with_the_rule_number() {
    let (sources, context) = files("/a", &["a.txt"], &[]);
    let cases = [
        (
            Rule::pattern("(?=x)", ""),
            "Rule 1: pattern needs care: look-around",
        ),
        (
            Rule::pattern("(a)", "$1_x"),
            "Rule 1: $1_x is a group named 1_x",
        ),
        (
            Rule::pattern("(a)", "$2"),
            "Rule 1: no group 2; the pattern has 1",
        ),
        (Rule::replace("", "x"), "Rule 1: nothing to find"),
        (Rule::template("{stme}"), "Rule 1: no placeholder {stme}"),
        (Rule::template("{stem"), "Rule 1: a brace is not closed"),
    ];
    for (rule, line) in cases {
        let error = plan_rename(&sources, &Recipe::of(vec![rule]), &context).unwrap_err();
        assert_eq!(error.line(), line);
        assert!(cells(&error.line()) <= 40, "{line}");
        assert!(!error.next_step().is_empty());
    }
    let wide = Recipe::default().counting(Counter {
        width: 21,
        ..Counter::default()
    });
    assert_eq!(
        plan_rename(&sources, &wide, &context).unwrap_err().line(),
        "Counter: counter wider than 20 digits"
    );
}

#[test]
fn the_count_line_leads_with_the_worst_news_inside_40_cells() {
    let none = plan(&["a.txt"], &Recipe::default());
    assert_eq!(none.count_line(), "No file would change.");
    assert_eq!(
        none.refusal_line().unwrap(),
        "Nothing to rename: no file would change."
    );
    let clean = plan(
        &["a.txt", "b.txt"],
        &Recipe::of(vec![Rule::replace("a", "c")]),
    );
    assert_eq!(clean.count_line(), "1 of 2 files will change. No problems.");
    assert!(clean.is_ready());
    // The folder's own name, not its path (lane AS).
    assert_eq!(clean.question(), "Rename 1 file in a?");
    let clash = plan(
        &["1.txt", "2.txt", "3.txt", "keep.txt"],
        &Recipe::of(vec![Rule::pattern("^[0-9]$", "keep")]),
    );
    assert_eq!(clash.count_line(), "3 problems. 0 of 4 files will change.");
    let care = plan(
        &["a.txt", "b.txt"],
        &Recipe::of(vec![Rule::Insert {
            text: " ".into(),
            at: Position::from_start(0),
            part: Part::Stem,
        }]),
    );
    assert_eq!(care.count_line(), "2 need care. 2 of 2 will change.");
    assert_eq!(care.question(), "Rename 2 files in a, 2 need care?");
    let lines = [
        clash.count_line(),
        care.count_line(),
        clean.count_line(),
        "2 problems. 46 of 50 files will change.".to_owned(),
        "3 need care. 47 of 50 will change.".to_owned(),
        "47 of 50 files will change. No problems.".to_owned(),
        "1,000 problems. 9,000 of 10,000 files will change.".to_owned(),
    ];
    let counts: Vec<usize> = lines.iter().map(|line| cells(line)).collect();
    assert_eq!(counts, [37, 32, 38, 39, 34, 40, 50]);
    // Past 40 cells, the problems and the first count still lead.
    assert!(
        lines[6]
            .chars()
            .take(40)
            .collect::<String>()
            .contains("9,000 of")
    );
}

#[test]
fn clashes_with_rows_and_with_names_on_disk_are_problems_first() {
    let (sources, context) = files("/a", &["a.txt", "b.txt", "c.txt"], &["taken.txt"]);
    let recipe = Recipe::of(vec![
        Rule::pattern("^[ab]$", "same"),
        Rule::replace("c", "taken"),
    ]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    let lines: Vec<String> = plan.rows().iter().map(RenameRow::line).collect();
    assert_eq!(
        lines,
        [
            "Problem, same new name as b.txt: a.txt becomes same.txt",
            "Problem, same new name as a.txt: b.txt becomes same.txt",
            "Problem, name taken: c.txt becomes taken.txt",
        ]
    );
    assert_eq!(plan.count_line(), "3 problems. 0 of 3 files will change.");
    assert_eq!(
        plan.refusal_line().unwrap(),
        "Cannot rename: 3 problems. First: same new name as b.txt, a.txt becomes same.txt."
    );
    assert!(matches!(
        plan.request(JobId::new(1)),
        Err(RenameRefused::Problems(_))
    ));
    assert_eq!(plan.next_problem(None), Some(0));
    assert_eq!(plan.next_problem(Some(2)), Some(0));
    assert_eq!(plan.previous_problem(Some(0)), Some(2));
    // The 40-cell rule: the state and the old name lead.
    assert!(lines[2].starts_with("Problem, name taken: c.txt"));
}

#[test]
fn a_row_that_cannot_change_keeps_its_name_and_others_clash_with_it() {
    let (sources, context) = files("/a", &["a.txt", "b.txt"], &[]);
    // a.txt would become a:x.txt (refused), so it stays; b.txt wants a.txt.
    let recipe = Recipe::of(vec![Rule::pattern("^a$", "a:x"), Rule::pattern("^b$", "a")]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    assert_eq!(
        plan.rows()[0].line(),
        "Problem, has the reserved character :: a.txt becomes a:x.txt"
    );
    assert_eq!(
        plan.rows()[1].line(),
        "Problem, name taken: b.txt becomes a.txt"
    );
    // Portable names off: Linux takes a colon, so both go ahead in turn.
    let plan = plan_rename(&sources, &recipe, &context.clone().portable(false)).unwrap();
    assert_eq!(plan.problems(), 0);
    assert_eq!(plan.count_line(), "1 needs care. 2 of 2 will change.");
}

#[test]
fn names_not_allowed_are_said_in_words() {
    let recipe = |text: &str| Recipe::of(vec![Rule::template(text).on(Part::Name)]);
    let cases = [
        ("", "Problem, empty name: a.txt"),
        (
            "x/y",
            "Problem, has the reserved character /: a.txt becomes x/y",
        ),
        (
            "con.txt",
            "Problem, name is a Windows device: a.txt becomes con.txt",
        ),
        (
            "b.",
            "Problem, name ends in a dot or space: a.txt becomes b.",
        ),
    ];
    for (text, line) in cases {
        assert_eq!(plan(&["a.txt"], &recipe(text)).rows()[0].line(), line);
    }
    let care = plan(&["a.txt"], &recipe("a\u{200b}.txt"));
    assert_eq!(
        care.rows()[0].line(),
        "Care, has hidden characters: a.txt becomes a[U+200B].txt"
    );
    assert_eq!(care.count_line(), "1 needs care. 1 of 1 will change.");
    assert_eq!(care.question(), "Rename 1 file in a, 1 needs care?");
    let mut odd = files("/a", &["x"], &[]).0;
    odd[0].entry.name_not_unicode = true;
    let plan = plan_rename(&odd, &recipe("y"), &RenameContext::new()).unwrap();
    assert_eq!(
        plan.rows()[0].line(),
        "Problem, name is not valid Unicode: x"
    );
}

#[test]
fn swaps_rings_and_case_only_changes_say_so() {
    let pairs = Recipe::of(vec![
        Rule::pattern("^a$", "B"),
        Rule::pattern("^b$", "a"),
        Rule::case(CaseForm::Lower),
    ]);
    let plan = plan(&["a.txt", "b.txt"], &pairs);
    let lines: Vec<String> = plan.rows().iter().map(RenameRow::line).collect();
    assert_eq!(
        lines,
        [
            "Swap with b.txt: a.txt becomes b.txt",
            "Swap with a.txt: b.txt becomes a.txt"
        ]
    );
    let ring = Recipe::of(vec![
        Rule::pattern("^1$", "x2"),
        Rule::pattern("^2$", "x3"),
        Rule::pattern("^3$", "x1"),
        Rule::replace("x", ""),
    ]);
    let plan = self::plan(&["1", "2", "3"], &ring);
    assert_eq!(plan.rows()[0].line(), "Ring of 3: 1 becomes 2");
    let case = self::plan(
        &["notes.txt"],
        &Recipe::of(vec![Rule::case(CaseForm::Title)]),
    );
    assert_eq!(
        case.rows()[0].line(),
        "Case only: notes.txt becomes Notes.txt"
    );
    assert_eq!(case.problems(), 0);
}

#[test]
fn a_case_twin_on_a_case_sensitive_folder_needs_care() {
    let (sources, mut context) = files("/a", &["b.txt"], &["A.txt"]);
    let mut facts = context.facts(&p("/a"));
    facts.case_sensitive = true;
    context = context.with_folder(p("/a"), facts);
    let plan = plan_rename(
        &sources,
        &Recipe::of(vec![Rule::replace("b", "a")]),
        &context,
    )
    .unwrap();
    assert_eq!(
        plan.rows()[0].line(),
        "Care, differs only by case from A.txt: b.txt becomes a.txt"
    );
}

#[test]
fn edited_rows_keep_their_names_across_new_recipes() {
    let (sources, context) = files("/a", &["a.txt", "b.txt"], &[]);
    let mut choices = Choices::new();
    choices.edit(1, "mine.txt");
    let plan = plan_rename_with(
        &sources,
        &Recipe::of(vec![Rule::replace("a", "z")]),
        &context,
        &choices,
    )
    .unwrap();
    assert_eq!(
        plan.rows()[1].line(),
        "Edited by you: b.txt becomes mine.txt"
    );
    assert_eq!(
        plan.pairs(),
        vec![
            (p("/a/a.txt"), "z.txt".to_owned()),
            (p("/a/b.txt"), "mine.txt".to_owned())
        ]
    );
    choices.reset(1);
    assert!(choices.edited.is_empty());
}

#[test]
fn the_keyed_editor_round_trip_reads_any_order_and_refuses_lost_lines() {
    let recipe = Recipe::of(vec![Rule::replace("IMG_", "trip-")]);
    let (sources, context) = files("/a", &["IMG_1.jpg", "IMG_2.jpg", "notes.txt"], &[]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    let text = editor_text(&plan, EditorFormat::Keyed).unwrap();
    assert_eq!(
        text,
        "trip-1.jpg (was IMG_1.jpg)\ntrip-2.jpg (was IMG_2.jpg)\nnotes.txt (was notes.txt)\n"
    );
    assert_eq!(cells("trip-01.jpg (was IMG_0001.jpg)"), 30);
    // Reordered, one put back, one with " (was " in its new name.
    let edited = "read (was it).txt (was notes.txt)\r\ntrip-2.jpg (was IMG_2.jpg)\nIMG_1.jpg (was IMG_1.jpg)\n";
    let choices = read_editor_text(&plan, edited, EditorFormat::Keyed).unwrap();
    assert_eq!(
        choices.edited,
        BTreeMap::from([
            (0, "IMG_1.jpg".to_owned()),
            (2, "read (was it).txt".to_owned())
        ])
    );
    let again = plan_rename_with(&sources, &recipe, &context, &choices).unwrap();
    assert_eq!(again.rows()[0].line(), "Same name: IMG_1.jpg");
    assert_eq!(again.count_line(), "2 of 3 files will change. No problems.");
    let lost = read_editor_text(
        &plan,
        "trip-1.jpg (was IMG_1.jpg)\nx.jpg\n",
        EditorFormat::Keyed,
    )
    .unwrap_err();
    let line = "Names not read: line 2, unknown (was)";
    assert_eq!(lost.line(), line);
    assert_eq!(line.chars().count(), 37);
    let twice = "a (was IMG_1.jpg)\nb (was IMG_1.jpg)\nc (was notes.txt)\n";
    let twice = read_editor_text(&plan, twice, EditorFormat::Keyed).unwrap_err();
    assert_eq!(twice, EditorRefused::UnknownKey(2));
    let short = read_editor_text(&plan, "a (was IMG_1.jpg)\n", EditorFormat::Keyed).unwrap_err();
    assert_eq!(short.line(), "Names not read: 2 files have no line");
    let empty = read_editor_text(
        &plan,
        " (was IMG_1.jpg)\nb (was IMG_2.jpg)\nc (was notes.txt)",
        EditorFormat::Keyed,
    )
    .unwrap_err();
    assert_eq!(empty.line(), "Names not read: line 1 is empty");
    for line in [lost.line(), short.line(), empty.line()] {
        assert!(cells(&line) <= 46, "{line}");
    }
}

#[test]
fn the_plain_editor_round_trip_refuses_a_changed_count_in_words() {
    let recipe = Recipe::of(vec![Rule::replace("IMG_", "trip-")]);
    let (sources, context) = files("/a", &["IMG_1.jpg", "IMG_2.jpg", "notes.txt"], &[]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    let text = editor_text(&plan, EditorFormat::Plain).unwrap();
    assert_eq!(text, "trip-1.jpg\ntrip-2.jpg\nnotes.txt\n");
    let fewer =
        read_editor_text(&plan, "trip-1.jpg\nnotes.txt\n", EditorFormat::Plain).unwrap_err();
    assert_eq!(fewer.line(), "Names not read: 2 lines for 3 files");
    assert_eq!(cells(&fewer.line()), 35);
    let empty = read_editor_text(&plan, "a\r\n\r\nb\r\n", EditorFormat::Plain).unwrap_err();
    assert_eq!(empty.line(), "Names not read: line 2 is empty");
    let choices = read_editor_text(
        &plan,
        "trip-1.jpg\nIMG_2.jpg\nread me.txt",
        EditorFormat::Plain,
    )
    .unwrap();
    assert_eq!(
        choices.edited,
        BTreeMap::from([(1, "IMG_2.jpg".to_owned()), (2, "read me.txt".to_owned())])
    );
    let mut odd = sources.clone();
    odd[0].entry.name = "two\nlines.jpg".into();
    let plan = plan_rename(&odd, &recipe, &context).unwrap();
    let refused = editor_text(&plan, EditorFormat::Keyed).unwrap_err();
    assert_eq!(
        refused.line(),
        "Cannot edit: 2 names hold a line break or tab"
    );
}

#[test]
fn make_names_unique_numbers_the_later_rows_for_review() {
    let (sources, context) = files("/a", &["1.txt", "2.txt", "3.txt"], &["report.txt"]);
    let recipe = Recipe::of(vec![Rule::template("report")]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    assert_eq!(plan.problems(), 3);
    let choices = plan.make_unique();
    let fixed = plan_rename_with(&sources, &recipe, &context, &choices).unwrap();
    assert_eq!(
        fixed.rows().iter().map(RenameRow::line).collect::<Vec<_>>(),
        [
            "Edited by you: 1.txt becomes report (2).txt",
            "Edited by you: 2.txt becomes report (3).txt",
            "Edited by you: 3.txt becomes report (4).txt",
        ]
    );
    assert_eq!(fixed.count_line(), "3 of 3 files will change. No problems.");
    let (sources, context) = files("/a", &["a.txt", "b.txt"], &[]);
    let same = plan_rename(&sources, &recipe, &context).unwrap();
    let fixed = plan_rename_with(&sources, &recipe, &context, &same.make_unique()).unwrap();
    assert_eq!(new_names(&fixed), ["report.txt", "report (2).txt"]);
}

#[test]
fn the_pairs_door_gives_the_same_rows_without_edited_words() {
    let (sources, context) = files("/a", &["a.txt", "b.txt"], &["c.txt"]);
    let pairs = vec![
        (sources[0].clone(), "b.txt".to_owned()),
        (sources[1].clone(), "c.txt".to_owned()),
    ];
    let plan = plan_pairs(&pairs, &context);
    assert_eq!(
        plan.rows().iter().map(RenameRow::line).collect::<Vec<_>>(),
        [
            "Problem, name taken: a.txt becomes b.txt",
            "Problem, name taken: b.txt becomes c.txt",
        ]
    );
    let swap = plan_pairs(
        &[
            (sources[0].clone(), "b.txt".to_owned()),
            (sources[1].clone(), "a.txt".to_owned()),
        ],
        &context,
    );
    assert_eq!(
        swap.rows()[0].line(),
        "Swap with b.txt: a.txt becomes b.txt"
    );
}

#[test]
fn gone_archived_and_overlong_sources_are_problems_in_words() {
    let (mut sources, context) = files("/a", &["a.txt", "b.txt"], &[]);
    sources[1] = sources[1].clone().gone();
    let recipe = Recipe::of(vec![Rule::template("{stem}{stem}{stem}")]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    assert_eq!(plan.rows()[1].line(), "Problem, source is gone: b.txt");
    let long = Recipe::of(vec![Rule::template("x".repeat(256))]);
    let plan = plan_rename(&sources[..1], &long, &context).unwrap();
    let line = plan.rows()[0].line();
    assert!(
        line.starts_with("Problem, too long (260 of 255): a.txt becomes xxx"),
        "{line}"
    );
    let inner = Place::parse(&format!("{}!x.txt", p("/a/c.zip"))).unwrap();
    let archived = [RenameSource::new(
        inner,
        Entry::new("x.txt", EntryKind::File),
    )];
    let plan = plan_rename(&archived, &recipe, &RenameContext::new()).unwrap();
    assert_eq!(plan.rows()[0].line(), "Problem, inside an archive: x.txt");
}

#[test]
fn pairs_go_deepest_first_and_the_problems_view_leads_with_problems() {
    let folder = files("/a", &["sub"], &[]).0;
    let mut sources = vec![RenameSource::new(
        folder[0].place.clone(),
        Entry::new("sub", EntryKind::Folder),
    )];
    sources.extend(files("/a/sub", &["x.txt", "y.txt"], &[]).0);
    let recipe = Recipe::of(vec![Rule::pattern("^(sub|x)$", "new-$1")]);
    let plan = plan_rename(&sources, &recipe, &RenameContext::new()).unwrap();
    assert_eq!(
        plan.pairs(),
        vec![
            (p("/a/sub/x.txt"), "new-x.txt".to_owned()),
            (p("/a/sub"), "new-sub".to_owned()),
        ]
    );
    let (sources, context) = files("/a", &["a.txt", "b.txt", "c.txt"], &["z.txt"]);
    let recipe = Recipe::of(vec![Rule::pattern("^c$", "z")]);
    let plan = plan_rename(&sources, &recipe, &context).unwrap();
    let view: Vec<&str> = plan
        .problems_first()
        .iter()
        .map(|row| row.old.as_str())
        .collect();
    assert_eq!(view, ["c.txt", "a.txt", "b.txt"]);
    assert_eq!(plan.next_problem(None), Some(2));
    assert_eq!(
        RenamePlan::next_problem_in(&plan.problems_first(), None),
        Some(0)
    );
}

#[test]
fn rule_summaries_put_the_action_first() {
    let lines = [
        (
            Rule::replace("IMG_", "trip-").describe(),
            "Replace IMG_ with trip-, in the name",
        ),
        (
            Rule::pattern("x", "").every_match().describe(),
            "Replace every pattern x with nothing, in the name",
        ),
        (
            Rule::case(CaseForm::Lower).on(Part::Extension).describe(),
            "Change case to lowercase, in the extension",
        ),
        (
            Rule::Remove {
                at: Position::from_end(0),
                count: 4,
                part: Part::Stem,
            }
            .describe(),
            "Remove 4 characters from the end, in the name",
        ),
        (
            Rule::Insert {
                text: "x".into(),
                at: Position::from_start(3),
                part: Part::Name,
            }
            .describe(),
            "Insert x after character 3, in the whole name",
        ),
        (
            Rule::template("{stem}-{counter}").describe(),
            "Make the name from {stem}-{counter}",
        ),
    ];
    for (said, want) in lines {
        assert_eq!(said, want);
    }
    for (placeholder, words) in PLACEHOLDERS {
        assert!(
            cells(&format!("{placeholder} {words}")) <= 60,
            "{placeholder}"
        );
        assert!(cells(words) <= 40, "{words}");
    }
}

#[test]
fn presets_travel_as_toml() {
    let recipe = Recipe::of(vec![
        Rule::pattern("(\\d+)", "n$1").every_match().ignoring_case(),
        Rule::Tidy {
            tidy: Tidy::SpacesTo { with: "-".into() },
            part: Part::Stem,
        },
        Rule::Remove {
            at: Position::from_end(2),
            count: 1,
            part: Part::Name,
        },
    ])
    .counting(Counter {
        start: 5,
        step: 5,
        width: 3,
        per_folder: true,
    })
    .then(Pass::of(vec![Rule::case(CaseForm::Sentence)]));
    let mut presets = Presets::default();
    presets.save("Trip photos", recipe.clone());
    presets.save("trip PHOTOS", recipe.clone());
    assert_eq!(presets.presets.len(), 1);
    let text = presets.to_toml().unwrap();
    let back = Presets::from_toml(&text).unwrap();
    assert_eq!(back, presets);
    assert_eq!(back.get("TRIP photos").unwrap().recipe, recipe);
    let hand = "[[pass]]\n[[pass.rule]]\nkind = \"replace\"\nfind = \"IMG_\"\nwith = \"trip-\"\n";
    assert_eq!(
        Recipe::from_toml(hand).unwrap(),
        Recipe::of(vec![Rule::replace("IMG_", "trip-")])
    );
    let bad = Recipe::from_toml("[[pass.rule]]\nkind = \"explode\"\n").unwrap_err();
    assert!(bad.to_string().starts_with("Rename presets not read: "));
}

// The runs, through the engine over the fake provider.

fn fake_folder(names: &[&str], switches: FakeSwitches) -> (FakeProvider, Vec<RenameSource>) {
    let fake = FakeProvider::with_switches(switches);
    let sources = names
        .iter()
        .map(|name| {
            let place = p("/a").join(name).unwrap();
            fake.add_file(&place, name.as_bytes());
            let entry = fake.entry(&place).unwrap();
            RenameSource::new(place, entry)
        })
        .collect();
    (fake, sources)
}

fn contents_by_name(fake: &FakeProvider, names: &[String]) -> Vec<Vec<u8>> {
    names
        .iter()
        .map(|name| {
            fake.contents(&p("/a").join(name).unwrap())
                .unwrap_or_default()
        })
        .collect()
}

fn run(
    fake: &FakeProvider,
    sources: &[RenameSource],
    recipe: &Recipe,
) -> (
    RenamePlan,
    Result<crate::JobReport, RenameRefused>,
    crate::Engine,
) {
    let engine = testing::engine(fake);
    let control = JobControl::new();
    let context = read_context(&engine, sources, &control);
    let plan = plan_rename(sources, recipe, &context).unwrap();
    let sink = Script::default();
    let env = RenameEnv {
        engine: &engine,
        job: JobId::new(7),
        control: &control,
        sink: &sink,
        lookup: None,
        offset: UtcOffset::UTC,
    };
    let report = run_rename(&plan, &env);
    (plan, report, engine)
}

#[test]
fn a_swap_runs_as_one_job_and_undoes_with_one_record() {
    let (fake, sources) = fake_folder(&["a.txt", "b.txt", "c.txt"], FakeSwitches::default());
    let recipe = Recipe::of(vec![
        Rule::pattern("^a$", "B"),
        Rule::pattern("^b$", "a"),
        Rule::case(CaseForm::Lower),
    ]);
    let (plan, report, engine) = run(&fake, &sources, &recipe);
    let report = report.unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(plan.count_line(), "2 of 3 files will change. No problems.");
    assert_eq!(fake.contents(&p("/a/a.txt")).unwrap(), b"b.txt");
    assert_eq!(fake.contents(&p("/a/b.txt")).unwrap(), b"a.txt");
    assert!(fake.unfinished().is_empty());
    let mut log = UndoLog::new(10);
    log.record(report.undo.clone());
    let undone = engine
        .undo_last(
            &mut log,
            JobId::new(8),
            &JobControl::new(),
            &Script::default(),
        )
        .unwrap();
    assert!(undone.failed.is_empty(), "{:?}", undone.failed);
    assert_eq!(fake.contents(&p("/a/a.txt")).unwrap(), b"a.txt");
    assert_eq!(fake.contents(&p("/a/b.txt")).unwrap(), b"b.txt");
}

#[test]
fn a_case_only_change_runs_on_a_case_insensitive_folder() {
    let switches = FakeSwitches {
        case_insensitive: true,
        ..FakeSwitches::default()
    };
    let (fake, sources) = fake_folder(&["notes.txt"], switches);
    let (plan, report, _) = run(
        &fake,
        &sources,
        &Recipe::of(vec![Rule::case(CaseForm::Upper)]),
    );
    assert_eq!(
        plan.rows()[0].line(),
        "Case only: notes.txt becomes NOTES.txt"
    );
    let report = report.unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(fake.snapshot().keys().any(|key| key.ends_with("NOTES.txt")));
}

#[test]
fn a_folder_that_changed_since_the_preview_refuses_the_run() {
    let (fake, sources) = fake_folder(&["a.txt"], FakeSwitches::default());
    let engine = testing::engine(&fake);
    let control = JobControl::new();
    let recipe = Recipe::of(vec![Rule::replace("a", "b")]);
    let plan = plan_rename(
        &sources,
        &recipe,
        &read_context(&engine, &sources, &control),
    )
    .unwrap();
    fake.add_file(&p("/a/b.txt"), b"new");
    let sink = Script::default();
    let env = RenameEnv {
        engine: &engine,
        job: JobId::new(3),
        control: &control,
        sink: &sink,
        lookup: None,
        offset: UtcOffset::UTC,
    };
    let refused = run_rename(&plan, &env).unwrap_err();
    assert_eq!(
        refused.line(),
        "The folder changed since the preview: name taken, a.txt becomes b.txt."
    );
    assert_eq!(fake.contents(&p("/a/b.txt")).unwrap(), b"new");
    assert_eq!(fake.contents(&p("/a/a.txt")).unwrap(), b"a.txt");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Any permutation of a folder's names, rings and swaps among them,
    /// renames every file to its planned name and undoes back.
    #[test]
    fn any_permutation_renames_and_undoes(order in Just((0..7usize).collect::<Vec<_>>()).prop_shuffle(), n in 2usize..7) {
        let names: Vec<String> = (0..n).map(|i| format!("f{i}.txt")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (fake, sources) = fake_folder(&refs, FakeSwitches::default());
        let targets: Vec<usize> = order.into_iter().filter(|&i| i < n).collect();
        let engine = testing::engine(&fake);
        let control = JobControl::new();
        let context = read_context(&engine, &sources, &control);
        let mut choices = Choices::new();
        for (i, &t) in targets.iter().enumerate() {
            choices.edit(i, names[t].clone());
        }
        let plan = plan_rename_with(&sources, &Recipe::default(), &context, &choices).unwrap();
        prop_assert_eq!(plan.problems(), 0);
        let sink = Script::default();
        let env = RenameEnv { engine: &engine, job: JobId::new(5), control: &control, sink: &sink, lookup: None, offset: UtcOffset::UTC };
        if plan.renaming() == 0 {
            prop_assert!(run_rename(&plan, &env).is_err());
            return Ok(());
        }
        let report = run_rename(&plan, &env).unwrap();
        prop_assert!(report.failed.is_empty());
        // The file that was f{i} is now named names[targets[i]].
        for (i, &t) in targets.iter().enumerate() {
            prop_assert_eq!(fake.contents(&p("/a").join(&names[t]).unwrap()).unwrap(), names[i].as_bytes().to_vec());
        }
        prop_assert!(fake.unfinished().is_empty());
        prop_assert_eq!(fake.snapshot().len(), n + 1);
        let mut log = UndoLog::new(4);
        log.record(report.undo);
        let undone = engine.undo_last(&mut log, JobId::new(6), &JobControl::new(), &Script::default()).unwrap();
        prop_assert!(undone.failed.is_empty());
        let back = contents_by_name(&fake, &names);
        prop_assert_eq!(back, names.iter().map(|n| n.as_bytes().to_vec()).collect::<Vec<_>>());
    }

    /// Planning never panics on any rule text, and every row it makes has
    /// words, its state or its old name first.
    #[test]
    fn any_rule_text_plans_or_is_refused_in_words(
        find in "\\PC{0,12}",
        with in "\\PC{0,12}",
        template in "[a-z{}:$\\\\ -]{0,16}",
        pattern in any::<bool>(),
        all in any::<bool>(),
    ) {
        let mode = if pattern { FindMode::Pattern } else { FindMode::Text };
        let recipe = Recipe::of(vec![
            Rule::Replace { find, with, mode, all, ignore_case: false, part: Part::Stem },
        ])
        .then(Pass::of(vec![Rule::template(template)]));
        let (sources, context) = files("/a", &["IMG_0001.jpg", "a b.txt", "x"], &["other"]);
        match plan_rename(&sources, &recipe, &context) {
            Ok(plan) => {
                prop_assert_eq!(plan.rows().len(), 3);
                prop_assert!(!plan.count_line().is_empty());
                for row in plan.rows() {
                    let line = row.line();
                    prop_assert!(!line.is_empty());
                    let old_first = line.starts_with(&row.old);
                    let state_first = ["Problem, ", "Care, ", "Same name: ", "Swap with ", "Ring of ", "Case only: ", "Edited by you: "]
                        .iter()
                        .any(|s| line.starts_with(s));
                    prop_assert!(old_first || state_first, "{}", line);
                }
            }
            Err(error) => {
                prop_assert!(!error.line().is_empty());
                prop_assert!(error.line().starts_with("Rule ") || error.line().starts_with("Pass "));
            }
        }
    }
}
