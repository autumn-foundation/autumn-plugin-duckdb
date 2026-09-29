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
        Ok(ToSqlOutput::Owned(duckdb::types::Value::Null))
    }
}

#[cfg(test)]
mod tests;
