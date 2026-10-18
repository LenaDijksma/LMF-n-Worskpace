//! Calendar dates and timestamps for tasks and events — with an explicit distinction between
//! the two (spec §3.12 item 310) and no hidden time-zone assumptions.
//!
//! * [`Date`] — a calendar day (`2026-11-03`). Date-only values are *floating*: they mean "that
//!   day, wherever you are" and are never shifted between time zones.
//! * [`DateTime`] — a moment with an **explicit UTC offset** (`2026-11-03T14:00+01:00`, or `Z`).
//!   A time without an offset is rejected as ambiguous.
//! * [`TaskDate`] — either of the two.
//!
//! The arithmetic uses the proleptic Gregorian calendar (no external crates, no local time).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Date {
    /// A validated date; `None` for impossible days (`2026-02-30`).
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        if !(1..=12).contains(&month) || day == 0 || day > Date::days_in_month(year, month) {
            return None;
        }
        Some(Date { year, month, day })
    }

    pub fn is_leap_year(year: i32) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }

    pub fn days_in_month(year: i32, month: u8) -> u8 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if Date::is_leap_year(year) {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        }
    }

    /// Strict `YYYY-MM-DD`.
    pub fn parse(s: &str) -> Option<Date> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let digits = |r: std::ops::Range<usize>| -> Option<u32> {
            let part = &s[r];
            if part.bytes().all(|c| c.is_ascii_digit()) {
                part.parse::<u32>().ok()
            } else {
                None
            }
        };
        let year = digits(0..4)? as i32;
        let month = digits(5..7)? as u8;
        let day = digits(8..10)? as u8;
        Date::new(year, month, day)
    }

    /// Days since 1970-01-01 (negative before).
    pub fn to_days(self) -> i64 {
        let y = self.year as i64 - if self.month <= 2 { 1 } else { 0 };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = self.month as i64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + self.day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(days: i64) -> Date {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        Date {
            year: (y + if m <= 2 { 1 } else { 0 }) as i32,
            month: m as u8,
            day: d as u8,
        }
    }

    pub fn add_days(self, n: i64) -> Date {
        Date::from_days(self.to_days() + n)
    }

    /// Adds calendar months; the day is clamped to the end of a shorter month
    /// (31 Jan + 1 month = 28/29 Feb).
    pub fn add_months(self, n: i32) -> Date {
        let total = self.year as i64 * 12 + (self.month as i64 - 1) + n as i64;
        let year = total.div_euclid(12) as i32;
        let month = (total.rem_euclid(12) + 1) as u8;
        let day = self.day.min(Date::days_in_month(year, month));
        Date { year, month, day }
    }

    pub fn add_years(self, n: i32) -> Date {
        self.add_months(n * 12)
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(self) -> u8 {
        (self.to_days() + 3).rem_euclid(7) as u8
    }

    /// The Monday of this date's week (ISO weeks start on Monday).
    pub fn week_start(self) -> Date {
        self.add_days(-(self.weekday() as i64))
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A moment with an explicit UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTime {
    /// The calendar day *in the stated offset*.
    pub date: Date,
    /// Minutes after local midnight (0..1440).
    pub minutes: u16,
    /// Offset from UTC in minutes (e.g. +01:00 = 60).
    pub offset_minutes: i16,
}

impl DateTime {
    /// Minutes since 1970-01-01T00:00Z — comparable across offsets.
    pub fn utc_minutes(&self) -> i64 {
        self.date.to_days() * 1440 + self.minutes as i64 - self.offset_minutes as i64
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}T{:02}:{:02}",
            self.date,
            self.minutes / 60,
            self.minutes % 60
        )?;
        if self.offset_minutes == 0 {
            write!(f, "Z")
        } else {
            let sign = if self.offset_minutes < 0 { '-' } else { '+' };
            let abs = self.offset_minutes.unsigned_abs();
            write!(f, "{}{:02}:{:02}", sign, abs / 60, abs % 60)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskDate {
    /// A floating calendar day.
    Date(Date),
    /// A moment with an explicit offset.
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateError {
    /// Not `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM…`.
    BadFormat,
    /// Month or day out of range (e.g. 2026-02-30).
    InvalidDate,
    InvalidTime,
    /// A time was given without `Z` or `±HH:MM`.
    MissingOffset,
    InvalidOffset,
}

impl fmt::Display for DateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DateError::BadFormat => write!(
                f,
                "expected a date like 2026-11-03 or a timestamp like 2026-11-03T14:00+01:00"
            ),
            DateError::InvalidDate => write!(f, "that calendar date does not exist"),
            DateError::InvalidTime => write!(f, "invalid time of day"),
            DateError::MissingOffset => write!(
                f,
                "a time needs an explicit UTC offset (Z or +01:00); times without one are ambiguous"
            ),
            DateError::InvalidOffset => write!(f, "invalid UTC offset (use Z or ±HH:MM)"),
        }
    }
}

impl TaskDate {
    pub fn parse(s: &str) -> Result<TaskDate, DateError> {
        let s = s.trim();
        if s.len() < 10 || !s.is_char_boundary(10) {
            return Err(DateError::BadFormat);
        }
        let (day_part, rest) = s.split_at(10);
        let looks_like_date = day_part.len() == 10
            && day_part.as_bytes()[4] == b'-'
            && day_part.as_bytes()[7] == b'-'
            && day_part
                .bytes()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
        if !looks_like_date {
            return Err(DateError::BadFormat);
        }
        let date = Date::parse(day_part).ok_or(DateError::InvalidDate)?;
        if rest.is_empty() {
            return Ok(TaskDate::Date(date));
        }
        let time = rest.strip_prefix('T').ok_or(DateError::BadFormat)?;
        // HH:MM[:SS] followed by Z / +HH:MM / -HH:MM
        let tb = time.as_bytes();
        if tb.len() < 5 || tb[2] != b':' {
            return Err(DateError::BadFormat);
        }
        let num = |a: usize, b: usize| -> Result<u16, DateError> {
            let part = time.get(a..b).ok_or(DateError::BadFormat)?;
            if part.bytes().all(|c| c.is_ascii_digit()) && !part.is_empty() {
                part.parse::<u16>().map_err(|_| DateError::InvalidTime)
            } else {
                Err(DateError::BadFormat)
            }
        };
        let hours = num(0, 2)?;
        let mins = num(3, 5)?;
        let mut idx = 5usize;
        if tb.get(idx) == Some(&b':') {
            let secs = num(idx + 1, idx + 3)?;
            if secs > 59 {
                return Err(DateError::InvalidTime);
            }
            idx += 3;
        }
        if hours > 23 || mins > 59 {
            return Err(DateError::InvalidTime);
        }
        let offset_str = &time[idx..];
        let offset_minutes: i16 = if offset_str.is_empty() {
            return Err(DateError::MissingOffset);
        } else if offset_str == "Z" {
            0
        } else {
            let sign: i16 = match offset_str.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return Err(DateError::InvalidOffset),
            };
            let ob = offset_str.as_bytes();
            if ob.len() != 6 || ob[3] != b':' {
                return Err(DateError::InvalidOffset);
            }
            let oh = offset_str.get(1..3).and_then(|p| p.parse::<i16>().ok());
            let om = offset_str.get(4..6).and_then(|p| p.parse::<i16>().ok());
            match (oh, om) {
                (Some(h), Some(m)) if h <= 14 && m <= 59 => sign * (h * 60 + m),
                _ => return Err(DateError::InvalidOffset),
            }
        };
        Ok(TaskDate::DateTime(DateTime {
            date,
            minutes: hours * 60 + mins,
            offset_minutes,
        }))
    }

    /// The calendar day (for a timestamp: the day in its own stated offset).
    pub fn date(&self) -> Date {
        match self {
            TaskDate::Date(d) => *d,
            TaskDate::DateTime(dt) => dt.date,
        }
    }

    pub fn is_date_only(&self) -> bool {
        matches!(self, TaskDate::Date(_))
    }

    /// The same value moved to another calendar day, keeping time and offset.
    pub fn with_date(&self, date: Date) -> TaskDate {
        match self {
            TaskDate::Date(_) => TaskDate::Date(date),
            TaskDate::DateTime(dt) => TaskDate::DateTime(DateTime { date, ..*dt }),
        }
    }
}

impl fmt::Display for TaskDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskDate::Date(d) => write!(f, "{}", d),
            TaskDate::DateTime(dt) => write!(f, "{}", dt),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    #[test]
    fn validation_and_leap_years() {
        assert!(Date::new(2026, 2, 29).is_none());
        assert!(Date::new(2028, 2, 29).is_some());
        assert!(Date::new(1900, 2, 29).is_none());
        assert!(Date::new(2000, 2, 29).is_some());
        assert!(Date::new(2026, 13, 1).is_none());
        assert!(Date::new(2026, 4, 31).is_none());
        assert!(Date::new(2026, 0, 1).is_none());
    }

    #[test]
    fn day_numbers_round_trip() {
        assert_eq!(d(1970, 1, 1).to_days(), 0);
        assert_eq!(d(2000, 3, 1).to_days(), 11_017);
        assert_eq!(Date::from_days(0), d(1970, 1, 1));
        assert_eq!(Date::from_days(-1), d(1969, 12, 31));
        for days in (-800_000..800_000).step_by(997) {
            assert_eq!(Date::from_days(days).to_days(), days);
        }
        assert_eq!(d(2026, 3, 1).add_days(-1), d(2026, 2, 28));
        assert_eq!(d(2028, 3, 1).add_days(-1), d(2028, 2, 29));
        assert_eq!(d(2026, 12, 31).add_days(1), d(2027, 1, 1));
    }

    #[test]
    fn weekdays_and_weeks() {
        assert_eq!(d(1970, 1, 1).weekday(), 3); // Thursday
        assert_eq!(d(2026, 11, 3).weekday(), 1); // Tuesday
        assert_eq!(d(2026, 11, 3).week_start(), d(2026, 11, 2));
        assert_eq!(d(2026, 11, 8).week_start(), d(2026, 11, 2)); // Sunday
        assert_eq!(d(2026, 11, 9).week_start(), d(2026, 11, 9)); // Monday
    }

    #[test]
    fn month_arithmetic_clamps() {
        assert_eq!(d(2026, 1, 31).add_months(1), d(2026, 2, 28));
        assert_eq!(d(2028, 1, 31).add_months(1), d(2028, 2, 29));
        assert_eq!(d(2026, 3, 31).add_months(-1), d(2026, 2, 28));
        assert_eq!(d(2026, 11, 15).add_months(3), d(2027, 2, 15));
        assert_eq!(d(2026, 11, 15).add_months(-11), d(2025, 12, 15));
        assert_eq!(d(2028, 2, 29).add_years(1), d(2029, 2, 28));
    }

    #[test]
    fn parsing_dates() {
        assert_eq!(Date::parse("2026-11-03"), Some(d(2026, 11, 3)));
        assert_eq!(Date::parse("2026-1-3"), None);
        assert_eq!(Date::parse("2026-02-30"), None);
        assert_eq!(Date::parse("20261103"), None);
        assert_eq!(Date::parse("2026-11-0x"), None);
        assert_eq!(format!("{}", d(2026, 1, 5)), "2026-01-05");
    }

    #[test]
    fn task_dates_distinguish_dates_and_timestamps() {
        assert_eq!(TaskDate::parse("2026-11-03"), Ok(TaskDate::Date(d(2026, 11, 3))));
        let dt = TaskDate::parse("2026-11-03T14:00+01:00").unwrap();
        assert!(!dt.is_date_only());
        assert_eq!(dt.date(), d(2026, 11, 3));
        assert_eq!(dt.to_string(), "2026-11-03T14:00+01:00");
        assert_eq!(TaskDate::parse("2026-11-03T14:00:30Z").unwrap().to_string(), "2026-11-03T14:00Z");
        assert_eq!(TaskDate::parse("2026-11-03T09:30-05:30").unwrap().to_string(), "2026-11-03T09:30-05:30");
    }

    #[test]
    fn timestamps_need_an_explicit_offset() {
        assert_eq!(TaskDate::parse("2026-11-03T14:00"), Err(DateError::MissingOffset));
        assert_eq!(TaskDate::parse("2026-11-03T14:00:00"), Err(DateError::MissingOffset));
        assert_eq!(TaskDate::parse("2026-11-03T25:00Z"), Err(DateError::InvalidTime));
        assert_eq!(TaskDate::parse("2026-11-03T14:60Z"), Err(DateError::InvalidTime));
        assert_eq!(TaskDate::parse("2026-11-03T14:00+1"), Err(DateError::InvalidOffset));
        assert_eq!(TaskDate::parse("2026-11-03T14:00+15:00"), Err(DateError::InvalidOffset));
        assert_eq!(TaskDate::parse("2026-13-03"), Err(DateError::InvalidDate));
        assert_eq!(TaskDate::parse("tomorrow"), Err(DateError::BadFormat));
        assert_eq!(TaskDate::parse(""), Err(DateError::BadFormat));
        assert_eq!(TaskDate::parse("2026-11-03 14:00Z"), Err(DateError::BadFormat));
    }

    #[test]
    fn instants_compare_across_offsets() {
        let a = match TaskDate::parse("2026-11-03T14:00+01:00").unwrap() {
            TaskDate::DateTime(x) => x,
            _ => unreachable!(),
        };
        let b = match TaskDate::parse("2026-11-03T13:00Z").unwrap() {
            TaskDate::DateTime(x) => x,
            _ => unreachable!(),
        };
        assert_eq!(a.utc_minutes(), b.utc_minutes());
    }

    #[test]
    fn with_date_keeps_time_and_offset() {
        let t = TaskDate::parse("2026-11-03T14:00+01:00").unwrap();
        assert_eq!(t.with_date(d(2026, 11, 10)).to_string(), "2026-11-10T14:00+01:00");
        let day = TaskDate::parse("2026-11-03").unwrap();
        assert_eq!(day.with_date(d(2026, 12, 1)).to_string(), "2026-12-01");
    }

    #[test]
    fn never_panics_on_garbage() {
        for s in ["é", "2026-11-0é", "2026-11-03Té", "2026-11-03T", "2026-11-03T1", "日本語日本語日本語", "2026-11-03T14:00+é:00"] {
            let _ = TaskDate::parse(s);
        }
    }
}
