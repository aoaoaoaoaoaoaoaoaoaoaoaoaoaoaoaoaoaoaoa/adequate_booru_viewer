use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

const UNIX_EPOCH_DAY: i32 = 719_468;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CreatedDay(u32);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DateRange {
    pub first: Option<CreatedDay>,
    pub last: Option<CreatedDay>,
}

impl CreatedDay {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let head = raw.get(..10)?;
        let bytes = head.as_bytes();
        if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
            return None;
        }
        let year = digits(head.get(..4)?)? as i32;
        let month = digits(head.get(5..7)?)?;
        let day = digits(head.get(8..10)?)?;
        Self::from_ymd(year, month, day)
    }

    pub fn parse_iso(raw: &str) -> Option<Self> {
        Self::parse(raw)
    }

    fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        let days = days_from_civil(year, month, day);
        (days >= 0).then_some(Self(days as u32))
    }

    pub fn get(self) -> u32 {
        self.0
    }

    pub fn from_unix_days(days: u32) -> Self {
        Self(days)
    }

    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0 as i32)
    }

    pub fn succ(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    pub fn pred(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl Display for CreatedDay {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let (year, month, day) = self.ymd();
        write!(f, "{year:04}-{month:02}-{day:02}")
    }
}

impl TryFrom<String> for CreatedDay {
    type Error = &'static str;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw).ok_or("invalid date")
    }
}

impl From<CreatedDay> for String {
    fn from(day: CreatedDay) -> Self {
        day.to_string()
    }
}

impl DateRange {
    pub fn normalized(self) -> Self {
        match (self.first, self.last) {
            (Some(first), Some(last)) if first > last => Self {
                first: Some(last),
                last: Some(first),
            },
            _ => self,
        }
    }

    pub fn active(self) -> bool {
        self.first.is_some() || self.last.is_some()
    }
}

fn digits(raw: &str) -> Option<u32> {
    raw.bytes().try_fold(0_u32, |acc, byte| {
        byte.is_ascii_digit()
            .then_some(acc * 10 + u32::from(byte - b'0'))
    })
}

fn leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    let year = year - i32::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - UNIX_EPOCH_DAY
}

fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + UNIX_EPOCH_DAY;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let day = doy - (153 * mp + 2).div_euclid(5) + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i32::from(month <= 2), month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for raw in ["1970-01-01", "1999-12-31", "2024-02-29", "2026-06-13"] {
            let day = CreatedDay::parse(raw).expect("parse date");
            assert_eq!(day.to_string(), raw);
        }
        assert!(CreatedDay::parse("2023-02-29").is_none());
    }

    #[test]
    fn range_normalizes_and_contains() {
        let early = CreatedDay::parse("2024-01-01").expect("early");
        let late = CreatedDay::parse("2024-12-31").expect("late");
        let range = DateRange {
            first: Some(late),
            last: Some(early),
        }
        .normalized();
        assert_eq!(range.first, Some(early));
        assert_eq!(range.last, Some(late));
    }
}
