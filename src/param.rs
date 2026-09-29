//! The bound value type.
//!
//! # Contract
//!
//! - Each Rust scalar converts to one [`Param`]. `None` converts to [`Param::Null`].
//! - A [`Param`] binds as the DuckDB value of the same type.
//! - DuckDB casts text to the parameter type. Bind a date as `"2024-01-31"`.
//! - Lists, structs and maps do not bind. `duckdb-rs` does not support them.

use duckdb::ToSql;
use duckdb::types::ToSqlOutput;

/// A value to bind to a `?` or `$1` parameter.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Param {
    /// SQL `NULL`.
    Null,
    /// A `BOOLEAN`.
    Bool(bool),
    /// A `BIGINT`.
    Int(i64),
    /// A `UBIGINT`.
    UInt(u64),
    /// A `HUGEINT`.
    HugeInt(i128),
    /// A `DOUBLE`.
    Float(f64),
    /// A `VARCHAR`.
    Text(String),
    /// A `BLOB`.
    Blob(Vec<u8>),
}

impl ToSql for Param {
    fn to_sql(&self) -> duckdb::Result<ToSqlOutput<'_>> {
        use duckdb::types::{Value, ValueRef};
        Ok(match self {
            Self::Null => ToSqlOutput::Owned(Value::Null),
            Self::Bool(v) => ToSqlOutput::Owned(Value::Boolean(*v)),
            Self::Int(v) => ToSqlOutput::Owned(Value::BigInt(*v)),
            Self::UInt(v) => ToSqlOutput::Owned(Value::UBigInt(*v)),
            Self::HugeInt(v) => ToSqlOutput::Owned(Value::HugeInt(*v)),
            Self::Float(v) => ToSqlOutput::Owned(Value::Double(*v)),
            Self::Text(v) => ToSqlOutput::Borrowed(ValueRef::Text(v.as_bytes())),
            Self::Blob(v) => ToSqlOutput::Borrowed(ValueRef::Blob(v)),
        })
    }
}

/// Implements `From` for types that convert with a variant and a cast.
macro_rules! from_scalar {
    ($variant:ident($target:ty): $($source:ty),+) => {
        $(
            impl From<$source> for Param {
                fn from(value: $source) -> Self {
                    Self::$variant(<$target>::from(value))
                }
            }
        )+
    };
}

from_scalar!(Bool(bool): bool);
from_scalar!(Int(i64): i8, i16, i32, i64);
from_scalar!(UInt(u64): u8, u16, u32, u64);
from_scalar!(HugeInt(i128): i128);
from_scalar!(Float(f64): f32, f64);
from_scalar!(Text(String): String, &str, &String);
from_scalar!(Blob(Vec<u8>): Vec<u8>, &[u8]);

impl<T: Into<Self>> From<Option<T>> for Param {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

#[cfg(test)]
mod tests;
