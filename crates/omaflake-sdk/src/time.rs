// SPDX-License-Identifier: AGPL-3.0-or-later
//! Times as omaflake's files and its API write them: UTC, ISO 8601 in whole seconds (`iso8601`) or milliseconds
//! (`iso8601_fractional`), as the Swift app writes them.

/// A point in time (UTC) – jiff's `Timestamp` (plan B167: replaces chrono's `DateTime<Utc>`)
pub type Date = jiff::Timestamp;

/// The current time in whole seconds – what survives a round trip through a Swift `.iso8601` file
pub fn now() -> Date {
    whole_seconds(Date::now())
}

/// `d` without its fraction of a second
pub fn whole_seconds(d: Date) -> Date {
    Date::from_second(d.as_second()).unwrap_or(d)
}

/// `d` cut to milliseconds
pub fn whole_millis(d: Date) -> Date {
    Date::from_millisecond(d.as_millisecond()).unwrap_or(d)
}

/// Whole seconds from `earlier` to `later` (negative when `later` is earlier)
pub fn seconds_between(later: Date, earlier: Date) -> i64 {
    later.as_second() - earlier.as_second()
}

/// A file time (`SystemTime`) as a `Date`
pub fn from_system_time(t: std::time::SystemTime) -> Option<Date> {
    Date::try_from(t).ok()
}

/// `d` in this computer's time zone, formatted with a strftime pattern (`%Y-%m-%d %H:%M`)
pub fn local(d: Date, pattern: &str) -> String {
    d.to_zoned(jiff::tz::TimeZone::system()).strftime(pattern).to_string()
}

/// Now in this computer's time zone, formatted with a strftime pattern (log lines, file names)
pub fn local_now(pattern: &str) -> String {
    local(Date::now(), pattern)
}

/// Parses an ISO 8601 date as Swift's `ISO8601DateFormatter` writes it (with or without fractional seconds)
pub fn parse_date(text: &str) -> Option<Date> {
    text.parse::<Date>().ok()
}

/// Swift's `.iso8601` date strategy: `2026-10-21T10:00:00Z` (whole seconds, UTC). Use as
/// `#[serde(with = "omaflake_sdk::time::iso8601")]`; `iso8601::option` for `Option<Date>`.
pub mod iso8601 {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn format(d: &Date) -> String {
        d.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    pub fn serialize<S: Serializer>(d: &Date, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format(d))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Date, D::Error> {
        let text = String::deserialize(d)?;
        parse_date(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("Expected date string to be ISO8601-formatted: {text}")))
    }

    /// For `Option<Date>` fields (add `#[serde(default, skip_serializing_if = "Option::is_none")]`)
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(d: &Option<Date>, s: S) -> std::result::Result<S::Ok, S::Error> {
            match d {
                Some(d) => s.serialize_str(&format(d)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<Date>, D::Error> {
            match Option::<String>::deserialize(d)? {
                None => Ok(None),
                Some(text) => parse_date(&text).map(Some).ok_or_else(|| {
                    serde::de::Error::custom(format!("Expected date string to be ISO8601-formatted: {text}"))
                }),
            }
        }
    }
}

/// The date strategy of RecipeSources.swift's `.installed.json`: written with milliseconds
/// (`.withFractionalSeconds`, so versions of a line keep their order, plan B111), read with or without them
pub mod iso8601_fractional {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn format(d: &Date) -> String {
        d.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
    }

    pub fn serialize<S: Serializer>(d: &Date, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format(d))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Date, D::Error> {
        let text = String::deserialize(d)?;
        parse_date(&text).ok_or_else(|| serde::de::Error::custom(format!("date {text}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_as_swift_writes_them() {
        let d = parse_date("2026-10-21T10:00:00Z").unwrap();
        assert_eq!(iso8601::format(&d), "2026-10-21T10:00:00Z");
        let f = parse_date("2026-10-18T17:13:59.619Z").unwrap();
        assert_eq!(iso8601_fractional::format(&f), "2026-10-18T17:13:59.619Z");
        assert_eq!(iso8601::format(&f), "2026-10-18T17:13:59Z");
        assert!(parse_date("2026-10-18").is_none());
        assert_eq!(now().subsec_nanosecond(), 0);
    }
}
