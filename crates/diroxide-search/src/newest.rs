//! Newest files: the files whose modified day is the latest day that has
//! any, a generated search over the scope; "Earlier day" steps back to
//! the latest day before that one that has files. Days are local days at
//! the clock's offset. Title: "Newest files: 12 changed on October 8,
//! 2026".

use std::ops::ControlFlow;

use diroxide_core::clock::month_name;
use diroxide_core::filter::{Filter, FilterError};
use diroxide_core::{Clock, ListModel, Timestamp, UtcOffset};
use diroxide_vfs::Registry;

use crate::{
    Scope, SearchControl, SearchError, SearchHit, SearchQuery, SearchSink, SearchSummary,
    results_list_under, run_search,
};

/// A day on the calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    /// The year, such as 2026.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day of the month.
    pub day: u8,
}

impl Day {
    /// The local day of a moment at an offset.
    pub fn of(time: Timestamp, offset: UtcOffset) -> Self {
        let civil = time.to_civil(offset);
        Self {
            year: civil.year,
            month: civil.month,
            day: civil.day,
        }
    }

    /// The day in words: "October 8, 2026".
    pub fn words(&self) -> String {
        format!("{} {}, {}", month_name(self.month), self.day, self.year)
    }
}

/// What newest files found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Newest {
    /// The day, or `None` when no file has one (before the day asked).
    pub day: Option<Day>,
    /// The day asked to look before, for "Earlier day".
    pub before: Option<Day>,
    /// The files changed on that day, newest first.
    pub hits: Vec<SearchHit>,
    /// How the walk went.
    pub summary: SearchSummary,
}

impl Newest {
    /// The title line, meaning first: "Newest files: 12 changed on
    /// October 8, 2026", "Newest files: none before October 8, 2026".
    pub fn title(&self) -> String {
        match (self.day, self.before) {
            (Some(day), _) => format!(
                "Newest files: {} changed on {}",
                self.hits.len(),
                day.words()
            ),
            (None, Some(before)) => format!("Newest files: none before {}", before.words()),
            (None, None) => "Newest files: none found".to_owned(),
        }
    }

    /// The day "Earlier day" looks before: this list's day.
    pub fn earlier(&self) -> Option<Day> {
        self.day
    }

    /// The list, titled as [`Newest::title`].
    pub fn list(&self, root: &diroxide_core::Place) -> ListModel {
        results_list_under(&self.title(), Some(root), &self.hits)
    }
}

/// Keeps only the files of the latest day seen.
struct Latest {
    offset: UtcOffset,
    before: Option<Day>,
    day: Option<Day>,
    hits: Vec<SearchHit>,
}

impl SearchSink for Latest {
    fn found(&mut self, hit: SearchHit) -> ControlFlow<()> {
        let Some(modified) = hit.entry.modified else {
            return ControlFlow::Continue(());
        };
        let day = Day::of(modified, self.offset);
        if self.before.is_some_and(|before| day >= before) {
            return ControlFlow::Continue(());
        }
        match self.day {
            Some(best) if day < best => {}
            Some(best) if day == best => self.hits.push(hit),
            _ => {
                self.day = Some(day);
                self.hits.clear();
                self.hits.push(hit);
            }
        }
        ControlFlow::Continue(())
    }
}

/// The expression newest files runs: every file.
fn every_file() -> Result<Filter, FilterError> {
    Filter::parse("kind file")
}

/// Newest files over a scope, or with `before` the latest day before it
/// that has files.
pub fn newest_files(
    scope: &Scope,
    registry: &Registry,
    clock: &dyn Clock,
    control: &SearchControl,
    before: Option<Day>,
) -> Result<Newest, SearchError> {
    let filter = every_file().map_err(|_| SearchError::NotBuilt("newest files".to_owned()))?;
    let query = SearchQuery {
        filter: Some(filter),
        scope: scope.clone().with_row_cap(None),
        rows: crate::RowShape::PerFile,
    };
    let mut latest = Latest {
        offset: clock.offset(),
        before,
        day: None,
        hits: Vec::new(),
    };
    let summary = run_search(&query, registry, clock, control, &mut latest)?;
    let mut hits = latest.hits;
    hits.sort_by(|a, b| {
        b.entry
            .modified
            .cmp(&a.entry.modified)
            .then_with(|| a.entry.name.cmp(&b.entry.name))
    });
    Ok(Newest {
        day: latest.day,
        before,
        hits,
        summary,
    })
}
