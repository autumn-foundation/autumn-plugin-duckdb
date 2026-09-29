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

/// The seconds in one day.
const DAY_SECONDS: i128 = 86_400;

/// The days from `0000-03-01` to `1970-01-01`.
const EPOCH_SHIFT: i64 = 719_468;

impl Unit {
    /// The units in one second.
    const fn per_second(self) -> i128 {
        match self {
            Self::Seconds => 1,
            Self::Millis => 1_000,
            Self::Micros => 1_000_000,
            Self::Nanos => 1_000_000_000,
        }
    }
}

/// Returns the date text for `days` from `1970-01-01`.
pub(crate) fn date_text(days: i32) -> String {
    match days {
        i32::MAX => "infinity".to_owned(),
        d if d == -i32::MAX => "-infinity".to_owned(),
        d => civil_text(i64::from(d)),
    }
}

/// Returns the time text for `value` units from midnight.
pub(crate) fn time_text(unit: Unit, value: i64) -> String {
    let (seconds, nanos) = split(unit, value);
    clock_text(seconds, nanos)
}

/// Returns the timestamp text for `value` units from the Unix epoch.
pub(crate) fn timestamp_text(unit: Unit, value: i64) -> String {
    match value {
        i64::MAX => "infinity".to_owned(),
        v if v == -i64::MAX => "-infinity".to_owned(),
        v => {
            let (seconds, nanos) = split(unit, v);
            let days = seconds.div_euclid(DAY_SECONDS);
            let clock = clock_text(seconds.rem_euclid(DAY_SECONDS), nanos);
            // An `i64` count of seconds gives fewer than 2^47 days.
            let days = i64::try_from(days).unwrap_or_default();
            format!("{}T{clock}", civil_text(days))
        }
    }
}

/// Splits a count into whole seconds and nanoseconds from 0 to 999 999 999.
fn split(unit: Unit, value: i64) -> (i128, u32) {
    let per = unit.per_second();
    let value = i128::from(value);
    let nanos = value.rem_euclid(per) * (1_000_000_000 / per);
    (
        value.div_euclid(per),
        u32::try_from(nanos).unwrap_or_default(),
    )
}

/// Returns `HH:MM:SS` and a fraction without trailing zeros.
fn clock_text(seconds: i128, nanos: u32) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    let mut text = format!("{h:02}:{m:02}:{s:02}");
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text
}

/// Returns the Gregorian date text for `days` from `1970-01-01`.
///
/// The algorithm is `civil_from_days` by Howard Hinnant.
fn civil_text(days: i64) -> String {
    let z = days + EPOCH_SHIFT;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let year = match year {
        0..=9999 => format!("{year:04}"),
        y if y > 9999 => format!("+{y}"),
        y => format!("-{:04}", y.unsigned_abs()),
    };
    format!("{year}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests;
