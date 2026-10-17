//! The tracker over time, on a fixed clock: the slow-producer simulator,
//! the named cases of R16, property tests over event interleavings, the
//! steppable watch over the fake provider, and one real watch.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use diroxide_core::{EntryKind, Place, Timestamp};
use diroxide_vfs::WatchEvent;
use proptest::prelude::*;

use super::*;
use crate::fake::FakeProvider;
use crate::testing::p;

/// A moment `ms` milliseconds into a test.
fn at(ms: u64) -> Timestamp {
    Timestamp::from_unix(
        1_800_000_000 + i64::try_from(ms / 1000).unwrap(),
        u32::try_from((ms % 1000) * 1_000_000).unwrap(),
    )
}

/// A producer writing one file in chunks.
#[derive(Clone, Debug)]
struct Producer {
    name: String,
    /// When each chunk is written, in milliseconds, and its bytes; the
    /// first makes the file.
    chunks: Vec<(u64, u64)>,
    /// It keeps the file open from its first chunk to its last, as a copy,
    /// a download or a scanner does; otherwise it opens and closes the
    /// file for each chunk.
    holds: bool,
}

impl Producer {
    fn new(name: &str, chunks: Vec<(u64, u64)>, holds: bool) -> Self {
        Self {
            name: name.to_owned(),
            chunks,
            holds,
        }
    }

    /// A producer that writes `bytes` every `every` ms from `from` to
    /// `until`, inclusive.
    fn steady(from: u64, until: u64, every: u64, bytes: u64) -> Vec<(u64, u64)> {
        (from..=until)
            .step_by(usize::try_from(every).unwrap())
            .map(|t| (t, bytes))
            .collect()
    }

    fn first(&self) -> u64 {
        self.chunks[0].0
    }

    fn last(&self) -> u64 {
        self.chunks[self.chunks.len() - 1].0
    }

    fn total(&self) -> u64 {
        self.chunks.iter().map(|(_, bytes)| bytes).sum()
    }

    fn size_at(&self, t: u64) -> Option<u64> {
        (t >= self.first()).then(|| {
            self.chunks
                .iter()
                .filter(|(w, _)| *w <= t)
                .map(|(_, bytes)| bytes)
                .sum()
        })
    }

    fn written_at(&self, t: u64) -> u64 {
        self.chunks
            .iter()
            .map(|(w, _)| *w)
            .filter(|w| *w <= t)
            .max()
            .unwrap_or(0)
    }

    /// What a look at `t` finds; the probe answers only where the platform
    /// can tell.
    fn look(&self, t: u64, probe: bool, platform_tells: bool) -> Look {
        match self.size_at(t) {
            None => Look::Absent,
            Some(size) => {
                let writer = (probe && platform_tells).then(|| self.holds && t < self.last());
                Look::Present(
                    Facts {
                        inode: Some(7),
                        volume: Some(1),
                        ..Facts::file(size, at(self.written_at(t)))
                    }
                    .with_writer_open(writer),
                )
            }
        }
    }
}

/// When the watch hears of each chunk: after a delay, or never.
#[derive(Clone, Debug)]
struct Deliveries {
    /// Per producer, per chunk: the delay in ms, or `None` when the event
    /// is lost. The first chunk's event always comes.
    delays: Vec<Vec<Option<u64>>>,
    /// A full listing at these moments, as after an overflow.
    rescans: Vec<u64>,
}

impl Deliveries {
    fn prompt(producers: &[Producer]) -> Self {
        Self {
            delays: producers
                .iter()
                .map(|p| p.chunks.iter().map(|_| Some(200)).collect())
                .collect(),
            rescans: Vec::new(),
        }
    }
}

/// The step of the simulation, in ms.
const STEP: u64 = 50;

/// Runs producers against a tracker the way [`ArrivalWatch`] drives one:
/// events look at their name, the tick runs every half second while
/// anything settles. Returns each event with its moment.
fn simulate(
    producers: &[Producer],
    rules: SettleRules,
    platform_tells: bool,
    deliveries: &Deliveries,
    until: u64,
) -> Vec<(u64, TrackerEvent)> {
    let mut tracker = Tracker::new(rules);
    let tick = u64::try_from(rules.tick.as_millis()).unwrap();
    let mut tick_at: Option<u64> = None;
    let mut out = Vec::new();
    let mut t = 0;
    while t <= until {
        let mut events = Vec::new();
        for (index, producer) in producers.iter().enumerate() {
            for (chunk, (written, _)) in producer.chunks.iter().enumerate() {
                let delay = deliveries.delays[index][chunk];
                let delay = if chunk == 0 { delay.or(Some(0)) } else { delay };
                if let Some(delay) = delay
                    && written + delay == t
                {
                    let look = producer.look(t, false, platform_tells);
                    events.extend(tracker.observe(&producer.name, look, at(t)));
                }
            }
        }
        if deliveries.rescans.contains(&t) {
            let listing: Vec<(String, Facts)> = producers
                .iter()
                .filter_map(|p| match p.look(t, false, platform_tells) {
                    Look::Present(facts) => Some((p.name.clone(), facts)),
                    _ => None,
                })
                .collect();
            events.extend(tracker.reconcile(listing, at(t)));
        }
        if tick_at.is_some_and(|due| due <= t) {
            tick_at = None;
            let mut look = |name: &str, probe: bool| {
                producers
                    .iter()
                    .find(|p| p.name == name)
                    .map_or(Look::Absent, |p| p.look(t, probe, platform_tells))
            };
            events.extend(tracker.tick(at(t), &mut look));
        }
        if tracker.is_settling() {
            tick_at.get_or_insert(t + tick);
        } else {
            tick_at = None;
        }
        out.extend(events.into_iter().map(|event| (t, event)));
        t += STEP;
    }
    out
}

fn arrivals(events: &[(u64, TrackerEvent)]) -> Vec<(u64, Arrival)> {
    events
        .iter()
        .filter_map(|(t, event)| match event {
            TrackerEvent::Arrived(arrival) => Some((*t, arrival.clone())),
            _ => None,
        })
        .collect()
}

/// The producer of the acceptance case: grows for two seconds, pauses for
/// three, grows for three more; five seconds of growth around a
/// three-second pause.
fn pausing_producer(holds: bool) -> Producer {
    let mut chunks = Producer::steady(0, 2_000, 250, 64_000);
    chunks.extend(Producer::steady(5_000, 8_000, 250, 64_000));
    Producer::new("scan.pdf", chunks, holds)
}

#[test]
fn a_file_growing_for_5_seconds_with_a_3_second_pause_is_not_ready_early() {
    let producer = pausing_producer(true);
    let events = simulate(
        std::slice::from_ref(&producer),
        SettleRules::WATCHED,
        true,
        &Deliveries::prompt(std::slice::from_ref(&producer)),
        20_000,
    );
    let arrived = arrivals(&events);
    assert_eq!(arrived.len(), 1, "{arrived:?}");
    let (t, arrival) = &arrived[0];
    // Not during the pause, not before the last write and the quiet
    // period, and at most a tick later.
    assert!(*t >= producer.last() + 2_000, "arrived at {t}");
    assert!(*t <= producer.last() + 2_000 + 500 + STEP, "arrived at {t}");
    assert_eq!(arrival.size, Some(producer.total()));
    assert!(!arrival.again);
    assert_eq!(arrival.name, "scan.pdf");
}

#[test]
fn without_the_probe_a_pause_longer_than_the_quiet_period_is_said_early_then_again() {
    // On a platform that cannot tell whether a writer holds the file, the
    // quiet period alone decides: the pause looks like the end, and the
    // second half arrives again with the final size.
    let producer = pausing_producer(false);
    let events = simulate(
        std::slice::from_ref(&producer),
        SettleRules::WATCHED,
        false,
        &Deliveries::prompt(std::slice::from_ref(&producer)),
        20_000,
    );
    let arrived = arrivals(&events);
    assert_eq!(arrived.len(), 2, "{arrived:?}");
    assert!(arrived[0].0 >= 4_000 && arrived[0].0 < 5_000, "{arrived:?}");
    assert!(!arrived[0].1.again);
    assert!(arrived[1].0 >= producer.last() + 2_000, "{arrived:?}");
    assert!(arrived[1].1.again);
    assert_eq!(arrived[1].1.size, Some(producer.total()));
}

#[test]
fn a_quiet_period_longer_than_the_pause_covers_it_without_the_probe() {
    let producer = pausing_producer(false);
    let rules = SettleRules::WATCHED.with_quiet(Duration::from_secs(4));
    let events = simulate(
        std::slice::from_ref(&producer),
        rules,
        false,
        &Deliveries::prompt(std::slice::from_ref(&producer)),
        20_000,
    );
    let arrived = arrivals(&events);
    assert_eq!(arrived.len(), 1, "{arrived:?}");
    assert!(arrived[0].0 >= producer.last() + 4_000, "{arrived:?}");
    assert_eq!(arrived[0].1.size, Some(producer.total()));
}

#[test]
fn a_steady_writer_is_never_ready_while_it_writes() {
    // Five seconds of growth with events only now and then: each tick's
    // look sees the growth the events missed.
    let producer = Producer::new(
        "video.mkv",
        Producer::steady(0, 5_000, 100, 1_000_000),
        false,
    );
    let mut deliveries = Deliveries::prompt(std::slice::from_ref(&producer));
    for (chunk, delay) in deliveries.delays[0].iter_mut().enumerate() {
        if chunk > 0 {
            *delay = None;
        }
    }
    let events = simulate(
        std::slice::from_ref(&producer),
        SettleRules::WATCHED,
        false,
        &deliveries,
        12_000,
    );
    let arrived = arrivals(&events);
    assert_eq!(arrived.len(), 1, "{arrived:?}");
    assert!(arrived[0].0 >= 7_000, "{arrived:?}");
    assert_eq!(arrived[0].1.size, Some(producer.total()));
}

#[test]
fn a_file_renamed_in_whole_settles_in_half_a_second() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    let facts = Facts::file(2_100_000, at(0));
    assert!(
        tracker
            .renamed_in("report.pdf", Look::Present(facts.clone()), at(0))
            .is_empty()
    );
    let mut look = |_: &str, _: bool| Look::Present(facts.clone());
    assert!(tracker.tick(at(250), &mut look).is_empty());
    let events = tracker.tick(at(500), &mut look);
    assert!(
        matches!(&events[..], [TrackerEvent::Arrived(a)] if a.name == "report.pdf"),
        "{events:?}"
    );
    assert_eq!(tracker.state("report.pdf"), Some(ArrivalState::Settled));
    assert!(
        !tracker.is_settling(),
        "the tick stops once nothing settles"
    );
}

#[test]
fn a_name_that_goes_away_before_it_settles_is_gone_and_never_arrives() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    tracker.observe("draft.txt", Look::Present(Facts::file(10, at(0))), at(0));
    assert_eq!(tracker.waiting(), vec!["draft.txt".to_owned()]);
    let events = tracker.tick(at(500), &mut |_, _| Look::Absent);
    assert_eq!(
        events,
        vec![TrackerEvent::Gone {
            name: "draft.txt".to_owned()
        }]
    );
    assert_eq!(tracker.state("draft.txt"), None);
    assert!(tracker.waiting().is_empty());
    assert_eq!(tracker.arrived(), 0);
}

#[test]
fn the_same_name_replaced_by_another_file_starts_again() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    let first = Facts {
        inode: Some(1),
        ..Facts::file(10, at(0))
    };
    tracker.observe("scan.pdf", Look::Present(first.clone()), at(0));
    // Another file under the same name, same size and time: the identity
    // differs, so the quiet period starts again.
    let second = Facts {
        inode: Some(2),
        ..first.clone()
    };
    tracker.tick(at(1_500), &mut |_, _| Look::Present(second.clone()));
    assert_eq!(tracker.state("scan.pdf"), Some(ArrivalState::Growing));
    let events = tracker.tick(at(2_500), &mut |_, _| Look::Present(second.clone()));
    assert!(events.is_empty(), "{events:?}");
    let events = tracker.tick(at(3_500), &mut |_, _| Look::Present(second.clone()));
    assert!(
        matches!(&events[..], [TrackerEvent::Arrived(_)]),
        "{events:?}"
    );
}

#[test]
fn a_listing_that_leaves_the_identity_unknown_is_not_a_change() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    let looked = Facts {
        inode: Some(9),
        volume: Some(3),
        ..Facts::file(10, at(0))
    };
    tracker.observe("a.txt", Look::Present(looked.clone()), at(0));
    tracker.tick(at(500), &mut |_, _| Look::Present(looked.clone()));
    assert_eq!(tracker.state("a.txt"), Some(ArrivalState::Paused));
    // Windows lists without file indexes: no reset.
    let listed = Facts {
        inode: None,
        ..looked.clone()
    };
    tracker.reconcile([("a.txt".to_owned(), listed)], at(1_000));
    assert_eq!(tracker.state("a.txt"), Some(ArrivalState::Paused));
    let events = tracker.tick(at(2_000), &mut |_, _| Look::Present(looked.clone()));
    assert!(
        matches!(&events[..], [TrackerEvent::Arrived(_)]),
        "{events:?}"
    );
}

#[test]
fn folders_links_and_online_only_files_are_skipped_and_only_online_only_is_said() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    let of = |kind| Facts {
        kind,
        ..Facts::file(0, at(0))
    };
    let mut said = Vec::new();
    for (name, facts) in [
        ("photos", of(EntryKind::Folder)),
        ("latest", of(EntryKind::Link)),
        ("fifo", of(EntryKind::Special)),
        (
            "scan-004.pdf",
            Facts {
                online_only: true,
                ..Facts::file(1_200_000, at(0))
            },
        ),
    ] {
        for event in tracker.observe(name, Look::Present(facts), at(0)) {
            if let TrackerEvent::Skipped { name, reason } = event
                && reason.is_said()
            {
                said.push(skipped_line(&name, reason));
            }
        }
    }
    assert_eq!(said, vec!["Skipped scan-004.pdf: online only"]);
    assert!(!tracker.is_settling());
    assert_eq!(
        tracker.state("photos"),
        Some(ArrivalState::Skipped(SkipReason::Folder))
    );
    // Seen again, nothing more is said.
    let again = tracker.observe(
        "scan-004.pdf",
        Look::Present(Facts {
            online_only: true,
            ..Facts::file(1_200_000, at(0))
        }),
        at(100),
    );
    assert!(again.is_empty());
    // Its data came: it settles like any new file.
    tracker.observe(
        "scan-004.pdf",
        Look::Present(Facts::file(1_200_000, at(0))),
        at(200),
    );
    assert_eq!(tracker.state("scan-004.pdf"), Some(ArrivalState::Growing));
}

#[test]
fn ignored_names_never_get_a_record() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    for name in [
        "report.pdf.crdownload",
        ".diro-part-00c0ffee",
        "~$memo.docx",
        "a.swp",
    ] {
        assert!(
            tracker
                .observe(name, Look::Present(Facts::file(1, at(0))), at(0))
                .is_empty()
        );
    }
    assert!(tracker.is_empty());
}

#[test]
fn the_baseline_is_not_an_arrival_but_a_change_to_it_is() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    tracker.baseline(
        [
            ("old.pdf".to_owned(), Facts::file(10, at(0))),
            (
                "folder".to_owned(),
                Facts {
                    kind: EntryKind::Folder,
                    ..Facts::file(0, at(0))
                },
            ),
            (".hidden".to_owned(), Facts::file(1, at(0))),
        ],
        at(0),
    );
    assert_eq!(tracker.len(), 2);
    assert!(!tracker.is_settling());
    // Looked at unchanged: nothing.
    tracker.observe("old.pdf", Look::Present(Facts::file(10, at(0))), at(1_000));
    assert!(!tracker.is_settling());
    // Overwritten: it settles, and arrives "again".
    let new = Facts::file(20, at(1_500));
    tracker.observe("old.pdf", Look::Present(new.clone()), at(1_500));
    let mut events = Vec::new();
    for t in [2_000, 2_500, 3_000, 3_500] {
        events.extend(tracker.tick(at(t), &mut |_, _| Look::Present(new.clone())));
    }
    match &events[..] {
        [TrackerEvent::Arrived(arrival)] => {
            assert!(arrival.again);
            assert_eq!(arrival.size, Some(20));
            assert_eq!(arrived_line(arrival), "Arrived again: old.pdf, 20 bytes");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_that_never_stops_changing_is_given_up_after_the_longest_wait() {
    let rules = SettleRules {
        max_wait: Duration::from_secs(10),
        ..SettleRules::WATCHED
    };
    let mut tracker = Tracker::new(rules);
    tracker.observe("app.log", Look::Present(Facts::file(0, at(0))), at(0));
    let mut gave_up = Vec::new();
    for step in 1..=24u64 {
        let t = step * 500;
        let facts = Facts::file(step, at(t));
        for event in tracker.tick(at(t), &mut |_, _| Look::Present(facts.clone())) {
            gave_up.push((t, event));
        }
    }
    assert_eq!(
        gave_up,
        vec![(
            10_000,
            TrackerEvent::GaveUp {
                name: "app.log".to_owned()
            }
        )]
    );
    assert_eq!(tracker.state("app.log"), Some(ArrivalState::GaveUp));
    assert!(!tracker.is_settling(), "no tick for a file given up");
    assert_eq!(gave_up_line("app.log"), "Gave up waiting for app.log");
}

#[test]
fn a_thousand_arrivals_in_one_tick_come_in_natural_order() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    let names: Vec<String> = (0..1000).rev().map(|n| format!("scan-{n}.pdf")).collect();
    for name in &names {
        tracker.observe(name, Look::Present(Facts::file(5, at(0))), at(0));
    }
    let mut look = |_: &str, _: bool| Look::Present(Facts::file(5, at(0)));
    tracker.tick(at(500), &mut look);
    let events = tracker.tick(at(2_000), &mut look);
    let arrived: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            TrackerEvent::Arrived(a) => Some(a.name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(arrived.len(), 1000);
    assert_eq!(arrived[0], "scan-0.pdf");
    assert_eq!(arrived[9], "scan-9.pdf");
    assert_eq!(arrived[10], "scan-10.pdf");
    assert_eq!(arrived[999], "scan-999.pdf");
    assert_eq!(tracker.arrived(), 1000);
}

#[test]
fn a_writer_still_holding_a_quiet_file_keeps_it_waiting() {
    let mut tracker = Tracker::new(SettleRules::WATCHED);
    let facts = Facts::file(4_000_000_000, at(0));
    tracker.observe("big.iso", Look::Present(facts.clone()), at(0));
    // A copy that set the whole size at once: nothing changes, but the
    // writer holds the file until 9 seconds.
    let mut asked = Vec::new();
    for step in 1..=24u64 {
        let t = step * 500;
        let events = tracker.tick(at(t), &mut |_, probe| {
            asked.push((t, probe));
            Look::Present(facts.clone().with_writer_open(probe.then_some(t < 9_000)))
        });
        if let [TrackerEvent::Arrived(_)] = &events[..] {
            assert_eq!(t, 9_000);
            // The probe is asked only once the quiet period has passed.
            assert!(asked.iter().all(|(when, probe)| *probe == (*when >= 2_000)));
            return;
        }
        assert!(events.is_empty(), "{events:?}");
    }
    panic!("never arrived");
}

/// The producers of a property case: names, chunk times and sizes, and
/// whether each holds its file.
fn producers() -> impl Strategy<Value = Vec<Producer>> {
    prop::collection::vec(
        (
            prop::sample::select(vec![
                "scan-1.pdf",
                "scan-2.pdf",
                "scan-10.pdf",
                "video.mkv",
                "notes.txt",
                "report.pdf.crdownload",
            ]),
            0u64..3_000,
            prop::collection::vec((1u64..2_500, 1u64..100_000), 1..8),
            any::<bool>(),
        ),
        1..4,
    )
    .prop_map(|specs| {
        let mut seen = std::collections::HashSet::new();
        specs
            .into_iter()
            .filter(|(name, ..)| seen.insert(*name))
            .map(|(name, start, gaps, holds)| {
                let mut t = start / STEP * STEP;
                let mut chunks = Vec::new();
                for (index, (gap, bytes)) in gaps.into_iter().enumerate() {
                    if index > 0 {
                        t += gap / STEP * STEP + STEP;
                    }
                    chunks.push((t, bytes));
                }
                Producer::new(name, chunks, holds)
            })
            .collect()
    })
}

/// Delays for each chunk's event (some lost when `lossy`), and rescans.
fn deliveries(producers: &[Producer], lossy: bool) -> impl Strategy<Value = Deliveries> + use<> {
    let shapes: Vec<usize> = producers.iter().map(|p| p.chunks.len()).collect();
    let delay = if lossy {
        prop::option::weighted(0.7, (0u64..24).prop_map(|d| d * STEP)).boxed()
    } else {
        (0u64..24).prop_map(|d| Some(d * STEP)).boxed()
    };
    (
        shapes
            .into_iter()
            .map(|n| prop::collection::vec(delay.clone(), n))
            .collect::<Vec<_>>(),
        prop::collection::vec((0u64..400).prop_map(|t| t * STEP), 0..3),
    )
        .prop_map(|(delays, rescans)| Deliveries { delays, rescans })
}

fn case(lossy: bool) -> impl Strategy<Value = (Vec<Producer>, Deliveries, bool)> {
    producers().prop_flat_map(move |producers| {
        let deliveries = deliveries(&producers, lossy);
        (Just(producers), deliveries, any::<bool>())
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// However the events come (late, lost, in any order, with rescans
    /// between), nothing arrives while it is still being written: a file
    /// arrives only after a whole quiet period with no write, at its size
    /// then, never twice without a change, ignored names never, and the
    /// files of one tick in natural order.
    #[test]
    fn nothing_arrives_early_or_twice((producers, deliveries, tells) in case(true)) {
        let quiet = 2_000;
        let events = simulate(&producers, SettleRules::WATCHED, tells, &deliveries, 25_000);
        let arrived = arrivals(&events);
        let mut last: std::collections::HashMap<String, (u64, Option<u64>)> = Default::default();
        for (t, arrival) in &arrived {
            let producer = producers.iter().find(|p| p.name == arrival.name).unwrap();
            prop_assert!(!is_ignored(&producer.name));
            prop_assert!(
                producer.chunks.iter().all(|(w, _)| !(t.saturating_sub(quiet) < *w && w <= t)),
                "{} arrived at {t} within the quiet period of a write: {:?}",
                arrival.name,
                producer.chunks
            );
            prop_assert_eq!(arrival.size, producer.size_at(*t));
            if tells && producer.holds {
                prop_assert!(*t >= producer.last() + quiet, "held until {}, arrived at {t}", producer.last());
            }
            if let Some((_, size)) = last.get(&arrival.name) {
                prop_assert_ne!(*size, arrival.size, "{} arrived twice unchanged", arrival.name);
                prop_assert!(arrival.again);
            } else {
                prop_assert!(!arrival.again);
            }
            last.insert(arrival.name.clone(), (*t, arrival.size));
        }
        let mut by_tick: std::collections::BTreeMap<u64, Vec<String>> = Default::default();
        for (t, arrival) in &arrived {
            by_tick.entry(*t).or_default().push(arrival.name.clone());
        }
        for names in by_tick.values() {
            let mut sorted = names.clone();
            sorted.sort_by(|a, b| diroxide_core::sort::compare_names(
                a, b, diroxide_core::sort::NameCase::Ignore, true));
            prop_assert_eq!(names, &sorted);
        }
    }

    /// When no event is lost, every file that is not ignored arrives in
    /// the end with its whole size, within the quiet period, a tick, the
    /// latest delivery and a step of its last write.
    #[test]
    fn every_file_arrives_whole_in_the_end((producers, deliveries, tells) in case(false)) {
        let events = simulate(&producers, SettleRules::WATCHED, tells, &deliveries, 30_000);
        let arrived = arrivals(&events);
        for producer in producers.iter().filter(|p| !is_ignored(&p.name)) {
            let last = arrived.iter().rev().find(|(_, a)| a.name == producer.name);
            let Some((t, arrival)) = last else {
                return Err(TestCaseError::fail(format!("{} never arrived", producer.name)));
            };
            prop_assert_eq!(arrival.size, Some(producer.total()));
            prop_assert!(
                *t <= producer.last() + 2_000 + 500 + 1_200 + STEP,
                "{} arrived at {t}, last written at {}",
                producer.name,
                producer.last()
            );
        }
    }

    /// The order the events come in does not change what arrives: with
    /// every event, the same files arrive with the same final sizes
    /// whether the events come at once or late.
    #[test]
    fn the_order_of_events_does_not_change_what_arrives(
        (producers, deliveries, tells) in case(false)
    ) {
        let late = simulate(&producers, SettleRules::WATCHED, tells, &deliveries, 30_000);
        let prompt = simulate(
            &producers,
            SettleRules::WATCHED,
            tells,
            &Deliveries::prompt(&producers),
            30_000,
        );
        let finals = |events: &[(u64, TrackerEvent)]| {
            let mut out: Vec<(String, Option<u64>)> = Vec::new();
            for (_, arrival) in arrivals(events) {
                out.retain(|(name, _)| *name != arrival.name);
                out.push((arrival.name, arrival.size));
            }
            out.sort();
            out
        };
        prop_assert_eq!(finals(&late), finals(&prompt));
    }
}

// The steppable watch over the fake provider, on a fixed clock.

fn fake_watch(provider: &Arc<FakeProvider>, pace: Option<Duration>) -> ArrivalWatch {
    let mut watch = ArrivalWatch::new(
        Arc::clone(provider) as Arc<dyn diroxide_vfs::Provider>,
        p("/in"),
        SettleRules::WATCHED,
    )
    .with_writer_probe(false);
    if let Some(pace) = pace {
        watch = watch.with_pace(pace);
    }
    watch
}

/// Runs the watch's time-driven work every 50 ms from `from` to `until`.
fn run(watch: &mut ArrivalWatch, from: u64, until: u64) -> Vec<(u64, WatchNote)> {
    let mut notes = Vec::new();
    let mut t = from;
    while t <= until {
        if watch.next_wake(at(t)).is_some_and(|d| d.is_zero()) {
            notes.extend(watch.wake(at(t)).into_iter().map(|n| (t, n)));
        }
        t += STEP;
    }
    notes
}

fn grow(provider: &FakeProvider, place: &Place, size: u64, t: u64) {
    provider.set_listed_size(place, size);
    provider.set_modified(place, at(t));
}

#[test]
fn the_watch_says_an_arrival_once_it_settles_and_waits_in_the_status_line() {
    let provider = Arc::new(FakeProvider::new());
    provider.add_file(&p("/in/old.txt"), b"old");
    provider.add_folder(&p("/in/sub"));
    let mut watch = fake_watch(&provider, Some(Duration::from_secs(10)));
    assert_eq!(
        watch.start(at(0)).unwrap(),
        vec![WatchNote::Started {
            already: 2,
            polled: None
        }]
    );
    assert_eq!(watch.next_wake(at(0)), None, "an idle watch needs no tick");
    let scan = p("/in/scan-004.pdf");
    provider.add_file(&scan, b"");
    grow(&provider, &scan, 400_000, 1_000);
    let notes = watch.event(&WatchEvent::Created(scan.clone()), at(1_000));
    assert_eq!(
        notes,
        vec![WatchNote::Waiting(Some(
            "Waiting for scan-004.pdf to settle".to_owned()
        ))]
    );
    let mut notes = Vec::new();
    // It grows for two seconds, with events, then stops.
    for t in (1_500..=3_000).step_by(500) {
        grow(&provider, &scan, 400_000 + t, t);
        notes.extend(watch.event(&WatchEvent::Changed(scan.clone()), at(t)));
        notes.extend(watch.wake(at(t)));
    }
    notes.extend(run(&mut watch, 3_050, 8_000).into_iter().map(|(_, n)| n));
    assert_eq!(
        notes,
        vec![
            WatchNote::Arrived(Arrival {
                name: "scan-004.pdf".to_owned(),
                size: Some(403_000),
                again: false,
                waited: Duration::from_millis(4_000),
            }),
            WatchNote::Say("Arrived: scan-004.pdf, 403 KB".to_owned()),
            WatchNote::Waiting(None),
        ]
    );
    assert_eq!(watch.arrived(), 1);
    assert_eq!(watch.next_wake(at(8_000)), None);
    // The baseline's folder and file are not arrivals; touching the
    // folder says nothing.
    assert!(
        watch
            .event(&WatchEvent::Changed(p("/in/sub")), at(9_000))
            .is_empty()
    );
}

#[test]
fn the_watch_paces_its_lines_and_sums_up_several() {
    let provider = Arc::new(FakeProvider::new());
    provider.add_folder(&p("/in"));
    let mut watch = fake_watch(&provider, Some(Duration::from_secs(10)));
    watch.start(at(0)).unwrap();
    let mut said = Vec::new();
    let mut arrived = 0;
    // Four files, a second apart.
    for (n, t) in [(1u64, 0u64), (2, 1_000), (3, 2_000), (4, 3_000)] {
        let place = p(&format!("/in/scan-{n}.pdf"));
        provider.add_file(&place, b"x");
        provider.set_modified(&place, at(t));
        for note in watch.event(&WatchEvent::Created(place), at(t)) {
            if let WatchNote::Say(line) = note {
                said.push((t, line));
            }
        }
        for (when, note) in run(&mut watch, t, t + 950) {
            match note {
                WatchNote::Say(line) => said.push((when, line)),
                WatchNote::Arrived(_) => arrived += 1,
                _ => {}
            }
        }
    }
    for (when, note) in run(&mut watch, 4_000, 30_000) {
        match note {
            WatchNote::Say(line) => said.push((when, line)),
            WatchNote::Arrived(_) => arrived += 1,
            _ => {}
        }
    }
    assert_eq!(arrived, 4);
    // The first is said when it settles; the other three wait for the
    // interval and become one summary: never more than one line in ten
    // seconds.
    assert_eq!(
        said,
        vec![
            (2_000, "Arrived: scan-1.pdf, 1 byte".to_owned()),
            (12_000, "Watching in: 4 arrived".to_owned()),
        ]
    );
}

#[test]
fn a_rescan_finds_what_the_events_missed_and_the_folder_gone_ends_the_watch() {
    let provider = Arc::new(FakeProvider::new());
    provider.add_folder(&p("/in"));
    let mut watch = fake_watch(&provider, None);
    watch.start(at(0)).unwrap();
    provider.add_file(&p("/in/a.txt"), b"a");
    provider.add_file(&p("/in/b.txt"), b"b");
    let notes = watch.event(&WatchEvent::Rescan, at(100));
    assert_eq!(
        notes,
        vec![WatchNote::Waiting(Some(
            "Waiting for 2 files to settle".to_owned()
        ))]
    );
    let arrivals: Vec<String> = run(&mut watch, 150, 5_000)
        .into_iter()
        .filter_map(|(_, n)| match n {
            WatchNote::Arrived(a) => Some(a.name),
            _ => None,
        })
        .collect();
    assert_eq!(arrivals, vec!["a.txt", "b.txt"]);
    assert_eq!(
        watch.event(&WatchEvent::Gone, at(6_000)),
        vec![WatchNote::Ended {
            line: "Stopped watching in: the folder is gone".to_owned()
        }]
    );
}

#[test]
fn a_place_that_cannot_be_watched_is_listed_every_few_seconds() {
    let provider = Arc::new(FakeProvider::new());
    provider.add_folder(&p("/in"));
    let mut watch = ArrivalWatch::new(
        Arc::clone(&provider) as Arc<dyn diroxide_vfs::Provider>,
        p("/in"),
        SettleRules::POLLED,
    )
    .with_writer_probe(false)
    .with_polling(POLL_EVERY);
    assert_eq!(
        watch.start(at(0)).unwrap(),
        vec![WatchNote::Started {
            already: 0,
            polled: Some(POLL_EVERY)
        }]
    );
    assert_eq!(watch.next_wake(at(0)), Some(POLL_EVERY));
    provider.add_file(&p("/in/share.pdf"), b"pdf");
    let notes = run(&mut watch, 50, 30_000);
    let arrived: Vec<(u64, String)> = notes
        .iter()
        .filter_map(|(t, n)| match n {
            WatchNote::Arrived(a) => Some((*t, a.name.clone())),
            _ => None,
        })
        .collect();
    // Seen on the poll at 5 seconds, settled 10 seconds later.
    assert_eq!(arrived, vec![(15_000, "share.pdf".to_owned())]);
    provider.vanish(&p("/in"));
    let notes = run(&mut watch, 30_050, 40_000);
    assert!(
        notes
            .iter()
            .any(|(_, n)| matches!(n, WatchNote::Ended { .. })),
        "{notes:?}"
    );
}

#[test]
fn a_folder_that_is_not_there_cannot_be_watched() {
    let provider = Arc::new(FakeProvider::new());
    let mut watch = fake_watch(&provider, None);
    assert!(watch.start(at(0)).is_err());
}

#[test]
fn a_real_watch_tells_of_a_file_written_into_a_folder() {
    // Real time, on the local provider: the bounds are generous, and the
    // test checks what arrives, not when.
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("before.txt"), b"there first").unwrap();
    let place = Place::parse(folder.path().to_str().unwrap()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&notes);
    let run = spawn_watch(
        Arc::new(diroxide_vfs::LocalProvider::new()),
        place,
        WatchOptions {
            quiet: Some(Duration::from_millis(500)),
            pace: None,
        },
        Arc::new(diroxide_core::SystemClock::utc()),
        Box::new(move |note| seen.lock().unwrap().push(note)),
    )
    .unwrap();
    let wait_for = |what: &dyn Fn(&[WatchNote]) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            if what(&notes.lock().unwrap()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    };
    assert!(wait_for(&|n| n
        .iter()
        .any(|n| matches!(n, WatchNote::Started { already: 1, .. }))));
    std::fs::write(folder.path().join("report.txt.part"), b"unfinished").unwrap();
    std::fs::write(folder.path().join("report.txt"), b"finished report").unwrap();
    assert!(wait_for(&|n| n
        .iter()
        .any(|n| matches!(n, WatchNote::Arrived(_)))));
    drop(run);
    let notes = notes.lock().unwrap();
    let arrived: Vec<&Arrival> = notes
        .iter()
        .filter_map(|n| match n {
            WatchNote::Arrived(a) => Some(a),
            _ => None,
        })
        .collect();
    assert_eq!(arrived.len(), 1, "{notes:?}");
    assert_eq!(arrived[0].name, "report.txt");
    assert_eq!(arrived[0].size, Some(15));
}

#[test]
fn time_helpers_measure_and_add() {
    assert_eq!(between(at(1_250), at(3_000)), Duration::from_millis(1_750));
    assert_eq!(between(at(3_000), at(1_250)), Duration::ZERO);
    assert_eq!(later_by(at(1_750), Duration::from_millis(1_500)), at(3_250));
}
