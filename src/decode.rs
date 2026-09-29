//! Serde decoding of rows and values.
//!
//! # Contract
//!
//! - A row decodes into a struct or a map by column name. Extra columns are ignored.
//! - A row decodes into a tuple or a sequence by column position. The length must match.
//! - A row with one column also decodes into the type of that column, for example `i64`.
//! - `NULL` decodes into `None` or `()`.
//! - Numbers decode into each Rust number type that holds them. A decimal decodes into `f64`, `f32` or `String`.
//! - Text decodes into `String`, `&str` or a unit enum variant. A blob decodes into `Vec<u8>`.
//! - Lists decode into sequences. Structs and maps decode into structs and maps.
//! - An error names the column. An error never shows a value, because values can hold personal data.

use serde::Deserialize;

use crate::value::{Row, Value};

/// A row or a value does not decode into the requested type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DecodeError {
    column: Option<String>,
    message: String,
}

impl DecodeError {
    /// The column that did not decode, if known.
    #[must_use]
    pub fn column(&self) -> Option<&str> {
        self.column.as_deref()
    }
}

impl serde::de::Error for DecodeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self {
            column: None,
            message: msg.to_string(),
        }
    }
}

impl Row {
    /// Decodes the row into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if a column does not match `T`.
    pub fn decode<'de, T: Deserialize<'de>>(&'de self) -> Result<T, DecodeError> {
        Err(serde::de::Error::custom("not done"))
    }
}

impl Value {
    /// Decodes the value into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the value does not match `T`.
    pub fn decode<'de, T: Deserialize<'de>>(&'de self) -> Result<T, DecodeError> {
        Err(serde::de::Error::custom("not done"))
    }
}

#[cfg(test)]
mod tests;
