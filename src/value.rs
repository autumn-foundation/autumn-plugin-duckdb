//! Result values and rows.
//!
//! # Contract
//!
//! - Each DuckDB value converts to one [`Value`].
//! - Signed integers become [`Value::Int`] or [`Value::HugeInt`]. Unsigned integers become [`Value::UInt`] or [`Value::UHugeInt`].
//! - A decimal becomes exact text. Dates, times and timestamps become ISO 8601 text. See [`crate::temporal`].
//! - An enum becomes text. An array becomes a list. A union becomes its value. A geometry becomes a blob.
//! - An unknown future DuckDB type becomes its debug text.
//! - [`Value::size`] is an estimate in bytes. Each value counts 16. Text and blobs add their bytes.
//!   Lists, structs and maps add their items. Struct keys add their bytes. The byte limit uses it.
//! - A [`Row`] serializes as a map from column name to value.
//! - A map serializes as a map if each key is a scalar. Else it serializes as a list of `{key, value}` entries.

use std::sync::Arc;

use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Serialize, Serializer};

use duckdb::core::{LogicalTypeHandle, LogicalTypeId};

use crate::temporal::{self, Unit};

/// The bytes that each value counts for the byte limit, also an empty or null value.
const VALUE_BYTES: usize = 16;

/// A result value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// SQL `NULL`.
    Null,
    /// A `BOOLEAN`.
    Bool(bool),
    /// A signed integer up to `BIGINT`.
    Int(i64),
    /// An unsigned integer up to `UBIGINT`.
    UInt(u64),
    /// A `HUGEINT`.
    HugeInt(i128),
    /// A `UHUGEINT`.
    UHugeInt(u128),
    /// A `FLOAT` or a `DOUBLE`.
    Float(f64),
    /// A `DECIMAL` as exact text, for example `"12.340"`.
    Decimal(String),
    /// A `VARCHAR`, an `ENUM` or a `UUID`.
    Text(String),
    /// A `BLOB`, a `BIT` or a `GEOMETRY`.
    Blob(Vec<u8>),
    /// A `DATE` as `YYYY-MM-DD`.
    Date(String),
    /// A `TIME` as `HH:MM:SS` with an optional fraction.
    Time(String),
    /// A `TIMESTAMP` as `YYYY-MM-DDTHH:MM:SS` with an optional fraction, in UTC.
    Timestamp(String),
    /// An `INTERVAL`.
    Interval {
        /// The months.
        months: i32,
        /// The days.
        days: i32,
        /// The nanoseconds.
        nanos: i64,
    },
    /// A `LIST` or an `ARRAY`.
    List(Vec<Self>),
    /// A `STRUCT`, in field order.
    Struct(Vec<(String, Self)>),
    /// A `MAP`, in entry order.
    Map(Vec<(Self, Self)>),
}

impl Value {
    /// The estimated size in bytes for the byte limit. See the module contract.
    pub(crate) fn size(&self) -> usize {
        let content = match self {
            Self::Decimal(v)
            | Self::Text(v)
            | Self::Date(v)
            | Self::Time(v)
            | Self::Timestamp(v) => v.len(),
            Self::Blob(v) => v.len(),
            Self::List(items) => items.iter().map(Self::size).sum(),
            Self::Struct(fields) => fields.iter().map(|(k, v)| k.len() + v.size()).sum(),
            Self::Map(entries) => entries.iter().map(|(k, v)| k.size() + v.size()).sum(),
            _ => 0,
        };
        VALUE_BYTES + content
    }

    /// Returns `true` for [`Value::Null`].
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Returns the text of a text, decimal, date, time or timestamp value.
    #[must_use]
    pub const fn as_str(&self) -> Option<&str> {
        match self {
            Self::Decimal(v)
            | Self::Text(v)
            | Self::Date(v)
            | Self::Time(v)
            | Self::Timestamp(v) => Some(v.as_str()),
            _ => None,
        }
    }

    /// Returns an integer value that fits in `i64`.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Int(v) => Some(v),
            Self::UInt(v) => i64::try_from(v).ok(),
            Self::HugeInt(v) => i64::try_from(v).ok(),
            Self::UHugeInt(v) => i64::try_from(v).ok(),
            _ => None,
        }
    }

    /// Returns a number as `f64`. The method parses a decimal.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "the caller asks for an f64")]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(*v),
            Self::Int(v) => Some(*v as f64),
            Self::UInt(v) => Some(*v as f64),
            Self::HugeInt(v) => Some(*v as f64),
            Self::UHugeInt(v) => Some(*v as f64),
            Self::Decimal(v) => v.parse().ok(),
            _ => None,
        }
    }

    /// Returns a boolean value.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }
}

impl Value {
    /// Converts a DuckDB value without its column type. See the module contract.
    pub(crate) fn from_raw(raw: duckdb::types::Value) -> Self {
        use duckdb::types::Value as Raw;
        match raw {
            Raw::Null => Self::Null,
            Raw::Boolean(v) => Self::Bool(v),
            Raw::TinyInt(v) => Self::Int(v.into()),
            Raw::SmallInt(v) => Self::Int(v.into()),
            Raw::Int(v) => Self::Int(v.into()),
            Raw::BigInt(v) => Self::Int(v),
            Raw::HugeInt(v) => Self::HugeInt(v),
            Raw::UTinyInt(v) => Self::UInt(v.into()),
            Raw::USmallInt(v) => Self::UInt(v.into()),
            Raw::UInt(v) => Self::UInt(v.into()),
            Raw::UBigInt(v) => Self::UInt(v),
            Raw::UHugeInt(v) => Self::UHugeInt(v),
            Raw::Float(v) => Self::Float(v.into()),
            Raw::Double(v) => Self::Float(v),
            Raw::Decimal(v) => Self::Decimal(v.to_string()),
            Raw::Timestamp(unit, v) => Self::Timestamp(temporal::timestamp_text(unit_of(unit), v)),
            Raw::Text(v) | Raw::Enum(v) => Self::Text(v),
            Raw::Blob(v) | Raw::Geometry(v) => Self::Blob(v),
            Raw::Date32(v) => Self::Date(temporal::date_text(v)),
            Raw::Time64(unit, v) => Self::Time(temporal::time_text(unit_of(unit), v)),
            Raw::Interval {
                months,
                days,
                nanos,
            } => Self::Interval {
                months,
                days,
                nanos,
            },
            Raw::List(items) | Raw::Array(items) => {
                Self::List(items.into_iter().map(Self::from_raw).collect())
            }
            // `OrderedMap` has no owned iterator. The entries are cloned.
            Raw::Struct(fields) => Self::Struct(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::from_raw(v.clone())))
                    .collect(),
            ),
            Raw::Map(entries) => Self::Map(
                entries
                    .iter()
                    .map(|(k, v)| (Self::from_raw(k.clone()), Self::from_raw(v.clone())))
                    .collect(),
            ),
            Raw::Union(inner) => Self::from_raw(*inner),
            other => Self::Text(format!("{other:?}")),
        }
    }
}

/// How to read the values of one result type.
///
/// `duckdb-rs` loses some facts in nested values. The shape keeps them from the column type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Use the plain conversion.
    Plain,
    /// A `TIMESTAMPTZ`: the text ends with `Z`.
    TimestampTz,
    /// A `DECIMAL` with a scale.
    Decimal(u8),
    /// A `UHUGEINT`. A nested value arrives as a `HUGEINT` with the same bits.
    UHugeInt,
    /// A `LIST` or an `ARRAY`.
    List(Box<Self>),
    /// A `STRUCT`, in field order.
    Struct(Vec<Self>),
    /// A `MAP`: the key shape and the value shape.
    Map(Box<Self>, Box<Self>),
}

/// Returns the shape of a column type, or the name of a type that the plugin refuses.
///
/// The refused types lose data or panic in `duckdb-rs`.
pub(crate) fn shape_of(ty: &LogicalTypeHandle) -> Result<Shape, &'static str> {
    Ok(match ty.id() {
        LogicalTypeId::TimeNs => return Err("TIME_NS"),
        LogicalTypeId::TimeTZ => return Err("TIMETZ"),
        LogicalTypeId::Bit => return Err("BIT"),
        LogicalTypeId::Bignum => return Err("BIGNUM"),
        LogicalTypeId::Variant => return Err("VARIANT"),
        LogicalTypeId::Unsupported => return Err("an unknown type"),
        LogicalTypeId::TimestampTZ => Shape::TimestampTz,
        LogicalTypeId::Decimal => Shape::Decimal(ty.decimal_scale()),
        LogicalTypeId::UHugeint => Shape::UHugeInt,
        LogicalTypeId::List | LogicalTypeId::Array => {
            Shape::List(Box::new(shape_of(&ty.child(0))?))
        }
        LogicalTypeId::Struct => Shape::Struct(
            (0..ty.num_children())
                .map(|index| shape_of(&ty.child(index)))
                .collect::<Result<_, _>>()?,
        ),
        LogicalTypeId::Map => Shape::Map(
            Box::new(shape_of(&ty.child(0))?),
            Box::new(shape_of(&ty.child(1))?),
        ),
        LogicalTypeId::Union => {
            for index in 0..ty.num_children() {
                shape_of(&ty.child(index))?;
            }
            Shape::Plain
        }
        _ => Shape::Plain,
    })
}

impl Value {
    /// Converts a DuckDB value with the facts of its shape.
    pub(crate) fn from_shaped(raw: duckdb::types::Value, shape: &Shape) -> Self {
        use duckdb::types::Value as Raw;
        match (raw, shape) {
            (Raw::Timestamp(unit, v), Shape::TimestampTz) => {
                let text = temporal::timestamp_text(unit_of(unit), v);
                if text.ends_with("infinity") {
                    Self::Timestamp(text)
                } else {
                    Self::Timestamp(text + "Z")
                }
            }
            (Raw::HugeInt(v), Shape::Decimal(scale)) => Self::Decimal(decimal_text(v, *scale)),
            #[allow(clippy::cast_sign_loss, reason = "the bits are a UHUGEINT")]
            (Raw::HugeInt(v), Shape::UHugeInt) => Self::UHugeInt(v as u128),
            (Raw::List(items) | Raw::Array(items), Shape::List(item)) => Self::List(
                items
                    .into_iter()
                    .map(|raw| Self::from_shaped(raw, item))
                    .collect(),
            ),
            // `OrderedMap` has no owned iterator. The code clones the entries.
            (Raw::Struct(fields), Shape::Struct(shapes)) => Self::Struct(
                fields
                    .iter()
                    .zip(shapes.iter().chain(std::iter::repeat(&Shape::Plain)))
                    .map(|((k, v), shape)| (k.clone(), Self::from_shaped(v.clone(), shape)))
                    .collect(),
            ),
            (Raw::Map(entries), Shape::Map(key, value)) => Self::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        (
                            Self::from_shaped(k.clone(), key),
                            Self::from_shaped(v.clone(), value),
                        )
                    })
                    .collect(),
            ),
            (raw, _) => Self::from_raw(raw),
        }
    }
}

/// Returns the text of `value` with `scale` decimal places.
fn decimal_text(value: i128, scale: u8) -> String {
    duckdb::types::Decimal::new(38, scale, value)
        .map_or_else(|_| value.to_string(), |decimal| decimal.to_string())
}

/// Maps the `duckdb-rs` time unit.
const fn unit_of(unit: duckdb::types::TimeUnit) -> Unit {
    use duckdb::types::TimeUnit;
    match unit {
        TimeUnit::Second => Unit::Seconds,
        TimeUnit::Millisecond => Unit::Millis,
        TimeUnit::Microsecond => Unit::Micros,
        TimeUnit::Nanosecond => Unit::Nanos,
    }
}

/// One map entry with a key that JSON cannot use as an object key.
#[derive(Serialize)]
struct Entry<'a> {
    key: &'a Value,
    value: &'a Value,
}

impl Value {
    /// Returns `true` for a value that serializes as text or a number. JSON can use it as a key.
    const fn is_scalar_key(&self) -> bool {
        !matches!(
            self,
            Self::Null
                | Self::Blob(_)
                | Self::Interval { .. }
                | Self::List(_)
                | Self::Struct(_)
                | Self::Map(_)
        )
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(v) => serializer.serialize_bool(*v),
            Self::Int(v) => serializer.serialize_i64(*v),
            Self::UInt(v) => serializer.serialize_u64(*v),
            Self::HugeInt(v) => serializer.serialize_i128(*v),
            Self::UHugeInt(v) => serializer.serialize_u128(*v),
            Self::Float(v) => serializer.serialize_f64(*v),
            Self::Decimal(v)
            | Self::Text(v)
            | Self::Date(v)
            | Self::Time(v)
            | Self::Timestamp(v) => serializer.serialize_str(v),
            Self::Blob(v) => serializer.collect_seq(v),
            Self::Interval {
                months,
                days,
                nanos,
            } => {
                let mut out = serializer.serialize_struct("Interval", 3)?;
                out.serialize_field("months", months)?;
                out.serialize_field("days", days)?;
                out.serialize_field("nanos", nanos)?;
                out.end()
            }
            Self::List(items) => {
                let mut out = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    out.serialize_element(item)?;
                }
                out.end()
            }
            Self::Struct(fields) => serializer.collect_map(fields.iter().map(|(k, v)| (k, v))),
            Self::Map(entries) if entries.iter().all(|(k, _)| k.is_scalar_key()) => {
                let mut out = serializer.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    out.serialize_entry(k, v)?;
                }
                out.end()
            }
            Self::Map(entries) => {
                let mut out = serializer.serialize_seq(Some(entries.len()))?;
                for (key, value) in entries {
                    out.serialize_element(&Entry { key, value })?;
                }
                out.end()
            }
        }
    }
}

/// One result row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    columns: Arc<[String]>,
    values: Vec<Value>,
}

impl Row {
    /// Makes a row. `columns` and `values` have the same length.
    pub(crate) const fn new(columns: Arc<[String]>, values: Vec<Value>) -> Self {
        Self { columns, values }
    }

    /// The column names.
    #[must_use]
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The values, in column order.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Returns the values and drops the column names.
    #[must_use]
    pub fn into_values(self) -> Vec<Value> {
        self.values
    }

    /// Returns the value of the first column with the name `column`.
    #[must_use]
    pub fn get(&self, column: &str) -> Option<&Value> {
        let index = self.columns.iter().position(|name| name == column)?;
        self.values.get(index)
    }

    /// The estimated size in bytes for the byte limit.
    pub(crate) fn size(&self) -> usize {
        self.values.iter().map(Value::size).sum()
    }
}

impl Serialize for Row {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.columns.iter().zip(&self.values))
    }
}

#[cfg(test)]
mod tests;
