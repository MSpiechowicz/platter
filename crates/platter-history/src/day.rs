//! Local calendar days. A day is a number (days since 1970-01-01 in local time) and a text
//! (`YYYY-MM-DD`), which sorts the same way.

/// The local day number of `at` (Unix seconds) in a zone `offset` seconds ahead of UTC.
pub fn day_number(at: i64, offset: i64) -> i64 {
    at.saturating_add(offset).div_euclid(86_400)
}

/// `YYYY-MM-DD` of a day number.
pub fn day_text(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);

    format!("{year:04}-{month:02}-{date:02}")
}

/// The day number of `YYYY-MM-DD`; `None` for anything else.
pub fn parse_day(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }

    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let date: i64 = text.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&date) {
        return None;
    }

    Some(days_from_civil(year, month, date))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_number_follows_the_local_clock_not_utc() {
        // 2026-11-17 23:30 UTC is already 2026-11-18 in Warsaw (UTC+1).
        let at = 1_794_958_200;

        assert_eq!(day_text(day_number(at, 0)), "2026-11-17");
        assert_eq!(day_text(day_number(at, 3600)), "2026-11-18");
    }

    #[test]
    fn text_and_number_round_trip_across_leap_years() {
        for text in [
            "1970-01-01",
            "2024-02-29",
            "2026-11-17",
            "2100-03-01",
            "1969-12-31",
        ] {
            assert_eq!(day_text(parse_day(text).unwrap()), text);
        }
        assert_eq!(
            parse_day("1970-01-02").unwrap() - parse_day("1970-01-01").unwrap(),
            1
        );
        assert_eq!(
            parse_day("2024-03-01").unwrap() - parse_day("2024-02-28").unwrap(),
            2
        );
    }

    #[test]
    fn anything_but_a_date_is_not_a_day() {
        for text in [
            "",
            "2026-1-17",
            "2026/11/17",
            "2026-13-01",
            "2026-11-00",
            "yesterday!",
        ] {
            assert_eq!(parse_day(text), None, "{text:?}");
        }
    }
}
