//! Date, time and timestamp text.
//!
//! # Contract
//!
//! - A date is a count of days from `1970-01-01`. The text is `YYYY-MM-DD` in the proleptic Gregorian calendar.
//! - A year from 0 to 9999 has four digits. A larger year has a `+` sign. A negative year has a `-` sign.
//! - A time is a count of units from midnight. The text is `HH:MM:SS`, then a fraction if it is not zero.
//! - The fraction has no trailing zeros.
//! - A timestamp is a count of units from `1970-01-01T00:00:00` UTC. The text is `<date>T<time>`.
//! - The DuckDB infinity values give `infinity` and `-infinity`.

/// The unit of a time or timestamp count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Seconds,
    Millis,
    Micros,
    Nanos,
}

/// Gives the date text for `days` from `1970-01-01`.
pub(crate) fn date_text(days: i32) -> String {
    let _ = days;
    String::new()
}

/// Gives the time text for `value` units from midnight.
pub(crate) fn time_text(unit: Unit, value: i64) -> String {
    let _ = (unit, value);
    String::new()
}

/// Gives the timestamp text for `value` units from the Unix epoch.
pub(crate) fn timestamp_text(unit: Unit, value: i64) -> String {
    let _ = (unit, value);
    String::new()
}

#[cfg(test)]
mod tests;
