//! Occurrences of the standard calendar's repeating events (`ext.rrule`, schemas/standard-tables.md): the Rust side of
//! buckyos-websdk `src/aiws_rrule.ts`; keep the two in step. The subset itself is checked by the core
//! (`standard::check_rrule`).

use aiworkspace_core::standard::check_rrule;
use aiworkspace_core::value::{civil_from_days, days_from_civil, parse_date, parse_datetime_ms};
use chrono::{Datelike, NaiveDate, TimeZone, Timelike};
use chrono_tz::Tz;
use std::collections::BTreeSet;

const DAY_MS: i64 = 86_400_000;
const DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
const MAX_PERIODS: usize = 200_000;
const MAX_OCCURRENCES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, Copy)]
enum Until {
    Day(i64),
    At(i64),
}

#[derive(Debug, Clone)]
pub struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<usize>,
    until: Option<Until>,
    /// `(weekday, ordinal)`, 0 = Monday.
    by_day: Vec<(i64, Option<i64>)>,
    by_month_day: Vec<i64>,
    by_month: Vec<i64>,
    set_pos: Option<i64>,
    wkst: i64,
}

impl Rule {
    /// `None` for a rule outside the subset: such a series shows its first occurrence only.
    pub fn parse(text: &str) -> Option<Rule> {
        check_rrule(text).ok()?;
        let mut rule = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: vec![], by_month_day: vec![], by_month: vec![], set_pos: None, wkst: 0 };
        let weekday = |s: &str| DAYS.iter().position(|d| *d == s).map(|i| i as i64);
        for part in text.split(';') {
            let (name, value) = part.split_once('=')?;
            match name {
                "FREQ" => {
                    rule.freq = match value {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        _ => Freq::Yearly,
                    }
                }
                "INTERVAL" => rule.interval = value.parse().ok()?,
                "COUNT" => rule.count = Some(value.parse().ok()?),
                "UNTIL" => {
                    let date = format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..8]);
                    rule.until = Some(match value.len() {
                        8 => Until::Day(day_of(&date)?),
                        _ => Until::At(parse_datetime_ms(&format!("{date}T{}:{}:{}Z", &value[9..11], &value[11..13], &value[13..15]))?),
                    });
                }
                "BYDAY" => {
                    for item in value.split(',') {
                        let (ord, day) = item.split_at(item.len() - 2);
                        rule.by_day.push((weekday(day)?, if ord.is_empty() { None } else { Some(ord.parse().ok()?) }));
                    }
                }
                "BYMONTHDAY" => rule.by_month_day = value.split(',').map(|v| v.parse().ok()).collect::<Option<_>>()?,
                "BYMONTH" => rule.by_month = value.split(',').map(|v| v.parse().ok()).collect::<Option<_>>()?,
                "BYSETPOS" => rule.set_pos = Some(value.parse().ok()?),
                "WKST" => rule.wkst = weekday(value)?,
                _ => return None,
            }
        }
        Some(rule)
    }

    fn week0(&self, start_day: i64) -> i64 {
        start_day - (weekday_of(start_day) - self.wkst).rem_euclid(7)
    }

    /// The first day of period `p`.
    fn period_start(&self, p: i64, start_day: i64) -> i64 {
        let (y, m, _) = civil_from_days(start_day);
        match self.freq {
            Freq::Daily => start_day + p * self.interval,
            Freq::Weekly => self.week0(start_day) + p * self.interval * 7,
            Freq::Monthly => {
                let index = y * 12 + (m - 1) + p * self.interval;
                days_from_civil(index.div_euclid(12), index.rem_euclid(12) + 1, 1)
            }
            Freq::Yearly => days_from_civil(y + p * self.interval, 1, 1),
        }
    }

    /// A period before which nothing can fall on or after `day`.
    fn first_period(&self, start_day: i64, day: i64) -> i64 {
        let ((sy, sm, _), (y, m, _)) = (civil_from_days(start_day), civil_from_days(day));
        let p = match self.freq {
            Freq::Daily => (day - start_day).div_euclid(self.interval),
            Freq::Weekly => (day - self.week0(start_day)).div_euclid(7 * self.interval),
            Freq::Monthly => (y * 12 + m - (sy * 12 + sm)).div_euclid(self.interval),
            Freq::Yearly => (y - sy).div_euclid(self.interval),
        };
        (p - 1).max(0)
    }

    /// Candidate days of one period, in order, before the start / COUNT / UNTIL filters.
    fn candidates(&self, p: i64, start_day: i64) -> Vec<i64> {
        let (sy, sm, sd) = civil_from_days(start_day);
        let mut out = BTreeSet::new();
        let by_days = |day0: i64, len: i64, out: &mut BTreeSet<i64>| {
            for &(weekday, ordinal) in &self.by_day {
                let days: Vec<i64> = (day0..day0 + len).filter(|d| weekday_of(*d) == weekday).collect();
                match ordinal {
                    None => out.extend(days),
                    Some(n) => {
                        let k = if n > 0 { n - 1 } else { days.len() as i64 + n };
                        out.extend(usize::try_from(k).ok().and_then(|k| days.get(k)));
                    }
                }
            }
        };
        let month_days = |y: i64, m: i64, out: &mut BTreeSet<i64>| {
            let n = days_in_month(y, m);
            for &d in &self.by_month_day {
                let d = if d > 0 { d } else { n + d + 1 };
                if (1..=n).contains(&d) {
                    out.insert(days_from_civil(y, m, d));
                }
            }
        };
        match self.freq {
            Freq::Daily => {
                let day = start_day + p * self.interval;
                if self.by_day.is_empty() || self.by_day.iter().any(|(w, _)| *w == weekday_of(day)) {
                    out.insert(day);
                }
            }
            Freq::Weekly => {
                let week = self.period_start(p, start_day);
                if self.by_day.is_empty() {
                    out.extend((week..week + 7).filter(|d| weekday_of(*d) == weekday_of(start_day)));
                } else {
                    by_days(week, 7, &mut out);
                }
            }
            Freq::Monthly => {
                let first = self.period_start(p, start_day);
                let (y, m, _) = civil_from_days(first);
                let len = days_in_month(y, m);
                if !self.by_month_day.is_empty() {
                    month_days(y, m, &mut out);
                    if !self.by_day.is_empty() {
                        out.retain(|d| self.by_day.iter().any(|(w, _)| *w == weekday_of(*d)));
                    }
                } else if !self.by_day.is_empty() {
                    by_days(first, len, &mut out);
                } else if sd <= len {
                    out.insert(days_from_civil(y, m, sd));
                }
            }
            Freq::Yearly => {
                let y = sy + p * self.interval;
                if !self.by_day.is_empty() && self.by_month.is_empty() && self.by_month_day.is_empty() {
                    by_days(days_from_civil(y, 1, 1), days_from_civil(y + 1, 1, 1) - days_from_civil(y, 1, 1), &mut out);
                } else {
                    let own = [sm];
                    for &m in if self.by_month.is_empty() { &own[..] } else { &self.by_month[..] } {
                        if !self.by_month_day.is_empty() {
                            month_days(y, m, &mut out);
                        } else if !self.by_day.is_empty() {
                            by_days(days_from_civil(y, m, 1), days_in_month(y, m), &mut out);
                        } else if sd <= days_in_month(y, m) {
                            out.insert(days_from_civil(y, m, sd));
                        }
                    }
                    if !self.by_month_day.is_empty() && !self.by_day.is_empty() {
                        out.retain(|d| self.by_day.iter().any(|(w, _)| *w == weekday_of(*d)));
                    }
                }
            }
        }
        let days: Vec<i64> = out.into_iter().collect();
        match self.set_pos {
            None => days,
            Some(n) => {
                let k = if n > 0 { n - 1 } else { days.len() as i64 + n };
                usize::try_from(k).ok().and_then(|k| days.get(k)).map(|d| vec![*d]).unwrap_or_default()
            }
        }
    }
}

/// Where a series starts: a date (its reminders count from `minute` o'clock on it in `tz`), or an instant with the zone
/// it repeats in.
#[derive(Debug, Clone, Copy)]
pub enum Start {
    Date { day: i64, minute: i64, tz: Tz },
    Time { ms: i64, tz: Tz },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub day: i64,
    /// A timed event's start; `minute` o'clock on the date otherwise.
    pub at_ms: i64,
}

/// The occurrences whose `at_ms` is in `[lo, hi]`. The first is always the start itself, counted by COUNT; excluded
/// ones are left to the caller (they still count).
pub fn expand(rule: &Rule, start: Start, lo: i64, hi: i64) -> Vec<Occurrence> {
    let (start_day, tz) = match start {
        Start::Date { day, tz, .. } => (day, tz),
        Start::Time { ms, tz } => (local_day(ms, tz), tz),
    };
    let at = |day: i64| match start {
        Start::Date { minute, tz, .. } => zoned(day, minute * 60, tz),
        Start::Time { ms, .. } if day == start_day => ms,
        Start::Time { ms, tz } => {
            let local = tz.timestamp_millis_opt(ms).unwrap();
            zoned(day, local.num_seconds_from_midnight() as i64, tz) + ms.rem_euclid(1000)
        }
    };
    let (from_day, to_day) = (local_day(lo, tz) - 1, local_day(hi, tz) + 1);
    let mut out = Vec::new();
    let mut count = 0;
    // false: the series is over
    let mut emit = |day: i64| -> bool {
        if rule.count.is_some_and(|n| count >= n) {
            return false;
        }
        let at_ms = at(day);
        let over = match (rule.until, start) {
            (Some(Until::Day(d)), _) => day > d,
            (Some(Until::At(ms)), Start::Date { .. }) => day * DAY_MS > ms,
            (Some(Until::At(ms)), Start::Time { .. }) => at_ms > ms,
            (None, _) => false,
        };
        if over {
            return false;
        }
        count += 1;
        if (lo..=hi).contains(&at_ms) {
            out.push(Occurrence { day, at_ms });
        }
        out.len() < MAX_OCCURRENCES
    };
    if start_day > to_day || !emit(start_day) {
        return out;
    }
    // nothing counted: the periods wholly before the window are skipped
    let first = if rule.count.is_none() { rule.first_period(start_day, from_day) } else { 0 };
    for p in first..first + MAX_PERIODS as i64 {
        if rule.period_start(p, start_day) > to_day {
            break;
        }
        for day in rule.candidates(p, start_day) {
            if day <= start_day {
                continue;
            }
            if day > to_day || !emit(day) {
                return out;
            }
        }
    }
    out
}

pub fn day_of(date: &str) -> Option<i64> {
    parse_date(date).map(|(y, m, d)| days_from_civil(y, m, d))
}

pub fn date_of(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn local_day(ms: i64, tz: Tz) -> i64 {
    let t = tz.timestamp_millis_opt(ms).unwrap();
    days_from_civil(t.year() as i64, t.month() as i64, t.day() as i64)
}

/// The instant of a wall clock in `tz` (a time skipped by a clock change moves forward by the change; of two, the first).
pub fn zoned(day: i64, second_of_day: i64, tz: Tz) -> i64 {
    let (y, m, d) = civil_from_days(day);
    let Some(local) = NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32).and_then(|d| d.and_hms_opt(0, 0, 0)) else { return day * DAY_MS };
    let local = local + chrono::Duration::seconds(second_of_day);
    match tz.from_local_datetime(&local).earliest() {
        Some(t) => t.timestamp_millis(),
        None => {
            let before = tz.from_local_datetime(&(local - chrono::Duration::days(1))).earliest().map(|t| t.timestamp_millis() + DAY_MS);
            before.unwrap_or(day * DAY_MS + second_of_day * 1000)
        }
    }
}

fn weekday_of(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(rule: &str, start: &str, tz: &str, lo: &str, hi: &str) -> Vec<String> {
        let start = Start::Time { ms: parse_datetime_ms(start).unwrap(), tz: tz.parse().unwrap() };
        expand(&Rule::parse(rule).unwrap(), start, parse_datetime_ms(lo).unwrap(), parse_datetime_ms(hi).unwrap())
            .into_iter()
            .map(|o| aiworkspace_core::value::format_utc_ms(o.at_ms))
            .collect()
    }

    fn dates(rule: &str, start: &str, lo: &str, hi: &str) -> Vec<String> {
        let start = Start::Date { day: day_of(start).unwrap(), minute: 540, tz: Tz::UTC };
        let ms = |d: &str| day_of(d).unwrap() * DAY_MS;
        expand(&Rule::parse(rule).unwrap(), start, ms(lo), ms(hi) + DAY_MS - 1).into_iter().map(|o| date_of(o.day)).collect()
    }

    #[test]
    fn rules_of_the_subset_expand_as_the_calendar_shows_them() {
        assert_eq!(dates("FREQ=DAILY;INTERVAL=3", "2026-11-01", "2026-11-01", "2026-11-10"), ["2026-11-01", "2026-11-04", "2026-11-07", "2026-11-10"]);
        // the start always is the first occurrence, also when the rule would not give it
        assert_eq!(dates("FREQ=WEEKLY;BYDAY=MO,FR", "2026-11-03", "2026-11-01", "2026-11-14"), ["2026-11-03", "2026-11-06", "2026-11-09", "2026-11-13"]);
        assert_eq!(dates("FREQ=WEEKLY;INTERVAL=2;BYDAY=SU;WKST=SU", "2026-11-01", "2026-11-01", "2026-11-30"), ["2026-11-01", "2026-11-15", "2026-11-29"]);
        assert_eq!(dates("FREQ=MONTHLY", "2026-01-31", "2026-01-01", "2026-06-30"), ["2026-01-31", "2026-03-31", "2026-05-31"]);
        assert_eq!(dates("FREQ=MONTHLY;BYMONTHDAY=-1", "2026-01-31", "2026-02-01", "2026-04-30"), ["2026-02-28", "2026-03-31", "2026-04-30"]);
        assert_eq!(dates("FREQ=MONTHLY;BYDAY=1MO", "2026-10-05", "2026-11-01", "2026-12-31"), ["2026-11-02", "2026-12-07"]);
        assert_eq!(dates("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", "2026-10-30", "2026-11-01", "2026-12-31"), ["2026-11-30", "2026-12-31"]);
        assert_eq!(dates("FREQ=YEARLY", "2024-02-29", "2025-01-01", "2028-12-31"), ["2028-02-29"]);
        assert_eq!(dates("FREQ=YEARLY;BYMONTH=3,9;BYDAY=-1FR", "2026-03-27", "2026-03-01", "2026-12-31"), ["2026-03-27", "2026-09-25"]);
        assert_eq!(dates("FREQ=DAILY;COUNT=3", "2026-11-01", "2026-11-02", "2026-11-30"), ["2026-11-02", "2026-11-03"]);
        assert_eq!(dates("FREQ=DAILY;UNTIL=20261103", "2026-11-01", "2026-11-01", "2026-11-30").len(), 3);
        // a long-running series is not walked from its start
        assert_eq!(dates("FREQ=DAILY", "1990-01-01", "2026-11-23", "2026-11-23"), ["2026-11-23"]);
        assert!(Rule::parse("FREQ=HOURLY").is_none());
    }

    #[test]
    fn a_timed_series_keeps_its_local_time_across_clock_changes() {
        // New York leaves daylight saving time on 2026-11-01
        assert_eq!(
            timed("FREQ=DAILY", "2026-10-30T13:00:00Z", "America/New_York", "2026-10-31T00:00:00Z", "2026-11-03T00:00:00Z"),
            ["2026-10-31T13:00:00Z", "2026-11-01T14:00:00Z", "2026-11-02T14:00:00Z"].map(|s| aiworkspace_core::value::format_utc_ms(parse_datetime_ms(s).unwrap()))
        );
        // 02:30 does not exist on 2026-03-08: it moves forward by the change
        assert_eq!(zoned(day_of("2026-03-08").unwrap(), 2 * 3600 + 1800, "America/New_York".parse().unwrap()), parse_datetime_ms("2026-03-08T07:30:00Z").unwrap());
    }
}
