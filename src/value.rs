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
        0
    }

    /// Returns `true` for [`Value::Null`].
    #[must_use]
    pub const fn is_null(&self) -> bool {
        false
    }

    /// Gives the text of a text, decimal, date, time or timestamp value.
    #[must_use]
    pub const fn as_str(&self) -> Option<&str> {
        None
    }

    /// Gives an integer value that fits in `i64`.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        None
    }

    /// Gives a number as `f64`. A decimal is parsed.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        None
    }

    /// Gives a boolean value.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        None
    }
}

impl From<duckdb::types::Value> for Value {
    fn from(raw: duckdb::types::Value) -> Self {
        let _ = raw;
        Self::Null
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_unit()
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
        let _ = column;
        None
    }

    /// Gives the size of the row for the byte limit.
    #[must_use]
    pub fn size(&self) -> usize {
        0
    }
}

impl Serialize for Row {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_unit()
    }
}

#[cfg(test)]
mod tests;
