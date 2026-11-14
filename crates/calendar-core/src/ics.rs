//! Importing `.ics` (iCalendar) files: VEVENTs with their title, times,
//! location, notes and basic repeat rule.
//!
//! Times in UTC (`...Z`) are exact. Times with a `TZID` are read as local
//! time: without a time zone database that's the best guess, and it's right
//! for files exported from this computer's zone.

use chrono::{Duration, NaiveDate, NaiveDateTime};

use crate::model::{Event, date_key, instant};

/// A parsed `.ics` file.
#[derive(Debug, Default)]
pub struct Import {
    /// `X-WR-CALNAME`, if the file names its calendar.
    pub name: Option<String>,
    /// Events with `id` 0 and `calendar` 0: the caller assigns both.
    pub events: Vec<Event>,
}

pub fn parse(text: &str) -> Import {
    let mut import = Import::default();
    let mut current: Option<Vec<(String, String, String)>> = None;
    for line in unfold(text) {
        let Some((head, value)) = line.split_once(':') else { continue };
        let (name, params) = head.split_once(';').unwrap_or((head, ""));
        let name = name.to_ascii_uppercase();
        match (name.as_str(), value.trim()) {
            ("BEGIN", "VEVENT") => current = Some(Vec::new()),
            ("END", "VEVENT") => {
                if let Some(ev) = current.take().and_then(|props| event(&props)) {
                    import.events.push(ev);
                }
            }
            ("X-WR-CALNAME", v) if current.is_none() => import.name = Some(unescape(v)),
            _ => {
                if let Some(props) = current.as_mut() {
                    props.push((name, params.to_ascii_uppercase(), value.to_string()));
                }
            }
        }
    }
    import
}

/// Joins folded lines (continuations start with a space or tab).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

/// A date or date-time value: `(timestamp, is_date)` in [`Event`]'s encoding.
fn when(params: &str, v: &str) -> Option<(i64, bool)> {
    let v = v.trim();
    if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") || v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some((date_key(d), true));
    }
    let t = NaiveDateTime::parse_from_str(v.get(..15)?, "%Y%m%dT%H%M%S").ok()?;
    Some((if v.ends_with('Z') { t.and_utc().timestamp() } else { instant(t) }, false))
}

/// `PT1H30M`, `P1D`, `P2W`, ...
fn duration(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let (mut total, mut n) = (0i64, 0i64);
    for c in v.chars() {
        match c {
            '0'..='9' => n = n * 10 + c.to_digit(10)? as i64,
            'W' => (total, n) = (total + n * 7 * 86400, 0),
            'D' => (total, n) = (total + n * 86400, 0),
            'H' => (total, n) = (total + n * 3600, 0),
            'M' => (total, n) = (total + n * 60, 0),
            'S' => (total, n) = (total + n, 0),
            'T' => {}
            _ => return None,
        }
    }
    Some(sign * total)
}

fn event(props: &[(String, String, String)]) -> Option<Event> {
    let get = |name: &str| props.iter().find(|(n, ..)| n == name);
    let (_, params, v) = get("DTSTART")?;
    let (start, all_day) = when(params, v)?;
    let end = match (get("DTEND"), get("DURATION")) {
        (Some((_, params, v)), _) => when(params, v).map(|(t, _)| t),
        (None, Some((_, _, v))) => duration(v).map(|d| start + d),
        // A date with no end lasts the day; a time with no end is an instant.
        (None, None) => Some(if all_day { start + Duration::days(1).num_seconds() } else { start }),
    }?;
    let text = |name: &str| get(name).map(|(_, _, v)| unescape(v)).unwrap_or_default();
    let t = y7_sync::now();
    Some(Event {
        id: 0,
        calendar: 0,
        title: text("SUMMARY"),
        start,
        end: end.max(start),
        all_day,
        location: text("LOCATION"),
        notes: text("DESCRIPTION"),
        rrule: get("RRULE").map(|(_, _, v)| v.trim().to_string()).unwrap_or_default(),
        created: t,
        modified: t,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{date_of, local};

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Holidays\r\n\
BEGIN:VEVENT\r\nUID:1\r\nDTSTART;VALUE=DATE:20261225\r\nDTEND;VALUE=DATE:20261226\r\nSUMMARY:Christmas\r\n\
RRULE:FREQ=YEARLY\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:2\r\nDTSTART:20261114T150000Z\r\nDURATION:PT1H30M\r\nSUMMARY:Coffee\\, then a walk\r\n\
LOCATION:Main St\r\nDESCRIPTION:Bring the\\nbook and a very long\r\n  line that was folded\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART;TZID=Europe/Berlin:20261116T090000\r\nDTEND;TZID=Europe/Berlin:20261116T100000\r\nSUMMARY:Standup\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_events() {
        let import = parse(ICS);
        assert_eq!(import.name.as_deref(), Some("Holidays"));
        assert_eq!(import.events.len(), 3);
        let xmas = &import.events[0];
        assert!(xmas.all_day);
        assert_eq!(date_of(xmas.start), NaiveDate::from_ymd_opt(2026, 12, 25).unwrap());
        assert_eq!(xmas.end - xmas.start, 86400);
        assert_eq!(xmas.rrule, "FREQ=YEARLY");
        let coffee = &import.events[1];
        assert_eq!(coffee.title, "Coffee, then a walk");
        assert_eq!(coffee.start, 1794668400);
        assert_eq!(coffee.duration(), 5400);
        assert_eq!(coffee.location, "Main St");
        assert_eq!(coffee.notes, "Bring the\nbook and a very long line that was folded");
        let standup = &import.events[2];
        assert_eq!(local(standup.start).format("%H:%M").to_string(), "09:00");
        assert_eq!(standup.duration(), 3600);
    }

    #[test]
    fn durations() {
        assert_eq!(duration("PT15M"), Some(900));
        assert_eq!(duration("P1DT2H"), Some(93600));
        assert_eq!(duration("P2W"), Some(1209600));
        assert_eq!(duration("-PT5M"), Some(-300));
    }
}
