//! The public error type.
//!
//! # Contract
//!
//! - A DuckDB message starts with its class, for example `Catalog Error: ...`. The class is the text before ` Error:`.
//! - A message with no class gives the class `Unknown`. A `duckdb-rs` error that is not a DuckDB failure gives `Client`.
//! - The error text shows the class only. [`DuckDbError::detail`] gives the full message.
//!   A DuckDB message can hold SQL text and key values.
//! - No rows gives HTTP 404. A constraint error gives HTTP 409. A timeout gives 504. A write conflict, a cancel and a shutdown give 503.
//!   All other errors give 500.
//! - A write conflict is retryable. A timeout is not: the query can have changed data before the interrupt.

use std::time::Duration;

use autumn_web::AutumnError;
use http::StatusCode;

use crate::config::ConfigError;
use crate::decode::DecodeError;

/// An error from the plugin.
///
/// The debug text also hides the DuckDB message.
#[derive(Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum DuckDbError {
    /// The configuration is not valid.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// DuckDB refused the call.
    ///
    /// The text does not show `detail`, because it can hold SQL text and key values.
    #[error("DuckDB refused the call: {class} error")]
    #[non_exhaustive]
    Database {
        /// The DuckDB error class, for example `Catalog` or `Constraint`.
        class: String,
        /// The full DuckDB message.
        detail: String,
    },
    /// The SQL text has more than one statement.
    #[error("the SQL has {statements} statements: send one statement in each query")]
    #[non_exhaustive]
    MultipleStatements {
        /// The statements in the SQL text.
        statements: usize,
    },
    /// The SQL text ends in an open literal or block comment. The plugin cannot count its statements.
    #[error("the SQL ends in an open literal or comment")]
    OpenLiteral,
    /// A result column has a type that the plugin cannot read without data loss.
    #[error("column `{column}` has the type {type_name}: cast it to VARCHAR in the SQL")]
    #[non_exhaustive]
    UnsupportedType {
        /// The column name.
        column: String,
        /// The DuckDB type name.
        type_name: String,
    },
    /// The number of parameters is not the number of placeholders.
    #[error("the SQL has {placeholders} placeholders, but the query has {parameters} parameters")]
    #[non_exhaustive]
    ParameterCount {
        /// The placeholders in the SQL.
        placeholders: usize,
        /// The bound parameters.
        parameters: usize,
    },
    /// The call did not complete in time. If the query started, the plugin interrupts it.
    ///
    /// The query can have changed data before the interrupt.
    #[error("the call did not complete in {timeout:?}")]
    #[non_exhaustive]
    Timeout {
        /// The timeout.
        timeout: Duration,
    },
    /// An interrupt stopped the query. The interrupt did not come from a timeout or a shutdown.
    #[error("an interrupt stopped the call")]
    Cancelled,
    /// The result has more rows than the limit.
    #[error("the query returned more than {limit} rows")]
    #[non_exhaustive]
    TooManyRows {
        /// The row limit.
        limit: usize,
    },
    /// The values of the result have more bytes than the limit.
    #[error("the query returned more than {limit_bytes} bytes")]
    #[non_exhaustive]
    ResultTooLarge {
        /// The byte limit.
        limit_bytes: usize,
    },
    /// The query returned no rows, and the caller needs one.
    #[error("the query returned no rows")]
    NotFound,
    /// The app shuts down. The plugin starts no new calls.
    #[error("the app shuts down: the DuckDB plugin starts no new calls")]
    ShuttingDown,
    /// A result value does not decode.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The blocking task of a call stopped. For example, the closure of `with_connection` panicked.
    #[error("the DuckDB task stopped before it gave a result")]
    TaskFailed,
    /// The app does not have the plugin.
    #[error("the DuckDB plugin is not installed: add `DuckDbPlugin` to the app")]
    NotInstalled,
}

impl std::fmt::Debug for DuckDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(err) => f.debug_tuple("Config").field(err).finish(),
            Self::Database { class, detail } => f
                .debug_struct("Database")
                .field("class", class)
                .field("detail_bytes", &detail.len())
                .finish(),
            Self::MultipleStatements { statements } => f
                .debug_struct("MultipleStatements")
                .field("statements", statements)
                .finish(),
            Self::OpenLiteral => f.write_str("OpenLiteral"),
            Self::UnsupportedType { column, type_name } => f
                .debug_struct("UnsupportedType")
                .field("column", column)
                .field("type_name", type_name)
                .finish(),
            Self::ParameterCount {
                placeholders,
                parameters,
            } => f
                .debug_struct("ParameterCount")
                .field("placeholders", placeholders)
                .field("parameters", parameters)
                .finish(),
            Self::Timeout { timeout } => {
                f.debug_struct("Timeout").field("timeout", timeout).finish()
            }
            Self::Cancelled => f.write_str("Cancelled"),
            Self::TooManyRows { limit } => {
                f.debug_struct("TooManyRows").field("limit", limit).finish()
            }
            Self::ResultTooLarge { limit_bytes } => f
                .debug_struct("ResultTooLarge")
                .field("limit_bytes", limit_bytes)
                .finish(),
            Self::NotFound => f.write_str("NotFound"),
            Self::ShuttingDown => f.write_str("ShuttingDown"),
            Self::Decode(err) => f.debug_tuple("Decode").field(err).finish(),
            Self::TaskFailed => f.write_str("TaskFailed"),
            Self::NotInstalled => f.write_str("NotInstalled"),
        }
    }
}

/// Returns the class of a DuckDB message.
pub(crate) fn class_of(message: &str) -> &str {
    let first_line = message.lines().next().unwrap_or_default();
    first_line
        .split_once(" Error:")
        .map(|(class, _)| class)
        .filter(|class| {
            !class.is_empty()
                && class
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b' ')
        })
        .unwrap_or("Unknown")
}

impl From<duckdb::Error> for DuckDbError {
    fn from(err: duckdb::Error) -> Self {
        let class = match &err {
            duckdb::Error::DuckDBFailure(_, Some(message)) => class_of(message),
            duckdb::Error::DuckDBFailure(_, None) => "Unknown",
            _ => "Client",
        };
        Self::Database {
            class: class.to_owned(),
            detail: err.to_string(),
        }
    }
}

impl DuckDbError {
    /// The full DuckDB message of a [`DuckDbError::Database`] error.
    ///
    /// The message can hold SQL text and key values. Do not show it to users.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Database { detail, .. } => Some(detail),
            _ => None,
        }
    }

    /// The DuckDB error class of a [`DuckDbError::Database`] error.
    #[must_use]
    pub fn class(&self) -> Option<&str> {
        match self {
            Self::Database { class, .. } => Some(class),
            _ => None,
        }
    }

    /// Returns `true` for a DuckDB write conflict. Another transaction changed the same rows.
    fn is_conflict(&self) -> bool {
        matches!(self, Self::Database { class, detail }
            if class == "TransactionContext" && detail.to_ascii_lowercase().contains("conflict"))
    }

    /// Returns `true` if a retry of the same call can succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.is_conflict()
    }

    /// The HTTP status for this error.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Cancelled | Self::ShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
            Self::Database { class, .. } if class == "Constraint" => StatusCode::CONFLICT,
            _ if self.is_conflict() => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Converts to an [`AutumnError`] with [`status`](Self::status).
    ///
    /// The `?` operator also converts, but always gives status 500.
    /// Autumn shows server error details only in development.
    #[must_use]
    pub fn into_autumn(self) -> AutumnError {
        let status = self.status();
        AutumnError::internal_server_error(self).with_status(status)
    }
}

/// Adds [`or_http`](DuckDbResultExt::or_http) to `Result<T, DuckDbError>`.
pub trait DuckDbResultExt<T> {
    /// Converts the error with [`DuckDbError::into_autumn`].
    ///
    /// # Errors
    ///
    /// Returns the converted error.
    fn or_http(self) -> Result<T, AutumnError>;
}

impl<T> DuckDbResultExt<T> for Result<T, DuckDbError> {
    fn or_http(self) -> Result<T, AutumnError> {
        self.map_err(DuckDbError::into_autumn)
    }
}

#[cfg(test)]
mod tests;
