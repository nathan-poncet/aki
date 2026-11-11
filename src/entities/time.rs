//! A point in time, as seconds since the Unix epoch.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Parses the timestamps the forges emit: `2026-04-02T11:21:30Z`,
    /// `2026-04-02T11:21:30.123+02:00`, and Jira's `...+0200`.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' {
            return None;
        }
        let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
        let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
        let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }

        let mut rest = &text[19..];
        if let Some(fraction) = rest.strip_prefix('.') {
            let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
            rest = &fraction[digits..];
        }
        let offset = match rest {
            "" | "Z" | "z" => 0,
            zone => {
                let sign = match zone.as_bytes()[0] {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
                if digits.len() != 4 {
                    return None;
                }
                let hours: i64 = digits[..2].parse().ok()?;
                let minutes: i64 = digits[2..].parse().ok()?;
                sign * (hours * 3600 + minutes * 60)
            }
        };

        let days = days_from_civil(year, month, day);
        Some(Self(
            days * 86_400 + hour * 3600 + minute * 60 + second - offset,
        ))
    }

    /// Whole seconds from `self` to `later`, never negative.
    pub fn until(self, later: Timestamp) -> i64 {
        (later.0 - self.0).max(0)
    }

    pub fn days_until(self, later: Timestamp) -> u32 {
        (self.until(later) / 86_400) as u32
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc() {
        assert_eq!(Timestamp::parse("1970-01-01T00:00:00Z"), Some(Timestamp(0)));
        assert_eq!(
            Timestamp::parse("2026-04-02T11:21:30Z"),
            Some(Timestamp(1_775_128_890))
        );
    }

    #[test]
    fn parses_fractions_and_offsets() {
        let utc = Timestamp::parse("2026-04-02T11:21:30Z").unwrap();
        assert_eq!(Timestamp::parse("2026-04-02T13:21:30.123+02:00"), Some(utc));
        assert_eq!(Timestamp::parse("2026-04-02T06:21:30.000-0500"), Some(utc));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Timestamp::parse("yesterday"), None);
        assert_eq!(Timestamp::parse("2026-13-02T11:21:30Z"), None);
    }
}
