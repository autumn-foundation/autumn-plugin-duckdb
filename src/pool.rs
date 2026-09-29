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
//!
//! Each function here that calls DuckDB blocks. Call it on a blocking thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use duckdb::Connection;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::DuckDbConfig;
use crate::error::DuckDbError;

/// Rust code that runs on the database at startup, before the plugin applies the limits.
pub type Setup = Arc<dyn Fn(&Connection) -> duckdb::Result<()> + Send + Sync>;

/// Opens the database. See the module contract.
pub(crate) fn open(config: &DuckDbConfig, setups: &[Setup]) -> Result<Connection, DuckDbError> {
    let _ = (config, setups);
    Err(DuckDbError::TaskFailed)
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
    pub(crate) async fn acquire(self: &Arc<Self>) -> Result<Lease, DuckDbError> {
        Err(DuckDbError::ShuttingDown)
    }

    /// The idle connections.
    pub(crate) fn idle(&self) -> usize {
        0
    }

    /// Returns `true` if no lease is out.
    pub(crate) fn is_quiet(&self) -> bool {
        false
    }

    /// Refuses new leases and drops the idle connections.
    pub(crate) fn close(&self) {}

    /// Runs `CHECKPOINT` on the root connection.
    pub(crate) fn checkpoint(&self) -> Result<(), DuckDbError> {
        Ok(())
    }
}

impl Lease {
    /// The connection.
    pub(crate) const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Gives the connection back to the pool.
    pub(crate) fn release(self) {}
}

#[cfg(test)]
mod tests;
