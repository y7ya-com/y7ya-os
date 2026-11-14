use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::recur::Rule;

/// Colors calendars can have, as in macOS Calendar.
pub const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "brown"];

/// The color's sRGB value.
pub fn color_rgb(name: &str) -> u32 {
    match name {
        "red" => 0xff3b30,
        "orange" => 0xff9500,
        "yellow" => 0xffcc00,
        "green" => 0x34c759,
        "purple" => 0xaf52de,
        "brown" => 0xa2845e,
        _ => 0x007aff,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    pub id: u64,
    pub name: String,
    /// One of [`COLORS`].
    pub color: String,
}

/// The calendars every device starts with. Fixed ids, so two devices that
/// start out separately end up with one "Home" and one "Work".
pub fn default_calendars() -> Vec<Calendar> {
    vec![
        Calendar { id: 1, name: "Home".into(), color: "blue".into() },
        Calendar { id: 2, name: "Work".into(), color: "red".into() },
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: u64,
    pub calendar: u64,
    pub title: String,
    /// Timed events: the start instant (Unix seconds). All-day events: the
    /// first day, as midnight UTC of that date (see [`Event::start_date`]).
    pub start: i64,
    /// Exclusive: the end instant, or midnight UTC after the last day.
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    pub notes: String,
    /// An iCalendar `RRULE` value, e.g. `FREQ=WEEKLY;INTERVAL=2`, or empty.
    pub rrule: String,
    pub created: i64,
    pub modified: i64,
}

impl Event {
    pub fn duration(&self) -> i64 {
        (self.end - self.start).max(0)
    }

    pub fn rule(&self) -> Option<Rule> {
        Rule::parse(&self.rrule)
    }

    /// First day of an all-day event.
    pub fn start_date(&self) -> NaiveDate {
        date_of(self.start)
    }

    /// Last day (inclusive) of an all-day event.
    pub fn last_date(&self) -> NaiveDate {
        date_of((self.end - 1).max(self.start))
    }
}

/// Midnight UTC of `date`: how all-day events store dates.
pub fn date_key(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
}

/// The date an all-day timestamp stands for.
pub fn date_of(key: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp(key, 0).map_or(NaiveDate::MIN, |t| t.date_naive())
}

/// Local wall-clock time of an instant.
pub fn local(ts: i64) -> NaiveDateTime {
    Local.timestamp_opt(ts, 0).single().map_or_else(|| Utc::now().naive_local(), |t| t.naive_local())
}

/// The instant for a local wall-clock time (the earlier one if ambiguous,
/// skipping forward over a DST gap).
pub fn instant(t: NaiveDateTime) -> i64 {
    for shift in 0..4 {
        let t = t + chrono::Duration::minutes(30 * shift);
        if let Some(dt) = Local.from_local_datetime(&t).earliest() {
            return dt.timestamp();
        }
    }
    t.and_utc().timestamp()
}

/// Local midnight starting `date`.
pub fn day_start(date: NaiveDate) -> i64 {
    instant(date.and_hms_opt(0, 0, 0).unwrap())
}
