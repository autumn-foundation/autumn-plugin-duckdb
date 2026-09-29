use proptest::prelude::*;

use super::*;

/// The first day of year 1 and the last day of year 9999.
const YEAR_1: i32 = -719_162;
const YEAR_9999_END: i32 = 2_932_896;
const DAY_MICROS: i64 = 86_400_000_000;

thread_local! {
    /// One oracle database for each test thread.
    static ORACLE: duckdb::Connection = duckdb::Connection::open_in_memory().unwrap();
}

/// Returns the DuckDB text for `value` with `sql`.
fn cast(sql: &str, value: i64) -> String {
    ORACLE.with(|conn| conn.query_row(sql, [value], |row| row.get(0)).unwrap())
}

#[test]
fn the_epoch_is_1970_01_01() {
    assert_eq!(date_text(0), "1970-01-01");
}

#[test]
fn dates_give_iso_text() {
    assert_eq!(date_text(-1), "1969-12-31");
    assert_eq!(date_text(19_782), "2024-02-29");
    assert_eq!(date_text(YEAR_1), "0001-01-01");
    assert_eq!(date_text(YEAR_9999_END), "9999-12-31");
}

#[test]
fn years_outside_four_digits_have_a_sign() {
    assert_eq!(date_text(YEAR_1 - 1), "0000-12-31");
    assert_eq!(date_text(YEAR_1 - 367), "-0001-12-31");
    assert_eq!(date_text(YEAR_9999_END + 1), "+10000-01-01");
}

#[test]
fn infinite_dates_give_words() {
    assert_eq!(date_text(i32::MAX), "infinity");
    assert_eq!(date_text(-i32::MAX), "-infinity");
}

#[test]
fn times_give_text_with_a_short_fraction() {
    assert_eq!(time_text(Unit::Micros, 0), "00:00:00");
    assert_eq!(time_text(Unit::Micros, 3_723_500_000), "01:02:03.5");
    assert_eq!(time_text(Unit::Micros, DAY_MICROS - 1), "23:59:59.999999");
    assert_eq!(time_text(Unit::Micros, DAY_MICROS), "24:00:00");
    assert_eq!(time_text(Unit::Nanos, 1), "00:00:00.000000001");
    assert_eq!(time_text(Unit::Millis, 1_001), "00:00:01.001");
    assert_eq!(time_text(Unit::Seconds, 59), "00:00:59");
}

#[test]
fn timestamps_join_the_date_and_the_time_with_t() {
    assert_eq!(timestamp_text(Unit::Micros, 0), "1970-01-01T00:00:00");
    assert_eq!(
        timestamp_text(Unit::Micros, -1),
        "1969-12-31T23:59:59.999999"
    );
    assert_eq!(
        timestamp_text(Unit::Seconds, 1_709_164_800),
        "2024-02-29T00:00:00"
    );
    assert_eq!(timestamp_text(Unit::Millis, 1_500), "1970-01-01T00:00:01.5");
    assert_eq!(
        timestamp_text(Unit::Nanos, 1_000_000_123),
        "1970-01-01T00:00:01.000000123"
    );
}

#[test]
fn infinite_timestamps_give_words() {
    assert_eq!(timestamp_text(Unit::Micros, i64::MAX), "infinity");
    assert_eq!(timestamp_text(Unit::Micros, -i64::MAX), "-infinity");
}

#[test]
fn extreme_timestamps_do_not_overflow() {
    for unit in [Unit::Seconds, Unit::Millis, Unit::Micros, Unit::Nanos] {
        assert!(!timestamp_text(unit, i64::MIN).is_empty());
        assert!(!timestamp_text(unit, i64::MAX - 1).is_empty());
        assert!(!time_text(unit, i64::MIN).is_empty());
        assert!(!time_text(unit, i64::MAX).is_empty());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn date_text_equals_the_duckdb_cast(days in YEAR_1..=YEAR_9999_END) {
        let expected = cast("SELECT CAST(DATE '1970-01-01' + ?::INTEGER AS VARCHAR)", i64::from(days));
        prop_assert_eq!(date_text(days), expected);
    }

    #[test]
    fn time_text_equals_the_duckdb_cast(micros in 0..DAY_MICROS) {
        let expected = cast("SELECT CAST(TIME '00:00:00' + to_microseconds(?::BIGINT) AS VARCHAR)", micros);
        prop_assert_eq!(time_text(Unit::Micros, micros), expected);
    }

    #[test]
    fn timestamp_text_equals_the_duckdb_cast(
        micros in i64::from(YEAR_1) * DAY_MICROS..(i64::from(YEAR_9999_END) + 1) * DAY_MICROS
    ) {
        let expected = cast("SELECT CAST(make_timestamp(?::BIGINT) AS VARCHAR)", micros);
        prop_assert_eq!(timestamp_text(Unit::Micros, micros), expected.replacen(' ', "T", 1));
    }
}

#[test]
fn century_leap_rules_hold() {
    assert_eq!(date_text(-25_508), "1900-03-01");
    assert_eq!(date_text(11_016), "2000-02-29");
    assert_eq!(date_text(47_541), "2100-03-01");
}

#[test]
fn extreme_timestamps_give_exact_text() {
    assert_eq!(
        timestamp_text(Unit::Micros, -i64::MAX + 1),
        "-290308-12-21T19:59:05.224194"
    );
    assert_eq!(
        timestamp_text(Unit::Micros, i64::MAX - 1),
        "+294247-01-10T04:00:54.775806"
    );
}
