use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Deserialize;

use crate::value::{Row, Value};

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn row(pairs: &[(&str, Value)]) -> Row {
    let columns: Vec<String> = pairs.iter().map(|(k, _)| (*k).to_owned()).collect();
    let values = pairs.iter().map(|(_, v)| v.clone()).collect();
    Row::new(Arc::from(columns), values)
}

#[derive(Debug, Deserialize, PartialEq)]
struct User {
    id: i32,
    name: String,
    email: Option<String>,
}

#[test]
fn a_row_decodes_into_a_struct_by_column_name() {
    let row = row(&[
        ("name", text("ada")),
        ("extra", Value::Bool(true)),
        ("id", Value::Int(7)),
        ("email", Value::Null),
    ]);
    assert_eq!(
        row.decode::<User>().unwrap(),
        User {
            id: 7,
            name: "ada".into(),
            email: None
        }
    );
}

#[test]
fn a_row_decodes_into_borrowed_text() {
    #[derive(Deserialize)]
    struct Name<'a> {
        name: &'a str,
    }
    let row = row(&[("name", text("ada"))]);
    assert_eq!(row.decode::<Name<'_>>().unwrap().name, "ada");
}

#[test]
fn a_missing_column_gives_an_error() {
    let row = row(&[("id", Value::Int(1))]);
    let err = row.decode::<User>().unwrap_err();
    assert!(err.to_string().contains("name"), "{err}");
}

#[test]
fn a_row_decodes_into_a_tuple_by_position() {
    let row = row(&[("a", Value::Int(1)), ("b", text("x"))]);
    assert_eq!(row.decode::<(i64, String)>().unwrap(), (1, "x".into()));
    assert!(row.decode::<(i64,)>().is_err());
    assert!(row.decode::<(i64, String, bool)>().is_err());
}

#[test]
fn a_row_decodes_into_a_map() {
    let row = row(&[("a", Value::Int(1)), ("b", Value::Int(2))]);
    let map: BTreeMap<String, i64> = row.decode().unwrap();
    assert_eq!(map, BTreeMap::from([("a".into(), 1), ("b".into(), 2)]));
}

#[test]
fn a_one_column_row_decodes_into_a_scalar() {
    assert_eq!(row(&[("n", Value::HugeInt(42))]).decode::<i64>().unwrap(), 42);
    assert_eq!(row(&[("s", text("x"))]).decode::<String>().unwrap(), "x");
    assert_eq!(row(&[("n", Value::Null)]).decode::<Option<i64>>().unwrap(), None);
    assert_eq!(row(&[("n", Value::Int(1))]).decode::<Option<i64>>().unwrap(), Some(1));
    let two = row(&[("a", Value::Int(1)), ("b", Value::Int(2))]);
    assert!(two.decode::<i64>().is_err());
}

#[test]
fn an_option_of_a_struct_decodes_from_a_row() {
    let row = row(&[("id", Value::Int(1)), ("name", text("a")), ("email", text("e"))]);
    assert!(row.decode::<Option<User>>().unwrap().is_some());
}

#[test]
fn numbers_decode_into_rust_numbers() {
    assert_eq!(Value::Int(-1).decode::<i8>().unwrap(), -1);
    assert_eq!(Value::UInt(255).decode::<u8>().unwrap(), 255);
    assert_eq!(Value::HugeInt(5).decode::<i64>().unwrap(), 5);
    assert_eq!(Value::UHugeInt(6).decode::<u32>().unwrap(), 6);
    assert_eq!(Value::HugeInt(i128::MIN).decode::<i128>().unwrap(), i128::MIN);
    assert_eq!(Value::Int(2).decode::<f64>().unwrap(), 2.0);
    assert_eq!(Value::Float(0.5).decode::<f32>().unwrap(), 0.5);
    assert!(Value::Int(300).decode::<u8>().is_err());
    assert!(Value::Int(-1).decode::<u64>().is_err());
}

#[test]
fn decimals_decode_into_floats_and_text() {
    let decimal = Value::Decimal("12.50".into());
    assert_eq!(decimal.decode::<f64>().unwrap(), 12.5);
    assert_eq!(decimal.decode::<f32>().unwrap(), 12.5);
    assert_eq!(decimal.decode::<String>().unwrap(), "12.50");
}

#[test]
fn text_decodes_into_strings_and_enums() {
    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum Mood {
        Happy,
        Sad,
    }
    assert_eq!(text("sad").decode::<Mood>().unwrap(), Mood::Sad);
    assert!(text("angry").decode::<Mood>().is_err());
    assert_eq!(Value::Date("2024-01-01".into()).decode::<String>().unwrap(), "2024-01-01");
    assert_eq!(Value::Time("10:00:00".into()).decode::<String>().unwrap(), "10:00:00");
    assert_eq!(Value::Timestamp("t".into()).decode::<String>().unwrap(), "t");
    assert!(Value::Int(1).decode::<Mood>().is_err());
    let _ = Mood::Happy;
}

#[test]
fn blobs_decode_into_bytes() {
    assert_eq!(Value::Blob(vec![1, 2]).decode::<Vec<u8>>().unwrap(), vec![1, 2]);
}

#[test]
fn null_decodes_into_none_and_unit() {
    Value::Null.decode::<()>().unwrap();
    assert_eq!(Value::Null.decode::<Option<String>>().unwrap(), None);
    assert!(Value::Null.decode::<String>().is_err());
    assert!(Value::Int(1).decode::<()>().is_err());
}

#[test]
fn nested_values_decode() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Point {
        x: i64,
        y: i64,
    }
    let list = Value::List(vec![Value::Int(1), Value::Int(2)]);
    assert_eq!(list.decode::<Vec<i64>>().unwrap(), vec![1, 2]);
    assert!(list.decode::<(i64,)>().is_err());
    let point = Value::Struct(vec![("x".into(), Value::Int(1)), ("y".into(), Value::Int(2))]);
    assert_eq!(point.decode::<Point>().unwrap(), Point { x: 1, y: 2 });
    let fields: HashMap<String, i64> = point.decode().unwrap();
    assert_eq!(fields.len(), 2);
    let map = Value::Map(vec![(text("k"), Value::Bool(true))]);
    let decoded: BTreeMap<String, bool> = map.decode().unwrap();
    assert_eq!(decoded, BTreeMap::from([("k".into(), true)]));
}

#[test]
fn intervals_decode_into_a_struct() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Interval {
        months: i32,
        days: i32,
        nanos: i64,
    }
    let value = Value::Interval {
        months: 1,
        days: 2,
        nanos: 3,
    };
    assert_eq!(
        value.decode::<Interval>().unwrap(),
        Interval {
            months: 1,
            days: 2,
            nanos: 3
        }
    );
}

#[test]
fn newtypes_and_ignored_values_decode() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Id(i64);
    #[derive(Debug, Deserialize, PartialEq)]
    struct Only {
        id: Id,
    }
    let row = row(&[("id", Value::Int(3)), ("skip", Value::List(vec![]))]);
    assert_eq!(row.decode::<Only>().unwrap(), Only { id: Id(3) });
    assert_eq!(Value::Int(4).decode::<Id>().unwrap(), Id(4));
}

#[test]
fn an_error_names_the_column_and_hides_the_value() {
    let row = row(&[("id", Value::Int(1)), ("name", text("secret@example.com")), ("email", Value::Null)]);
    #[derive(Debug, Deserialize)]
    struct Wrong {
        #[allow(dead_code)]
        name: i64,
    }
    let err = row.decode::<Wrong>().unwrap_err();
    assert_eq!(err.column(), Some("name"));
    let message = err.to_string();
    assert!(message.contains("`name`"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn an_unknown_variant_error_hides_the_value() {
    #[derive(Debug, Deserialize)]
    enum Kind {
        A,
    }
    let err = text("secret").decode::<Kind>().unwrap_err();
    assert!(!err.to_string().contains("secret"), "{err}");
    let _ = Kind::A;
}

#[test]
fn an_out_of_range_error_hides_the_value() {
    let err = Value::Int(123_456).decode::<u8>().unwrap_err();
    assert!(!err.to_string().contains("123456"), "{err}");
    let err = Value::Decimal("9e999x".into()).decode::<f64>().unwrap_err();
    assert!(!err.to_string().contains("9e999x"), "{err}");
}
