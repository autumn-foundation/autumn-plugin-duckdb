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
//! - A timeout and a write conflict are retryable.

use std::time::Duration;

use autumn_web::AutumnError;
use http::StatusCode;

use crate::config::ConfigError;
use crate::decode::DecodeError;

/// An error from the plugin.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
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
    /// The number of parameters is not the number of placeholders.
    #[error("the SQL has {placeholders} placeholders, but the query has {parameters} parameters")]
    #[non_exhaustive]
    ParameterCount {
        /// The placeholders in the SQL.
        placeholders: usize,
        /// The bound parameters.
        parameters: usize,
    },
    /// The call did not complete in time. The plugin interrupted it.
    #[error("the call did not complete in {timeout:?}")]
    #[non_exhaustive]
    Timeout {
        /// The timeout.
        timeout: Duration,
    },
    /// Someone interrupted the call.
    #[error("the call was cancelled")]
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

/// Gives the class of a DuckDB message.
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
        matches!(self, Self::Timeout { .. }) || self.is_conflict()
    }

    /// The HTTP status for this error.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
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
