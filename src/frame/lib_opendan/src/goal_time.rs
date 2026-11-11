//! Calendar timetables of Goals (TODO D1): a 5-field cron expression on the
//! wall clock of an IANA time zone. A slot is the UTC minute (minutes since
//! the epoch) a matching wall-clock minute falls on; a wall-clock time that
//! does not exist (a daylight saving gap) is skipped, an ambiguous one is
//! taken at its earlier instant.

use std::collections::BTreeSet;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

/// A parsed cron expression (`minute hour day-of-month month day-of-week`,
/// `@hourly | @daily | @weekly | @monthly | @yearly`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minute: BTreeSet<u32>,
    hour: BTreeSet<u32>,
    dom: BTreeSet<u32>,
    month: BTreeSet<u32>,
    dow: BTreeSet<u32>,
    dom_star: bool,
    dow_star: bool,
}

fn field(raw: &str, min: u32, max: u32, dow: bool) -> Result<(BTreeSet<u32>, bool), String> {
    let mut values = BTreeSet::new();
    for part in raw.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(|| format!("bad step in `{part}`"))?),
            None => (part, 1),
        };
        let num = |v: &str| -> Result<u32, String> {
            let n = v.parse::<u32>().map_err(|_| format!("bad value `{v}`"))?;
            if n < min || n > max {
                return Err(format!("`{n}` is outside {min}-{max}"));
            }
            Ok(n)
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (num(a)?, num(b)?)
        } else {
            let n = num(range)?;
            (n, if part.contains('/') { max } else { n })
        };
        if start > end {
            return Err(format!("bad range `{part}`"));
        }
        let mut v = start;
        while v <= end {
            values.insert(if dow && v == 7 { 0 } else { v });
            v += step;
        }
    }
    Ok((values, raw == "*"))
}

impl Cron {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let expr = match expr.trim() {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            "@yearly" | "@annually" => "0 0 1 1 *",
            other => other,
        };
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!("a cron expression has 5 fields (minute hour day month weekday), `{expr}` has {}", parts.len()));
        }
        let bad = |what: &str, e: String| format!("cron {what}: {e}");
        let (minute, _) = field(parts[0], 0, 59, false).map_err(|e| bad("minute", e))?;
        let (hour, _) = field(parts[1], 0, 23, false).map_err(|e| bad("hour", e))?;
        let (dom, dom_star) = field(parts[2], 1, 31, false).map_err(|e| bad("day of month", e))?;
        let (month, _) = field(parts[3], 1, 12, false).map_err(|e| bad("month", e))?;
        let (dow, dow_star) = field(parts[4], 0, 7, true).map_err(|e| bad("day of week", e))?;
        Ok(Self { minute, hour, dom, month, dow, dom_star, dow_star })
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        if !self.month.contains(&d.month()) {
            return false;
        }
        let dom = self.dom.contains(&d.day());
        let dow = self.dow.contains(&d.weekday().num_days_from_sunday());
        match (self.dom_star, self.dow_star) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }

    /// The latest matching wall-clock minute at or before `now_ms`, as a
    /// slot (UTC minutes since the epoch). Looks back at most 400 days.
    pub fn slot_at_or_before(&self, tz: Tz, now_ms: u64) -> Option<u64> {
        let now = Utc.timestamp_millis_opt(now_ms as i64).single()?;
        let local_now = now.with_timezone(&tz).naive_local();
        for back in 0..=400i64 {
            let day = local_now.date() - Duration::days(back);
            if !self.day_matches(day) {
                continue;
            }
            for h in self.hour.iter().rev() {
                for m in self.minute.iter().rev() {
                    let t = NaiveDateTime::new(day, NaiveTime::from_hms_opt(*h, *m, 0)?);
                    if t > local_now {
                        continue;
                    }
                    let Some(at) = tz.from_local_datetime(&t).earliest() else {
                        continue;
                    };
                    let ms = at.with_timezone(&Utc).timestamp_millis();
                    if ms >= 0 && ms as u64 <= now_ms {
                        return Some(ms as u64 / 60_000);
                    }
                }
            }
        }
        None
    }

    /// Matching minutes after slot `after` up to slot `upto` (both slots),
    /// counted up to `cap`.
    pub fn slots_between(&self, tz: Tz, after: u64, upto: u64, cap: u64) -> u64 {
        let mut n = 0;
        let mut cur = upto;
        while cur > after && n < cap {
            n += 1;
            match self.slot_at_or_before(tz, cur.saturating_mul(60_000).saturating_sub(60_000)) {
                Some(prev) => cur = prev,
                None => break,
            }
        }
        n
    }
}

/// An IANA zone by name.
pub fn parse_tz(name: &str) -> Result<Tz, String> {
    name.trim().parse::<Tz>().map_err(|_| format!("unknown time zone `{name}` (an IANA name such as Asia/Shanghai)"))
}

/// The zone of a Goal's cron timetable (its own, else UTC).
pub fn goal_tz(timezone: Option<&str>) -> Tz {
    timezone.and_then(|t| parse_tz(t).ok()).unwrap_or(Tz::UTC)
}

/// Check a timetable before it is stored: `every_secs` or `cron`, not both;
/// at least the floor; a valid zone.
pub fn validate_timetable(every_secs: Option<u64>, cron: Option<&str>, timezone: Option<&str>) -> Result<(), String> {
    if every_secs.is_some() && cron.is_some() {
        return Err("a check timetable is every_secs or cron, not both".into());
    }
    if let Some(e) = every_secs {
        if e < crate::protocol::CHECK_MIN_EVERY_SECS {
            return Err(format!("every_secs is at least {}", crate::protocol::CHECK_MIN_EVERY_SECS));
        }
    }
    if let Some(c) = cron {
        Cron::parse(c)?;
    }
    if let Some(t) = timezone {
        parse_tz(t)?;
    }
    Ok(())
}

/// The check timetable of a Goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timetable {
    Every(u64),
    Cron { expr: String, tz: Tz, cron: Cron },
}

impl Timetable {
    /// The Goal's timetable (`None`: no scheduled checks).
    pub fn of(goal: &crate::protocol::GoalDefinition, d: &crate::template::GoalDefaults) -> Option<Self> {
        if let Some(expr) = &goal.schedule.cron {
            let cron = Cron::parse(expr).ok()?;
            return Some(Timetable::Cron {
                expr: expr.clone(),
                tz: goal_tz(goal.schedule.timezone.as_deref()),
                cron,
            });
        }
        crate::goal::check_every_secs(goal, d).map(Timetable::Every)
    }

    /// What `schedule.json` records (a change starts counting again).
    pub fn key(&self) -> String {
        match self {
            Timetable::Every(e) => format!("every:{e}"),
            Timetable::Cron { expr, tz, .. } => format!("cron:{expr}@{tz}"),
        }
    }

    /// A short tag for input keys.
    pub fn tag(&self) -> String {
        match self {
            Timetable::Every(e) => format!("e{e}"),
            Timetable::Cron { .. } => {
                use sha2::{Digest, Sha256};
                format!("c{}", &hex::encode(Sha256::digest(self.key().as_bytes()))[..8])
            }
        }
    }

    /// The slot `now_ms` is in (the latest that has come).
    pub fn slot(&self, now_ms: u64) -> Option<u64> {
        match self {
            Timetable::Every(e) => Some(now_ms / 1000 / (*e).max(1)),
            Timetable::Cron { tz, cron, .. } => cron.slot_at_or_before(*tz, now_ms),
        }
    }

    /// Slots after `last` up to `slot` (1 = on time).
    pub fn missed(&self, last: u64, slot: u64) -> u64 {
        match self {
            Timetable::Every(_) => slot.saturating_sub(last),
            Timetable::Cron { tz, cron, .. } => cron.slots_between(*tz, last, slot, 1000).max(1),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Timetable::Every(e) => format!("every {e}s"),
            Timetable::Cron { expr, tz, .. } => format!("cron `{expr}` {tz}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(s: &str) -> u64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis() as u64
    }

    #[test]
    fn a_daily_cron_follows_the_zone() {
        let c = Cron::parse("0 9 * * *").unwrap();
        let tz = parse_tz("Asia/Shanghai").unwrap();
        // 2026-11-10 02:30 UTC = 10:30 Shanghai: the slot is 09:00 Shanghai = 01:00 UTC.
        let slot = c.slot_at_or_before(tz, ms("2026-11-10T02:30:00Z")).unwrap();
        assert_eq!(slot * 60_000, ms("2026-11-10T01:00:00Z"));
        // Before 09:00 local: yesterday's.
        let slot = c.slot_at_or_before(tz, ms("2026-11-10T00:30:00Z")).unwrap();
        assert_eq!(slot * 60_000, ms("2026-11-09T01:00:00Z"));
    }

    #[test]
    fn weekdays_steps_and_gaps() {
        let c = Cron::parse("*/15 8-10 * * 1-5").unwrap();
        // Saturday 2026-11-14 12:00 UTC: the last match is Friday 10:45.
        let slot = c.slot_at_or_before(Tz::UTC, ms("2026-11-14T12:00:00Z")).unwrap();
        assert_eq!(slot * 60_000, ms("2026-11-13T10:45:00Z"));
        // A daylight saving gap (2026-03-08 02:30 New York does not exist).
        let c = Cron::parse("30 2 * * *").unwrap();
        let tz = parse_tz("America/New_York").unwrap();
        let slot = c.slot_at_or_before(tz, ms("2026-03-08T12:00:00Z")).unwrap();
        assert_eq!(slot * 60_000, ms("2026-03-07T07:30:00Z"), "the missing time is skipped");
        assert_eq!(c.slots_between(Tz::UTC, 0, 0, 10), 0);
        assert!(Cron::parse("61 * * * *").is_err());
        assert!(Cron::parse("* * *").is_err());
        assert!(validate_timetable(Some(30), None, None).is_err());
        assert!(validate_timetable(Some(60), Some("@daily"), None).is_err());
        assert!(validate_timetable(None, Some("@daily"), Some("Mars/Base")).is_err());
    }

    #[test]
    fn slots_between_counts_missed_matches() {
        let c = Cron::parse("0 * * * *").unwrap();
        let now = ms("2026-11-10T05:10:00Z");
        let upto = c.slot_at_or_before(Tz::UTC, now).unwrap();
        let after = ms("2026-11-10T02:00:00Z") / 60_000;
        assert_eq!(c.slots_between(Tz::UTC, after, upto, 100), 3, "03:00, 04:00, 05:00");
    }
}
