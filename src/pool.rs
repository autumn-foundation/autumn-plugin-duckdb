//! The connection pool and the open steps.
//!
//! # Contract
//!
//! [`open`] opens the database in these steps:
//!
//! 1. Open with the access mode, the threads, the memory limit, the extension flags and `settings`.
//! 2. Set `allowed_directories`.
//! 3. Run the setup hooks, in order. They have full file access.
//! 4. Disable external access, unless `enable_external_access` is `true`.
//! 5. Lock the configuration, unless `lock_configuration` is `false`.
//!
//! [`Pool`] gives each call one connection:
//!
//! - A permit limits the connections in use to `max_connections`. The permit stays with the lease.
//! - A lease takes an idle connection, or else makes a new one from the root connection.
//! - [`Lease::release`] sends `ROLLBACK`, then keeps the connection for the next call.
//!   An unexpected error drops the connection. A dropped lease drops its connection.
//! - After [`Pool::close`], `acquire` fails and the pool keeps no connections.
//! - The ping and the checkpoint use the root connection. They do not wait for a permit.
//!
//! Each function here that calls DuckDB blocks. Call it on a blocking thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use duckdb::Connection;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::{AccessMode, DuckDbConfig};
use crate::error::DuckDbError;

/// Rust code that runs on the database at startup, before the plugin applies the limits.
pub type Setup = Arc<dyn Fn(&Connection) -> duckdb::Result<()> + Send + Sync>;

/// Opens the database. See the module contract.
pub(crate) fn open(config: &DuckDbConfig, setups: &[Setup]) -> Result<Connection, DuckDbError> {
    let conn = Connection::open_with_flags(&config.path, flags(config)?)?;
    if !config.allowed_directories.is_empty() {
        let list = config
            .allowed_directories
            .iter()
            .map(|dir| quote(dir))
            .collect::<Vec<_>>()
            .join(", ");
        conn.execute_batch(&format!("SET allowed_directories = [{list}]"))?;
    }
    for setup in setups {
        setup(&conn)?;
    }
    if !config.enable_external_access {
        conn.execute_batch("SET enable_external_access = false")?;
    }
    if config.lock_configuration {
        conn.execute_batch("SET lock_configuration = true")?;
    }
    Ok(conn)
}

/// Builds the DuckDB open flags.
fn flags(config: &DuckDbConfig) -> duckdb::Result<duckdb::Config> {
    let mode = match config.access_mode {
        AccessMode::Automatic => duckdb::AccessMode::Automatic,
        AccessMode::ReadOnly => duckdb::AccessMode::ReadOnly,
        AccessMode::ReadWrite => duckdb::AccessMode::ReadWrite,
    };
    let mut flags = duckdb::Config::default();
    for (key, value) in &config.settings {
        flags = flags.with(key, value)?;
    }
    flags = flags
        .access_mode(mode)?
        .with(
            "autoinstall_known_extensions",
            bool_text(config.autoinstall_extensions),
        )?
        .with(
            "autoload_known_extensions",
            bool_text(config.autoload_extensions),
        )?;
    if let Some(threads) = config.threads {
        flags = flags.threads(i64::from(threads))?;
    }
    if let Some(limit) = &config.memory_limit {
        flags = flags.max_memory(limit)?;
    }
    Ok(flags)
}

const fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Gives `text` as a SQL string literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Returns `true` if a `ROLLBACK` error only says that no transaction was open.
fn no_transaction(err: &duckdb::Error) -> bool {
    matches!(err, duckdb::Error::DuckDBFailure(_, Some(message))
        if message.contains("no transaction is active"))
}

/// The connection pool.
pub(crate) struct Pool {
    root: Mutex<Connection>,
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
    size: usize,
    closed: AtomicBool,
}

/// One connection and its permit.
pub(crate) struct Lease {
    conn: Connection,
    pool: Arc<Pool>,
    _permit: OwnedSemaphorePermit,
}

impl Pool {
    /// Makes a pool of `size` connections from `root`.
    pub(crate) fn new(root: Connection, size: usize) -> Arc<Self> {
        Arc::new(Self {
            root: Mutex::new(root),
            idle: Mutex::new(Vec::new()),
            permits: Arc::new(Semaphore::new(size)),
            size,
            closed: AtomicBool::new(false),
        })
    }

    /// Waits for a permit and gives a connection.
    ///
    /// An idle connection is ready at once. A new connection is a fast in-process call.
    pub(crate) async fn acquire(self: &Arc<Self>) -> Result<Lease, DuckDbError> {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| DuckDbError::ShuttingDown)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(DuckDbError::ShuttingDown);
        }
        let idle = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let conn = match idle {
            Some(conn) => conn,
            None => self
                .root
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_clone()?,
        };
        Ok(Lease {
            conn,
            pool: Arc::clone(self),
            _permit: permit,
        })
    }

    /// The idle connections.
    #[cfg(test)]
    pub(crate) fn idle(&self) -> usize {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Returns `true` if no lease is out.
    pub(crate) fn is_quiet(&self) -> bool {
        self.permits.available_permits() == self.size
    }

    /// Refuses new leases and drops the idle connections.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.permits.close();
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Runs `SELECT 1` on the root connection.
    pub(crate) fn ping(&self) -> Result<(), DuckDbError> {
        self.root
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .execute_batch("SELECT 1")?;
        Ok(())
    }

    /// Runs `CHECKPOINT` on the root connection.
    pub(crate) fn checkpoint(&self) -> Result<(), DuckDbError> {
        self.root
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .execute_batch("CHECKPOINT")?;
        Ok(())
    }
}

impl Lease {
    /// The connection.
    pub(crate) const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Gives the connection back to the pool.
    pub(crate) fn release(self) {
        let Self {
            conn,
            pool,
            _permit,
        } = self;
        match conn.execute_batch("ROLLBACK") {
            Ok(()) => {
                tracing::warn!("a DuckDB call left a transaction open: the plugin rolled it back");
            }
            Err(err) if no_transaction(&err) => {}
            Err(_) => return,
        }
        if !pool.closed.load(Ordering::Acquire) {
            pool.idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(conn);
        }
    }
}

#[cfg(test)]
mod tests;
