//! Serde helpers for timestamps and dates so JSON on disk matches Python's
//! `datetime.isoformat()` / `date.isoformat()` output closely enough for
//! Python's `fromisoformat()` to parse it back.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

/// `DateTime<Utc>` as an RFC3339 string with a `+00:00` offset (matching
/// Python's `datetime.isoformat()` for a UTC-aware datetime) rather than the
/// `Z` suffix chrono defaults to.
pub mod rfc3339 {
    use super::*;

    pub fn serialize<S>(dt: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&to_python_isoformat(dt))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_python_isoformat(&s).map_err(D::Error::custom)
    }
}

/// Same as [`rfc3339`] but for `Option<DateTime<Utc>>`.
pub mod rfc3339_option {
    use super::*;

    pub fn serialize<S>(dt: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match dt {
            Some(dt) => serializer.serialize_str(&to_python_isoformat(dt)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            Some(s) => parse_python_isoformat(&s)
                .map(Some)
                .map_err(D::Error::custom),
            None => Ok(None),
        }
    }
}

pub fn to_python_isoformat(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
        .replace('Z', "+00:00")
}

pub fn parse_python_isoformat(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn formats_like_python_isoformat_without_fraction() {
        let dt = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
        assert_eq!(to_python_isoformat(&dt), "2026-05-03T12:00:00+00:00");
    }

    #[test]
    fn parses_z_and_offset_suffixes() {
        let a = parse_python_isoformat("2026-05-03T12:00:00Z").unwrap();
        let b = parse_python_isoformat("2026-05-03T12:00:00+00:00").unwrap();
        assert_eq!(a, b);
    }
}
