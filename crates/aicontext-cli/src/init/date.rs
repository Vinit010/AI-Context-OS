//! The creation date stamped into the documents `init` writes.
//!
//! `docs/CONTEXT_SPEC.md` §2 rule 8 says `updated` is set by the tool when it writes the file and
//! never guessed. That needs a calendar, and a calendar is not worth a dependency: the conversion
//! from a Unix day number to a proleptic Gregorian date is the well-known `civil_from_days`
//! arithmetic, correct for every date this binary will ever see, and small enough to read.
//!
//! The clock is read once, in `main`, and passed down. No test ever calls [`today_utc`], so no test
//! can fail because a day changed (`RULES.md` §8).

use std::time::{SystemTime, UNIX_EPOCH};

use super::error::InitError;

/// A calendar date, without a time and without a zone: exactly what `created` may hold.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Date {
    year: i64,
    month: u32,
    day: u32,
}

impl Date {
    /// The `YYYY-MM-DD` spelling, which is what the format requires.
    pub(crate) fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The date `days` days after 1970-01-01, where a negative value is before the epoch.
    ///
    /// The year is shifted to start in March, which puts the leap day last and makes the month
    /// lengths irrelevant to the arithmetic. This is Howard Hinnant's `civil_from_days`, and the
    /// expected dates in this module's tests were computed independently of it.
    pub(crate) fn from_days_since_epoch(days: i64) -> Self {
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        } as u32;
        let year = if month <= 2 { year + 1 } else { year };
        Self { year, month, day }
    }
}

/// Today's date in UTC.
///
/// UTC rather than local time because the value is written into a committed document: a date that
/// depends on the developer's machine makes two clones of one repository differ for no reason.
pub(crate) fn today_utc() -> Result<Date, InitError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| {
        InitError::UnusableRoot {
            path: "the system clock".to_string(),
            reason: format!("it reads before 1970 ({error})"),
        }
    })?;
    let days = i64::try_from(elapsed.as_secs() / 86_400).map_err(|_| InitError::UnusableRoot {
        path: "the system clock".to_string(),
        reason: "its value does not fit in a date".to_string(),
    })?;
    Ok(Date::from_days_since_epoch(days))
}

#[cfg(test)]
mod tests {
    use super::Date;

    #[test]
    fn the_epoch_and_its_neighbours_are_right() {
        for (days, expected) in [
            (0i64, "1970-01-01"),
            (-1, "1969-12-31"),
            (1, "1970-01-02"),
            (59, "1970-03-01"),
        ] {
            assert_eq!(Date::from_days_since_epoch(days).iso(), expected, "day {days}");
        }
    }

    #[test]
    fn a_year_boundary_rolls_over_in_both_directions() {
        assert_eq!(Date::from_days_since_epoch(364).iso(), "1970-12-31");
        assert_eq!(Date::from_days_since_epoch(365).iso(), "1971-01-01");
        assert_eq!(Date::from_days_since_epoch(-1).iso(), "1969-12-31");
    }

    #[test]
    fn every_month_boundary_of_2021_lands_where_it_should() {
        for (days, expected) in [
            (18_628i64, "2021-01-01"),
            (18_658, "2021-01-31"),
            (18_686, "2021-02-28"),
            (18_687, "2021-03-01"),
            (18_717, "2021-03-31"),
            (18_718, "2021-04-01"),
            (18_747, "2021-04-30"),
            (18_748, "2021-05-01"),
            (18_778, "2021-05-31"),
            (18_779, "2021-06-01"),
            (18_808, "2021-06-30"),
            (18_809, "2021-07-01"),
            (18_839, "2021-07-31"),
            (18_840, "2021-08-01"),
            (18_870, "2021-08-31"),
            (18_871, "2021-09-01"),
            (18_900, "2021-09-30"),
            (18_901, "2021-10-01"),
            (18_931, "2021-10-31"),
            (18_932, "2021-11-01"),
            (18_961, "2021-11-30"),
            (18_962, "2021-12-01"),
            (18_992, "2021-12-31"),
            (18_993, "2022-01-01"),
        ] {
            assert_eq!(Date::from_days_since_epoch(days).iso(), expected, "day {days}");
        }
    }

    #[test]
    fn a_leap_day_appears_only_in_a_leap_year() {
        assert_eq!(Date::from_days_since_epoch(19_781).iso(), "2024-02-28");
        assert_eq!(Date::from_days_since_epoch(19_782).iso(), "2024-02-29");
        assert_eq!(Date::from_days_since_epoch(19_783).iso(), "2024-03-01");
        assert_eq!(Date::from_days_since_epoch(20_147).iso(), "2025-02-28");
        assert_eq!(Date::from_days_since_epoch(20_148).iso(), "2025-03-01");
    }

    #[test]
    fn a_far_future_date_does_not_overflow() {
        assert_eq!(Date::from_days_since_epoch(2_932_896).iso(), "9999-12-31");
    }
}