use duckdb::types::{OrderedMap, TimeUnit, Value as Raw};
use serde_json::json;

use super::*;

/// Runs `sql` and converts the first value.
fn select(sql: &str) -> Value {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let raw: Raw = conn.query_row(sql, [], |row| row.get(0)).unwrap();
    Value::from_raw(raw)
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

#[test]
fn integers_convert_by_sign_and_width() {
    assert_eq!(Value::from_raw(Raw::TinyInt(-1)), Value::Int(-1));
    assert_eq!(Value::from_raw(Raw::SmallInt(-2)), Value::Int(-2));
    assert_eq!(Value::from_raw(Raw::Int(-3)), Value::Int(-3));
    assert_eq!(Value::from_raw(Raw::BigInt(-4)), Value::Int(-4));
    assert_eq!(Value::from_raw(Raw::UTinyInt(1)), Value::UInt(1));
    assert_eq!(Value::from_raw(Raw::USmallInt(2)), Value::UInt(2));
    assert_eq!(Value::from_raw(Raw::UInt(3)), Value::UInt(3));
    assert_eq!(Value::from_raw(Raw::UBigInt(4)), Value::UInt(4));
    assert_eq!(Value::from_raw(Raw::HugeInt(-5)), Value::HugeInt(-5));
    assert_eq!(Value::from_raw(Raw::UHugeInt(5)), Value::UHugeInt(5));
}

#[test]
fn scalars_convert() {
    assert_eq!(Value::from_raw(Raw::Null), Value::Null);
    assert_eq!(Value::from_raw(Raw::Boolean(true)), Value::Bool(true));
    assert_eq!(Value::from_raw(Raw::Float(0.5)), Value::Float(0.5));
    assert_eq!(Value::from_raw(Raw::Double(0.25)), Value::Float(0.25));
    assert_eq!(Value::from_raw(Raw::Text("a".into())), text("a"));
    assert_eq!(Value::from_raw(Raw::Enum("e".into())), text("e"));
    assert_eq!(Value::from_raw(Raw::Blob(vec![1])), Value::Blob(vec![1]));
    assert_eq!(
        Value::from_raw(Raw::Geometry(vec![2])),
        Value::Blob(vec![2])
    );
}

#[test]
fn decimals_become_exact_text() {
    assert_eq!(
        select("SELECT 12.340::DECIMAL(10,3)"),
        Value::Decimal("12.340".into())
    );
    assert_eq!(
        select("SELECT -0.05::DECIMAL(38,2)"),
        Value::Decimal("-0.05".into())
    );
}

#[test]
fn temporal_values_become_iso_text() {
    assert_eq!(
        Value::from_raw(Raw::Date32(0)),
        Value::Date("1970-01-01".into())
    );
    assert_eq!(
        Value::from_raw(Raw::Time64(TimeUnit::Microsecond, 1_500_000)),
        Value::Time("00:00:01.5".into())
    );
    assert_eq!(
        Value::from_raw(Raw::Timestamp(TimeUnit::Second, 60)),
        Value::Timestamp("1970-01-01T00:01:00".into())
    );
    assert_eq!(
        Value::from_raw(Raw::Timestamp(TimeUnit::Millisecond, 1)),
        Value::Timestamp("1970-01-01T00:00:00.001".into())
    );
    assert_eq!(
        Value::from_raw(Raw::Timestamp(TimeUnit::Nanosecond, 1)),
        Value::Timestamp("1970-01-01T00:00:00.000000001".into())
    );
    assert_eq!(
        select("SELECT TIMESTAMPTZ '2024-01-01 10:00:00+02'"),
        Value::Timestamp("2024-01-01T08:00:00".into())
    );
}

#[test]
fn intervals_keep_their_parts() {
    assert_eq!(
        select("SELECT INTERVAL '1 month 2 days 3 seconds'"),
        Value::Interval {
            months: 1,
            days: 2,
            nanos: 3_000_000_000
        }
    );
}

#[test]
fn nested_values_convert() {
    assert_eq!(
        select("SELECT [1, NULL]"),
        Value::List(vec![Value::Int(1), Value::Null])
    );
    assert_eq!(
        select("SELECT [1, 2]::INTEGER[2]"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        select("SELECT {'a': 1, 'b': 'x'}"),
        Value::Struct(vec![("a".into(), Value::Int(1)), ("b".into(), text("x"))])
    );
    assert_eq!(
        select("SELECT MAP {'k': [true]}"),
        Value::Map(vec![(text("k"), Value::List(vec![Value::Bool(true)]))])
    );
    assert_eq!(select("SELECT union_value(n := 2)"), Value::Int(2));
    assert_eq!(
        Value::from_raw(Raw::Struct(OrderedMap::from(vec![(
            "u".to_owned(),
            Raw::Union(Box::new(Raw::Null))
        )]))),
        Value::Struct(vec![("u".into(), Value::Null)])
    );
}

#[test]
fn size_counts_bytes_and_scalars() {
    assert_eq!(Value::Null.size(), 16);
    assert_eq!(Value::Bool(true).size(), 16);
    assert_eq!(Value::Int(1).size(), 16);
    assert_eq!(Value::UInt(1).size(), 16);
    assert_eq!(Value::Float(1.0).size(), 16);
    assert_eq!(Value::HugeInt(1).size(), 16);
    assert_eq!(Value::UHugeInt(1).size(), 16);
    assert_eq!(text("").size(), 16);
    assert_eq!(text("héllo").size(), 22);
    assert_eq!(Value::Decimal("1.50".into()).size(), 20);
    assert_eq!(Value::Date("1970-01-01".into()).size(), 26);
    assert_eq!(Value::Time("00:00:00".into()).size(), 24);
    assert_eq!(Value::Timestamp("x".into()).size(), 17);
    assert_eq!(Value::Blob(vec![0; 7]).size(), 23);
    let interval = Value::Interval {
        months: 0,
        days: 0,
        nanos: 0,
    };
    assert_eq!(interval.size(), 16);
}

#[test]
fn size_counts_nested_values_and_keys() {
    let list = Value::List(vec![Value::Int(1), text("ab")]);
    assert_eq!(list.size(), 50);
    let empty_items = Value::List(vec![text(""); 100]);
    assert_eq!(empty_items.size(), 16 + 1600);
    let fields = Value::Struct(vec![("key".into(), Value::Int(1))]);
    assert_eq!(fields.size(), 35);
    let map = Value::Map(vec![(text("k"), Value::Int(1))]);
    assert_eq!(map.size(), 49);
}

#[test]
fn accessors_give_typed_values() {
    assert!(Value::Null.is_null());
    assert!(!Value::Int(0).is_null());
    assert_eq!(text("a").as_str(), Some("a"));
    assert_eq!(Value::Decimal("1.5".into()).as_str(), Some("1.5"));
    assert_eq!(Value::Date("d".into()).as_str(), Some("d"));
    assert_eq!(Value::Time("t".into()).as_str(), Some("t"));
    assert_eq!(Value::Timestamp("s".into()).as_str(), Some("s"));
    assert_eq!(Value::Int(1).as_str(), None);
    assert_eq!(Value::Int(-1).as_i64(), Some(-1));
    assert_eq!(Value::UInt(2).as_i64(), Some(2));
    assert_eq!(Value::UInt(u64::MAX).as_i64(), None);
    assert_eq!(Value::HugeInt(3).as_i64(), Some(3));
    assert_eq!(Value::HugeInt(i128::MAX).as_i64(), None);
    assert_eq!(Value::UHugeInt(4).as_i64(), Some(4));
    assert_eq!(text("1").as_i64(), None);
    assert_eq!(Value::Float(0.5).as_f64(), Some(0.5));
    assert_eq!(Value::Int(2).as_f64(), Some(2.0));
    assert_eq!(Value::UInt(3).as_f64(), Some(3.0));
    assert_eq!(Value::HugeInt(4).as_f64(), Some(4.0));
    assert_eq!(Value::UHugeInt(5).as_f64(), Some(5.0));
    assert_eq!(Value::Decimal("1.25".into()).as_f64(), Some(1.25));
    assert_eq!(text("1.25").as_f64(), None);
    assert_eq!(Value::Bool(true).as_bool(), Some(true));
    assert_eq!(Value::Int(1).as_bool(), None);
}

#[test]
fn values_serialize_to_json() {
    let value = Value::Struct(vec![
        ("null".into(), Value::Null),
        ("bool".into(), Value::Bool(true)),
        ("int".into(), Value::Int(-1)),
        ("uint".into(), Value::UInt(1)),
        ("huge".into(), Value::HugeInt(-2)),
        ("uhuge".into(), Value::UHugeInt(2)),
        ("float".into(), Value::Float(0.5)),
        ("decimal".into(), Value::Decimal("1.50".into())),
        ("text".into(), text("t")),
        ("blob".into(), Value::Blob(vec![1, 2])),
        ("date".into(), Value::Date("1970-01-01".into())),
        ("time".into(), Value::Time("00:00:00".into())),
        ("ts".into(), Value::Timestamp("1970-01-01T00:00:00".into())),
        (
            "interval".into(),
            Value::Interval {
                months: 1,
                days: 2,
                nanos: 3,
            },
        ),
        ("list".into(), Value::List(vec![Value::Int(1)])),
        ("map".into(), Value::Map(vec![(text("k"), Value::Int(1))])),
    ]);
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        json!({
            "null": null, "bool": true, "int": -1, "uint": 1, "huge": -2, "uhuge": 2,
            "float": 0.5, "decimal": "1.50", "text": "t", "blob": [1, 2],
            "date": "1970-01-01", "time": "00:00:00", "ts": "1970-01-01T00:00:00",
            "interval": {"months": 1, "days": 2, "nanos": 3},
            "list": [1], "map": {"k": 1}
        })
    );
}

fn row() -> Row {
    Row::new(
        Arc::from(vec!["id".to_owned(), "name".to_owned(), "id".to_owned()]),
        vec![Value::Int(1), text("ada"), Value::Int(2)],
    )
}

#[test]
fn a_row_gives_values_by_column_name() {
    let row = row();
    assert_eq!(row.columns(), ["id", "name", "id"]);
    assert_eq!(row.values().len(), 3);
    assert_eq!(row.get("name"), Some(&text("ada")));
    assert_eq!(row.get("id"), Some(&Value::Int(1)));
    assert_eq!(row.get("missing"), None);
    assert_eq!(row.size(), 51);
    assert_eq!(row.into_values().len(), 3);
}

#[test]
fn a_row_serializes_as_a_map() {
    let row = Row::new(
        Arc::from(vec!["id".to_owned(), "name".to_owned()]),
        vec![Value::Int(1), text("ada")],
    );
    assert_eq!(
        serde_json::to_value(&row).unwrap(),
        json!({"id": 1, "name": "ada"})
    );
}

#[test]
fn a_map_with_a_nested_key_serializes_as_entries() {
    let map = Value::Map(vec![(Value::List(vec![Value::Int(1)]), text("a"))]);
    assert_eq!(
        serde_json::to_value(&map).unwrap(),
        json!([{"key": [1], "value": "a"}])
    );
    let blob_key = Value::Map(vec![(Value::Blob(vec![1]), Value::Int(2))]);
    assert_eq!(
        serde_json::to_value(&blob_key).unwrap(),
        json!([{"key": [1], "value": 2}])
    );
    let scalar_keys = Value::Map(vec![(Value::Int(1), text("a"))]);
    assert_eq!(
        serde_json::to_value(&scalar_keys).unwrap(),
        json!({"1": "a"})
    );
}
