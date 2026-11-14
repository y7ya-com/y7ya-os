//! Calendar arithmetic the views are built on.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};

/// The day weeks start on, from the locale (`LC_TIME`'s territory): Monday in
/// most of the world, Sunday in the Americas and a few other places.
pub fn first_weekday() -> Weekday {
    let locale = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let territory = locale.split(['.', '@']).next().and_then(|l| l.split('_').nth(1)).unwrap_or("US");
    match territory {
        "US" | "CA" | "MX" | "BR" | "JP" | "KR" | "TW" | "HK" | "IL" | "PH" | "ZA" | "AU" | "IN" | "SA" => Weekday::Sun,
        _ => Weekday::Mon,
    }
}

/// The first day of the week containing `d`.
pub fn week_start(d: NaiveDate, first: Weekday) -> NaiveDate {
    let back = (d.weekday().num_days_from_monday() + 7 - first.num_days_from_monday()) % 7;
    d - Duration::days(back as i64)
}

/// The seven days of the week containing `d`.
pub fn week_days(d: NaiveDate, first: Weekday) -> [NaiveDate; 7] {
    let start = week_start(d, first);
    std::array::from_fn(|i| start + Duration::days(i as i64))
}

pub fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap()
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (add_months(first, 1) - first).num_days() as u32
}

/// `d` moved by `n` months, keeping the day where possible (Jan 31 + 1 month
/// is the last day of February).
pub fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    if n >= 0 { d.checked_add_months(Months::new(n as u32)) } else { d.checked_sub_months(Months::new(n.unsigned_abs())) }
        .unwrap_or(d)
}

/// The days a month view shows: whole weeks covering the month. Always six
/// weeks (42 days) when `six_weeks`, like the year view's mini months;
/// otherwise just the weeks the month touches (4 to 6), like Calendar's month view.
pub fn month_grid(year: i32, month: u32, first: Weekday, six_weeks: bool) -> Vec<NaiveDate> {
    let start = week_start(NaiveDate::from_ymd_opt(year, month, 1).unwrap(), first);
    let last = NaiveDate::from_ymd_opt(year, month, days_in_month(year, month)).unwrap();
    let weeks = if six_weeks { 6 } else { ((last - start).num_days() / 7 + 1) as usize };
    (0..weeks * 7).map(|i| start + Duration::days(i as i64)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn weeks() {
        // Saturday, November 14 2026.
        assert_eq!(week_start(d(2026, 11, 14), Weekday::Sun), d(2026, 11, 8));
        assert_eq!(week_start(d(2026, 11, 14), Weekday::Mon), d(2026, 11, 9));
        assert_eq!(week_start(d(2026, 11, 8), Weekday::Sun), d(2026, 11, 8));
        assert_eq!(week_days(d(2026, 12, 31), Weekday::Mon)[6], d(2027, 1, 3));
    }

    #[test]
    fn months() {
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2026, 12), 31);
        assert_eq!(add_months(d(2026, 1, 31), 1), d(2026, 2, 28));
        assert_eq!(add_months(d(2026, 3, 15), -3), d(2025, 12, 15));
        let nov = month_grid(2026, 11, Weekday::Sun, false);
        assert_eq!((nov[0], nov.len()), (d(2026, 11, 1), 35));
        let feb = month_grid(2026, 2, Weekday::Sun, false);
        assert_eq!(feb.len(), 28, "Feb 2026 starts on a Sunday and fills four weeks exactly");
        assert_eq!(month_grid(2026, 2, Weekday::Sun, true).len(), 42);
        let may = month_grid(2026, 5, Weekday::Sun, false);
        assert_eq!((may[0], may.len()), (d(2026, 4, 26), 42));
    }
}
