//! The `[duckdb]` section of `autumn.toml`.
//!
//! # Contract
//!
//! Each layer overrides the layers before it:
//!
//! 1. The defaults.
//! 2. `[duckdb]` in `autumn.toml`.
//! 3. `[profile.<name>.duckdb]` in `autumn.toml`.
//! 4. `[duckdb]` in `autumn-<name>.toml`.
//! 5. `AUTUMN_DUCKDB__<KEY>` variables. `AUTUMN_DUCKDB__MAX_ROWS` sets `max_rows`.
//!    A list variable has comma-separated items. `settings` has no variables.
//!
//! The result must pass [`DuckDbConfig::validate`]. Unknown keys are errors.
//!
//! ```toml
//! [duckdb]
//! path = "data/app.duckdb"
//! access_mode = "read_only"
//! threads = 4
//! memory_limit = "2GB"
//! max_connections = 8
//! timeout_ms = 30000
//! max_rows = 10000
//! max_result_bytes = 67108864
//! allowed_directories = ["data/"]
//!
//! [duckdb.settings]
//! default_order = "desc"
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use autumn_web::config::Env;
use serde::{Deserialize, Serialize};

/// The default section name.
pub const DEFAULT_SECTION: &str = "duckdb";

/// The path of an in-memory database.
pub const IN_MEMORY: &str = ":memory:";

/// A configuration that is not valid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub(crate) String);

/// How the plugin opens the database file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AccessMode {
    /// DuckDB selects the mode. It is read-write for a file.
    #[default]
    Automatic,
    /// Reads only. Many processes can open the file.
    ReadOnly,
    /// Reads and writes. One process can open the file.
    ReadWrite,
}

/// The plugin settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
#[non_exhaustive]
pub struct DuckDbConfig {
    /// The database file. `:memory:` gives an in-memory database.
    pub path: String,
    /// How the plugin opens the file.
    pub access_mode: AccessMode,
    /// The DuckDB worker threads. `None` uses the DuckDB default.
    pub threads: Option<u32>,
    /// The DuckDB memory limit, for example `"2GB"`. `None` uses the DuckDB default.
    pub memory_limit: Option<String>,
    /// The most connections that can run calls at the same time.
    pub max_connections: usize,
    /// The call timeout in milliseconds. It includes the wait for a connection.
    pub timeout_ms: u64,
    /// The most rows that one query can return.
    pub max_rows: usize,
    /// The most bytes of values that one query can return.
    pub max_result_bytes: usize,
    /// If `true`, SQL can read and write files and URLs.
    pub enable_external_access: bool,
    /// Directories that SQL can use when external access is off.
    pub allowed_directories: Vec<String>,
    /// If `true`, DuckDB downloads a known extension when SQL needs it.
    pub autoinstall_extensions: bool,
    /// If `true`, DuckDB loads an installed extension when SQL needs it.
    pub autoload_extensions: bool,
    /// If `true`, SQL cannot change the DuckDB configuration after startup.
    pub lock_configuration: bool,
    /// If `true`, the plugin adds a readiness check.
    pub health_check: bool,
    /// If `true`, the plugin runs `CHECKPOINT` at shutdown on a writable file.
    pub checkpoint_on_shutdown: bool,
    /// Other DuckDB options, for example `default_order = "desc"`.
    ///
    /// The keys that the plugin sets are not allowed here.
    pub settings: BTreeMap<String, String>,
}

impl Default for DuckDbConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            access_mode: AccessMode::Automatic,
            threads: None,
            memory_limit: None,
            max_connections: 0,
            timeout_ms: 0,
            max_rows: 0,
            max_result_bytes: 0,
            enable_external_access: true,
            allowed_directories: Vec::new(),
            autoinstall_extensions: true,
            autoload_extensions: true,
            lock_configuration: false,
            health_check: false,
            checkpoint_on_shutdown: false,
            settings: BTreeMap::new(),
        }
    }
}

/// The type of a configuration leaf, for environment values.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Integer,
    Bool,
    List,
}

/// Each leaf key and its type.
const LEAVES: &[(&str, Kind)] = &[];

impl DuckDbConfig {
    /// Reads `[section]` from the app files and the environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a file is not valid TOML or a value is not valid.
    pub fn resolve(section: &str) -> Result<Self, ConfigError> {
        Self::resolve_with_env(section, &autumn_web::config::OsEnv)
    }

    /// Reads `[section]` with `env` as the environment.
    ///
    /// # Errors
    ///
    /// See [`resolve`](Self::resolve).
    pub fn resolve_with_env(section: &str, env: &dyn Env) -> Result<Self, ConfigError> {
        let _ = (section, env);
        Err(ConfigError(String::new()))
    }

    /// Checks each value.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] that names the first key that is not valid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_section(DEFAULT_SECTION)
    }

    /// Checks each value. The errors name keys in `section`.
    pub(crate) fn validate_section(&self, section: &str) -> Result<(), ConfigError> {
        let _ = section;
        Ok(())
    }

    /// The call timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    /// Returns `true` for an in-memory database.
    #[must_use]
    pub fn is_in_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
