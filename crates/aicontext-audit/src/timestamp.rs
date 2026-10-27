//! UTC timestamps in the one spelling the audit log accepts.
//!
//! The workspace has no date-time crate, and the audit log needs exactly two operations: render the
//! current instant and validate the rendering on read. Both fit in a few dozen lines, so adding a
//! calendar dependency for them would be weight without a second use (`RULES.md` §6).
//!
//! The wire form is RFC 3339 UTC with millisecond precision: `YYYY-MM-DDTHH:MM:SS.mmmZ`. A narrower
//! form than RFC 3339 (no offsets, no fractional digits other than three) is deliberate: an audit
//! timeline that has to normalise six spellings of the same instant is harder to compare, and the
//! writer always knows the canonical form.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// One instant, as whole milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Timestamp {
    millis: i64,
}

impl Timestamp {
    /// The current wall-clock instant.
    ///
    /// A clock set before 1970 produces a negative value rather than an error: the value is honest,
    /// and [`Timestamp::as_str`] still renders it. Appending a record whose rendering is not a valid
    /// four-digit year is refused, so a broken clock cannot enter the log.
    #[must_use]
    pub fn now() -> Self {
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
            Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
        };
        Self { millis }
    }

    /// An instant from whole milliseconds since the Unix epoch.
    #[must_use]
    pub const fn from_unix_millis(millis: i64) -> Self {
        Self { millis }
    }

    /// Whole milliseconds since the Unix epoch.
    #[must_use]
    pub const fn unix_millis(self) -> i64 {
        self.millis
    }

    /// The canonical rendering, `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    #[must_use]
    pub fn as_str(self) -> String {
        let seconds = self.millis.div_euclid(1_000);
        let millis = self.millis.rem_euclid(1_000);
        let days = seconds.div_euclid(86_400);
        let within_day = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        let hour = within_day / 3_600;
        let minute = (within_day % 3_600) / 60;
        let second = within_day % 60;

        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }

    /// Reads the canonical rendering, rejecting every other spelling.
    ///
    /// This is the check verification applies to `ts`: a record that claims a timestamp outside the
    /// Gregorian calendar, or in a spelling the writer never produces, is malformed rather than
    /// quietly normalised.
    pub fn parse(text: &str) -> Result<Self, InvalidTimestamp> {
        let bytes = text.as_bytes();
        if bytes.len() != 24 {
            return Err(InvalidTimestamp::new(
                text,
                "expected exactly YYYY-MM-DDTHH:MM:SS.mmmZ",
            ));
        }
        for (index, expected) in [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':'), (19, b'.'), (23, b'Z')]
        {
            if bytes[index] != expected {
                return Err(InvalidTimestamp::new(
                    text,
                    "expected exactly YYYY-MM-DDTHH:MM:SS.mmmZ",
                ));
            }
        }
        let digits = |start: usize, end: usize| -> Option<i64> {
            let slice = &bytes[start..end];
            if slice.iter().all(u8::is_ascii_digit) {
                // Four digits cannot overflow an i64; the cast is exact for the bounded slices.
                let mut value = 0_i64;
                for digit in slice {
                    value = value * 10 + i64::from(digit - b'0');
                }
                Some(value)
            } else {
                None
            }
        };

        let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second), Some(millis)) = (
            digits(0, 4),
            digits(5, 7),
            digits(8, 10),
            digits(11, 13),
            digits(14, 16),
            digits(17, 19),
            digits(20, 23),
        ) else {
            return Err(InvalidTimestamp::new(
                text,
                "every field must be digits in the documented positions",
            ));
        };

        if !(1..=12).contains(&month) {
            return Err(InvalidTimestamp::new(text, "month must be 01 to 12"));
        }
        if day < 1 || day > days_in_month(year, month) {
            return Err(InvalidTimestamp::new(
                text,
                "day is outside the month it names",
            ));
        }
        if hour > 23 {
            return Err(InvalidTimestamp::new(text, "hour must be 00 to 23"));
        }
        if minute > 59 {
            return Err(InvalidTimestamp::new(text, "minute must be 00 to 59"));
        }
        // Leap seconds are rejected: the log's ordering comes from `seq`, and a `:60` second makes
        // arithmetic on the timestamp harder without making the chain stronger.
        if second > 59 {
            return Err(InvalidTimestamp::new(text, "second must be 00 to 59"));
        }

        let days = days_from_civil(year, month, day);
        let millis = ((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000) + millis;
        Ok(Self { millis })
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_str())
    }
}

/// A timestamp that is not the canonical spelling.
#[derive(Debug, thiserror::Error)]
#[error("`{input}` is not an audit timestamp: {reason}")]
pub struct InvalidTimestamp {
    input: String,
    reason: &'static str,
}

impl InvalidTimestamp {
    fn new(input: &str, reason: &'static str) -> Self {
        Self {
            input: input.to_string(),
            reason,
        }
    }

    /// The text that was rejected.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Why it was rejected.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
const fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// The number of days in `month` of `year`.
const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The Gregorian date for a count of days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// The count of days since 1970-01-01 for a Gregorian date (the inverse of [`civil_from_days`]).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{InvalidTimestamp, Timestamp};

    #[test]
    fn the_epoch_renders_in_the_canonical_form() {
        assert_eq!(
            Timestamp::from_unix_millis(0).as_str(),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn the_documented_example_round_trips() {
        let text = "2026-09-27T14:53:06.412Z";
        let timestamp = Timestamp::parse(text).expect("the documented example is valid");
        assert_eq!(timestamp.as_str(), text);
        assert_eq!(
            Timestamp::from_unix_millis(timestamp.unix_millis()).as_str(),
            text
        );
    }

    #[test]
    fn a_leap_day_and_the_day_after_it_round_trip() {
        for text in [
            "2024-02-29T23:59:59.999Z",
            "2000-02-29T00:00:00.000Z",
            "2026-03-01T12:34:56.789Z",
            "1999-12-31T23:59:59.999Z",
        ] {
            let timestamp = Timestamp::parse(text).expect("valid");
            assert_eq!(timestamp.as_str(), text);
        }
    }

    #[test]
    fn a_day_outside_its_month_is_rejected() {
        for text in [
            "2026-02-29T00:00:00.000Z",
            "1900-02-29T00:00:00.000Z",
            "2026-04-31T00:00:00.000Z",
            "2026-13-01T00:00:00.000Z",
            "2026-00-01T00:00:00.000Z",
        ] {
            assert!(
                Timestamp::parse(text).is_err(),
                "{text} must not parse as a date"
            );
        }
    }

    #[test]
    fn an_out_of_range_or_non_canonical_time_is_rejected() {
        for text in [
            "2026-09-27T24:00:00.000Z",
            "2026-09-27T00:60:00.000Z",
            "2026-09-27T00:00:60.000Z",
            "2026-09-27T00:00:00.000+00:00",
            "2026-09-27T00:00:00Z",
            "2026-09-27 00:00:00.000Z",
            "2026-9-27T00:00:00.000Z",
            "not a timestamp",
        ] {
            assert!(
                Timestamp::parse(text).is_err(),
                "{text} must not parse as a timestamp"
            );
        }
    }

    #[test]
    fn a_rejection_names_the_input_and_the_reason() {
        let error = Timestamp::parse("2026-02-30T00:00:00.000Z").expect_err("invalid date");
        assert_eq!(error.input(), "2026-02-30T00:00:00.000Z");
        assert!(error.reason().contains("day"));
        assert!(error.to_string().contains("2026-02-30"));
    }

    #[test]
    fn a_time_before_the_epoch_still_renders_honestly() {
        let timestamp = Timestamp::from_unix_millis(-1);
        assert_eq!(timestamp.as_str(), "1969-12-31T23:59:59.999Z");
        let parsed = Timestamp::parse("1969-12-31T23:59:59.999Z").expect("valid");
        assert_eq!(parsed.unix_millis(), -1);
    }

    #[test]
    fn now_is_after_the_epoch_on_a_sane_clock() {
        // This is the one test that reads the wall clock. It asserts the format, not the value, so a
        // slow or fast machine cannot make it flaky.
        let now = Timestamp::now();
        let text = now.as_str();
        assert_eq!(Timestamp::parse(&text).expect("now is canonical"), now);
        assert!(text.ends_with('Z'));

        let error: InvalidTimestamp = Timestamp::parse("1999-01-01").expect_err("short");
        assert_eq!(error.to_string().split(':').next(), Some("`1999-01-01` is not an audit timestamp"));
    }
}
