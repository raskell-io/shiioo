//! When a routine fires: a small schedule language the owner can type on
//! a phone, evaluated in the office machine's local time.
//!
//! ```text
//! hourly                 every hour
//! every 30m, every 6h    every N minutes or hours
//! daily 07:00            every day at 07:00
//! weekdays 07:00         Monday to Friday
//! weekends 10:00         Saturday and Sunday
//! mon,thu 09:30          named days (mon tue wed thu fri sat sun)
//! 07:00                  same as daily 07:00
//! ```
//!
//! Cron expressions were considered and rejected: nobody remembers which
//! column is the weekday from a beach.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Timelike, Weekday};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schedule {
    /// Every `minutes` minutes, counted from the previous firing.
    Every { minutes: u32 },
    /// At `at` on the days in `days`.
    Daily { at: NaiveTime, days: DaySet },
}

/// A set of weekdays as a bitmask, Monday = bit 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaySet(u8);

impl DaySet {
    pub const ALL: DaySet = DaySet(0b111_1111);
    pub const WEEKDAYS: DaySet = DaySet(0b001_1111);
    pub const WEEKENDS: DaySet = DaySet(0b110_0000);

    pub fn contains(self, day: Weekday) -> bool {
        self.0 & (1 << day.num_days_from_monday()) != 0
    }

    fn with(self, day: Weekday) -> Self {
        DaySet(self.0 | (1 << day.num_days_from_monday()))
    }
}

pub fn parse(text: &str) -> Result<Schedule, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        ["hourly"] => Ok(Schedule::Every { minutes: 60 }),
        ["every", amount] => {
            let (number, unit) = amount.split_at(
                amount
                    .find(|c: char| !c.is_ascii_digit())
                    .ok_or_else(|| format!("{amount:?}: give a unit, m or h"))?,
            );
            let n: u32 = number
                .parse()
                .map_err(|_| format!("{amount:?}: not a number"))?;
            let minutes = match unit {
                "m" | "min" | "minutes" => n,
                "h" | "hours" => n * 60,
                other => return Err(format!("{other:?}: use m or h")),
            };
            if minutes == 0 {
                return Err("every 0: nothing would fire".into());
            }
            Ok(Schedule::Every { minutes })
        }
        [time] => Ok(Schedule::Daily {
            at: parse_time(time)?,
            days: DaySet::ALL,
        }),
        [days, time] => Ok(Schedule::Daily {
            at: parse_time(time)?,
            days: parse_days(days)?,
        }),
        _ => Err(format!(
            "{text:?}: expected hourly, every <N>m|h, daily HH:MM, weekdays HH:MM, weekends HH:MM or mon,thu HH:MM"
        )),
    }
}

fn parse_time(s: &str) -> Result<NaiveTime, String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("{s:?}: time must be HH:MM"))?;
    let hour: u32 = h.parse().map_err(|_| format!("{s:?}: bad hour"))?;
    let minute: u32 = m.parse().map_err(|_| format!("{s:?}: bad minute"))?;
    NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| format!("{s:?}: no such time"))
}

fn parse_days(s: &str) -> Result<DaySet, String> {
    match s {
        "daily" | "every" | "everyday" => return Ok(DaySet::ALL),
        "weekdays" => return Ok(DaySet::WEEKDAYS),
        "weekends" => return Ok(DaySet::WEEKENDS),
        _ => {}
    }
    let mut set = DaySet(0);
    for name in s.split(',') {
        let day = match name.trim().to_ascii_lowercase().as_str() {
            "mon" | "monday" => Weekday::Mon,
            "tue" | "tuesday" => Weekday::Tue,
            "wed" | "wednesday" => Weekday::Wed,
            "thu" | "thursday" => Weekday::Thu,
            "fri" | "friday" => Weekday::Fri,
            "sat" | "saturday" => Weekday::Sat,
            "sun" | "sunday" => Weekday::Sun,
            other => return Err(format!("{other:?}: not a day")),
        };
        set = set.with(day);
    }
    Ok(set)
}

impl Schedule {
    /// The first firing strictly after `after`.
    pub fn next_after(&self, after: DateTime<Local>) -> DateTime<Local> {
        match self {
            Schedule::Every { minutes } => after + Duration::minutes(i64::from(*minutes)),
            Schedule::Daily { at, days } => {
                for offset in 0..9 {
                    let date = (after + Duration::days(offset)).date_naive();
                    if !days.contains(date.weekday()) {
                        continue;
                    }
                    let naive = date.and_time(*at);
                    let Some(candidate) = Local
                        .from_local_datetime(&naive)
                        .earliest()
                        .or_else(|| Local.from_local_datetime(&naive).latest())
                    else {
                        continue;
                    };
                    if candidate > after {
                        return candidate;
                    }
                }
                // Unreachable with a non-empty day set; an empty one never fires.
                after + Duration::days(36_500)
            }
        }
    }
}

impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Schedule::Every { minutes: 60 } => f.write_str("hourly"),
            Schedule::Every { minutes } if minutes % 60 == 0 => {
                write!(f, "every {}h", minutes / 60)
            }
            Schedule::Every { minutes } => write!(f, "every {minutes}m"),
            Schedule::Daily { at, days } => {
                let when = match *days {
                    DaySet::ALL => "daily".to_owned(),
                    DaySet::WEEKDAYS => "weekdays".to_owned(),
                    DaySet::WEEKENDS => "weekends".to_owned(),
                    other => [
                        Weekday::Mon,
                        Weekday::Tue,
                        Weekday::Wed,
                        Weekday::Thu,
                        Weekday::Fri,
                        Weekday::Sat,
                        Weekday::Sun,
                    ]
                    .into_iter()
                    .filter(|d| other.contains(*d))
                    .map(|d| {
                        let s = d.to_string();
                        s[..3].to_ascii_lowercase()
                    })
                    .collect::<Vec<_>>()
                    .join(","),
                };
                write!(f, "{when} {:02}:{:02}", at.hour(), at.minute())
            }
        }
    }
}

/// `Sat 25 Oct 07:00`, for messages.
pub fn humanize(t: DateTime<Local>) -> String {
    t.format("%a %-d %b %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(y, mo, d)
                    .unwrap_or_default()
                    .and_hms_opt(h, mi, 0)
                    .unwrap_or_default(),
            )
            .earliest()
            .unwrap_or_else(|| panic!("no local time"))
    }

    #[test]
    fn parses_the_documented_forms() {
        assert_eq!(parse("hourly"), Ok(Schedule::Every { minutes: 60 }));
        assert_eq!(parse("every 30m"), Ok(Schedule::Every { minutes: 30 }));
        assert_eq!(parse("every 6h"), Ok(Schedule::Every { minutes: 360 }));
        assert_eq!(
            parse("daily 07:00"),
            Ok(Schedule::Daily {
                at: NaiveTime::from_hms_opt(7, 0, 0).unwrap_or_default(),
                days: DaySet::ALL
            })
        );
        assert_eq!(parse("07:00"), parse("daily 07:00"));
        assert_eq!(
            parse("weekdays 07:00").map(|s| s.to_string()),
            Ok("weekdays 07:00".to_owned())
        );
        assert_eq!(
            parse("mon,Thu 09:30").map(|s| s.to_string()),
            Ok("mon,thu 09:30".to_owned())
        );
        assert_eq!(
            parse("every 6h").map(|s| s.to_string()),
            Ok("every 6h".to_owned())
        );
        for bad in [
            "",
            "every",
            "every 0m",
            "daily 25:00",
            "fun 07:00",
            "7am",
            "every 3d",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn daily_fires_at_the_next_matching_day() {
        // 2026-10-24 is a Saturday.
        let sat_morning = local(2026, 10, 24, 6, 0);
        let weekdays = parse("weekdays 07:00").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(weekdays.next_after(sat_morning), local(2026, 10, 26, 7, 0));
        let daily = parse("daily 07:00").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(daily.next_after(sat_morning), local(2026, 10, 24, 7, 0));
        // Exactly at the time means the next day: "strictly after".
        assert_eq!(
            daily.next_after(local(2026, 10, 24, 7, 0)),
            local(2026, 10, 25, 7, 0)
        );
        let weekends = parse("weekends 10:00").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            weekends.next_after(local(2026, 10, 26, 12, 0)),
            local(2026, 10, 31, 10, 0)
        );
        let every = parse("every 90m").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(every.next_after(sat_morning), local(2026, 10, 24, 7, 30));
    }
}
