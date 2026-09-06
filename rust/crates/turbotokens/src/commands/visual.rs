//! Shared helpers for the visual commands (`heatmap`, `wrapped`): civil date
//! math (std-only, no calendar crate on these paths) and SVG text escaping.

/// A civil day, stored as days since the Unix epoch.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Day(i64);

impl Day {
    pub(super) fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self(days_from_civil(year, month, day)))
    }

    /// Parses a `YYYY-MM-DD` date, the format daily aggregates carry.
    pub(super) fn parse(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        Self::from_ymd(
            parse_digits(&bytes[0..4])? as i32,
            parse_digits(&bytes[5..7])?,
            parse_digits(&bytes[8..10])?,
        )
    }

    pub(super) fn checked_add(self, days: i64) -> Option<Self> {
        self.0.checked_add(days).map(Self)
    }

    pub(super) fn days_since(self, earlier: Self) -> i64 {
        self.0 - earlier.0
    }

    /// 0 = Sunday .. 6 = Saturday.
    pub(super) fn weekday(self) -> usize {
        (self.0 + 4).rem_euclid(7) as usize
    }

    pub(super) fn month(self) -> u32 {
        civil_from_days(self.0).1
    }

    pub(super) fn to_ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0)
    }

    pub(super) fn format(self) -> String {
        let (year, month, day) = self.to_ymd();
        format!("{year:04}-{month:02}-{day:02}")
    }

    /// The compact `YYYYMMDD` form the shared `--since`/`--until` bounds use.
    pub(super) fn format_compact(self) -> String {
        self.format().replace('-', "")
    }

    pub(super) fn month_name(self) -> &'static str {
        MONTH_NAMES[(self.month() - 1) as usize]
    }
}

pub(super) const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(super) const WEEKDAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    let mut value: u32 = 0;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    Some(value)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

/// Escapes the five XML special characters for text interpolated into SVG.
pub(super) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_days() {
        let day = Day::parse("2026-09-02").unwrap();
        assert_eq!(day.to_ymd(), (2026, 9, 2));
        assert_eq!(day.format(), "2026-09-02");
        assert_eq!(day.format_compact(), "20260902");
        assert_eq!(day.weekday(), 3); // Wednesday
        assert_eq!(day.month_name(), "Sep");
        assert_eq!(Day::parse("2026-1-2"), None);
        assert_eq!(Day::parse("2026-02-30"), None);
    }

    #[test]
    fn adds_days_across_month_and_year_bounds() {
        let day = Day::parse("2026-12-31").unwrap().checked_add(1).unwrap();
        assert_eq!(day.format(), "2027-01-01");

        // 2024 is a leap year.
        let day = Day::parse("2024-02-28").unwrap().checked_add(1).unwrap();
        assert_eq!(day.format(), "2024-02-29");
    }

    #[test]
    fn weekday_matches_known_dates() {
        assert_eq!(Day::parse("2026-09-06").unwrap().weekday(), 0); // Sunday
        assert_eq!(Day::parse("1970-01-01").unwrap().weekday(), 4); // Thursday
    }

    #[test]
    fn escapes_xml_specials() {
        assert_eq!(
            xml_escape("a<b>&\"'\""),
            "a&lt;b&gt;&amp;&quot;&apos;&quot;"
        );
    }
}
