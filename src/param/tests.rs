use duckdb::types::Value as Raw;

use super::*;

/// Binds `param` in `SELECT ?` and gives the DuckDB value.
fn round_trip(param: impl Into<Param>) -> Raw {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.query_row("SELECT ?", [param.into()], |row| row.get(0))
        .unwrap()
}

#[test]
fn integers_convert() {
    assert_eq!(Param::from(-1_i8), Param::Int(-1));
    assert_eq!(Param::from(-2_i16), Param::Int(-2));
    assert_eq!(Param::from(-3_i32), Param::Int(-3));
    assert_eq!(Param::from(-4_i64), Param::Int(-4));
    assert_eq!(Param::from(1_u8), Param::UInt(1));
    assert_eq!(Param::from(2_u16), Param::UInt(2));
    assert_eq!(Param::from(3_u32), Param::UInt(3));
    assert_eq!(Param::from(4_u64), Param::UInt(4));
    assert_eq!(Param::from(i128::MAX), Param::HugeInt(i128::MAX));
}

#[test]
fn other_scalars_convert() {
    assert_eq!(Param::from(true), Param::Bool(true));
    assert_eq!(Param::from(1.5_f32), Param::Float(1.5));
    assert_eq!(Param::from(2.5_f64), Param::Float(2.5));
    assert_eq!(Param::from("a"), Param::Text("a".into()));
    assert_eq!(Param::from(String::from("b")), Param::Text("b".into()));
    assert_eq!(Param::from(&String::from("c")), Param::Text("c".into()));
    assert_eq!(Param::from(vec![1_u8]), Param::Blob(vec![1]));
    assert_eq!(Param::from(&[2_u8][..]), Param::Blob(vec![2]));
}

#[test]
fn options_convert() {
    assert_eq!(Param::from(None::<i64>), Param::Null);
    assert_eq!(Param::from(Some("x")), Param::Text("x".into()));
}

#[test]
fn params_bind_as_the_same_duckdb_type() {
    assert_eq!(round_trip(Param::Null), Raw::Null);
    assert_eq!(round_trip(true), Raw::Boolean(true));
    assert_eq!(round_trip(-7_i64), Raw::BigInt(-7));
    assert_eq!(round_trip(u64::MAX), Raw::UBigInt(u64::MAX));
    assert_eq!(round_trip(i128::MIN + 1), Raw::HugeInt(i128::MIN + 1));
    assert_eq!(round_trip(0.25_f64), Raw::Double(0.25));
    assert_eq!(round_trip("it's"), Raw::Text("it's".into()));
    assert_eq!(round_trip(vec![0_u8, 255]), Raw::Blob(vec![0, 255]));
}

#[test]
fn duckdb_casts_text_to_the_parameter_type() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let raw: Raw = conn
        .query_row("SELECT ?::DATE + 1", [Param::from("2024-02-28")], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(raw, Raw::Date32(19_782));
}
