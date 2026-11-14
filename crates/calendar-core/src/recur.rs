//! Repeating events: the basics of iCalendar's RRULE (FREQ, INTERVAL, UNTIL,
//! COUNT), expanded in local time so a 9:00 meeting stays at 9:00 across
//! daylight-saving changes.

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime};

use crate::model::{Event, date_key, date_of, day_start, instant, local};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    /// Last day an occurrence may start on (local date).
    pub until: Option<NaiveDate>,
    pub count: Option<u32>,
}

impl Rule {
    pub fn new(freq: Freq) -> Self {
        Rule { freq, interval: 1, until: None, count: None }
    }

    /// Parses an RRULE value (with or without the `RRULE:` prefix). Parts
    /// beyond the basics (BYDAY and friends) are ignored.
    pub fn parse(s: &str) -> Option<Rule> {
        let s = s.trim();
        let s = s.strip_prefix("RRULE:").unwrap_or(s);
        let mut rule = None;
        let (mut interval, mut until, mut count) = (1, None, None);
        for part in s.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            match k.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    rule = Some(match v.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => interval = v.parse().ok().filter(|&n| n > 0).unwrap_or(1),
                "COUNT" => count = v.parse().ok(),
                "UNTIL" => until = parse_until(v),
                _ => {}
            }
        }
        Some(Rule { freq: rule?, interval, until, count })
    }

    pub fn to_rrule(&self) -> String {
        let freq = match self.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        };
        let mut s = format!("FREQ={freq}");
        if self.interval > 1 {
            s += &format!(";INTERVAL={}", self.interval);
        }
        if let Some(c) = self.count {
            s += &format!(";COUNT={c}");
        }
        if let Some(u) = self.until {
            s += &format!(";UNTIL={}", u.format("%Y%m%d"));
        }
        s
    }

    /// "Every day", "Every 2 weeks", ...
    pub fn describe(&self) -> String {
        let unit = match self.freq {
            Freq::Daily => "day",
            Freq::Weekly => "week",
            Freq::Monthly => "month",
            Freq::Yearly => "year",
        };
        if self.interval == 1 { format!("Every {unit}") } else { format!("Every {} {unit}s", self.interval) }
    }

    /// The `k`th repetition of `base`, or `None` when it doesn't exist (the
    /// 31st in a short month, Feb 29 in other years: skipped, as RFC 5545 says).
    fn nth(&self, base: NaiveDateTime, k: u32) -> Option<NaiveDateTime> {
        let n = k.checked_mul(self.interval)?;
        match self.freq {
            Freq::Daily => base.checked_add_signed(Duration::days(n as i64)),
            Freq::Weekly => base.checked_add_signed(Duration::weeks(n as i64)),
            Freq::Monthly | Freq::Yearly => {
                let months = if self.freq == Freq::Yearly { n.checked_mul(12)? } else { n };
                let t = base.checked_add_months(Months::new(months))?;
                (t.day() == base.day()).then_some(t)
            }
        }
    }
}

fn parse_until(v: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()?;
    // A UTC time can fall on another local day.
    if v.ends_with('Z') && v.len() >= 15 {
        let t = NaiveDateTime::parse_from_str(&v[..15], "%Y%m%dT%H%M%S").ok()?;
        return Some(local(t.and_utc().timestamp()).date());
    }
    Some(date)
}

/// One time an event happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub event: u64,
    /// Same encoding as [`Event::start`]/[`Event::end`].
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// 0 for the event itself, then 1, 2, ... for repetitions.
    pub index: u32,
}

/// Occurrences of `ev` touching the local days `first..=last`.
pub fn occurrences(ev: &Event, first: NaiveDate, last: NaiveDate) -> Vec<Occurrence> {
    let (from, to) = if ev.all_day {
        (date_key(first), date_key(last + Duration::days(1)))
    } else {
        (day_start(first), day_start(last + Duration::days(1)))
    };
    let dur = ev.duration();
    let occ = |start: i64, index| Occurrence { event: ev.id, start, end: start + dur, all_day: ev.all_day, index };
    let touches = |start: i64| start < to && (start + dur > from || (dur == 0 && start >= from));
    let Some(rule) = ev.rule() else {
        return if touches(ev.start) { vec![occ(ev.start, 0)] } else { vec![] };
    };
    let base = if ev.all_day { date_of(ev.start).and_hms_opt(0, 0, 0).unwrap() } else { local(ev.start) };
    let mut out = Vec::new();
    let mut index = 0;
    for k in 0..100_000 {
        if rule.count.is_some_and(|c| index >= c) {
            break;
        }
        let Some(t) = rule.nth(base, k) else {
            if matches!(rule.freq, Freq::Monthly | Freq::Yearly) {
                continue;
            }
            break;
        };
        if rule.until.is_some_and(|u| t.date() > u) {
            break;
        }
        let start = if ev.all_day { date_key(t.date()) } else { instant(t) };
        if start >= to {
            break;
        }
        if touches(start) {
            out.push(occ(start, index));
        }
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn timed(start: NaiveDateTime, minutes: i64, rrule: &str) -> Event {
        let start = instant(start);
        Event {
            id: 1,
            calendar: 1,
            title: "x".into(),
            start,
            end: start + minutes * 60,
            all_day: false,
            location: String::new(),
            notes: String::new(),
            rrule: rrule.into(),
            created: 0,
            modified: 0,
        }
    }

    fn at(date: NaiveDate, h: u32) -> NaiveDateTime {
        date.and_time(NaiveTime::from_hms_opt(h, 0, 0).unwrap())
    }

    #[test]
    fn parses_and_prints_rules() {
        let r = Rule::parse("RRULE:FREQ=WEEKLY;INTERVAL=2;COUNT=5;BYDAY=MO").unwrap();
        assert_eq!(r, Rule { freq: Freq::Weekly, interval: 2, until: None, count: Some(5) });
        assert_eq!(r.to_rrule(), "FREQ=WEEKLY;INTERVAL=2;COUNT=5");
        assert_eq!(Rule::parse("FREQ=DAILY;UNTIL=20261231").unwrap().until, Some(d(2026, 12, 31)));
        assert!(Rule::parse("FREQ=HOURLY").is_none());
        assert!(Rule::parse("").is_none());
        assert_eq!(r.describe(), "Every 2 weeks");
    }

    #[test]
    fn single_events() {
        let ev = timed(at(d(2026, 11, 14), 9), 60, "");
        assert_eq!(occurrences(&ev, d(2026, 11, 14), d(2026, 11, 14)).len(), 1);
        assert!(occurrences(&ev, d(2026, 11, 15), d(2026, 11, 20)).is_empty());
        // Spanning midnight shows on both days.
        let late = timed(at(d(2026, 11, 14), 23), 120, "");
        assert_eq!(occurrences(&late, d(2026, 11, 15), d(2026, 11, 15)).len(), 1);
    }

    #[test]
    fn daily_with_count_and_interval() {
        let ev = timed(at(d(2026, 11, 1), 9), 30, "FREQ=DAILY;INTERVAL=2;COUNT=4");
        let days: Vec<NaiveDate> = occurrences(&ev, d(2026, 10, 1), d(2026, 12, 31)).iter().map(|o| local(o.start).date()).collect();
        assert_eq!(days, [d(2026, 11, 1), d(2026, 11, 3), d(2026, 11, 5), d(2026, 11, 7)]);
        // Indices count from the first occurrence even when the range starts later.
        let later = occurrences(&ev, d(2026, 11, 5), d(2026, 11, 30));
        assert_eq!(later.iter().map(|o| o.index).collect::<Vec<_>>(), [2, 3]);
    }

    #[test]
    fn weekly_until_keeps_wall_clock_time() {
        // Crosses the end of DST in most zones that have one.
        let ev = timed(at(d(2026, 10, 19), 9), 60, "FREQ=WEEKLY;UNTIL=20261116");
        let occ = occurrences(&ev, d(2026, 10, 1), d(2026, 12, 31));
        assert_eq!(occ.len(), 5);
        assert!(occ.iter().all(|o| local(o.start).time() == NaiveTime::from_hms_opt(9, 0, 0).unwrap()));
    }

    #[test]
    fn monthly_skips_missing_days_and_yearly_leap_days() {
        let ev = timed(at(d(2026, 1, 31), 12), 60, "FREQ=MONTHLY;COUNT=4");
        let months: Vec<u32> = occurrences(&ev, d(2026, 1, 1), d(2026, 12, 31)).iter().map(|o| local(o.start).month()).collect();
        assert_eq!(months, [1, 3, 5, 7]);
        let mut leap = timed(at(d(2024, 2, 29), 0), 0, "FREQ=YEARLY");
        leap.all_day = true;
        leap.start = date_key(d(2024, 2, 29));
        leap.end = date_key(d(2024, 3, 1));
        let years: Vec<i32> = occurrences(&leap, d(2024, 1, 1), d(2032, 12, 31)).iter().map(|o| date_of(o.start).year()).collect();
        assert_eq!(years, [2024, 2028, 2032]);
    }
}
