//! Result values and rows.
//!
//! # Contract
//!
//! - Each DuckDB value converts to one [`Value`].
//! - Signed integers become [`Value::Int`] or [`Value::HugeInt`]. Unsigned integers become [`Value::UInt`] or [`Value::UHugeInt`].
//! - A decimal becomes exact text. Dates, times and timestamps become ISO 8601 text. See [`crate::temporal`].
//! - An enum becomes text. An array becomes a list. A union becomes its value. A geometry becomes a blob.
//! - An unknown future DuckDB type becomes its debug text.
//! - [`Value::size`] counts the bytes of text and blobs, 8 for a scalar and 16 for a 128-bit value.
//!   Struct keys count. The row limit and the byte limit use it.
//! - A [`Row`] serializes as a map from column name to value.

use std::sync::Arc;

use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Serialize, Serializer};

use crate::temporal::{self, Unit};

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
    /// The size of the value for the byte limit.
    #[must_use]
    pub fn size(&self) -> usize {
        match self {
            Self::Null | Self::Bool(_) => 1,
            Self::Int(_) | Self::UInt(_) | Self::Float(_) => 8,
            Self::HugeInt(_) | Self::UHugeInt(_) | Self::Interval { .. } => 16,
            Self::Decimal(v)
            | Self::Text(v)
            | Self::Date(v)
            | Self::Time(v)
            | Self::Timestamp(v) => v.len(),
            Self::Blob(v) => v.len(),
            Self::List(items) => items.iter().map(Self::size).sum(),
            Self::Struct(fields) => fields.iter().map(|(k, v)| k.len() + v.size()).sum(),
            Self::Map(entries) => entries.iter().map(|(k, v)| k.size() + v.size()).sum(),
        }
    }

    /// Returns `true` for [`Value::Null`].
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Gives the text of a text, decimal, date, time or timestamp value.
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

    /// Gives an integer value that fits in `i64`.
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

    /// Gives a number as `f64`. A decimal is parsed.
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

    /// Gives a boolean value.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }
}

impl From<duckdb::types::Value> for Value {
    fn from(raw: duckdb::types::Value) -> Self {
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
                Self::List(items.into_iter().map(Self::from).collect())
            }
            // `OrderedMap` has no owned iterator. The entries are cloned.
            Raw::Struct(fields) => Self::Struct(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::from(v.clone())))
                    .collect(),
            ),
            Raw::Map(entries) => Self::Map(
                entries
                    .iter()
                    .map(|(k, v)| (Self::from(k.clone()), Self::from(v.clone())))
                    .collect(),
            ),
            Raw::Union(inner) => Self::from(*inner),
            other => Self::Text(format!("{other:?}")),
        }
    }
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
            Self::Map(entries) => {
                let mut out = serializer.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    out.serialize_entry(k, v)?;
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

    /// Gives the values, in column order.
    #[must_use]
    pub fn into_values(self) -> Vec<Value> {
        self.values
    }

    /// Gives the value of the first column with the name `column`.
    #[must_use]
    pub fn get(&self, column: &str) -> Option<&Value> {
        let index = self.columns.iter().position(|name| name == column)?;
        self.values.get(index)
    }

    /// Gives the size of the row for the byte limit.
    #[must_use]
    pub fn size(&self) -> usize {
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
