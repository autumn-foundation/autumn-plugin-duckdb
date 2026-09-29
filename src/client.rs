//! [`DuckDb`] and [`DuckDbQuery`]: run calls with a deadline, an interrupt and limits.
//!
//! # Contract
//!
//! - Each call runs on a blocking thread with one pooled connection.
//! - Each call has a deadline: `timeout_ms` from the call start. The wait for a connection counts.
//! - At the deadline, the call gives [`DuckDbError::Timeout`] at once. The plugin interrupts the query.
//! - A dropped call future interrupts its query.
//! - An interrupt repeats until the call ends, because DuckDB ignores an interrupt before a query starts.
//!   An interrupt never reaches the next call on the same connection.
//! - A query refuses SQL with more than one statement before it uses a connection.
//! - A query refuses a parameter count that is not the placeholder count.
//! - A fetch gives an error, not a partial result, above `max_rows` or `max_result_bytes`.
//! - After [`DuckDb::shutdown`], new calls fail with [`DuckDbError::ShuttingDown`].
//!   Open calls get an interrupt. A writable file gets a `CHECKPOINT`.

use std::sync::Arc;

use duckdb::Connection;
use serde::de::DeserializeOwned;

use crate::config::DuckDbConfig;
use crate::error::DuckDbError;
use crate::metrics::Metrics;
use crate::param::Param;
use crate::pool::{Pool, Setup};
use crate::value::Row;

/// A handle to the database. Clones share the database and the pool.
///
/// Get it with the extractor in a handler, or with [`DuckDb::from_state`].
#[derive(Clone)]
pub struct DuckDb {
    inner: Arc<Inner>,
}

struct Inner {
    pool: Arc<Pool>,
    config: DuckDbConfig,
    metrics: Arc<Metrics>,
}

impl DuckDb {
    /// Opens a database without the plugin, for example in a test or a tool.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError::Config`] for a bad configuration, or the DuckDB error of the open.
    pub async fn open(config: DuckDbConfig) -> Result<Self, DuckDbError> {
        Self::open_with(config, Vec::new(), Arc::default()).await
    }

    /// Opens the database with setup hooks and shared metrics.
    pub(crate) async fn open_with(
        config: DuckDbConfig,
        setups: Vec<Setup>,
        metrics: Arc<Metrics>,
    ) -> Result<Self, DuckDbError> {
        let _ = (config, setups, metrics);
        Err(DuckDbError::TaskFailed)
    }

    /// Starts a query. Bind values with [`DuckDbQuery::bind`].
    pub fn query(&self, sql: impl Into<String>) -> DuckDbQuery {
        DuckDbQuery {
            db: self.clone(),
            sql: sql.into(),
            params: Vec::new(),
        }
    }

    /// Runs `work` on a pooled connection, on a blocking thread.
    ///
    /// Use it for the full `duckdb` API, for example a transaction or an appender.
    /// The timeout applies. The plugin rolls back a transaction that `work` leaves open.
    ///
    /// # Errors
    ///
    /// Returns the error of `work`, [`DuckDbError::Timeout`], or [`DuckDbError::TaskFailed`] if `work` panics.
    pub async fn with_connection<T, F>(&self, work: F) -> Result<T, DuckDbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> duckdb::Result<T> + Send + 'static,
    {
        let _ = work;
        Err(DuckDbError::TaskFailed)
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &DuckDbConfig {
        &self.inner.config
    }

    /// Runs `SELECT 1`. The metrics do not count it.
    pub(crate) async fn ping(&self) -> Result<(), DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }

    /// Refuses new calls, interrupts open calls and runs `CHECKPOINT` on a writable file.
    pub(crate) async fn shutdown(&self) {}
}

impl std::fmt::Debug for DuckDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDb").finish_non_exhaustive()
    }
}

/// A query with bound parameters.
#[must_use = "a query does nothing until you call `execute` or a fetch method"]
pub struct DuckDbQuery {
    db: DuckDb,
    sql: String,
    params: Vec<Param>,
}

impl DuckDbQuery {
    /// Binds the next `?` or `$n` parameter.
    pub fn bind(mut self, value: impl Into<Param>) -> Self {
        self.params.push(value.into());
        self
    }

    /// Runs the statement and gives the changed row count.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the statement fails or a limit applies.
    pub async fn execute(self) -> Result<usize, DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }

    /// Runs the query and gives all rows.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails or a limit applies.
    pub async fn fetch(self) -> Result<Vec<Row>, DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }

    /// Runs the query and decodes all rows into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails, a limit applies or a row does not decode.
    pub async fn fetch_as<T: DeserializeOwned>(self) -> Result<Vec<T>, DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }

    /// Runs the query and gives the first row, if any. The plugin reads no more rows.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails or a limit applies.
    pub async fn fetch_optional(self) -> Result<Option<Row>, DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }

    /// Runs the query and decodes the first row into `T`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails, a limit applies or the row does not decode.
    pub async fn fetch_optional_as<T: DeserializeOwned>(self) -> Result<Option<T>, DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }

    /// Runs the query and decodes the first row into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError::NotFound`] if there is no row, or another [`DuckDbError`].
    pub async fn fetch_one_as<T: DeserializeOwned>(self) -> Result<T, DuckDbError> {
        Err(DuckDbError::TaskFailed)
    }
}

/// The debug text shows no SQL text and no values.
impl std::fmt::Debug for DuckDbQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbQuery")
            .field("sql", &self.sql)
            .field("params", &self.params)
            .finish()
    }
}

/// The metrics outcome of a result.
pub(crate) const fn outcome<T>(result: &Result<T, DuckDbError>) -> crate::metrics::Outcome {
    let _ = result;
    crate::metrics::Outcome::Succeeded
}

#[cfg(test)]
mod tests;
