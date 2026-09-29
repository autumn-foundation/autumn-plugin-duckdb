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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use duckdb::{Connection, InterruptHandle, Statement, params_from_iter};
use serde::de::DeserializeOwned;
use tokio::sync::OnceCell;
use tokio::time::Instant;

use crate::config::{AccessMode, DuckDbConfig};
use crate::error::DuckDbError;
use crate::metrics::{Metrics, Outcome};
use crate::param::Param;
use crate::pool::{self, Pool, Setup};
use crate::statement;
use crate::value::{Row, Value};

/// The pause between repeated interrupts of one call.
const INTERRUPT_EVERY: Duration = Duration::from_millis(10);

/// The largest call timeout: one day. A larger deadline can overflow.
const MAX_TIMEOUT: Duration = Duration::from_secs(86_400);

/// The longest wait at shutdown for open calls to stop before the checkpoint.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

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
    shutting_down: AtomicBool,
    shutdown_done: OnceCell<()>,
    open: Mutex<HashMap<u64, Arc<Ticket>>>,
    next_id: AtomicU64,
}

/// Why the plugin cancelled a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Timeout,
    Dropped,
    Shutdown,
}

/// The interrupt state of one call.
#[derive(Default)]
struct Ticket {
    state: Mutex<TicketState>,
}

#[derive(Default)]
struct TicketState {
    done: bool,
    reason: Option<Reason>,
    handle: Option<Arc<InterruptHandle>>,
}

impl Ticket {
    fn state(&self) -> std::sync::MutexGuard<'_, TicketState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks the call as cancelled. Interrupts it every 10 ms until it ends.
    ///
    /// DuckDB ignores an interrupt before a query starts. The repeat uses an OS thread,
    /// because a runtime shutdown drops async tasks.
    fn cancel(self: &Arc<Self>, reason: Reason) {
        {
            let mut state = self.state();
            if state.done || state.reason.is_some() {
                return;
            }
            state.reason = Some(reason);
        }
        if !self.interrupt() {
            return;
        }
        let ticket = Arc::clone(self);
        let repeat = std::thread::Builder::new()
            .name("duckdb-interrupt".to_owned())
            .spawn(move || {
                while ticket.interrupt() {
                    std::thread::sleep(INTERRUPT_EVERY);
                }
            });
        if repeat.is_err() {
            tracing::warn!("the DuckDB plugin cannot start a thread to repeat an interrupt");
        }
    }

    /// Keeps the interrupt handle of the connection. Returns the cancel reason, if any.
    fn attach(&self, handle: Arc<InterruptHandle>) -> Option<Reason> {
        let mut state = self.state();
        state.handle = Some(handle);
        state.reason
    }

    /// Interrupts the query while the call runs. Returns `false` after the call ends.
    ///
    /// The lock makes sure that no interrupt comes after [`finish`](Self::finish).
    fn interrupt(&self) -> bool {
        let state = self.state();
        if !state.done
            && let Some(handle) = &state.handle
        {
            handle.interrupt();
        }
        !state.done
    }

    /// Marks the end of the call. Returns the cancel reason.
    fn finish(&self) -> Option<Reason> {
        let mut state = self.state();
        state.done = true;
        state.handle = None;
        state.reason
    }
}

/// Tells the blocking work that the plugin cancelled the call.
struct Stop(Arc<Ticket>);

impl Stop {
    /// Returns `true` after a timeout, a drop or a shutdown.
    fn requested(&self) -> bool {
        self.0.state().reason.is_some()
    }
}

/// Ends a ticket when the blocking work ends, also after a panic.
struct Finish(Arc<Ticket>);

impl Drop for Finish {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// Counts one call. Cancels the call if the caller drops the future.
struct Guard<'a> {
    inner: &'a Inner,
    counted: bool,
    ticket: Option<(u64, Arc<Ticket>)>,
    outcome: Option<Outcome>,
}

impl<'a> Guard<'a> {
    fn new(inner: &'a Inner, counted: bool) -> Self {
        if counted {
            inner.metrics.started();
        }
        Self {
            inner,
            counted,
            ticket: None,
            outcome: None,
        }
    }

    fn track(&mut self, ticket: Arc<Ticket>) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, Arc::clone(&ticket));
        self.ticket = Some((id, ticket));
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if let Some((id, ticket)) = self.ticket.take() {
            self.inner
                .open
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            if self.outcome.is_none() {
                ticket.cancel(Reason::Dropped);
            }
        }
        if self.counted {
            self.inner
                .metrics
                .ended(self.outcome.unwrap_or(Outcome::Cancelled));
        }
    }
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
        config.validate()?;
        let (root, config) = tokio::task::spawn_blocking(move || {
            pool::open(&config, &setups).map(|root| (root, config))
        })
        .await
        .map_err(|_| DuckDbError::TaskFailed)??;
        Ok(Self {
            inner: Arc::new(Inner {
                pool: Pool::new(root, config.max_connections),
                config,
                metrics,
                shutting_down: AtomicBool::new(false),
                shutdown_done: OnceCell::new(),
                open: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(0),
            }),
        })
    }

    /// Starts a query. Bind values with [`DuckDbQuery::bind`].
    pub fn query(&self, sql: impl Into<String>) -> DuckDbQuery {
        DuckDbQuery {
            db: self.clone(),
            sql: sql.into(),
            params: Vec::new(),
            timeout: None,
            max_rows: None,
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
        let timeout = self.inner.config.timeout();
        self.call(true, timeout, move |conn, _| {
            work(conn).map_err(DuckDbError::from)
        })
        .await
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &DuckDbConfig {
        &self.inner.config
    }

    /// Runs `SELECT 1` on the root connection. The metrics do not count it.
    ///
    /// The ping does not wait for the pool. A busy pool is not a failed database.
    pub(crate) async fn ping(&self) -> Result<(), DuckDbError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(DuckDbError::ShuttingDown);
        }
        let pool = Arc::clone(&self.inner.pool);
        tokio::task::spawn_blocking(move || pool.ping())
            .await
            .map_err(|_| DuckDbError::TaskFailed)?
    }

    /// Refuses new calls, interrupts open calls and runs `CHECKPOINT` on a writable file.
    ///
    /// A second call waits for the first one to end.
    pub(crate) async fn shutdown(&self) {
        self.inner
            .shutdown_done
            .get_or_init(|| async {
                self.inner.shutting_down.store(true, Ordering::Release);
                let open: Vec<Arc<Ticket>> = self
                    .inner
                    .open
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .values()
                    .cloned()
                    .collect();
                for ticket in open {
                    ticket.cancel(Reason::Shutdown);
                }
                self.inner.pool.close();
                let start = Instant::now();
                while !self.inner.pool.is_quiet() && start.elapsed() < SHUTDOWN_WAIT {
                    tokio::time::sleep(INTERRUPT_EVERY).await;
                }
                if self.needs_checkpoint() {
                    let pool = Arc::clone(&self.inner.pool);
                    match tokio::task::spawn_blocking(move || pool.checkpoint()).await {
                        Ok(Ok(())) => tracing::info!("the DuckDB checkpoint is complete"),
                        Ok(Err(err)) => {
                            tracing::warn!(class = err.class(), "the DuckDB checkpoint failed");
                        }
                        Err(_) => tracing::warn!("the DuckDB checkpoint task stopped"),
                    }
                }
            })
            .await;
    }

    fn needs_checkpoint(&self) -> bool {
        let config = &self.inner.config;
        config.checkpoint_on_shutdown
            && !config.is_in_memory()
            && config.access_mode != AccessMode::ReadOnly
    }

    /// Runs `work` with a connection, a deadline and an interrupt ticket.
    async fn call<T, F>(&self, counted: bool, timeout: Duration, work: F) -> Result<T, DuckDbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &Stop) -> Result<T, DuckDbError> + Send + 'static,
    {
        let inner = &*self.inner;
        if inner.shutting_down.load(Ordering::Acquire) {
            return Err(DuckDbError::ShuttingDown);
        }
        let mut guard = Guard::new(inner, counted);
        let result = self.run(&mut guard, timeout, work).await;
        guard.outcome = Some(outcome(&result));
        result
    }

    async fn run<T, F>(
        &self,
        guard: &mut Guard<'_>,
        timeout: Duration,
        work: F,
    ) -> Result<T, DuckDbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &Stop) -> Result<T, DuckDbError> + Send + 'static,
    {
        let inner = &*self.inner;
        let deadline = Instant::now() + timeout;
        let permit = tokio::time::timeout_at(deadline, inner.pool.acquire())
            .await
            .map_err(|_| DuckDbError::Timeout { timeout })??;
        let ticket = Arc::new(Ticket::default());
        guard.track(Arc::clone(&ticket));
        // A shutdown can start after the first check and before `track`.
        if inner.shutting_down.load(Ordering::Acquire) {
            ticket.cancel(Reason::Shutdown);
        }
        // `Finish` ends the ticket also if the closure never runs.
        let finish = Finish(Arc::clone(&ticket));
        let task = tokio::task::spawn_blocking(move || {
            let lease = match permit.connect() {
                Ok(lease) => lease,
                Err(err) => return (Err(err), finish.0.finish()),
            };
            // `finish` drops before `lease`, also in a panic. No interrupt reaches a closed connection.
            let finish = finish;
            let result = match finish.0.attach(lease.connection().interrupt_handle()) {
                Some(_) => Err(DuckDbError::Cancelled),
                None => work(lease.connection(), &Stop(Arc::clone(&finish.0))),
            };
            let reason = finish.0.finish();
            drop(finish);
            drop(lease);
            (result, reason)
        });
        match tokio::time::timeout_at(deadline, task).await {
            Ok(Ok((Err(_), Some(Reason::Shutdown)))) => Err(DuckDbError::ShuttingDown),
            Ok(Ok((Err(err), _))) if err.class() == Some("INTERRUPT") => {
                Err(DuckDbError::Cancelled)
            }
            Ok(Ok((result, _))) => result,
            Ok(Err(_)) => Err(DuckDbError::TaskFailed),
            Err(_) => {
                ticket.cancel(Reason::Timeout);
                Err(DuckDbError::Timeout { timeout })
            }
        }
    }
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
    timeout: Option<Duration>,
    max_rows: Option<usize>,
}

impl DuckDbQuery {
    /// Sets the timeout of this query. It replaces `timeout_ms`. The largest timeout is one day.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout.min(MAX_TIMEOUT));
        self
    }

    /// Sets the row limit of this query. It replaces `max_rows`.
    pub const fn max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = Some(max_rows);
        self
    }

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
        check_statements(&self.sql)?;
        let timeout = self.call_timeout();
        let Self {
            db, sql, params, ..
        } = self;
        db.call(true, timeout, move |conn, _| {
            let mut stmt = prepare(conn, &sql, params.len())?;
            Ok(stmt.execute(params_from_iter(params.iter()))?)
        })
        .await
    }

    /// Runs the query and gives all rows.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails or a limit applies.
    pub async fn fetch(self) -> Result<Vec<Row>, DuckDbError> {
        self.rows(usize::MAX).await
    }

    /// Runs the query and decodes all rows into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails, a limit applies or a row does not decode.
    pub async fn fetch_as<T: DeserializeOwned>(self) -> Result<Vec<T>, DuckDbError> {
        let rows = self.fetch().await?;
        Ok(rows.iter().map(Row::decode).collect::<Result<_, _>>()?)
    }

    /// Runs the query and gives the first row, if any. The plugin reads no more rows.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails or a limit applies.
    pub async fn fetch_optional(self) -> Result<Option<Row>, DuckDbError> {
        Ok(self.rows(1).await?.into_iter().next())
    }

    /// Runs the query and decodes the first row into `T`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError`] if the query fails, a limit applies or the row does not decode.
    pub async fn fetch_optional_as<T: DeserializeOwned>(self) -> Result<Option<T>, DuckDbError> {
        match self.fetch_optional().await? {
            Some(row) => Ok(Some(row.decode()?)),
            None => Ok(None),
        }
    }

    /// Runs the query and decodes the first row into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`DuckDbError::NotFound`] if there is no row, or another [`DuckDbError`].
    pub async fn fetch_one_as<T: DeserializeOwned>(self) -> Result<T, DuckDbError> {
        self.fetch_optional_as().await?.ok_or(DuckDbError::NotFound)
    }

    /// The timeout of this query.
    fn call_timeout(&self) -> Duration {
        self.timeout
            .unwrap_or_else(|| self.db.inner.config.timeout())
    }

    /// Reads up to `take` rows, in the row and byte limits.
    async fn rows(self, take: usize) -> Result<Vec<Row>, DuckDbError> {
        check_statements(&self.sql)?;
        let timeout = self.call_timeout();
        let max_rows = self.max_rows.unwrap_or(self.db.inner.config.max_rows);
        let Self {
            db, sql, params, ..
        } = self;
        let max_bytes = db.inner.config.max_result_bytes;
        let limits = Limits {
            take,
            max_rows,
            max_bytes,
        };
        let rows = db
            .call(true, timeout, move |conn, stop| {
                read_rows(conn, &sql, &params, limits, &|| stop.requested())
            })
            .await?;
        db.inner.metrics.rows(rows.len());
        Ok(rows)
    }
}

/// The debug text shows no SQL text and no values.
#[allow(
    clippy::missing_fields_in_debug,
    reason = "SQL text and values can hold personal data"
)]
impl std::fmt::Debug for DuckDbQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbQuery")
            .field("sql_bytes", &self.sql.len())
            .field("params", &self.params.len())
            .finish()
    }
}

/// The row limits of one fetch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// The rows to read. The fetch stops after them.
    pub(crate) take: usize,
    /// More rows give an error.
    pub(crate) max_rows: usize,
    /// More bytes give an error.
    pub(crate) max_bytes: usize,
}

/// Runs a query and reads its rows in the limits.
pub(crate) fn read_rows(
    conn: &Connection,
    sql: &str,
    params: &[Param],
    limits: Limits,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<Row>, DuckDbError> {
    let mut stmt = prepare(conn, sql, params.len())?;
    let mut rows = stmt.query(params_from_iter(params.iter()))?;
    let columns: Arc<[String]> = rows
        .as_ref()
        .map(Statement::column_names)
        .unwrap_or_default()
        .into();
    let mut out = Vec::new();
    let mut bytes = 0_usize;
    while out.len() < limits.take {
        if stop() {
            return Err(DuckDbError::Cancelled);
        }
        let Some(row) = rows.next()? else {
            break;
        };
        if out.len() == limits.max_rows {
            return Err(DuckDbError::TooManyRows {
                limit: limits.max_rows,
            });
        }
        let values = (0..columns.len())
            .map(|index| row.get::<_, duckdb::types::Value>(index).map(Value::from))
            .collect::<duckdb::Result<Vec<_>>>()?;
        let row = Row::new(Arc::clone(&columns), values);
        bytes = bytes.saturating_add(row.size());
        if bytes > limits.max_bytes {
            return Err(DuckDbError::ResultTooLarge {
                limit_bytes: limits.max_bytes,
            });
        }
        out.push(row);
    }
    Ok(out)
}

/// Refuses SQL text with more than one statement.
fn check_statements(sql: &str) -> Result<(), DuckDbError> {
    match statement::count(sql) {
        Some(statements @ 2..) => Err(DuckDbError::MultipleStatements { statements }),
        Some(_) => Ok(()),
        None => Err(DuckDbError::OpenLiteral),
    }
}

/// Prepares `sql` and checks the parameter count.
fn prepare<'c>(
    conn: &'c Connection,
    sql: &str,
    parameters: usize,
) -> Result<Statement<'c>, DuckDbError> {
    let stmt = conn.prepare(sql)?;
    let placeholders = stmt.parameter_count();
    if placeholders != parameters {
        return Err(DuckDbError::ParameterCount {
            placeholders,
            parameters,
        });
    }
    Ok(stmt)
}

/// The metrics outcome of a result.
pub(crate) const fn outcome<T>(result: &Result<T, DuckDbError>) -> Outcome {
    match result {
        Ok(_) => Outcome::Succeeded,
        Err(DuckDbError::Timeout { .. }) => Outcome::TimedOut,
        Err(DuckDbError::ShuttingDown | DuckDbError::Cancelled) => Outcome::Cancelled,
        Err(_) => Outcome::Failed,
    }
}

#[cfg(test)]
mod tests;
