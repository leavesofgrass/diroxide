//! The photo helpers (alpha 4, lane AP, R30 commits 83 to 85; R29 5.5
//! and 5.6; R23's lessons in `research/file-management-lessons.md`):
//! the photo columns, the days, the bursts, the review and the month
//! folders, in words, for the terminal and the window alike. No picture
//! is drawn: the facts come from each picture's own Exif block, read by
//! the metadata crate (lane ML), never by decoding a pixel.
//!
//! - **The photo columns** are keys of the `columns` setting:
//!   `photo.taken` (Taken: the date taken, else the modified time with
//!   "(modified)" after it), `photo.camera` (Camera), `photo.pixels`
//!   (Pixels, as shown, after the orientation) and `photo.kind` (Kind:
//!   photo, screenshot or picture). A row says each after its label, as
//!   the other columns do: "taken 2026-10-12 14:03", "camera Pixel 8".
//! - **A picture's moment** is its Exif date and time as written, with
//!   no zone, or its modified time in local time when it has none ("No
//!   date taken: using modified time"); days, bursts and months come from
//!   it, and every weekday said is computed from the calendar.
//! - **A burst** is pictures from the same camera, each within the gap of
//!   the one before (the `photos.burst_gap_seconds` setting, 2 to start):
//!   "Burst, 7 pictures in 2 seconds".

use std::collections::BTreeMap;

use diroxide_core::format::count;
use diroxide_core::{Place, Side, Timestamp, UtcOffset};
use diroxide_meta::{Cell, Outcome, Pipeline, ReadLimits};
use diroxide_vfs::{OpContext, Provider};

/// The Taken column's key.
pub const TAKEN: &str = "photo.taken";
/// The Camera column's key.
pub const CAMERA: &str = "photo.camera";
/// The Pixels column's key: the size as shown.
pub const PIXELS: &str = "photo.pixels";
/// The Kind column's key: photo, screenshot or picture.
pub const KIND: &str = "photo.kind";

/// The word the readers leave in a picture's Taken cell when it has no
/// date taken; the app puts the modified time in its place.
pub const NO_DATE: &str = "no date taken";

/// Said where a picture's moment is its modified time (34 cells).
pub const NO_DATE_LINE: &str = "No date taken: using modified time";

/// A helper in a folder without pictures.
pub const NO_PICTURES: &str = "No pictures here";
/// Next day past the last date.
pub const NO_LATER_DAY: &str = "No later day here";
/// Previous day before the first date.
pub const NO_EARLIER_DAY: &str = "No earlier day here";
/// Select the screenshots where there are none.
pub const NO_SCREENSHOTS: &str = "No screenshots here";
/// Select pictures with no date where every picture has one.
pub const ALL_DATED: &str = "Every picture here has a date taken";
/// Back on the review's first picture.
pub const FIRST_PICTURE: &str = "This is the first picture";
/// No to the month folders' question.
pub const NOTHING_MOVED: &str = "Nothing moved.";

/// "Reading 14 pictures", while the dates are read.
pub fn reading_line(pictures: usize) -> String {
    format!("Reading {}", count(pictures as u64, "picture", "pictures"))
}

/// "No bursts here: no pictures from one camera within 2 seconds".
pub fn no_bursts_line(gap: u64) -> String {
    format!(
        "No bursts here: no pictures from one camera within {}",
        count(gap, "second", "seconds")
    )
}

/// "Cannot make 2026-10: a file has that name. Nothing moved."
pub fn month_taken_line(month: &str) -> String {
    format!("Cannot make {month}: a file has that name. Nothing moved.")
}

/// "Selected 4 screenshots. 4 selected.": what was selected, then the
/// count of the marks.
pub fn selected_line(what: &str, total: usize) -> String {
    format!("Selected {what}. {total} selected.")
}

/// One of the photo columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhotoField {
    /// When it was taken.
    Taken,
    /// The camera.
    Camera,
    /// The size as shown.
    Pixels,
    /// Photo, screenshot or picture.
    Kind,
}

impl PhotoField {
    /// Every photo column.
    pub const ALL: [PhotoField; 4] = [
        PhotoField::Taken,
        PhotoField::Camera,
        PhotoField::Pixels,
        PhotoField::Kind,
    ];

    /// The column of a key, if it is a photo column's.
    pub fn of(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.key() == key)
    }

    /// Its key in the `columns` setting.
    pub const fn key(self) -> &'static str {
        match self {
            PhotoField::Taken => TAKEN,
            PhotoField::Camera => CAMERA,
            PhotoField::Pixels => PIXELS,
            PhotoField::Kind => KIND,
        }
    }

    /// Its label, said before each cell.
    pub const fn label(self) -> &'static str {
        match self {
            PhotoField::Taken => "Taken",
            PhotoField::Camera => "Camera",
            PhotoField::Pixels => "Pixels",
            PhotoField::Kind => "Kind",
        }
    }
}

/// A cell's words as an entry keeps them: `None` while a reader was too
/// busy, so it is asked again; empty when the file has none.
fn kept(cell: Cell) -> Option<String> {
    match cell {
        Cell::Value(value) => Some(value),
        Cell::NotAvailable => Some(String::new()),
        Cell::Reading | Cell::NotComputed => None,
        other => Some(other.words()),
    }
}

/// One file's photo cell from the readers' outcomes (the picture readers
/// of the background class): the date taken or [`NO_DATE`]; the camera;
/// the size as shown; the kind, photo when a camera wrote it and picture
/// otherwise (the app makes a screenshot of a file the screenshots group
/// takes). A file no picture reader took has none.
pub fn cell(field: PhotoField, pipeline: &Pipeline, outcomes: &[Outcome]) -> Option<String> {
    let picture = outcomes
        .iter()
        .any(|(id, _)| *id == "image" || *id == "photo");
    if !picture {
        return Some(String::new());
    }
    match field {
        PhotoField::Taken => match pipeline.cell("image.taken", outcomes) {
            Cell::NotAvailable => Some(NO_DATE.to_owned()),
            other => kept(other),
        },
        PhotoField::Camera => kept(pipeline.cell("image.camera", outcomes)),
        PhotoField::Pixels => kept(pipeline.cell("image.dimensions", outcomes))
            .map(|v| v.trim_end_matches(", as shown").to_owned()),
        PhotoField::Kind => match pipeline.cell("image.camera", outcomes) {
            Cell::Value(_) => Some(Kind::Photo.word().to_owned()),
            Cell::Reading => None,
            _ => Some(Kind::Picture.word().to_owned()),
        },
    }
}

/// A moment as a picture says it: a civil date and time with no zone,
/// as the camera's clock showed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Moment {
    /// The year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u8,
    /// The day, 1 to 31.
    pub day: u8,
    /// The hour, 0 to 23.
    pub hour: u8,
    /// The minute.
    pub minute: u8,
    /// The second.
    pub second: u8,
}

impl Moment {
    /// An Exif date and time as written, `2026:10:12 14:03:07`, or the
    /// ISO form `2026-10-12 14:03`; `None` for anything else, a date that
    /// does not exist among them.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let digits = |at: usize, len: usize| -> Option<u32> {
            let part = text.get(at..at + len)?;
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse().ok())
                .flatten()
        };
        let year = i32::try_from(digits(0, 4)?).ok()?;
        let month = u8::try_from(digits(5, 2)?).ok()?;
        let day = u8::try_from(digits(8, 2)?).ok()?;
        let (hour, minute) = match text.as_bytes().get(10) {
            Some(b' ' | b'T') => (
                u8::try_from(digits(11, 2)?).ok()?,
                u8::try_from(digits(14, 2)?).ok()?,
            ),
            _ => (0, 0),
        };
        let second = digits(17, 2)
            .and_then(|s| u8::try_from(s).ok())
            .unwrap_or(0);
        let moment = Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        };
        // A date the calendar has, and not the zeros a camera without a
        // clock writes.
        moment.timestamp()?;
        Some(moment)
    }

    /// A time in local time at `offset`.
    pub fn of_time(time: Timestamp, offset: UtcOffset) -> Self {
        let civil = time.to_civil(offset);
        Self {
            year: civil.year,
            month: civil.month,
            day: civil.day,
            hour: civil.hour,
            minute: civil.minute,
            second: civil.second,
        }
    }

    /// The moment as if it were UTC: what orders moments and measures the
    /// gaps between them.
    fn timestamp(self) -> Option<Timestamp> {
        Timestamp::from_civil(
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            UtcOffset::UTC,
        )
    }

    /// Seconds on one scale, for gaps.
    pub fn seconds(self) -> i64 {
        self.timestamp().map_or(0, Timestamp::unix_seconds)
    }

    /// Its day.
    pub fn day(self) -> Day {
        Day {
            year: self.year,
            month: self.month,
            day: self.day,
        }
    }

    /// In ISO order to the minute, as the Taken cell says it: "2026-10-12
    /// 14:03"; sorting these words sorts the moments.
    pub fn words(self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }

    /// Its month folder's name: `2026-10`.
    pub fn month_folder(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }
}

/// A day of the calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    /// The year.
    pub year: i32,
    /// The month.
    pub month: u8,
    /// The day of the month.
    pub day: u8,
}

impl Day {
    /// The weekday, computed from the calendar, never typed: "Monday".
    pub fn weekday(self) -> &'static str {
        Timestamp::from_civil(self.year, self.month, self.day, 12, 0, 0, UtcOffset::UTC)
            .map_or("", |t| t.to_civil(UtcOffset::UTC).weekday.name())
    }

    /// "October 12", with its year when it is not `this_year`'s: "October
    /// 12, 2025".
    pub fn words(self, this_year: i32) -> String {
        let month = diroxide_core::clock::month_name(self.month);
        if self.year == this_year {
            format!("{month} {}", self.day)
        } else {
            format!("{month} {}, {}", self.day, self.year)
        }
    }
}

/// What the photo reader found in one picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhotoFact {
    /// When it was taken, as the camera wrote it.
    pub taken: Option<Moment>,
    /// The camera, maker and model in words.
    pub camera: Option<String>,
}

/// The camera in words: the model, the maker before it when the model
/// does not already say it (the metadata crate's way).
fn camera_words(make: Option<&str>, model: Option<&str>) -> Option<String> {
    fn clean(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    match (clean(make), clean(model)) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
            Some(model.to_owned())
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (Some(one), None) | (None, Some(one)) => Some(one.to_owned()),
        (None, None) => None,
    }
}

/// Reads one picture's date taken and camera from its Exif block, the
/// metadata crate's careful reader over a bounded read of the file's
/// head; nothing when it has none or cannot be read.
pub fn read_fact(provider: &dyn Provider, place: &Place, ctx: &OpContext) -> PhotoFact {
    let Ok(source) = provider.open_seekable(place, ctx) else {
        return PhotoFact::default();
    };
    let limits = ReadLimits::default();
    let mut bounded = diroxide_meta::Bounded::new(source, &limits, ctx.cancel.clone());
    let Ok(parts) = diroxide_meta::image::read_parts(&mut bounded) else {
        return PhotoFact::default();
    };
    let facts = parts
        .exif
        .as_deref()
        .and_then(diroxide_meta::exif::parse)
        .unwrap_or_default();
    PhotoFact {
        taken: facts.taken.as_deref().and_then(Moment::parse),
        camera: camera_words(facts.make.as_deref(), facts.model.as_deref()),
    }
}

/// What a picture is, for its Kind cell and the review's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A camera wrote it.
    Photo,
    /// Its name is a screenshot's (the screenshots group).
    Screenshot,
    /// Any other picture.
    Picture,
}

impl Kind {
    /// The word: "photo".
    pub const fn word(self) -> &'static str {
        match self {
            Kind::Photo => "photo",
            Kind::Screenshot => "screenshot",
            Kind::Picture => "picture",
        }
    }
}

/// A picture of a panel, as the helpers see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Its name.
    pub name: String,
    /// When it was taken, else its modified time in local time.
    pub at: Moment,
    /// Whether `at` is the date taken.
    pub taken: bool,
    /// The camera.
    pub camera: Option<String>,
    /// What it is.
    pub kind: Kind,
}

/// The first picture of the next day after `from`, or of the day before
/// it: its index among `pictures` (in their order), the day, and how many
/// pictures that day has.
pub fn step_day(pictures: &[Picture], from: Day, forward: bool) -> Option<(usize, Day, usize)> {
    let target = if forward {
        pictures
            .iter()
            .map(|p| p.at.day())
            .filter(|d| *d > from)
            .min()
    } else {
        pictures
            .iter()
            .map(|p| p.at.day())
            .filter(|d| *d < from)
            .max()
    }?;
    let on: Vec<usize> = pictures
        .iter()
        .enumerate()
        .filter(|(_, p)| p.at.day() == target)
        .map(|(i, _)| i)
        .collect();
    on.first().map(|first| (*first, target, on.len()))
}

/// "Next day: October 12, 9 pictures, Monday" (32 cells to the count),
/// the date from the pictures and the weekday from the calendar;
/// "Previous day: ...".
pub fn day_line(forward: bool, day: Day, pictures: usize, this_year: i32) -> String {
    let which = if forward { "Next day" } else { "Previous day" };
    format!(
        "{which}: {}, {}, {}",
        day.words(this_year),
        count(pictures as u64, "picture", "pictures"),
        day.weekday()
    )
}

/// The bursts among `pictures`: runs of two or more pictures with a date
/// taken from the same camera, each within `gap` seconds of the one
/// before, as indexes into `pictures`, in time order.
pub fn bursts(pictures: &[Picture], gap: u64) -> Vec<Vec<usize>> {
    let mut by_camera: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, picture) in pictures.iter().enumerate() {
        if let (true, Some(camera)) = (picture.taken, picture.camera.as_deref()) {
            by_camera.entry(camera).or_default().push(i);
        }
    }
    let gap = i64::try_from(gap).unwrap_or(i64::MAX);
    let mut found = Vec::new();
    for mut list in by_camera.into_values() {
        list.sort_by_key(|i| (pictures[*i].at, pictures[*i].name.clone()));
        let mut run: Vec<usize> = Vec::new();
        for i in list {
            let joins = run
                .last()
                .is_some_and(|last| pictures[i].at.seconds() - pictures[*last].at.seconds() <= gap);
            if !joins {
                if run.len() > 1 {
                    found.push(std::mem::take(&mut run));
                }
                run.clear();
            }
            run.push(i);
        }
        if run.len() > 1 {
            found.push(run);
        }
    }
    found.sort_by_key(|run| pictures[run[0]].at);
    found
}

/// "Burst, 7 pictures in 2 seconds" (30 cells), the time from the first
/// to the last.
pub fn burst_line(pictures: &[Picture], burst: &[usize]) -> String {
    let first = burst.first().map_or(0, |i| pictures[*i].at.seconds());
    let last = burst.last().map_or(0, |i| pictures[*i].at.seconds());
    let seconds = u64::try_from(last - first).unwrap_or(0);
    format!(
        "Burst, {} in {}",
        count(burst.len() as u64, "picture", "pictures"),
        count(seconds, "second", "seconds")
    )
}

/// The pictures by month folder, `2026-10`, each list in `pictures`'
/// order.
pub fn months(pictures: &[Picture]) -> BTreeMap<String, Vec<usize>> {
    let mut months: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, picture) in pictures.iter().enumerate() {
        months.entry(picture.at.month_folder()).or_default().push(i);
    }
    months
}

/// "Move 214 pictures into 9 month folders" (38 cells).
pub fn month_question(pictures: usize, folders: usize) -> String {
    format!(
        "Move {} into {}",
        count(pictures as u64, "picture", "pictures"),
        count(folders as u64, "month folder", "month folders")
    )
}

/// "2026-10: 40 pictures, a new folder" or "2026-09: 12 pictures".
pub fn month_row(month: &str, pictures: usize, new: bool) -> String {
    let what = count(pictures as u64, "picture", "pictures");
    if new {
        format!("{month}: {what}, a new folder")
    } else {
        format!("{month}: {what}")
    }
}

/// "Date taken; 12 use modified time" (32 cells): the note after a sort
/// by date taken, or a month plan, when some pictures have no date.
pub fn fallback_note(without: usize) -> String {
    // The verb agrees with the count: "1 uses", "12 use".
    format!(
        "Date taken; {} modified time",
        count(without as u64, "uses", "use")
    )
}

/// "Review: 3 of 14, IMG_2041.jpg, photo" (36 cells).
pub fn review_line(at: usize, of: usize, name: &str, kind: Kind) -> String {
    format!("Review: {at} of {of}, {name}, {}", kind.word())
}

/// What the app asks the runtime for the photo helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhotoAsk {
    /// Read the date taken and the camera of a panel's pictures.
    Facts {
        /// The panel.
        side: Side,
        /// The panel's entries they belong to.
        listing: u64,
        /// The pictures' places.
        places: Vec<Place>,
    },
}

/// What the runtime found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhotoNote {
    /// Each picture's date taken and camera, by name.
    Facts {
        /// The panel.
        side: Side,
        /// The panel's entries they belong to.
        listing: u64,
        /// Each picture's name and facts.
        facts: Vec<(String, PhotoFact)>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use diroxide_core::format::within_cells;

    fn moment(text: &str) -> Moment {
        Moment::parse(text).unwrap()
    }

    fn picture(name: &str, at: &str, camera: Option<&str>) -> Picture {
        Picture {
            name: name.into(),
            at: moment(at),
            taken: camera.is_some(),
            camera: camera.map(str::to_owned),
            kind: if camera.is_some() {
                Kind::Photo
            } else {
                Kind::Picture
            },
        }
    }

    #[test]
    fn a_moment_is_read_as_the_camera_wrote_it() {
        let at = moment("2026:10:12 14:03:07");
        assert_eq!(at.words(), "2026-10-12 14:03");
        assert_eq!(at.second, 7);
        assert_eq!(moment("2026-10-12 14:03").words(), "2026-10-12 14:03");
        assert_eq!(moment("2026-10-12").words(), "2026-10-12 00:00");
        assert_eq!(at.month_folder(), "2026-10");
        for bad in ["0000:00:00 00:00:00", "2026:02:30 10:00:00", "now", ""] {
            assert_eq!(Moment::parse(bad), None, "{bad}");
        }
        let local = Moment::of_time(
            Timestamp::from_civil(2026, 10, 16, 23, 30, 0, UtcOffset::UTC).unwrap(),
            UtcOffset::from_hours(2),
        );
        assert_eq!(local.words(), "2026-10-17 01:30");
    }

    #[test]
    fn next_day_goes_to_the_first_picture_of_the_next_date_with_its_weekday() {
        let pictures = vec![
            picture("a.jpg", "2026:10:11 09:00:00", Some("Pixel 8")),
            picture("b.jpg", "2026:10:12 09:00:00", Some("Pixel 8")),
            picture("c.jpg", "2026:10:12 10:00:00", Some("Pixel 8")),
            picture("d.jpg", "2025:12:31 10:00:00", None),
        ];
        let from = moment("2026:10:11 09:00:00").day();
        let (first, day, n) = step_day(&pictures, from, true).unwrap();
        assert_eq!((first, n), (1, 2));
        // October 12, 2026 is a Monday: computed from the calendar.
        let line = day_line(true, day, n, 2026);
        assert_eq!(line, "Next day: October 12, 2 pictures, Monday");
        assert!(within_cells(&line, "Next day: October 12, 2 pictures"));
        let (first, day, n) = step_day(&pictures, from, false).unwrap();
        assert_eq!((first, n), (3, 1));
        assert_eq!(
            day_line(false, day, n, 2026),
            "Previous day: December 31, 2025, 1 picture, Wednesday"
        );
        assert_eq!(
            step_day(&pictures, moment("2026:10:12 00:00:00").day(), true),
            None
        );
        let nine = day_line(
            true,
            Day {
                year: 2026,
                month: 10,
                day: 12,
            },
            9,
            2026,
        );
        assert!(nine.starts_with("Next day: October 12, 9 pictures"));
    }

    #[test]
    fn a_burst_is_one_cameras_pictures_within_the_gap() {
        let mut pictures = Vec::new();
        for s in 0..7 {
            pictures.push(picture(
                &format!("burst{s}.jpg"),
                &format!("2026:10:12 14:03:0{}", s / 3),
                Some("Pixel 8"),
            ));
        }
        pictures.push(picture("later.jpg", "2026:10:12 14:03:09", Some("Pixel 8")));
        pictures.push(picture("other.jpg", "2026:10:12 14:03:01", Some("EOS R5")));
        pictures.push(picture("nodate.png", "2026:10:12 14:03:01", None));
        let found = bursts(&pictures, 2);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].len(), 7);
        let line = burst_line(&pictures, &found[0]);
        assert_eq!(line, "Burst, 7 pictures in 2 seconds");
        assert!(within_cells(&line, &line));
        // A wider gap takes the later picture in.
        assert_eq!(bursts(&pictures, 7)[0].len(), 8);
    }

    /// Bursts read the date taken to the second: [`read_fact`] hands the
    /// Exif text as written to [`Moment::parse`], which keeps its seconds
    /// (the same bytes `image.taken_second` words, so no second header
    /// read), and a date with no seconds falls back to the minute.
    #[test]
    fn bursts_tell_seconds_apart_within_one_minute() {
        let gap = diroxide_config::Settings::default()
            .photos
            .burst_gap_seconds;
        assert_eq!(gap, 2);
        // Three frames two seconds apart in one minute: one burst.
        let three = [
            picture("a.jpg", "2026:10:12 14:03:10", Some("Pixel 8")),
            picture("b.jpg", "2026:10:12 14:03:12", Some("Pixel 8")),
            picture("c.jpg", "2026:10:12 14:03:14", Some("Pixel 8")),
        ];
        let found = bursts(&three, gap);
        assert_eq!(found, [vec![0, 1, 2]]);
        assert_eq!(
            burst_line(&three, &found[0]),
            "Burst, 3 pictures in 4 seconds"
        );
        // Two frames 50 seconds apart in the same minute: no burst.
        let two = [
            picture("d.jpg", "2026:10:12 14:03:05", Some("Pixel 8")),
            picture("e.jpg", "2026:10:12 14:03:55", Some("Pixel 8")),
        ];
        assert!(bursts(&two, gap).is_empty());
        // A date written to the minute counts as its first second.
        let minute = [
            picture("f.jpg", "2026-10-12 14:03", Some("Pixel 8")),
            picture("g.jpg", "2026:10:12 14:03:01", Some("Pixel 8")),
        ];
        assert_eq!(bursts(&minute, gap), [vec![0, 1]]);
    }

    #[test]
    fn the_month_plan_and_the_review_say_their_meaning_first() {
        let pictures = vec![
            picture("a.jpg", "2026:10:11 09:00:00", Some("Pixel 8")),
            picture("b.jpg", "2026:09:12 09:00:00", Some("Pixel 8")),
            picture("c.jpg", "2026:10:30 10:00:00", None),
        ];
        let months = months(&pictures);
        assert_eq!(months.keys().collect::<Vec<_>>(), ["2026-09", "2026-10"]);
        assert_eq!(months["2026-10"], [0, 2]);
        let question = month_question(214, 9);
        assert_eq!(question, "Move 214 pictures into 9 month folders");
        assert!(within_cells(&question, &question));
        assert_eq!(
            month_row("2026-10", 40, true),
            "2026-10: 40 pictures, a new folder"
        );
        let review = review_line(3, 14, "IMG_2041.jpg", Kind::Photo);
        assert_eq!(review, "Review: 3 of 14, IMG_2041.jpg, photo");
        assert!(within_cells(&review, &review));
        let note = fallback_note(12);
        assert_eq!(note, "Date taken; 12 use modified time");
        assert!(within_cells(&note, &note));
        assert_eq!(fallback_note(1), "Date taken; 1 uses modified time");
        assert!(within_cells(NO_DATE_LINE, NO_DATE_LINE));
        assert_eq!(
            camera_words(Some("Google"), Some("Pixel 8")).as_deref(),
            Some("Google Pixel 8")
        );
        assert_eq!(
            camera_words(Some("Canon"), Some("Canon EOS R5")).as_deref(),
            Some("Canon EOS R5")
        );
        assert_eq!(camera_words(Some(" "), None), None);
    }

    /// Every other line the helpers say, its meaning in its first 40
    /// cells.
    #[test]
    fn the_helpers_other_lines_say_their_meaning_first() {
        for (line, meaning) in [
            (NO_PICTURES.to_owned(), NO_PICTURES),
            (NO_LATER_DAY.to_owned(), NO_LATER_DAY),
            (NO_EARLIER_DAY.to_owned(), NO_EARLIER_DAY),
            (NO_SCREENSHOTS.to_owned(), NO_SCREENSHOTS),
            (ALL_DATED.to_owned(), ALL_DATED),
            (FIRST_PICTURE.to_owned(), FIRST_PICTURE),
            (NOTHING_MOVED.to_owned(), NOTHING_MOVED),
            (reading_line(14), "Reading 14 pictures"),
            (no_bursts_line(2), "No bursts here"),
            (month_taken_line("2026-10"), "Cannot make 2026-10"),
            (
                selected_line("3 bursts, 17 pictures", 17),
                "Selected 3 bursts, 17 pictures",
            ),
            (
                selected_line("12 pictures with no date taken", 12),
                "Selected 12 pictures with no date taken",
            ),
            (selected_line("4 screenshots", 4), "Selected 4 screenshots"),
        ] {
            assert!(within_cells(&line, meaning), "{line}");
        }
        assert_eq!(reading_line(1), "Reading 1 picture");
        assert_eq!(
            no_bursts_line(1),
            "No bursts here: no pictures from one camera within 1 second"
        );
    }
}
