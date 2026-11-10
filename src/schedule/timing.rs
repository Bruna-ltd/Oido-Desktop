//! When a schedule fires: once, every so many minutes on a fixed grid, or on a cron expression.
//! Daily and weekly times are cron expressions of one shape ([`AtTime`]), which the form shows as
//! presets. Times are seconds since the epoch; the time zone (the local one in the app) only
//! matters to cron and to how a time is written.

use chrono::{DateTime, TimeZone};
use croner::Cron;
use croner::parser::{CronParser, Seconds, Year};
use serde::{Deserialize, Serialize};

/// The shortest gap between two fires of an "every" schedule, in seconds.
pub const MIN_EVERY_SECS: i64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Stored", into = "Stored")]
pub enum Timing {
    /// One fire, at this time.
    Once(i64),
    /// Fires at `start + k * secs` for k = 0, 1, 2, ...: late or skipped fires never shift the
    /// later ones.
    Every { start: i64, secs: i64 },
    /// Fires when a 5-field cron expression (minute hour day month weekday) matches, in local
    /// time. The text is kept as written, to store and show it.
    Cron { expr: String, cron: Box<Cron> },
}

/// How a timing is saved: `{"once": t}`, `{"every": {"start": t, "secs": n}}` or `{"cron": "..."}`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stored {
    Once(i64),
    Every { start: i64, secs: i64 },
    Cron(String),
}

impl TryFrom<Stored> for Timing {
    type Error = String;

    fn try_from(s: Stored) -> Result<Timing, String> {
        match s {
            Stored::Once(at) => Ok(Timing::Once(at)),
            Stored::Every { start, secs } => Timing::every(start, secs),
            Stored::Cron(expr) => Timing::cron(&expr),
        }
    }
}

impl From<Timing> for Stored {
    fn from(t: Timing) -> Stored {
        match t {
            Timing::Once(at) => Stored::Once(at),
            Timing::Every { start, secs } => Stored::Every { start, secs },
            Timing::Cron { expr, .. } => Stored::Cron(expr),
        }
    }
}

impl Timing {
    /// Every `secs` seconds from `start`. Errs, in words, below [`MIN_EVERY_SECS`] or when `secs`
    /// is not whole minutes.
    pub fn every(start: i64, secs: i64) -> Result<Timing, String> {
        if secs < MIN_EVERY_SECS || secs % 60 != 0 {
            return Err(format!(
                "the gap must be a whole number of minutes, at least one (got {secs} seconds)"
            ));
        }
        Ok(Timing::Every { start, secs })
    }

    /// A 5-field cron expression: minute, hour, day of month, month, day of week. Errs, in
    /// words, if it does not parse.
    pub fn cron(expr: &str) -> Result<Timing, String> {
        let expr = expr.split_whitespace().collect::<Vec<_>>().join(" ");
        let parser = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build();
        let cron = parser
            .parse(&expr)
            .map_err(|e| format!("{expr:?} is not a cron expression: {e}"))?;
        Ok(Timing::Cron {
            expr,
            cron: Box::new(cron),
        })
    }

    /// At a time of day on chosen weekdays.
    pub fn at(at: AtTime) -> Result<Timing, String> {
        Timing::cron(&at.to_cron())
    }

    /// The first fire strictly after `t`, or `None` if there is none (a "once" that has passed).
    pub fn next_after<Tz: TimeZone>(&self, t: i64, tz: &Tz) -> Option<i64> {
        match self {
            Timing::Once(at) => (*at > t).then_some(*at),
            Timing::Every { start, secs } => {
                if t < *start {
                    return Some(*start);
                }
                let k = (t - start) / secs + 1;
                start.checked_add(k.checked_mul(*secs)?)
            }
            Timing::Cron { cron, .. } => {
                let now = tz.timestamp_opt(t, 0).earliest()?;
                cron.find_next_occurrence(&now, false)
                    .ok()
                    .map(|d| d.timestamp())
            }
        }
    }

    /// The daily or weekly preset this is, if it is one.
    pub fn preset(&self) -> Option<AtTime> {
        match self {
            Timing::Cron { expr, .. } => AtTime::from_cron(expr),
            _ => None,
        }
    }

    /// The timing in words: "every 15 minutes", "weekdays at 09:00", "once, Tue 11 Nov 2026 09:00".
    pub fn describe<Tz: TimeZone>(&self, tz: &Tz) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        match self {
            Timing::Once(at) => format!("once, {}", when(*at, tz)),
            Timing::Every { secs, .. } => match secs {
                60 => "every minute".to_owned(),
                3600 => "every hour".to_owned(),
                s if s % 3600 == 0 => format!("every {} hours", s / 3600),
                s => format!("every {} minutes", s / 60),
            },
            Timing::Cron { expr, cron } => match AtTime::from_cron(expr) {
                Some(at) => at.describe(),
                None => format!("cron {expr} ({})", cron.describe()),
            },
        }
    }
}

/// A time written for people: "Tue 11 Nov 2026 09:00".
pub fn when<Tz: TimeZone>(t: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let at: Option<DateTime<Tz>> = tz.timestamp_opt(t, 0).earliest();
    at.map_or_else(
        || t.to_string(),
        |d| d.format("%a %-d %b %Y %H:%M").to_string(),
    )
}

/// A time of day on some weekdays: the "daily" and "weekly" presets, kept as the cron
/// expression `M H * * D`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtTime {
    pub hour: u8,
    pub minute: u8,
    pub days: Days,
}

impl AtTime {
    fn to_cron(self) -> String {
        format!("{} {} * * {}", self.minute, self.hour, self.days.to_cron())
    }

    /// Reads back an expression [`AtTime::to_cron`] made: plain minute and hour, any day and
    /// month, and `*` or a list of weekdays.
    fn from_cron(expr: &str) -> Option<AtTime> {
        let [minute, hour, "*", "*", dow] = expr.split_whitespace().collect::<Vec<_>>()[..] else {
            return None;
        };
        let minute: u8 = plain(minute)?;
        let hour: u8 = plain(hour)?;
        (minute < 60 && hour < 24).then_some(())?;
        Some(AtTime {
            hour,
            minute,
            days: Days::from_cron(dow)?,
        })
    }

    fn describe(self) -> String {
        format!(
            "{} at {:02}:{:02}",
            self.days.describe(),
            self.hour,
            self.minute
        )
    }
}

/// A number written with digits only (no `*/5`, ranges or names).
fn plain<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| s.parse().ok())
        .flatten()
}

/// A set of weekdays. Bit `d` is cron's day `d`: 0 is Sunday, 6 Saturday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Days(u8);

impl Days {
    pub const NONE: Days = Days(0);
    pub const EVERY: Days = Days(0b111_1111);
    pub const WEEKDAYS: Days = Days(0b011_1110);
    const WEEKEND: Days = Days(0b100_0001);
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    /// Monday first, as the form lists them: (cron day, name).
    pub fn week() -> impl Iterator<Item = (u8, &'static str)> {
        (1..7).chain([0]).map(|d| (d, Self::NAMES[usize::from(d)]))
    }

    pub fn contains(self, day: u8) -> bool {
        day < 7 && self.0 & (1 << day) != 0
    }

    pub fn toggle(&mut self, day: u8) {
        if day < 7 {
            self.0 ^= 1 << day;
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn to_cron(self) -> String {
        if self == Days::EVERY || self.is_empty() {
            return "*".to_owned();
        }
        (0..7)
            .filter(|d| self.contains(*d))
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn from_cron(field: &str) -> Option<Days> {
        if field == "*" {
            return Some(Days::EVERY);
        }
        let mut days = Days(0);
        for part in field.split(',') {
            let d: u8 = plain(part)?;
            // Cron also writes Sunday as 7.
            let d = if d == 7 { 0 } else { d };
            (d < 7).then_some(())?;
            days.0 |= 1 << d;
        }
        Some(days)
    }

    fn describe(self) -> String {
        match self {
            Days::EVERY => "every day".to_owned(),
            Days::WEEKDAYS => "weekdays".to_owned(),
            Days::WEEKEND => "weekends".to_owned(),
            _ => Days::week()
                .filter(|(d, _)| self.contains(*d))
                .map(|(_, name)| name)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::{FixedOffset, NaiveDate, Utc};

    use super::*;

    /// 2026-11-10 (a Tuesday) at `h:m` UTC.
    fn tue(h: u32, m: u32) -> i64 {
        NaiveDate::from_ymd_opt(2026, 11, 10)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    }

    #[test]
    fn once_fires_only_before_its_time() {
        let t = Timing::Once(tue(9, 0));
        assert_eq!(t.next_after(tue(8, 0), &Utc), Some(tue(9, 0)));
        assert_eq!(t.next_after(tue(9, 0), &Utc), None);
    }

    #[test]
    fn every_keeps_its_grid_however_late_it_is_asked() {
        let t = Timing::every(tue(9, 0), 15 * 60).unwrap();
        assert_eq!(t.next_after(tue(8, 0), &Utc), Some(tue(9, 0)));
        assert_eq!(t.next_after(tue(9, 0), &Utc), Some(tue(9, 15)));
        // Asked 7 minutes late (a skipped fire): the next stays on the grid.
        assert_eq!(t.next_after(tue(9, 22), &Utc), Some(tue(9, 30)));
        // Asked days later (the app was closed): one fire, on the grid, after now.
        assert_eq!(
            t.next_after(tue(9, 0) + 3 * 86_400 + 1, &Utc),
            Some(tue(9, 15) + 3 * 86_400)
        );
    }

    #[test]
    fn every_refuses_gaps_that_are_not_whole_minutes() {
        assert!(Timing::every(0, 59).is_err());
        assert!(Timing::every(0, 90).is_err());
        assert!(Timing::every(0, 0).is_err());
        assert!(Timing::every(0, 120).is_ok());
    }

    #[test]
    fn cron_finds_the_next_weekday_in_the_given_zone() {
        let t = Timing::cron("0 9 * * 1-5").unwrap();
        // Tuesday 09:30 UTC: next is Wednesday 09:00.
        assert_eq!(t.next_after(tue(9, 30), &Utc), Some(tue(9, 0) + 86_400));
        // In UTC+2, 09:00 local is 07:00 UTC, so at 06:00 UTC it is still today.
        let plus2 = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(t.next_after(tue(6, 0), &plus2), Some(tue(7, 0)));
    }

    #[test]
    fn cron_takes_five_fields_only() {
        assert!(Timing::cron("*/5 * * * *").is_ok());
        assert!(Timing::cron("0 0 9 * * *").is_err(), "seconds field");
        assert!(Timing::cron("not cron").is_err());
        assert!(Timing::cron("").is_err());
    }

    #[test]
    fn presets_round_trip_through_cron() {
        let mut days = Days::WEEKDAYS;
        for at in [
            AtTime {
                hour: 9,
                minute: 0,
                days: Days::EVERY,
            },
            AtTime {
                hour: 8,
                minute: 30,
                days: Days::WEEKDAYS,
            },
            AtTime {
                hour: 23,
                minute: 59,
                days: {
                    days.toggle(3);
                    days
                },
            },
        ] {
            let t = Timing::at(at).unwrap();
            assert_eq!(t.preset(), Some(at), "{t:?}");
        }
        assert_eq!(
            Timing::cron("0 9 * * 7").unwrap().preset().unwrap().days,
            Days(1)
        );
        assert_eq!(Timing::cron("*/5 * * * *").unwrap().preset(), None);
        assert_eq!(Timing::cron("0 9 1 * *").unwrap().preset(), None);
        assert_eq!(Timing::every(0, 60).unwrap().preset(), None);
    }

    #[test]
    fn timings_read_well() {
        let at = |hour, minute, days| Timing::at(AtTime { hour, minute, days }).unwrap();
        let mut mon_wed = Days(0);
        mon_wed.toggle(1);
        mon_wed.toggle(3);
        let cases = [
            (Timing::every(0, 60).unwrap(), "every minute"),
            (Timing::every(0, 15 * 60).unwrap(), "every 15 minutes"),
            (Timing::every(0, 3600).unwrap(), "every hour"),
            (Timing::every(0, 2 * 3600).unwrap(), "every 2 hours"),
            (Timing::every(0, 90 * 60).unwrap(), "every 90 minutes"),
            (at(9, 0, Days::EVERY), "every day at 09:00"),
            (at(8, 30, Days::WEEKDAYS), "weekdays at 08:30"),
            (at(10, 5, Days::WEEKEND), "weekends at 10:05"),
            (at(7, 0, mon_wed), "Mon, Wed at 07:00"),
            (Timing::Once(tue(9, 0)), "once, Tue 10 Nov 2026 09:00"),
        ];
        for (t, want) in cases {
            assert_eq!(t.describe(&Utc), want);
        }
        assert!(
            Timing::cron("0 */4 * * *")
                .unwrap()
                .describe(&Utc)
                .starts_with("cron 0 */4 * * * (")
        );
    }

    #[test]
    fn timings_store_as_tagged_json_and_check_what_they_read() {
        let cases = [
            (Timing::Once(5), r#"{"once":5}"#),
            (
                Timing::every(0, 60).unwrap(),
                r#"{"every":{"start":0,"secs":60}}"#,
            ),
            (
                Timing::cron("0  9 * * *").unwrap(),
                r#"{"cron":"0 9 * * *"}"#,
            ),
        ];
        for (t, json) in cases {
            assert_eq!(serde_json::to_string(&t).unwrap(), json);
            assert_eq!(serde_json::from_str::<Timing>(json).unwrap(), t);
        }
        assert!(serde_json::from_str::<Timing>(r#"{"every":{"start":0,"secs":1}}"#).is_err());
        assert!(serde_json::from_str::<Timing>(r#"{"cron":"nope"}"#).is_err());
    }
}
