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
//! - An error names the column. The plugin messages never show a value, because values can hold personal data.
//!   A custom `Deserialize` impl can put a value in its own message.

use serde::de::value::{BorrowedStrDeserializer, MapDeserializer, SeqDeserializer};
use serde::de::{
    DeserializeSeed, Deserializer, Error as _, IntoDeserializer, MapAccess, SeqAccess, Unexpected,
    Visitor,
};
use serde::{Deserialize, forward_to_deserialize_any};

use crate::value::{Row, Value};

/// A row or a value does not decode into the requested type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{message}", column_text(.column.as_deref()))]
pub struct DecodeError {
    column: Option<String>,
    message: String,
}

fn column_text(column: Option<&str>) -> String {
    column.map_or_else(String::new, |name| format!("column `{name}`: "))
}

impl DecodeError {
    /// The column that did not decode, if known.
    #[must_use]
    pub fn column(&self) -> Option<&str> {
        self.column.as_deref()
    }

    /// Adds the column name if the error has none.
    fn in_column(mut self, name: &str) -> Self {
        if self.column.is_none() {
            self.column = Some(name.to_owned());
        }
        self
    }
}

/// Names the kind of a value. The name never shows the value.
const fn kind(unexpected: &Unexpected<'_>) -> &'static str {
    match unexpected {
        Unexpected::Bool(_) => "a boolean",
        Unexpected::Unsigned(_) | Unexpected::Signed(_) => "an integer",
        Unexpected::Float(_) => "a float",
        Unexpected::Char(_) | Unexpected::Str(_) => "text",
        Unexpected::Bytes(_) => "bytes",
        Unexpected::Unit => "null",
        Unexpected::Seq => "a list",
        Unexpected::Map => "a map",
        _ => "a value",
    }
}

impl serde::de::Error for DecodeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self {
            column: None,
            message: msg.to_string(),
        }
    }

    fn invalid_type(unexpected: Unexpected<'_>, expected: &dyn serde::de::Expected) -> Self {
        Self::custom(format_args!(
            "invalid type: {}, expected {expected}",
            kind(&unexpected)
        ))
    }

    fn invalid_value(unexpected: Unexpected<'_>, expected: &dyn serde::de::Expected) -> Self {
        Self::custom(format_args!(
            "invalid value: {}, expected {expected}",
            kind(&unexpected)
        ))
    }

    fn unknown_field(_field: &str, expected: &'static [&'static str]) -> Self {
        Self::custom(format_args!("unknown field, expected one of {expected:?}"))
    }

    fn unknown_variant(_variant: &str, expected: &'static [&'static str]) -> Self {
        Self::custom(format_args!(
            "unknown variant, expected one of {expected:?}"
        ))
    }
}

impl Row {
    /// Decodes the row into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if a column does not match `T`.
    pub fn decode<'de, T: Deserialize<'de>>(&'de self) -> Result<T, DecodeError> {
        T::deserialize(RowDe(self))
    }
}

impl Value {
    /// Decodes the value into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] if the value does not match `T`.
    pub fn decode<'de, T: Deserialize<'de>>(&'de self) -> Result<T, DecodeError> {
        T::deserialize(self)
    }
}

impl<'de> IntoDeserializer<'de, DecodeError> for &'de Value {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

/// Visits a sequence and checks that the visitor reads each item.
fn visit_seq<'de, I, V>(items: I, visitor: V) -> Result<V::Value, DecodeError>
where
    I: Iterator,
    I::Item: IntoDeserializer<'de, DecodeError>,
    V: Visitor<'de>,
{
    let mut seq = SeqDeserializer::new(items);
    let value = visitor.visit_seq(&mut seq)?;
    seq.end()?;
    Ok(value)
}

/// Visits a map and checks that the visitor reads each entry.
fn visit_map<'de, I, K, W, V>(entries: I, visitor: V) -> Result<V::Value, DecodeError>
where
    I: Iterator<Item = (K, W)>,
    K: IntoDeserializer<'de, DecodeError>,
    W: IntoDeserializer<'de, DecodeError>,
    V: Visitor<'de>,
{
    let mut map = MapDeserializer::new(entries);
    let value = visitor.visit_map(&mut map)?;
    map.end()?;
    Ok(value)
}

/// Decodes a whole decimal into an integer. Other values use `deserialize_any`.
macro_rules! integers {
    ($($method:ident)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
                match self {
                    Value::Decimal(text) => visit_whole(text, visitor),
                    _ => self.deserialize_any(visitor),
                }
            }
        )*
    };
}

/// Visits a decimal with no fraction as an integer. The error does not show the text.
fn visit_whole<'de, V: Visitor<'de>>(text: &str, visitor: V) -> Result<V::Value, DecodeError> {
    let whole = match text.split_once('.') {
        Some((whole, fraction)) if fraction.bytes().all(|b| b == b'0') => whole,
        Some(_) => return Err(DecodeError::custom("the decimal has a fraction")),
        None => text,
    };
    let value: i128 = whole
        .parse()
        .map_err(|_| DecodeError::custom("the decimal is not an integer"))?;
    match (i64::try_from(value), u64::try_from(value)) {
        (Ok(small), _) => visitor.visit_i64(small),
        (_, Ok(small)) => visitor.visit_u64(small),
        _ => visitor.visit_i128(value),
    }
}

impl<'de> Deserializer<'de> for &'de Value {
    type Error = DecodeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self {
            Value::Null => visitor.visit_unit(),
            Value::Bool(v) => visitor.visit_bool(*v),
            Value::Int(v) => visitor.visit_i64(*v),
            Value::UInt(v) => visitor.visit_u64(*v),
            // Serde visitors for 64-bit types refuse 128-bit visits. `SUM` gives a `HUGEINT`.
            Value::HugeInt(v) => match (i64::try_from(*v), u64::try_from(*v)) {
                (Ok(small), _) => visitor.visit_i64(small),
                (_, Ok(small)) => visitor.visit_u64(small),
                _ => visitor.visit_i128(*v),
            },
            Value::UHugeInt(v) => match u64::try_from(*v) {
                Ok(small) => visitor.visit_u64(small),
                Err(_) => visitor.visit_u128(*v),
            },
            Value::Float(v) => visitor.visit_f64(*v),
            Value::Decimal(v)
            | Value::Text(v)
            | Value::Date(v)
            | Value::Time(v)
            | Value::Timestamp(v) => visitor.visit_borrowed_str(v),
            Value::Blob(v) => visitor.visit_borrowed_bytes(v),
            Value::Interval {
                months,
                days,
                nanos,
            } => visit_map(
                [
                    ("months", i64::from(*months)),
                    ("days", i64::from(*days)),
                    ("nanos", *nanos),
                ]
                .into_iter(),
                visitor,
            ),
            Value::List(items) => visit_seq(items.iter(), visitor),
            Value::Struct(fields) => {
                visit_map(fields.iter().map(|(k, v)| (k.as_str(), v)), visitor)
            }
            Value::Map(entries) => visit_map(entries.iter().map(|(k, v)| (k, v)), visitor),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self {
            Value::Decimal(v) => visitor.visit_f64(parse_decimal(v)?),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        self.deserialize_f64(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self {
            Value::Blob(v) => visit_seq(v.iter().copied(), visitor),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        match self {
            Value::Text(v) => visitor.visit_enum(v.as_str().into_deserializer()),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        visitor.visit_unit()
    }

    integers! {
        deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64 deserialize_i128
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64 deserialize_u128
    }

    forward_to_deserialize_any! {
        bool char str string bytes byte_buf unit unit_struct tuple tuple_struct map struct identifier
    }
}

/// Parses decimal text. The error does not show the text.
fn parse_decimal(text: &str) -> Result<f64, DecodeError> {
    text.parse()
        .map_err(|_| DecodeError::custom("the decimal is not a number"))
}

/// Decodes a row. See the module contract.
struct RowDe<'de>(&'de Row);

impl<'de> RowDe<'de> {
    /// Decodes the columns as map entries, by name.
    fn by_name<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        visitor.visit_map(Columns::new(self.0))
    }

    /// Decodes the columns as sequence items, by position. The length must match.
    fn by_position<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        let mut columns = Columns::new(self.0);
        let value = visitor.visit_seq(&mut columns)?;
        match columns.remaining() {
            0 => Ok(value),
            _ => Err(DecodeError::invalid_length(
                self.0.values().len(),
                &"fewer columns",
            )),
        }
    }

    /// Gives the only value of a row with one column.
    fn only(&self) -> Result<(&'de str, &'de Value), DecodeError> {
        match (self.0.columns(), self.0.values()) {
            ([name], [value]) => Ok((name.as_str(), value)),
            (columns, _) => Err(DecodeError::custom(format_args!(
                "the row has {} columns: a scalar needs one column",
                columns.len()
            ))),
        }
    }
}

/// Decodes the only column of the row with a `deserialize_*` method.
macro_rules! only_column {
    ($($method:ident)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
                let (name, value) = self.only()?;
                value.$method(visitor).map_err(|err| err.in_column(name))
            }
        )*
    };
}

impl<'de> Deserializer<'de> for RowDe<'de> {
    type Error = DecodeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        self.by_name(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match (self.0.columns(), self.0.values()) {
            ([name], [value @ (Value::Map(_) | Value::Struct(_))]) => value
                .deserialize_map(visitor)
                .map_err(|err| err.in_column(name)),
            _ => self.by_name(visitor),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        self.by_name(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match (self.0.columns(), self.0.values()) {
            ([name], [value @ Value::List(_)]) => value
                .deserialize_seq(visitor)
                .map_err(|err| err.in_column(name)),
            _ => self.by_position(visitor),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        self.by_position(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        self.by_position(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DecodeError> {
        match self.0.values() {
            [Value::Null] => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        let (column, value) = self.only()?;
        value
            .deserialize_enum(name, variants, visitor)
            .map_err(|err| err.in_column(column))
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DecodeError> {
        self.deserialize_unit(visitor)
    }

    only_column! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_i128 deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_u128 deserialize_f32 deserialize_f64 deserialize_char deserialize_str
        deserialize_string deserialize_bytes deserialize_byte_buf deserialize_unit
        deserialize_identifier deserialize_ignored_any
    }
}

/// Reads the columns of a row as map entries or as sequence items.
struct Columns<'de> {
    row: &'de Row,
    next: usize,
}

impl<'de> Columns<'de> {
    const fn new(row: &'de Row) -> Self {
        Self { row, next: 0 }
    }

    fn remaining(&self) -> usize {
        self.row.values().len().saturating_sub(self.next)
    }

    /// Decodes the next value and adds the column name to an error.
    fn value<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, DecodeError> {
        let index = self.next;
        self.next += 1;
        let name = self.row.columns().get(index).map_or("", String::as_str);
        let value = self
            .row
            .values()
            .get(index)
            .ok_or_else(|| DecodeError::custom("the row has no more columns"))?;
        seed.deserialize(value).map_err(|err| err.in_column(name))
    }
}

impl<'de> MapAccess<'de> for Columns<'de> {
    type Error = DecodeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DecodeError> {
        self.row
            .columns()
            .get(self.next)
            .map(|name| seed.deserialize(BorrowedStrDeserializer::new(name)))
            .transpose()
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<T::Value, DecodeError> {
        self.value(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining())
    }
}

impl<'de> SeqAccess<'de> for Columns<'de> {
    type Error = DecodeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DecodeError> {
        if self.remaining() == 0 {
            return Ok(None);
        }
        self.value(seed).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining())
    }
}

#[cfg(test)]
mod tests;
