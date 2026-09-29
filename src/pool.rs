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
//! [`Pool`] gives each call one new connection:
//!
//! - A permit limits the connections in use to `max_connections`. [`Pool::acquire`] waits for a permit.
//! - [`Permit::connect`] makes a new connection from the root connection. The lease keeps the permit.
//! - A dropped lease closes its connection. DuckDB rolls back an open transaction.
//!   So no session state, for example `USE` or a temp table, reaches the next call.
//! - After [`Pool::close`], `acquire` fails.
//! - The ping and the checkpoint use the root connection. They do not wait for a permit.
//!
//! Each function here that calls DuckDB blocks. Call it on a blocking thread. `acquire` does not call DuckDB.

use std::sync::{Arc, Mutex, PoisonError};

use duckdb::Connection;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::{AccessMode, DuckDbConfig};
use crate::error::DuckDbError;

/// Rust code that runs on the database at startup.
///
/// It runs before the plugin disables external access and locks the configuration.
pub(crate) type Setup = Arc<dyn Fn(&Connection) -> duckdb::Result<()> + Send + Sync>;

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

/// The connection pool.
pub(crate) struct Pool {
    root: Mutex<Connection>,
    permits: Arc<Semaphore>,
    size: usize,
}

/// A right to open one connection.
pub(crate) struct Permit {
    pool: Arc<Pool>,
    permit: OwnedSemaphorePermit,
}

/// One connection and its permit.
pub(crate) struct Lease {
    conn: Connection,
    _permit: OwnedSemaphorePermit,
}

impl Pool {
    /// Makes a pool of `size` connections from `root`.
    pub(crate) fn new(root: Connection, size: usize) -> Arc<Self> {
        Arc::new(Self {
            root: Mutex::new(root),
            permits: Arc::new(Semaphore::new(size)),
            size,
        })
    }

    /// Waits for a permit.
    pub(crate) async fn acquire(self: &Arc<Self>) -> Result<Permit, DuckDbError> {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| DuckDbError::ShuttingDown)?;
        Ok(Permit {
            pool: Arc::clone(self),
            permit,
        })
    }

    /// Returns `true` if no permit is out.
    pub(crate) fn is_quiet(&self) -> bool {
        self.permits.available_permits() == self.size
    }

    /// Refuses new permits.
    pub(crate) fn close(&self) {
        self.permits.close();
    }

    /// Runs `SELECT 1` on the root connection.
    pub(crate) fn ping(&self) -> Result<(), DuckDbError> {
        self.root().execute_batch("SELECT 1")?;
        Ok(())
    }

    /// Runs `CHECKPOINT` on the root connection.
    pub(crate) fn checkpoint(&self) -> Result<(), DuckDbError> {
        self.root().execute_batch("CHECKPOINT")?;
        Ok(())
    }

    fn root(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.root.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Permit {
    /// Makes a new connection.
    pub(crate) fn connect(self) -> Result<Lease, DuckDbError> {
        let conn = self.pool.root().try_clone()?;
        Ok(Lease {
            conn,
            _permit: self.permit,
        })
    }
}

impl Lease {
    /// The connection.
    pub(crate) const fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests;
